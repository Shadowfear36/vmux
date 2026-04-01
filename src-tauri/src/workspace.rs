use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;
use anyhow::Result;
use rusqlite::{Connection, params};

use crate::terminal::TerminalId;

/// A pane is a single terminal or context view in a split layout.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum PaneKind {
    Terminal {
        terminal_id: TerminalId,
        #[serde(default)]
        shell_id: Option<String>,
        #[serde(default)]
        working_dir: Option<String>,
    },
    Context,
    Browser { url: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Pane {
    pub id: String,
    pub kind: PaneKind,
}

/// A tab contains a split layout of panes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Tab {
    pub id: String,
    pub name: String,
    pub panes: Vec<Pane>,
    /// Serialized allotment split sizes (JSON array of percentages)
    pub layout: Option<String>,
    /// Split direction: "horizontal" (side-by-side) or "vertical" (top/bottom)
    #[serde(default = "default_direction")]
    pub direction: String,
}

fn default_direction() -> String {
    "horizontal".to_string()
}

/// A workspace groups tabs together (e.g. one per project).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Workspace {
    pub id: String,
    pub name: String,
    pub tabs: Vec<Tab>,
    pub active_tab_id: Option<String>,
    /// Project directory for this workspace. Agents launch here by default.
    #[serde(default)]
    pub directory: Option<String>,
}

pub struct WorkspaceManager {
    pub workspaces: HashMap<String, Workspace>,
    pub active_workspace_id: Option<String>,
    db: Connection,
}

impl WorkspaceManager {
    pub fn new(db_path: &str) -> Result<Self> {
        let db = Connection::open(db_path)?;
        db.execute_batch("
            CREATE TABLE IF NOT EXISTS workspaces (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                data TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS vmux_meta (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL
            );
        ")?;

        let mut mgr = WorkspaceManager {
            workspaces: HashMap::new(),
            active_workspace_id: None,
            db,
        };

        mgr.load()?;

        // Create default workspace if none exist
        if mgr.workspaces.is_empty() {
            mgr.create_workspace("Default")?;
        }

        // Restore persisted active workspace
        if let Ok(saved_id) = mgr.load_meta("active_workspace_id") {
            if mgr.workspaces.contains_key(&saved_id) {
                mgr.active_workspace_id = Some(saved_id);
            }
        }

        Ok(mgr)
    }

    fn load(&mut self) -> Result<()> {
        let mut stmt = self.db.prepare("SELECT id, data FROM workspaces")?;
        let rows = stmt.query_map([], |row| {
            let id: String = row.get(0)?;
            let data: String = row.get(1)?;
            Ok((id, data))
        })?;

        for row in rows {
            let (id, data) = row?;
            if let Ok(ws) = serde_json::from_str::<Workspace>(&data) {
                if self.active_workspace_id.is_none() {
                    self.active_workspace_id = Some(id.clone());
                }
                self.workspaces.insert(id, ws);
            }
        }
        Ok(())
    }

    pub fn save_workspace(&self, ws: &Workspace) -> Result<()> {
        let data = serde_json::to_string(ws)?;
        self.db.execute(
            "INSERT OR REPLACE INTO workspaces (id, name, data) VALUES (?1, ?2, ?3)",
            params![ws.id, ws.name, data],
        )?;
        Ok(())
    }

    pub fn create_workspace(&mut self, name: &str) -> Result<Workspace> {
        let id = Uuid::new_v4().to_string();
        let ws = Workspace {
            id: id.clone(),
            name: name.to_string(),
            tabs: vec![],
            active_tab_id: None,
            directory: None,
        };
        self.save_workspace(&ws)?;
        self.workspaces.insert(id.clone(), ws.clone());
        if self.active_workspace_id.is_none() {
            self.active_workspace_id = Some(id);
        }
        Ok(ws)
    }

    pub fn add_tab(&mut self, workspace_id: &str, name: &str) -> Result<Tab> {
        let tab = Tab {
            id: Uuid::new_v4().to_string(),
            name: name.to_string(),
            panes: vec![],
            layout: None,
            direction: "horizontal".to_string(),
        };
        if let Some(ws) = self.workspaces.get_mut(workspace_id) {
            ws.tabs.push(tab.clone());
            ws.active_tab_id = Some(tab.id.clone());
            let ws_clone = ws.clone();
            self.save_workspace(&ws_clone)?;
        }
        Ok(tab)
    }

    pub fn add_pane_to_tab(&mut self, workspace_id: &str, tab_id: &str, kind: PaneKind) -> Result<Pane> {
        let pane = Pane {
            id: Uuid::new_v4().to_string(),
            kind,
        };
        if let Some(ws) = self.workspaces.get_mut(workspace_id) {
            if let Some(tab) = ws.tabs.iter_mut().find(|t| t.id == tab_id) {
                tab.panes.push(pane.clone());
            }
            let ws_clone = ws.clone();
            self.save_workspace(&ws_clone)?;
        }
        Ok(pane)
    }

    pub fn update_layout(&mut self, workspace_id: &str, tab_id: &str, layout: String) -> Result<()> {
        if let Some(ws) = self.workspaces.get_mut(workspace_id) {
            if let Some(tab) = ws.tabs.iter_mut().find(|t| t.id == tab_id) {
                tab.layout = Some(layout);
            }
            let ws_clone = ws.clone();
            self.save_workspace(&ws_clone)?;
        }
        Ok(())
    }

    pub fn list(&self) -> Vec<Workspace> {
        self.workspaces.values().cloned().collect()
    }

    #[allow(dead_code)]
    pub fn get_active(&self) -> Option<&Workspace> {
        self.active_workspace_id.as_ref()
            .and_then(|id| self.workspaces.get(id))
    }

    pub fn set_active(&mut self, workspace_id: &str) {
        if self.workspaces.contains_key(workspace_id) {
            self.active_workspace_id = Some(workspace_id.to_string());
            let _ = self.save_meta("active_workspace_id", workspace_id);
        }
    }

    fn save_meta(&self, key: &str, value: &str) -> Result<()> {
        self.db.execute(
            "INSERT OR REPLACE INTO vmux_meta (key, value) VALUES (?1, ?2)",
            params![key, value],
        )?;
        Ok(())
    }

    fn load_meta(&self, key: &str) -> Result<String> {
        let value: String = self.db.query_row(
            "SELECT value FROM vmux_meta WHERE key=?1",
            params![key],
            |row| row.get(0),
        )?;
        Ok(value)
    }

    pub fn reorder_panes(&mut self, workspace_id: &str, tab_id: &str, pane_ids: &[String]) -> Result<()> {
        if let Some(ws) = self.workspaces.get_mut(workspace_id) {
            if let Some(tab) = ws.tabs.iter_mut().find(|t| t.id == tab_id) {
                let mut reordered = Vec::new();
                for id in pane_ids {
                    if let Some(pane) = tab.panes.iter().find(|p| p.id == *id) {
                        reordered.push(pane.clone());
                    }
                }
                tab.panes = reordered;
            }
            let ws_clone = ws.clone();
            self.save_workspace(&ws_clone)?;
        }
        Ok(())
    }

    pub fn set_tab_direction(&mut self, workspace_id: &str, tab_id: &str, direction: &str) -> Result<()> {
        if let Some(ws) = self.workspaces.get_mut(workspace_id) {
            if let Some(tab) = ws.tabs.iter_mut().find(|t| t.id == tab_id) {
                tab.direction = direction.to_string();
            }
            let ws_clone = ws.clone();
            self.save_workspace(&ws_clone)?;
        }
        Ok(())
    }

    pub fn remove_pane(&mut self, workspace_id: &str, tab_id: &str, pane_id: &str) -> Result<()> {
        if let Some(ws) = self.workspaces.get_mut(workspace_id) {
            if let Some(tab) = ws.tabs.iter_mut().find(|t| t.id == tab_id) {
                tab.panes.retain(|p| p.id != pane_id);
            }
            let ws_clone = ws.clone();
            self.save_workspace(&ws_clone)?;
        }
        Ok(())
    }

    pub fn close_tab(&mut self, workspace_id: &str, tab_id: &str) -> Result<()> {
        if let Some(ws) = self.workspaces.get_mut(workspace_id) {
            ws.tabs.retain(|t| t.id != tab_id);
            if ws.active_tab_id.as_deref() == Some(tab_id) {
                ws.active_tab_id = ws.tabs.last().map(|t| t.id.clone());
            }
            let ws_clone = ws.clone();
            self.save_workspace(&ws_clone)?;
        }
        Ok(())
    }

    pub fn set_workspace_directory(&mut self, workspace_id: &str, directory: Option<&str>) -> Result<()> {
        if let Some(ws) = self.workspaces.get_mut(workspace_id) {
            ws.directory = directory.map(|s| s.to_string());
            let ws_clone = ws.clone();
            self.save_workspace(&ws_clone)?;
        }
        Ok(())
    }

    pub fn rename_workspace(&mut self, workspace_id: &str, name: &str) -> Result<()> {
        if let Some(ws) = self.workspaces.get_mut(workspace_id) {
            ws.name = name.to_string();
            let ws_clone = ws.clone();
            self.save_workspace(&ws_clone)?;
        }
        Ok(())
    }

    pub fn delete_workspace(&mut self, workspace_id: &str) -> Result<()> {
        self.workspaces.remove(workspace_id);
        self.db.execute("DELETE FROM workspaces WHERE id=?1", params![workspace_id])?;
        if self.active_workspace_id.as_deref() == Some(workspace_id) {
            self.active_workspace_id = self.workspaces.keys().next().cloned();
        }
        // Ensure at least one workspace exists
        if self.workspaces.is_empty() {
            self.create_workspace("Default")?;
        }
        Ok(())
    }
}

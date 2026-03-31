/// Context store for managing persistent context between agent sessions.
/// Contexts can be attached to workspaces or individual terminal tabs.
use serde::{Deserialize, Serialize};
use rusqlite::{Connection, params};
use uuid::Uuid;
use anyhow::Result;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextEntry {
    pub id: String,
    pub title: String,
    pub content: String,
    pub workspace_id: Option<String>,
    pub tab_id: Option<String>,
    pub tags: Vec<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

pub struct ContextStore {
    db: Connection,
}

impl ContextStore {
    pub fn new(db_path: &str) -> Result<Self> {
        let db = Connection::open(db_path)?;
        db.execute_batch("
            CREATE TABLE IF NOT EXISTS context_entries (
                id TEXT PRIMARY KEY,
                title TEXT NOT NULL,
                content TEXT NOT NULL,
                workspace_id TEXT,
                tab_id TEXT,
                tags TEXT NOT NULL DEFAULT '[]',
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_workspace ON context_entries(workspace_id);
            CREATE INDEX IF NOT EXISTS idx_tab ON context_entries(tab_id);
        ")?;
        Ok(ContextStore { db })
    }

    pub fn create(&self, title: &str, content: &str, workspace_id: Option<&str>, tab_id: Option<&str>, tags: Vec<String>) -> Result<ContextEntry> {
        let now = chrono_now();
        let entry = ContextEntry {
            id: Uuid::new_v4().to_string(),
            title: title.to_string(),
            content: content.to_string(),
            workspace_id: workspace_id.map(|s| s.to_string()),
            tab_id: tab_id.map(|s| s.to_string()),
            tags,
            created_at: now,
            updated_at: now,
        };
        self.db.execute(
            "INSERT INTO context_entries (id, title, content, workspace_id, tab_id, tags, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                entry.id,
                entry.title,
                entry.content,
                entry.workspace_id,
                entry.tab_id,
                serde_json::to_string(&entry.tags).unwrap_or_default(),
                entry.created_at,
                entry.updated_at
            ],
        )?;
        Ok(entry)
    }

    pub fn update(&self, id: &str, title: Option<&str>, content: Option<&str>, tags: Option<Vec<String>>) -> Result<()> {
        let now = chrono_now();
        if let Some(t) = title {
            self.db.execute("UPDATE context_entries SET title=?1, updated_at=?2 WHERE id=?3", params![t, now, id])?;
        }
        if let Some(c) = content {
            self.db.execute("UPDATE context_entries SET content=?1, updated_at=?2 WHERE id=?3", params![c, now, id])?;
        }
        if let Some(t) = tags {
            let tags_json = serde_json::to_string(&t).unwrap_or_default();
            self.db.execute("UPDATE context_entries SET tags=?1, updated_at=?2 WHERE id=?3", params![tags_json, now, id])?;
        }
        Ok(())
    }

    pub fn delete(&self, id: &str) -> Result<()> {
        self.db.execute("DELETE FROM context_entries WHERE id=?1", params![id])?;
        Ok(())
    }

    pub fn list(&self, workspace_id: Option<&str>) -> Result<Vec<ContextEntry>> {
        let rows = if let Some(wid) = workspace_id {
            let mut stmt = self.db.prepare(
                "SELECT id, title, content, workspace_id, tab_id, tags, created_at, updated_at
                 FROM context_entries WHERE workspace_id=?1 ORDER BY updated_at DESC"
            )?;
            let entries = stmt.query_map(params![wid], row_to_entry)?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            entries
        } else {
            let mut stmt = self.db.prepare(
                "SELECT id, title, content, workspace_id, tab_id, tags, created_at, updated_at
                 FROM context_entries ORDER BY updated_at DESC"
            )?;
            let entries = stmt.query_map([], row_to_entry)?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            entries
        };
        Ok(rows)
    }

    #[allow(dead_code)]
    pub fn get(&self, id: &str) -> Result<Option<ContextEntry>> {
        let mut stmt = self.db.prepare(
            "SELECT id, title, content, workspace_id, tab_id, tags, created_at, updated_at
             FROM context_entries WHERE id=?1"
        )?;
        let mut rows = stmt.query_map(params![id], row_to_entry)?;
        Ok(rows.next().transpose()?)
    }
}

fn row_to_entry(row: &rusqlite::Row) -> rusqlite::Result<ContextEntry> {
    let tags_str: String = row.get(5)?;
    let tags: Vec<String> = serde_json::from_str(&tags_str).unwrap_or_default();
    Ok(ContextEntry {
        id: row.get(0)?,
        title: row.get(1)?,
        content: row.get(2)?,
        workspace_id: row.get(3)?,
        tab_id: row.get(4)?,
        tags,
        created_at: row.get(6)?,
        updated_at: row.get(7)?,
    })
}

fn chrono_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

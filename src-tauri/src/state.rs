use std::collections::HashMap;

use crate::terminal::TerminalManager;
use crate::terminal::shell::{detect_shells, ShellProfile};
use crate::terminal::agents::{detect_agents, AgentProfile};
use crate::workspace::WorkspaceManager;
use crate::context_store::ContextStore;
use crate::theme::Theme;
use crate::browser::BrowserManager;
use crate::embeddings::EmbeddingConfig;
use crate::settings::Settings;

pub struct AppState {
    pub terminals: TerminalManager,
    pub workspaces: WorkspaceManager,
    pub context: ContextStore,
    pub theme: Theme,
    /// HWND of the Tauri main window, used as parent for terminal child windows.
    pub main_hwnd: isize,
    /// Detected shells — computed once at startup, not on every create_terminal call.
    pub shells: Vec<ShellProfile>,
    /// Detected AI agent CLIs — computed once at startup.
    pub agents: Vec<AgentProfile>,
    /// Per-pane browser state, keyed by browser pane UUID.
    pub browsers: HashMap<String, BrowserManager>,
    /// Path to the SQLite database (for creating additional connections in async contexts).
    pub context_db_path: String,
    /// Embedding provider configuration.
    pub embedding_config: EmbeddingConfig,
    /// User-configurable app settings (theme, font size, default shell, prefix key).
    pub settings: Settings,
}

impl AppState {
    pub fn new(data_dir: &str, main_hwnd: isize) -> anyhow::Result<Self> {
        let db_path = format!("{}/vmux.db", data_dir);
        let settings = crate::settings::load(&db_path).unwrap_or_default();
        let theme = crate::settings::theme_from_name(&settings.theme_name);
        Ok(AppState {
            terminals: TerminalManager::new(),
            workspaces: WorkspaceManager::new(&db_path)?,
            context: ContextStore::new(&db_path)?,
            theme,
            main_hwnd,
            shells: detect_shells(),
            agents: detect_agents(),
            browsers: HashMap::new(),
            context_db_path: db_path,
            embedding_config: EmbeddingConfig::default(),
            settings,
        })
    }

    /// Find-or-create a workspace pointed at `path` and make it active —
    /// the shared logic behind `vmux <dir>` (see `main.rs`/`ipc.rs`),
    /// whether that lands on a fresh launch or an already-running instance
    /// via the IPC pipe. Reuses an existing workspace with a matching
    /// `directory` instead of creating a duplicate on repeated calls.
    /// Returns the workspace ID.
    pub fn open_path_as_workspace(&mut self, path: &str) -> anyhow::Result<String> {
        if let Some(existing) = self.workspaces.list().into_iter().find(|w| w.directory.as_deref() == Some(path)) {
            self.workspaces.set_active(&existing.id);
            return Ok(existing.id);
        }

        let name = std::path::Path::new(path)
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| path.to_string());
        let ws = self.workspaces.create_workspace(&name)?;
        self.workspaces.set_workspace_directory(&ws.id, Some(path))?;
        self.workspaces.set_active(&ws.id);
        Ok(ws.id)
    }
}

use crate::terminal::TerminalManager;
use crate::terminal::shell::{detect_shells, ShellProfile};
use crate::terminal::agents::{detect_agents, AgentProfile};
use crate::workspace::WorkspaceManager;
use crate::context_store::ContextStore;
use crate::theme::Theme;
use crate::browser::BrowserManager;

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
    pub browser: BrowserManager,
}

impl AppState {
    pub fn new(data_dir: &str, main_hwnd: isize) -> anyhow::Result<Self> {
        let db_path = format!("{}/vmux.db", data_dir);
        Ok(AppState {
            terminals: TerminalManager::new(),
            workspaces: WorkspaceManager::new(&db_path)?,
            context: ContextStore::new(&db_path)?,
            theme: Theme::default(),
            main_hwnd,
            shells: detect_shells(),
            agents: detect_agents(),
            browser: BrowserManager::new(),
        })
    }
}

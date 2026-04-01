use serde::{Serialize, Deserialize};
use tauri::WebviewWindow;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrowserTabInfo {
    pub id: String,
    pub url: String,
    pub title: String,
}

/// Lightweight tab state — no WebviewWindow per tab.
/// Only the active tab gets a live WebviewWindow.
pub struct BrowserTab {
    pub id: String,
    pub url: String,
    pub title: String,
}

pub struct BrowserManager {
    pub tabs: Vec<BrowserTab>,
    pub active_tab_id: Option<String>,
    /// Only the active tab has a live WebviewWindow.
    pub window: Option<WebviewWindow<tauri::Wry>>,
}

impl BrowserManager {
    pub fn new() -> Self {
        BrowserManager { tabs: Vec::new(), active_tab_id: None, window: None }
    }

    pub fn add_tab(&mut self, url: &str) -> String {
        let id = Uuid::new_v4().to_string();
        self.tabs.push(BrowserTab {
            id: id.clone(),
            url: url.to_string(),
            title: String::new(),
        });
        id
    }

    pub fn close_tab(&mut self, tab_id: &str) -> bool {
        let was_active = self.active_tab_id.as_deref() == Some(tab_id);
        self.tabs.retain(|t| t.id != tab_id);
        if was_active {
            self.active_tab_id = self.tabs.last().map(|t| t.id.clone());
        }
        was_active // caller needs to recreate window if active tab changed
    }

    pub fn set_active(&mut self, tab_id: &str) {
        if self.tabs.iter().any(|t| t.id == tab_id) {
            self.active_tab_id = Some(tab_id.to_string());
        }
    }

    pub fn active_tab(&self) -> Option<&BrowserTab> {
        self.active_tab_id.as_ref()
            .and_then(|id| self.tabs.iter().find(|t| t.id == *id))
    }

    pub fn active_url(&self) -> Option<&str> {
        self.active_tab().map(|t| t.url.as_str())
    }

    pub fn update_url(&mut self, tab_id: &str, url: &str) {
        if let Some(tab) = self.tabs.iter_mut().find(|t| t.id == tab_id) {
            tab.url = url.to_string();
        }
    }

    pub fn update_title(&mut self, tab_id: &str, title: &str) {
        if let Some(tab) = self.tabs.iter_mut().find(|t| t.id == tab_id) {
            tab.title = title.to_string();
        }
    }

    pub fn list_tabs(&self) -> Vec<BrowserTabInfo> {
        self.tabs.iter().map(|t| BrowserTabInfo {
            id: t.id.clone(),
            url: t.url.clone(),
            title: t.title.clone(),
        }).collect()
    }

    /// Destroy the current window. Caller must create a new one for the new active tab.
    pub fn take_window(&mut self) -> Option<WebviewWindow<tauri::Wry>> {
        self.window.take()
    }
}

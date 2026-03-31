use serde::{Serialize, Deserialize};
use tauri::WebviewWindow;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrowserTabInfo {
    pub id: String,
    pub url: String,
}

pub struct BrowserTab {
    pub id: String,
    pub url: String,
    pub window: WebviewWindow<tauri::Wry>,
}

pub struct BrowserManager {
    pub tabs: Vec<BrowserTab>,
    pub active_tab_id: Option<String>,
    /// Legacy single-window field for backwards compat during transition
    pub window: Option<WebviewWindow<tauri::Wry>>,
}

impl BrowserManager {
    pub fn new() -> Self {
        BrowserManager {
            tabs: Vec::new(),
            active_tab_id: None,
            window: None,
        }
    }

    pub fn active_window(&self) -> Option<&WebviewWindow<tauri::Wry>> {
        // Prefer new tab system, fall back to legacy single window
        if let Some(active_id) = &self.active_tab_id {
            self.tabs.iter()
                .find(|t| t.id == *active_id)
                .map(|t| &t.window)
        } else {
            self.window.as_ref()
        }
    }

    pub fn add_tab(&mut self, id: String, url: String, window: WebviewWindow<tauri::Wry>) {
        self.tabs.push(BrowserTab { id: id.clone(), url, window });
        self.active_tab_id = Some(id);
    }

    pub fn remove_tab(&mut self, id: &str) -> Option<WebviewWindow<tauri::Wry>> {
        if let Some(idx) = self.tabs.iter().position(|t| t.id == id) {
            let tab = self.tabs.remove(idx);
            if self.active_tab_id.as_deref() == Some(id) {
                self.active_tab_id = self.tabs.last().map(|t| t.id.clone());
            }
            Some(tab.window)
        } else {
            None
        }
    }

    pub fn switch_to(&mut self, id: &str) {
        if self.tabs.iter().any(|t| t.id == id) {
            self.active_tab_id = Some(id.to_string());
        }
    }

    pub fn list_tabs(&self) -> Vec<BrowserTabInfo> {
        self.tabs.iter().map(|t| BrowserTabInfo {
            id: t.id.clone(),
            url: t.url.clone(),
        }).collect()
    }

    pub fn update_url(&mut self, id: &str, url: &str) {
        if let Some(tab) = self.tabs.iter_mut().find(|t| t.id == id) {
            tab.url = url.to_string();
        }
    }
}

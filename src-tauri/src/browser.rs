use serde::{Serialize, Deserialize};
use tauri::WebviewWindow;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrowserTabInfo {
    pub id: String,
    pub url: String,
}

pub struct BrowserManager {
    pub window: Option<WebviewWindow<tauri::Wry>>,
    pub current_url: String,
}

impl BrowserManager {
    pub fn new() -> Self {
        BrowserManager {
            window: None,
            current_url: String::new(),
        }
    }
}

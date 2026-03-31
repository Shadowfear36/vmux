use tauri::WebviewWindow;

pub struct BrowserManager {
    pub window: Option<WebviewWindow<tauri::Wry>>,
}

impl BrowserManager {
    pub fn new() -> Self {
        BrowserManager { window: None }
    }
}

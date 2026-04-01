mod terminal;
mod osc;
mod workspace;
mod git_meta;
mod context_store;
mod state;
mod commands;
mod theme;
mod browser;
mod claude_hooks;

use std::sync::Mutex;
use tauri::Manager;
use state::AppState;

#[cfg(target_os = "windows")]
mod window_tracking;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    env_logger::init();

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_shell::init())
        .setup(|app| {
            let data_dir = app.path().app_data_dir()
                .expect("failed to get app data dir");
            std::fs::create_dir_all(&data_dir)?;

            // Get the main window HWND so we can parent native terminal windows to it
            let main_hwnd = get_main_hwnd(app);

            let state = AppState::new(data_dir.to_str().unwrap(), main_hwnd)
                .expect("failed to init app state");
            app.manage(Mutex::new(state));

            // Subclass the main window to track moves and reposition terminal popups
            #[cfg(target_os = "windows")]
            if main_hwnd != 0 {
                unsafe { window_tracking::subclass_main_window(main_hwnd); }
            }

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::list_shells,
            commands::create_terminal,
            commands::write_terminal,
            commands::set_terminal_bounds,
            commands::focus_terminal,
            commands::show_terminal,
            commands::hide_terminal,
            commands::close_terminal,
            commands::clear_notification,
            commands::list_terminals,
            commands::list_agents,
            commands::create_agent_terminal,
            commands::list_workspaces,
            commands::create_workspace,
            commands::set_active_workspace,
            commands::add_tab,
            commands::close_tab,
            commands::add_pane,
            commands::restore_workspace_terminals,
            commands::update_layout,
            commands::remove_pane,
            commands::rename_workspace,
            commands::delete_workspace,
            commands::reorder_panes,
            commands::set_tab_direction,
            commands::get_git_metadata,
            commands::git_changed_files,
            commands::list_context,
            commands::create_context,
            commands::update_context,
            commands::delete_context,
            commands::set_theme,
            commands::get_theme,
            commands::open_browser,
            commands::set_browser_bounds,
            commands::browser_navigate,
            commands::browser_back,
            commands::browser_forward,
            commands::browser_reload,
            commands::browser_evaluate,
            commands::browser_get_source,
            commands::show_browser,
            commands::hide_browser,
            commands::close_browser,
            commands::close_browser_tab,
            commands::switch_browser_tab,
            commands::list_browser_tabs,
            commands::browser_open_devtools,
            commands::list_directory,
            commands::install_claude_hooks,
        ])
        .run(tauri::generate_context!())
        .expect("error while running vmux");
}

fn get_main_hwnd(app: &tauri::App) -> isize {
    #[cfg(target_os = "windows")]
    {
        use raw_window_handle::{HasWindowHandle, RawWindowHandle};
        if let Some(window) = app.get_webview_window("main") {
            if let Ok(handle) = window.window_handle() {
                if let RawWindowHandle::Win32(h) = handle.as_raw() {
                    return h.hwnd.get() as isize;
                }
            }
        }
    }
    0
}

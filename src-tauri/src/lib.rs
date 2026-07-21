pub mod terminal;
mod osc;
mod workspace;
mod git_meta;
mod context_store;
mod state;
mod commands;
mod theme;
mod browser;
mod claude_hooks;
mod transcript;
mod embeddings;
mod rag;
mod worktree;
mod settings;
mod ipc;

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
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let data_dir = app.path().app_data_dir()
                .expect("failed to get app data dir");
            std::fs::create_dir_all(&data_dir)?;

            // Get the main window HWND so we can parent native terminal windows to it
            let main_hwnd = get_main_hwnd(app);

            let state = AppState::new(data_dir.to_str().unwrap(), main_hwnd)
                .expect("failed to init app state");
            // Apply the persisted prefix key to the native WndProc immediately —
            // update_settings only updates it on a later settings change.
            terminal::window::set_prefix_vk(commands::prefix_key_to_vk(&state.settings.prefix_key));
            app.manage(Mutex::new(state));

            // Subclass the main window to track moves and reposition terminal popups
            #[cfg(target_os = "windows")]
            if main_hwnd != 0 {
                unsafe { window_tracking::subclass_main_window(main_hwnd); }
            }

            // Start the IPC control pipe so `vmux ping`, `vmux list`, etc. work.
            ipc::start_ipc_server(app.handle().clone());

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
            commands::save_workspace_state,
            commands::remove_pane,
            commands::rename_workspace,
            commands::rename_tab,
            commands::set_workspace_directory,
            commands::delete_workspace,
            commands::reorder_panes,
            commands::set_tab_direction,
            commands::get_git_metadata,
            commands::git_changed_files,
            commands::list_context,
            commands::create_context,
            commands::update_context,
            commands::delete_context,
            commands::get_settings,
            commands::update_settings,
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
            commands::open_browser_tab,
            commands::browser_history,
            commands::clear_browser_history,
            commands::get_browser_url,
            commands::get_any_browser_url,
            commands::screenshot_browser,
            commands::create_worktree,
            commands::list_worktrees,
            commands::delete_worktree,
            commands::save_terminal_scrollback,
            commands::restore_terminal_scrollback,
            commands::list_directory,
            commands::has_vmux_hooks,
            commands::install_claude_hooks,
            commands::list_projects,
            commands::ensure_project,
            commands::list_conversations,
            commands::get_conversation_chunks,
            commands::import_transcripts,
            commands::get_agent_config,
            commands::save_agent_config,
            commands::export_agent_config,
            commands::detect_agent_files,
            commands::rag_search,
            commands::embed_chunks,
            commands::set_embedding_config,
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

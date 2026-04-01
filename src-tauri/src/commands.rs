use tauri::{AppHandle, Manager, State, Emitter};
use serde::Serialize;
use std::sync::Mutex;
use tokio::sync::mpsc;

use crate::state::AppState;
use crate::terminal::{TerminalInfo, PaneBounds};
use crate::terminal::shell::ShellProfile;
use crate::terminal::agents::AgentProfile;
use crate::workspace::{Workspace, PaneKind, Tab, Pane};
use crate::git_meta::{GitMeta, get_git_meta};
use crate::context_store::ContextEntry;
use crate::theme::Theme;

// ─── Terminal pane commands ───────────────────────────────────────────────────

/// Return all shells detected on this machine (reads from cached AppState).
#[tauri::command]
pub fn list_shells(state: State<'_, Mutex<AppState>>) -> Vec<ShellProfile> {
    state.lock().map_or_else(|_| vec![], |s| s.shells.clone())
}

/// Phase 1: Spawn a PTY and return the terminal ID immediately (<5ms).
/// The UI can render the placeholder div right away.
/// Phase 2 (GPU renderer + Win32 window) is triggered by the first set_terminal_bounds call.
///
/// `shell_id` selects which detected shell to launch ("cmd", "powershell", "pwsh", "gitbash").
/// Falls back to the first detected shell if the requested id is not found.
#[tauri::command]
pub fn create_terminal(
    state: State<'_, Mutex<AppState>>,
    working_dir: Option<String>,
    shell_id: Option<String>,
) -> Result<TerminalInfo, String> {
    let mut s = state.lock().map_err(|e| e.to_string())?;
    let shell = shell_id
        .as_deref()
        .and_then(|id| s.shells.iter().find(|sh| sh.id == id))
        .or_else(|| s.shells.first())
        .ok_or("no shells detected")?
        .clone();
    let result = s.terminals.spawn(working_dir, &shell)
        .map_err(|e| e.to_string());
    match &result {
        Ok(info) => eprintln!("[vmux] create_terminal OK: id={} shell={} pid={:?}", info.id, info.shell_name, info.pid),
        Err(e) => eprintln!("[vmux] create_terminal FAILED: {e}"),
    }
    result
}

/// Called by the frontend's ResizeObserver once the placeholder div has real pixel bounds.
/// First call triggers Phase 2: Win32 HWND creation + wgpu init (async, ~200ms).
/// Subsequent calls just reposition the native window and resize the PTY (<1ms).
#[tauri::command]
pub async fn set_terminal_bounds(
    app: AppHandle,
    state: State<'_, Mutex<AppState>>,
    terminal_id: String,
    bounds: PaneBounds,
) -> Result<(), String> {
    let (needs_init, main_hwnd, theme) = {
        let s = state.lock().map_err(|e| e.to_string())?;
        (s.terminals.panes_needing_init(&terminal_id), s.main_hwnd, s.theme.clone())
    };

    if needs_init {
        // Skip Phase 2 if bounds are too small (transient layout state).
        // The ResizeObserver will fire again with proper bounds.
        if bounds.width < 64 || bounds.height < 32 {
            return Ok(());
        }

        let (mut pane, pty_rx) = {
            let mut s = state.lock().map_err(|e| e.to_string())?;
            // Double-check — another call may have already started Phase 2
            let pty_rx = match s.terminals.pending_rx.remove(&terminal_id) {
                Some(rx) => rx,
                None => return Ok(()), // Phase 2 already in progress
            };
            let pane = match s.terminals.panes.remove(&terminal_id) {
                Some(p) => p,
                None => return Ok(()),
            };
            (pane, pty_rx)
        }; // ← AppState lock released here

        let (notif_tx, mut notif_rx) = mpsc::unbounded_channel::<(String, String)>();
        let app_notif = app.clone();
        tokio::spawn(async move {
            while let Some((tid, msg)) = notif_rx.recv().await {
                let _ = app_notif.emit("terminal:notification",
                    serde_json::json!({ "terminalId": tid, "message": msg }));
            }
        });

        // Expensive: Win32 CreateWindowExW (~5ms) + wgpu adapter/device (~200ms).
        // AppState is fully unlocked during this time — other commands proceed normally.
        pane.init_renderer(&app, main_hwnd, bounds, notif_tx, theme, pty_rx)
            .await
            .map_err(|e| {
                eprintln!("[vmux] init_renderer FAILED for {terminal_id}: {e}");
                e.to_string()
            })?;

        // Notify the frontend that the terminal is ready for input.
        let _ = app.emit("terminal:ready",
            serde_json::json!({ "terminalId": terminal_id }));

        // Re-insert the now-initialised pane.
        state.lock().map_err(|e| e.to_string())?
            .terminals.panes.insert(terminal_id, pane);
    } else {
        state.lock().map_err(|e| e.to_string())?
            .terminals.set_bounds(&terminal_id, &bounds);
    }
    Ok(())
}

/// Send keyboard input bytes to a terminal (from the frontend on key events, or
/// from Win32 WndProc messages forwarded via IPC).
#[tauri::command]
pub fn write_terminal(
    state: State<'_, Mutex<AppState>>,
    terminal_id: String,
    data: Vec<u8>,
) -> Result<(), String> {
    state.lock().map_err(|e| e.to_string())?
        .terminals.write(&terminal_id, &data)
        .map_err(|e| e.to_string())
}


#[tauri::command]
pub fn focus_terminal(
    state: State<'_, Mutex<AppState>>,
    terminal_id: String,
) -> Result<(), String> {
    state.lock().map_err(|e| e.to_string())?
        .terminals.focus(&terminal_id);
    Ok(())
}

#[tauri::command]
pub fn show_terminal(
    state: State<'_, Mutex<AppState>>,
    terminal_id: String,
) -> Result<(), String> {
    state.lock().map_err(|e| e.to_string())?
        .terminals.show(&terminal_id);
    Ok(())
}

#[tauri::command]
pub fn hide_terminal(
    state: State<'_, Mutex<AppState>>,
    terminal_id: String,
) -> Result<(), String> {
    state.lock().map_err(|e| e.to_string())?
        .terminals.hide(&terminal_id);
    Ok(())
}

#[tauri::command]
pub fn close_terminal(
    state: State<'_, Mutex<AppState>>,
    terminal_id: String,
) -> Result<(), String> {
    crate::claude_hooks::stop_notify_watcher(&terminal_id);

    let mut s = state.lock().map_err(|e| e.to_string())?;

    // Flush captured agent output to context store before closing
    if let Some(pane) = s.terminals.panes.get(&terminal_id) {
        if pane.info.is_agent {
            if let Some(ref buf) = pane.capture_buf {
                if let Ok(captured) = buf.lock() {
                    if !captured.is_empty() {
                        let agent_type = pane.info.agent_id.clone().unwrap_or_else(|| "unknown".into());
                        let cwd = pane.info.working_dir.clone().unwrap_or_default();
                        let title = format!("{} session", pane.info.shell_name);

                        // Register project + create conversation + store as single chunk
                        let proj_name = cwd.split(['/', '\\']).filter(|s| !s.is_empty()).last()
                            .unwrap_or("unknown").to_string();
                        if let Ok(project) = s.context.ensure_project(&cwd, &proj_name) {
                            if let Ok(conv) = s.context.create_conversation(
                                &project.id, &agent_type, None, Some(&title), "terminal_capture",
                            ) {
                                // Simple chunking: split on blank lines, max ~4KB per chunk
                                let chunks = chunk_terminal_output(&captured, 4000);
                                for (i, chunk) in chunks.iter().enumerate() {
                                    let _ = s.context.add_chunk(&conv.id, i as i32, "mixed", chunk);
                                }
                                let _ = s.context.end_conversation(&conv.id);
                                eprintln!("[vmux] captured {} chunks from {} agent session", chunks.len(), agent_type);
                            }
                        }
                    }
                }
            }
        }
    }

    s.terminals.close(&terminal_id);
    Ok(())
}

/// Split terminal output into chunks of approximately `max_size` bytes,
/// splitting on blank lines or newlines.
fn chunk_terminal_output(text: &str, max_size: usize) -> Vec<String> {
    let mut chunks = Vec::new();
    let mut current = String::new();

    for line in text.lines() {
        if current.len() + line.len() > max_size && !current.is_empty() {
            chunks.push(std::mem::take(&mut current));
        }
        if !current.is_empty() { current.push('\n'); }
        current.push_str(line);
    }
    if !current.is_empty() {
        chunks.push(current);
    }
    // Filter out chunks that are just whitespace/escape sequences
    chunks.into_iter().filter(|c| c.trim().len() > 10).collect()
}

#[tauri::command]
pub fn clear_notification(
    state: State<'_, Mutex<AppState>>,
    terminal_id: String,
) -> Result<(), String> {
    state.lock().map_err(|e| e.to_string())?
        .terminals.set_notification(&terminal_id, None);
    Ok(())
}

#[tauri::command]
pub fn list_terminals(
    state: State<'_, Mutex<AppState>>,
) -> Result<Vec<TerminalInfo>, String> {
    Ok(state.lock().map_err(|e| e.to_string())?.terminals.list())
}

// ─── Agent commands ───────────────────────────────────────────────────────────

#[tauri::command]
pub fn list_agents(state: State<'_, Mutex<AppState>>) -> Vec<AgentProfile> {
    state.lock().map_or_else(|_| vec![], |s| s.agents.clone())
}

#[tauri::command]
pub fn create_agent_terminal(
    app: AppHandle,
    state: State<'_, Mutex<AppState>>,
    agent_id: String,
    working_dir: Option<String>,
    session_name: Option<String>,
    resume_session: Option<String>,
) -> Result<TerminalInfo, String> {
    // Auto-install Claude hooks on first Claude launch
    if agent_id == "claude" && !crate::claude_hooks::has_vmux_hooks() {
        let _ = crate::claude_hooks::ensure_vmux_hooks();
    }

    let mut s = state.lock().map_err(|e| e.to_string())?;
    let agent = s.agents.iter().find(|a| a.id == agent_id)
        .ok_or_else(|| format!("agent not found: {agent_id}"))?
        .clone();
    let result = s.terminals.spawn_agent(working_dir, &agent, session_name, resume_session)
        .map_err(|e| e.to_string());
    match &result {
        Ok(info) => {
            eprintln!("[vmux] create_agent_terminal OK: id={} agent={} pid={:?}", info.id, info.shell_name, info.pid);
            // Start notify file watcher for Claude terminals
            if let Some(notify_path) = &info.notify_file {
                crate::claude_hooks::start_notify_watcher(
                    info.id.clone(),
                    notify_path.clone(),
                    app,
                );
            }
        }
        Err(e) => eprintln!("[vmux] create_agent_terminal FAILED: {e}"),
    }
    result
}

#[tauri::command]
pub fn install_claude_hooks() -> Result<bool, String> {
    crate::claude_hooks::ensure_vmux_hooks().map_err(|e| e.to_string())
}

// ─── Workspace commands ───────────────────────────────────────────────────────

#[tauri::command]
pub fn list_workspaces(state: State<'_, Mutex<AppState>>) -> Result<Vec<Workspace>, String> {
    Ok(state.lock().map_err(|e| e.to_string())?.workspaces.list())
}

#[tauri::command]
pub fn create_workspace(state: State<'_, Mutex<AppState>>, name: String) -> Result<Workspace, String> {
    state.lock().map_err(|e| e.to_string())?
        .workspaces.create_workspace(&name)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn set_active_workspace(state: State<'_, Mutex<AppState>>, workspace_id: String) -> Result<(), String> {
    state.lock().map_err(|e| e.to_string())?
        .workspaces.set_active(&workspace_id);
    Ok(())
}

#[tauri::command]
pub fn add_tab(state: State<'_, Mutex<AppState>>, workspace_id: String, name: String) -> Result<Tab, String> {
    state.lock().map_err(|e| e.to_string())?
        .workspaces.add_tab(&workspace_id, &name)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn close_tab(state: State<'_, Mutex<AppState>>, workspace_id: String, tab_id: String) -> Result<(), String> {
    state.lock().map_err(|e| e.to_string())?
        .workspaces.close_tab(&workspace_id, &tab_id)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn add_pane(state: State<'_, Mutex<AppState>>, workspace_id: String, tab_id: String, kind: PaneKind) -> Result<Pane, String> {
    state.lock().map_err(|e| e.to_string())?
        .workspaces.add_pane_to_tab(&workspace_id, &tab_id, kind)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn update_layout(state: State<'_, Mutex<AppState>>, workspace_id: String, tab_id: String, layout: String) -> Result<(), String> {
    state.lock().map_err(|e| e.to_string())?
        .workspaces.update_layout(&workspace_id, &tab_id, layout)
        .map_err(|e| e.to_string())
}

/// Restore terminals for persisted panes after app restart.
/// Spawns fresh PTYs for each saved terminal pane, updates pane terminal_ids.
#[tauri::command]
pub fn restore_workspace_terminals(
    state: State<'_, Mutex<AppState>>,
    workspace_id: String,
) -> Result<Vec<TerminalInfo>, String> {
    let mut s = state.lock().map_err(|e| e.to_string())?;

    let ws = s.workspaces.workspaces.get(&workspace_id)
        .ok_or("workspace not found")?.clone();

    let mut infos = Vec::new();
    // (tab_id, pane_id, new PaneKind)
    let mut updates: Vec<(String, String, PaneKind)> = Vec::new();

    for tab in &ws.tabs {
        for pane in &tab.panes {
            if let PaneKind::Terminal { shell_id, .. } = &pane.kind {
                let shell = shell_id.as_deref()
                    .and_then(|id| s.shells.iter().find(|sh| sh.id == id))
                    .or_else(|| s.shells.first())
                    .cloned();

                if let Some(shell) = shell {
                    match s.terminals.spawn(None, &shell) {
                        Ok(info) => {
                            updates.push((tab.id.clone(), pane.id.clone(), PaneKind::Terminal {
                                terminal_id: info.id.clone(),
                                shell_id: Some(shell.id.clone()),
                            }));
                            infos.push(info);
                        }
                        Err(e) => eprintln!("[vmux] restore terminal failed: {e}"),
                    }
                }
            }
        }
    }

    // Apply updates in bulk, single DB write
    if !updates.is_empty() {
        if let Some(ws) = s.workspaces.workspaces.get_mut(&workspace_id) {
            for (tab_id, pane_id, kind) in updates {
                if let Some(tab) = ws.tabs.iter_mut().find(|t| t.id == tab_id) {
                    if let Some(pane) = tab.panes.iter_mut().find(|p| p.id == pane_id) {
                        pane.kind = kind;
                    }
                }
            }
            let ws_clone = ws.clone();
            let _ = s.workspaces.save_workspace(&ws_clone);
        }
    }

    Ok(infos)
}

/// Remove a pane from a workspace tab and persist to SQLite.
#[tauri::command]
pub fn remove_pane(
    state: State<'_, Mutex<AppState>>,
    workspace_id: String,
    tab_id: String,
    pane_id: String,
) -> Result<(), String> {
    state.lock().map_err(|e| e.to_string())?
        .workspaces.remove_pane(&workspace_id, &tab_id, &pane_id)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn rename_workspace(state: State<'_, Mutex<AppState>>, workspace_id: String, name: String) -> Result<(), String> {
    state.lock().map_err(|e| e.to_string())?
        .workspaces.rename_workspace(&workspace_id, &name)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn delete_workspace(state: State<'_, Mutex<AppState>>, workspace_id: String) -> Result<Vec<Workspace>, String> {
    let mut s = state.lock().map_err(|e| e.to_string())?;
    s.workspaces.delete_workspace(&workspace_id).map_err(|e| e.to_string())?;
    Ok(s.workspaces.list())
}

#[tauri::command]
pub fn reorder_panes(
    state: State<'_, Mutex<AppState>>,
    workspace_id: String,
    tab_id: String,
    pane_ids: Vec<String>,
) -> Result<(), String> {
    state.lock().map_err(|e| e.to_string())?
        .workspaces.reorder_panes(&workspace_id, &tab_id, &pane_ids)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn set_tab_direction(
    state: State<'_, Mutex<AppState>>,
    workspace_id: String,
    tab_id: String,
    direction: String,
) -> Result<(), String> {
    state.lock().map_err(|e| e.to_string())?
        .workspaces.set_tab_direction(&workspace_id, &tab_id, &direction)
        .map_err(|e| e.to_string())
}

// ─── Git metadata ─────────────────────────────────────────────────────────────

#[tauri::command]
pub fn get_git_metadata(dir: String) -> Result<GitMeta, String> {
    get_git_meta(&dir).map_err(|e| e.to_string())
}

/// Get changed files with diffs for the git diff panel.
#[tauri::command]
pub fn git_changed_files(dir: String) -> Result<Vec<crate::git_meta::ChangedFile>, String> {
    crate::git_meta::get_changed_files(&dir).map_err(|e| e.to_string())
}

// ─── Context store ────────────────────────────────────────────────────────────

#[tauri::command]
pub fn list_context(state: State<'_, Mutex<AppState>>, workspace_id: Option<String>) -> Result<Vec<ContextEntry>, String> {
    state.lock().map_err(|e| e.to_string())?
        .context.list(workspace_id.as_deref())
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn create_context(
    state: State<'_, Mutex<AppState>>,
    title: String,
    content: String,
    workspace_id: Option<String>,
    tab_id: Option<String>,
    tags: Vec<String>,
) -> Result<ContextEntry, String> {
    state.lock().map_err(|e| e.to_string())?
        .context.create(&title, &content, workspace_id.as_deref(), tab_id.as_deref(), tags)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn update_context(
    state: State<'_, Mutex<AppState>>,
    id: String,
    title: Option<String>,
    content: Option<String>,
    tags: Option<Vec<String>>,
) -> Result<(), String> {
    state.lock().map_err(|e| e.to_string())?
        .context.update(&id, title.as_deref(), content.as_deref(), tags)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn delete_context(state: State<'_, Mutex<AppState>>, id: String) -> Result<(), String> {
    state.lock().map_err(|e| e.to_string())?
        .context.delete(&id)
        .map_err(|e| e.to_string())
}

// ─── Browser pane commands ────────────────────────────────────────────────────

/// Simplified browser — standalone popup WebviewWindow, no parent/owner tricks.
/// Mirrors the first-commit pattern that previously worked.
#[tauri::command]
pub async fn open_browser(
    app: AppHandle,
    state: State<'_, Mutex<AppState>>,
    bounds: PaneBounds,
    url: Option<String>,
) -> Result<String, String> {
    let url_str = url.unwrap_or_else(|| "https://example.com".to_string());
    eprintln!("[vmux] open_browser: bounds={},{} {}x{}, url={url_str}",
        bounds.x, bounds.y, bounds.width, bounds.height);

    // Close any existing browser window first
    {
        let mut s = state.lock().map_err(|e| e.to_string())?;
        if let Some(existing) = s.browser.window.take() {
            eprintln!("[vmux] closing existing browser window");
            let _ = existing.destroy();
        }
    }

    let tab_id = uuid::Uuid::new_v4().to_string();
    let url_parsed = tauri::Url::parse(&url_str).map_err(|e| e.to_string())?;
    let label = format!("browser-{}", uuid::Uuid::new_v4());

    let main_hwnd = state.lock().map_err(|e| e.to_string())?.main_hwnd;
    let (screen_x, screen_y) = unsafe {
        use windows::Win32::Foundation::*;
        use windows::Win32::Graphics::Gdi::ClientToScreen;
        let hwnd = HWND(main_hwnd as *mut _);
        let mut pt = POINT { x: bounds.x, y: bounds.y };
        let _ = ClientToScreen(hwnd, &mut pt);
        (pt.x, pt.y)
    };

    let app_nav = app.clone();
    let browser_win = tauri::WebviewWindowBuilder::new(
        &app,
        label,
        tauri::WebviewUrl::External(url_parsed),
    )
    .title("vmux browser")
    .inner_size(bounds.width as f64, bounds.height as f64)
    .position(screen_x as f64, screen_y as f64)
    .decorations(false)
    .always_on_top(false)
    .skip_taskbar(true)
    .visible(true)
    .focused(false)
    .on_navigation(move |nav_url| {
        let _ = app_nav.emit("browser:url-changed", nav_url.to_string());
        true
    })
    .build()
    .map_err(|e| {
        eprintln!("[vmux] browser window build FAILED: {e}");
        e.to_string()
    })?;

    // Make it owned by the main window so it stays on top and moves with it
    unsafe {
        use windows::Win32::Foundation::HWND;
        use windows::Win32::UI::WindowsAndMessaging::*;
        use raw_window_handle::{HasWindowHandle, RawWindowHandle};
        if let Ok(handle) = browser_win.window_handle() {
            if let RawWindowHandle::Win32(h) = handle.as_raw() {
                let browser_hwnd = HWND(h.hwnd.get() as *mut _);
                let owner = HWND(main_hwnd as *mut _);
                SetWindowLongPtrW(browser_hwnd, GWLP_HWNDPARENT, owner.0 as isize);
                let ex_style = GetWindowLongPtrW(browser_hwnd, GWL_EXSTYLE);
                SetWindowLongPtrW(browser_hwnd, GWL_EXSTYLE,
                    ex_style | WS_EX_TOOLWINDOW.0 as isize);
            }
        }
    }

    eprintln!("[vmux] browser window created successfully: {tab_id}");
    state.lock().map_err(|e| e.to_string())?.browser.window = Some(browser_win);
    state.lock().map_err(|e| e.to_string())?.browser.current_url = url_str;
    Ok(tab_id)
}

#[tauri::command]
pub fn set_browser_bounds(
    state: State<'_, Mutex<AppState>>,
    bounds: PaneBounds,
) -> Result<(), String> {
    let s = state.lock().map_err(|e| e.to_string())?;
    if let Some(win) = &s.browser.window {
        let (screen_x, screen_y) = unsafe {
            use windows::Win32::Foundation::*;
            use windows::Win32::Graphics::Gdi::ClientToScreen;
            let hwnd = HWND(s.main_hwnd as *mut _);
            let mut pt = POINT { x: bounds.x, y: bounds.y };
            let _ = ClientToScreen(hwnd, &mut pt);
            (pt.x, pt.y)
        };
        let _ = win.set_position(tauri::PhysicalPosition::new(screen_x, screen_y));
        let _ = win.set_size(tauri::PhysicalSize::new(bounds.width as u32, bounds.height as u32));
    }
    Ok(())
}

#[tauri::command]
pub fn browser_navigate(
    state: State<'_, Mutex<AppState>>,
    url: String,
) -> Result<(), String> {
    let mut s = state.lock().map_err(|e| e.to_string())?;
    s.browser.current_url = url.clone();
    if let Some(win) = &s.browser.window {
        let parsed = tauri::Url::parse(&url).map_err(|e| e.to_string())?;
        win.navigate(parsed).map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[tauri::command]
pub fn browser_back(state: State<'_, Mutex<AppState>>) -> Result<(), String> {
    let s = state.lock().map_err(|e| e.to_string())?;
    if let Some(win) = &s.browser.window {
        win.eval("window.history.back()").map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[tauri::command]
pub fn browser_forward(state: State<'_, Mutex<AppState>>) -> Result<(), String> {
    let s = state.lock().map_err(|e| e.to_string())?;
    if let Some(win) = &s.browser.window {
        win.eval("window.history.forward()").map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[tauri::command]
pub fn browser_reload(state: State<'_, Mutex<AppState>>) -> Result<(), String> {
    let s = state.lock().map_err(|e| e.to_string())?;
    if let Some(win) = &s.browser.window {
        win.eval("location.reload()").map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[tauri::command]
pub async fn browser_evaluate(
    state: State<'_, Mutex<AppState>>,
    js: String,
    call_id: String,
) -> Result<(), String> {
    let s = state.lock().map_err(|e| e.to_string())?;
    if let Some(win) = &s.browser.window {
        let script = format!(
            r#"(async () => {{
                try {{
                    const __result = await (async () => {{ {js} }})();
                    window.__TAURI_INTERNALS__?.emit('browser:eval-result', {{ id: '{call_id}', ok: true, value: JSON.stringify(__result) }});
                }} catch(e) {{
                    window.__TAURI_INTERNALS__?.emit('browser:eval-result', {{ id: '{call_id}', ok: false, value: e.message }});
                }}
            }})();"#
        );
        win.eval(&script).map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[tauri::command]
pub async fn browser_get_source(
    state: State<'_, Mutex<AppState>>,
    call_id: String,
) -> Result<(), String> {
    let s = state.lock().map_err(|e| e.to_string())?;
    if let Some(win) = &s.browser.window {
        let script = format!(
            r#"window.__TAURI_INTERNALS__?.emit('browser:source', {{ id: '{call_id}', html: document.documentElement.outerHTML }});"#
        );
        win.eval(&script).map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[tauri::command]
pub fn show_browser(state: State<'_, Mutex<AppState>>) -> Result<(), String> {
    let s = state.lock().map_err(|e| e.to_string())?;
    if let Some(win) = &s.browser.window {
        win.show().map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[tauri::command]
pub fn hide_browser(state: State<'_, Mutex<AppState>>) -> Result<(), String> {
    let s = state.lock().map_err(|e| e.to_string())?;
    if let Some(win) = &s.browser.window {
        let _ = win.hide();
    }
    Ok(())
}

#[tauri::command]
pub fn close_browser(state: State<'_, Mutex<AppState>>) -> Result<(), String> {
    let mut s = state.lock().map_err(|e| e.to_string())?;
    if let Some(win) = s.browser.window.take() {
        let _ = win.destroy();
    }
    Ok(())
}

#[tauri::command]
pub fn browser_open_devtools(state: State<'_, Mutex<AppState>>) -> Result<(), String> {
    let s = state.lock().map_err(|e| e.to_string())?;
    if let Some(win) = &s.browser.window {
        win.open_devtools();
    }
    Ok(())
}

// Stubs for multi-tab commands (registered in lib.rs but simplified away)
#[tauri::command]
pub fn close_browser_tab() -> Result<Vec<crate::browser::BrowserTabInfo>, String> { Ok(vec![]) }
#[tauri::command]
pub fn switch_browser_tab() -> Result<(), String> { Ok(()) }
#[tauri::command]
pub fn list_browser_tabs() -> Result<Vec<crate::browser::BrowserTabInfo>, String> { Ok(vec![]) }

// ─── Theme ────────────────────────────────────────────────────────────────────

#[tauri::command]
pub fn set_theme(state: State<'_, Mutex<AppState>>, theme_name: String) -> Result<(), String> {
    let theme = match theme_name.as_str() {
        "catppuccin_mocha" => Theme::catppuccin_mocha(),
        _ => Theme::tokyo_night(),
    };
    state.lock().map_err(|e| e.to_string())?.theme = theme;
    Ok(())
}

#[tauri::command]
pub fn get_theme(state: State<'_, Mutex<AppState>>) -> Result<Theme, String> {
    Ok(state.lock().map_err(|e| e.to_string())?.theme.clone())
}

// ─── File tree ───────────────────────────────────────────────────────────────

#[derive(Serialize)]
pub struct FileEntry {
    name: String,
    path: String,
    is_dir: bool,
}

#[tauri::command]
pub fn list_directory(path: String) -> Result<Vec<FileEntry>, String> {
    let mut entries = Vec::new();
    let dir = std::fs::read_dir(&path).map_err(|e| e.to_string())?;
    for entry in dir {
        let entry = entry.map_err(|e| e.to_string())?;
        let name = entry.file_name().to_string_lossy().to_string();
        // Skip hidden files/dirs
        if name.starts_with('.') { continue; }
        // Skip common noise dirs
        if name == "node_modules" || name == "target" || name == "__pycache__" || name == ".git" {
            continue;
        }
        let ft = entry.file_type().map_err(|e| e.to_string())?;
        entries.push(FileEntry {
            path: entry.path().to_string_lossy().to_string(),
            name,
            is_dir: ft.is_dir(),
        });
    }
    entries.sort_by(|a, b| {
        if a.is_dir != b.is_dir { return if a.is_dir { std::cmp::Ordering::Less } else { std::cmp::Ordering::Greater }; }
        a.name.to_lowercase().cmp(&b.name.to_lowercase())
    });
    Ok(entries)
}

// ─── Project & Conversation commands ─────────────────────────────────────────

use crate::context_store::{Project, Conversation, ConversationChunk, AgentConfig, SearchResult};

#[tauri::command]
pub fn list_projects(state: State<'_, Mutex<AppState>>) -> Result<Vec<Project>, String> {
    state.lock().map_err(|e| e.to_string())?
        .context.list_projects().map_err(|e| e.to_string())
}

#[tauri::command]
pub fn ensure_project(
    state: State<'_, Mutex<AppState>>,
    path: String,
    name: String,
) -> Result<Project, String> {
    state.lock().map_err(|e| e.to_string())?
        .context.ensure_project(&path, &name).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn list_conversations(
    state: State<'_, Mutex<AppState>>,
    project_id: Option<String>,
) -> Result<Vec<Conversation>, String> {
    state.lock().map_err(|e| e.to_string())?
        .context.list_conversations(project_id.as_deref()).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn get_conversation_chunks(
    state: State<'_, Mutex<AppState>>,
    conversation_id: String,
) -> Result<Vec<ConversationChunk>, String> {
    state.lock().map_err(|e| e.to_string())?
        .context.get_chunks(&conversation_id).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn import_transcripts(
    state: State<'_, Mutex<AppState>>,
    project_path: Option<String>,
) -> Result<usize, String> {
    let s = state.lock().map_err(|e| e.to_string())?;
    if let Some(path) = project_path {
        crate::transcript::import_all_transcripts_for_project(&s.context, &path)
            .map_err(|e| e.to_string())
    } else {
        crate::transcript::import_all_transcripts(&s.context)
            .map_err(|e| e.to_string())
    }
}

#[tauri::command]
pub fn get_agent_config(
    state: State<'_, Mutex<AppState>>,
    project_id: String,
) -> Result<Option<AgentConfig>, String> {
    state.lock().map_err(|e| e.to_string())?
        .context.get_agent_config(&project_id).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn save_agent_config(
    state: State<'_, Mutex<AppState>>,
    project_id: String,
    name: String,
    content: String,
    auto_generated: bool,
) -> Result<AgentConfig, String> {
    state.lock().map_err(|e| e.to_string())?
        .context.save_agent_config(&project_id, &name, &content, auto_generated)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn export_agent_config(
    state: State<'_, Mutex<AppState>>,
    project_id: String,
    output_path: String,
) -> Result<(), String> {
    state.lock().map_err(|e| e.to_string())?
        .context.export_agent_config(&project_id, &output_path)
        .map_err(|e| e.to_string())
}

// ─── RAG Search commands ─────────────────────────────────────────────────────

#[tauri::command]
pub async fn rag_search(
    state: State<'_, Mutex<AppState>>,
    query: String,
    project_id: Option<String>,
    top_k: Option<usize>,
) -> Result<Vec<SearchResult>, String> {
    let (db_path, config) = {
        let s = state.lock().map_err(|e| e.to_string())?;
        (s.context_db_path.clone(), s.embedding_config.clone())
    };
    let top_k = top_k.unwrap_or(10);

    // Step 1: embed the query (async, no DB)
    let provider = crate::embeddings::create_provider(&config);
    let query_embeddings = provider.embed(&[query.clone()])
        .await.map_err(|e| e.to_string())?;
    let query_vec = query_embeddings.into_iter().next()
        .ok_or("empty embedding result")?;

    // Step 2: search DB (sync, on blocking thread)
    tokio::task::spawn_blocking(move || {
        let store = crate::context_store::ContextStore::new(&db_path)
            .map_err(|e| e.to_string())?;
        crate::rag::search_with_embedding(&store, &query_vec, project_id.as_deref(), top_k)
            .map_err(|e| e.to_string())
    }).await.map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn embed_chunks(
    state: State<'_, Mutex<AppState>>,
) -> Result<usize, String> {
    let (db_path, config) = {
        let s = state.lock().map_err(|e| e.to_string())?;
        (s.context_db_path.clone(), s.embedding_config.clone())
    };

    // Get unembedded chunks (blocking)
    let chunks = tokio::task::spawn_blocking({
        let db_path = db_path.clone();
        move || {
            let store = crate::context_store::ContextStore::new(&db_path)
                .map_err(|e| e.to_string())?;
            store.get_unembedded_chunks(1000).map_err(|e| e.to_string())
        }
    }).await.map_err(|e| e.to_string())??;

    if chunks.is_empty() { return Ok(0); }

    // Embed (async API call)
    let provider = crate::embeddings::create_provider(&config);
    let texts: Vec<String> = chunks.iter().map(|c| c.content.clone()).collect();
    let embeddings = provider.embed(&texts).await.map_err(|e| e.to_string())?;

    // Store embeddings (blocking)
    let count = chunks.len();
    tokio::task::spawn_blocking(move || {
        let store = crate::context_store::ContextStore::new(&db_path)
            .map_err(|e| e.to_string())?;
        for (chunk, embedding) in chunks.iter().zip(embeddings.iter()) {
            store.set_chunk_embedding(&chunk.id, embedding).map_err(|e| e.to_string())?;
        }
        Ok::<usize, String>(count)
    }).await.map_err(|e| e.to_string())?
}

#[tauri::command]
pub fn set_embedding_config(
    state: State<'_, Mutex<AppState>>,
    provider: String,
    api_key: String,
    base_url: Option<String>,
    model: Option<String>,
) -> Result<(), String> {
    let mut s = state.lock().map_err(|e| e.to_string())?;
    s.embedding_config = crate::embeddings::EmbeddingConfig {
        provider, api_key, base_url, model,
    };
    Ok(())
}

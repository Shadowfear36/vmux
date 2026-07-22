use tauri::{AppHandle, State, Emitter, Manager};
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
pub async fn create_terminal(
    state: State<'_, Mutex<AppState>>,
    working_dir: Option<String>,
    shell_id: Option<String>,
) -> Result<TerminalInfo, String> {
    let shell = {
        let s = state.lock().map_err(|e| e.to_string())?;
        shell_id
            .as_deref()
            .and_then(|id| s.shells.iter().find(|sh| sh.id == id))
            .or_else(|| s.shells.first())
            .ok_or("no shells detected")?
            .clone()
    }; // ← AppState lock released before the (possibly daemon-connecting) spawn

    let result = crate::terminal::TerminalPane::spawn_maybe_daemon(working_dir, &shell)
        .await
        .map_err(|e| e.to_string());

    match result {
        Ok((pane, pty_rx)) => {
            let info = state.lock().map_err(|e| e.to_string())?
                .terminals.insert(pane, pty_rx);
            log::info!("create_terminal OK: id={} shell={} pid={:?}", info.id, info.shell_name, info.pid);
            Ok(info)
        }
        Err(e) => {
            log::error!("create_terminal FAILED: {e}");
            Err(e)
        }
    }
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
    let (needs_init, main_hwnd, theme, font_size) = {
        let s = state.lock().map_err(|e| e.to_string())?;
        (s.terminals.panes_needing_init(&terminal_id), s.main_hwnd, s.theme.clone(), s.settings.font_size)
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
        pane.init_renderer(&app, main_hwnd, bounds, notif_tx, theme, font_size, pty_rx)
            .await
            .map_err(|e| {
                log::error!("init_renderer FAILED for {terminal_id}: {e}");
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
                                    if let Err(e) = s.context.add_chunk(&conv.id, i as i32, "mixed", chunk) {
                                        log::error!("failed to store captured chunk {i} for conversation {}: {e}", conv.id);
                                    }
                                }
                                if let Err(e) = s.context.end_conversation(&conv.id) {
                                    log::error!("failed to finalize conversation {}: {e}", conv.id);
                                }
                                log::info!("captured {} chunks from {} agent session", chunks.len(), agent_type);
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
pub async fn create_agent_terminal(
    app: AppHandle,
    state: State<'_, Mutex<AppState>>,
    agent_id: String,
    working_dir: Option<String>,
    session_name: Option<String>,
    resume_session: Option<String>,
    continue_session: Option<bool>,
) -> Result<TerminalInfo, String> {
    // Hook installation into ~/.claude/settings.json requires explicit user
    // consent — see `install_claude_hooks`. We never do it implicitly here.
    let agent = {
        let s = state.lock().map_err(|e| e.to_string())?;
        s.agents.iter().find(|a| a.id == agent_id)
            .ok_or_else(|| format!("agent not found: {agent_id}"))?
            .clone()
    }; // ← AppState lock released before the (possibly daemon-connecting) spawn

    let working_dir_for_watcher = working_dir.clone();
    let result = crate::terminal::TerminalPane::spawn_agent_maybe_daemon(
        working_dir, &agent, session_name, resume_session,
        continue_session.unwrap_or(false),
    )
        .await
        .map_err(|e| e.to_string());

    let result = match result {
        Ok((pane, pty_rx)) => {
            let info = state.lock().map_err(|e| e.to_string())?
                .terminals.insert(pane, pty_rx);
            Ok(info)
        }
        Err(e) => Err(e),
    };

    match &result {
        Ok(info) => {
            log::info!("create_agent_terminal OK: id={} agent={} pid={:?}", info.id, info.shell_name, info.pid);
            // Start notify file watcher for Claude terminals
            if let Some(notify_path) = &info.notify_file {
                crate::claude_hooks::start_notify_watcher(
                    info.id.clone(),
                    notify_path.clone(),
                    working_dir_for_watcher,
                    app,
                );
            }
        }
        Err(e) => log::error!("create_agent_terminal FAILED: {e}"),
    }
    result
}

/// Check whether vmux's Claude Code hooks are already installed in
/// ~/.claude/settings.json, without modifying anything.
#[tauri::command]
pub fn has_vmux_hooks() -> bool {
    crate::claude_hooks::has_vmux_hooks()
}

/// Install vmux's Claude Code hooks into ~/.claude/settings.json.
/// Must only be called after explicit user consent (see frontend prompt in
/// store.ts) — this mutates the user's real, shared Claude Code config.
#[tauri::command]
pub fn install_claude_hooks() -> Result<bool, String> {
    crate::claude_hooks::ensure_vmux_hooks().map_err(|e| e.to_string())
}

/// Check whether the vmux-browser Claude Code skill is already installed at
/// ~/.claude/skills/vmux-browser/SKILL.md, without modifying anything.
#[tauri::command]
pub fn has_vmux_browser_skill() -> bool {
    crate::agent_skills::has_vmux_browser_skill()
}

/// Install (or update) the vmux-browser skill into ~/.claude/skills/. Must
/// only be called after explicit user consent (see the Settings panel) —
/// this writes outside the project sandbox, to the user's shared Claude
/// Code skills directory.
#[tauri::command]
pub fn install_vmux_browser_skill() -> Result<(), String> {
    crate::agent_skills::install_vmux_browser_skill().map_err(|e| e.to_string())
}

/// Check whether the vmux-context Claude Code skill is already installed at
/// ~/.claude/skills/vmux-context/SKILL.md, without modifying anything.
#[tauri::command]
pub fn has_vmux_context_skill() -> bool {
    crate::agent_skills::has_vmux_context_skill()
}

/// Install (or update) the vmux-context skill into ~/.claude/skills/. Must
/// only be called after explicit user consent (see the Settings panel) —
/// this writes outside the project sandbox, to the user's shared Claude
/// Code skills directory.
#[tauri::command]
pub fn install_vmux_context_skill() -> Result<(), String> {
    crate::agent_skills::install_vmux_context_skill().map_err(|e| e.to_string())
}

// ─── Daemon (vmuxd) session management ────────────────────────────────────────
// Thin wrappers around the daemon_client control-plane functions (see
// docs/session-reattach-design.md §17). These talk directly to vmuxd's own
// registry over the named pipe — they don't touch `Mutex<AppState>` at all,
// since the daemon (not AppState) is the source of truth for its sessions.

#[tauri::command]
pub async fn list_daemon_sessions() -> Result<Vec<crate::terminal::daemon_client::SessionMeta>, String> {
    crate::terminal::daemon_client::list_sessions().await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn kill_daemon_session(session_id: String) -> Result<(), String> {
    crate::terminal::daemon_client::kill_session(&session_id).await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn list_daemon_orphans() -> Result<Vec<crate::terminal::daemon_client::OrphanInfo>, String> {
    crate::terminal::daemon_client::list_orphans().await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn kill_daemon_orphan(pid: u32) -> Result<(), String> {
    crate::terminal::daemon_client::kill_orphan(pid).await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn is_daemon_running() -> bool {
    crate::terminal::daemon_client::is_daemon_running().await
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

/// Save the full workspace state to SQLite (pane CWDs, layout, etc).
/// Declared async so Tauri v2 dispatches it on the tokio thread pool instead
/// of the WebView2 main thread, keeping the Win32 message pump free.
#[tauri::command]
pub async fn save_workspace_state(
    state: State<'_, Mutex<AppState>>,
    workspace_id: String,
    workspace_json: String,
) -> Result<(), String> {
    let mut s = state.lock().map_err(|e| e.to_string())?;
    if let Ok(ws) = serde_json::from_str::<Workspace>(&workspace_json) {
        s.workspaces.workspaces.insert(workspace_id.clone(), ws.clone());
        s.workspaces.save_workspace(&ws).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Either a plain shell or an agent to restore a pane as — panes distinguish
/// the two via `PaneKind::Terminal`'s `agent_id` vs `shell_id`.
enum RestoreTarget {
    Shell(ShellProfile),
    Agent(AgentProfile, Option<String> /* notify_file */),
}

/// Restore terminals for persisted panes after app restart.
/// For panes with a saved `daemon_session_id`, tries to reattach to the
/// still-running vmuxd session first (Phase 3 of the session-reattach
/// design doc); falls back to spawning a fresh PTY if the daemon is
/// unreachable or the session no longer exists. Local panes just spawn
/// fresh, as before. Agent panes (identified by `agent_id`) are restored
/// the same way, reattaching to a still-running daemon-backed agent
/// process when possible instead of always starting a fresh one.
#[tauri::command]
pub async fn restore_workspace_terminals(
    app: AppHandle,
    state: State<'_, Mutex<AppState>>,
    workspace_id: String,
) -> Result<Vec<TerminalInfo>, String> {
    // (tab_id, pane_id, target, cwd, daemon_session_id)
    let to_restore: Vec<(String, String, RestoreTarget, Option<String>, Option<String>)> = {
        let s = state.lock().map_err(|e| e.to_string())?;
        let ws = s.workspaces.workspaces.get(&workspace_id)
            .ok_or("workspace not found")?.clone();
        let ws_dir = ws.directory.clone();

        let mut entries = Vec::new();
        for tab in &ws.tabs {
            for pane in &tab.panes {
                if let PaneKind::Terminal { shell_id, agent_id, working_dir, daemon_session_id, notify_file, .. } = &pane.kind {
                    // Priority: saved pane CWD > workspace directory > default
                    let cwd = working_dir.clone().or_else(|| ws_dir.clone());
                    let target = if let Some(agent_id) = agent_id {
                        s.agents.iter().find(|a| &a.id == agent_id).cloned()
                            .map(|a| RestoreTarget::Agent(a, notify_file.clone()))
                    } else {
                        shell_id.as_deref()
                            .and_then(|id| s.shells.iter().find(|sh| sh.id == id))
                            .or_else(|| s.shells.first())
                            .cloned()
                            .map(RestoreTarget::Shell)
                    };
                    if let Some(target) = target {
                        entries.push((tab.id.clone(), pane.id.clone(), target, cwd, daemon_session_id.clone()));
                    }
                }
            }
        }
        entries
    }; // ← AppState lock released before any (possibly daemon-connecting) spawn/attach

    let mut infos = Vec::new();
    // (tab_id, pane_id, new PaneKind, TerminalPane, receiver)
    let mut spawned: Vec<(String, String, PaneKind, crate::terminal::TerminalPane, mpsc::UnboundedReceiver<Vec<u8>>)> = Vec::new();
    // (terminal_id, notify_file, working_dir) — agent notify watchers to (re)start once panes are inserted
    let mut notify_watchers: Vec<(String, String, Option<String>)> = Vec::new();

    for (tab_id, pane_id, target, cwd, daemon_session_id) in to_restore {
        let attached = if let Some(session_id) = &daemon_session_id {
            let result = match &target {
                RestoreTarget::Shell(shell) => crate::terminal::TerminalPane::attach_daemon(session_id, cwd.clone(), shell).await,
                RestoreTarget::Agent(agent, notify_file) => crate::terminal::TerminalPane::attach_daemon_agent(session_id, cwd.clone(), agent, notify_file.clone()).await,
            };
            match result {
                Ok(result) => Some(result),
                Err(e) => {
                    log::warn!("daemon session {session_id} unreachable, respawning: {e}");
                    None
                }
            }
        } else {
            None
        };

        // No saved session to attach to (or attach failed) — spawn fresh,
        // same as before Phase 3. Restore intentionally never starts NEW
        // daemon sessions; it only reattaches ones that already existed.
        let (pane, rx, session_id_for_pane) = match attached {
            Some((pane, rx)) => (pane, rx, daemon_session_id.clone()),
            None => {
                let fresh = match &target {
                    RestoreTarget::Shell(shell) => crate::terminal::TerminalPane::spawn(cwd.clone(), shell),
                    RestoreTarget::Agent(agent, _) => crate::terminal::TerminalPane::spawn_agent(cwd.clone(), agent, None, None, false),
                };
                match fresh {
                    Ok((pane, rx)) => (pane, rx, None),
                    Err(e) => { log::warn!("restore terminal failed: {e}"); continue; }
                }
            }
        };

        if pane.info.is_agent {
            if let Some(notify_path) = &pane.info.notify_file {
                notify_watchers.push((pane.info.id.clone(), notify_path.clone(), cwd.clone()));
            }
        }

        let kind = match &target {
            RestoreTarget::Shell(shell) => PaneKind::Terminal {
                terminal_id: pane.info.id.clone(),
                shell_id: Some(shell.id.clone()),
                agent_id: None,
                working_dir: cwd,
                daemon_session_id: session_id_for_pane,
                notify_file: None,
            },
            RestoreTarget::Agent(agent, _) => PaneKind::Terminal {
                terminal_id: pane.info.id.clone(),
                shell_id: None,
                agent_id: Some(agent.id.clone()),
                working_dir: cwd,
                daemon_session_id: session_id_for_pane,
                notify_file: pane.info.notify_file.clone(),
            },
        };
        spawned.push((tab_id, pane_id, kind, pane, rx));
    }

    if !spawned.is_empty() {
        let mut s = state.lock().map_err(|e| e.to_string())?;

        // Insert all panes first (borrows s.terminals), then mutate
        // s.workspaces in a separate pass — doing both in one loop trips
        // the borrow checker since both go through the same MutexGuard.
        let mut kind_updates: Vec<(String, String, PaneKind)> = Vec::new();
        for (tab_id, pane_id, kind, pane, rx) in spawned {
            infos.push(s.terminals.insert(pane, rx));
            kind_updates.push((tab_id, pane_id, kind));
        }

        if let Some(ws) = s.workspaces.workspaces.get_mut(&workspace_id) {
            for (tab_id, pane_id, kind) in kind_updates {
                if let Some(tab) = ws.tabs.iter_mut().find(|t| t.id == tab_id) {
                    if let Some(p) = tab.panes.iter_mut().find(|p| p.id == pane_id) {
                        p.kind = kind;
                    }
                }
            }
            let ws_clone = ws.clone();
            if let Err(e) = s.workspaces.save_workspace(&ws_clone) {
                log::error!("failed to persist workspace {workspace_id}: {e}");
            }
        }
    } // ← AppState lock released before starting notify watchers

    for (terminal_id, notify_path, cwd) in notify_watchers {
        crate::claude_hooks::start_notify_watcher(terminal_id, notify_path, cwd, app.clone());
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
pub fn rename_tab(state: State<'_, Mutex<AppState>>, workspace_id: String, tab_id: String, name: String) -> Result<(), String> {
    state.lock().map_err(|e| e.to_string())?
        .workspaces.rename_tab(&workspace_id, &tab_id, &name)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn set_workspace_directory(state: State<'_, Mutex<AppState>>, workspace_id: String, directory: Option<String>) -> Result<(), String> {
    state.lock().map_err(|e| e.to_string())?
        .workspaces.set_workspace_directory(&workspace_id, directory.as_deref())
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

/// Helper: create a WebviewWindow for the given browser pane.
/// `browser_id` becomes part of the Tauri window label and is included in
/// `browser:url-changed` events so the frontend can route to the right pane.
fn create_browser_window(
    app: &AppHandle,
    main_hwnd: isize,
    browser_id: &str,
    bounds: &PaneBounds,
    url: &str,
) -> Result<tauri::WebviewWindow, String> {
    let url_parsed = tauri::Url::parse(url).map_err(|e| e.to_string())?;
    // Each pane gets a deterministic label so we can look it up later.
    let label = format!("browser-{}", browser_id);

    // A previous window under this label may still be mid-teardown (destroy()
    // tears down the native HWND but deregisters the label asynchronously on
    // the main event loop), which would otherwise make the build() below fail
    // with "a webview with label ... already exists". Force it out first.
    if let Some(stale) = app.get_webview_window(&label) {
        let _ = stale.destroy();
    }

    let (screen_x, screen_y) = unsafe {
        use windows::Win32::Foundation::*;
        use windows::Win32::Graphics::Gdi::ClientToScreen;
        let hwnd = HWND(main_hwnd as *mut _);
        let mut pt = POINT { x: bounds.x, y: bounds.y };
        let _ = ClientToScreen(hwnd, &mut pt);
        (pt.x, pt.y)
    };

    let app_nav = app.clone();
    let bid = browser_id.to_string();
    let win = tauri::WebviewWindowBuilder::new(app, &label, tauri::WebviewUrl::External(url_parsed))
        .title("vmux browser")
        .inner_size(bounds.width as f64, bounds.height as f64)
        .position(screen_x as f64, screen_y as f64)
        .decorations(false)
        .always_on_top(false)
        .skip_taskbar(true)
        .visible(true)
        .focused(false)
        .on_navigation(move |nav_url| {
            let _ = app_nav.emit("browser:url-changed", serde_json::json!({
                "browserId": bid,
                "url": nav_url.to_string(),
            }));
            true
        })
        .build()
        .map_err(|e| { log::error!("browser build FAILED: {e}"); e.to_string() })?;

    unsafe {
        use windows::Win32::Foundation::HWND;
        use windows::Win32::UI::WindowsAndMessaging::*;
        use raw_window_handle::{HasWindowHandle, RawWindowHandle};
        if let Ok(handle) = win.window_handle() {
            if let RawWindowHandle::Win32(h) = handle.as_raw() {
                let browser_hwnd = HWND(h.hwnd.get() as *mut _);
                let owner = HWND(main_hwnd as *mut _);
                SetWindowLongPtrW(browser_hwnd, GWLP_HWNDPARENT, owner.0 as isize);
                let ex_style = GetWindowLongPtrW(browser_hwnd, GWL_EXSTYLE);
                SetWindowLongPtrW(browser_hwnd, GWL_EXSTYLE, ex_style | WS_EX_TOOLWINDOW.0 as isize);
            }
        }
    }
    Ok(win)
}

/// Open (or reopen) a browser pane identified by `browser_id`.
#[tauri::command]
pub async fn open_browser(
    app: AppHandle,
    state: State<'_, Mutex<AppState>>,
    browser_id: String,
    bounds: PaneBounds,
    url: Option<String>,
) -> Result<String, String> {
    let url_str = url.unwrap_or_else(|| "https://example.com".to_string());

    // Destroy any existing window for this pane
    {
        let mut s = state.lock().map_err(|e| e.to_string())?;
        if let Some(mgr) = s.browsers.get_mut(&browser_id) {
            if let Some(w) = mgr.take_window() { let _ = w.destroy(); }
        }
    }

    let main_hwnd = state.lock().map_err(|e| e.to_string())?.main_hwnd;
    let win = create_browser_window(&app, main_hwnd, &browser_id, &bounds, &url_str)?;

    let mut s = state.lock().map_err(|e| e.to_string())?;
    let mgr = s.browsers.entry(browser_id).or_insert_with(crate::browser::BrowserManager::new);
    let tab_id = mgr.add_tab(&url_str);
    mgr.set_active(&tab_id);
    mgr.window = Some(win);
    let _ = s.context.add_history(&url_str, None);
    Ok(tab_id)
}

/// Open a new browser tab within an existing pane.
#[tauri::command]
pub async fn open_browser_tab(
    app: AppHandle,
    state: State<'_, Mutex<AppState>>,
    browser_id: String,
    bounds: PaneBounds,
    url: Option<String>,
) -> Result<String, String> {
    let url_str = url.unwrap_or_else(|| "https://example.com".to_string());

    {
        let mut s = state.lock().map_err(|e| e.to_string())?;
        if let Some(mgr) = s.browsers.get_mut(&browser_id) {
            if let Some(w) = mgr.take_window() { let _ = w.destroy(); }
        }
    }

    let main_hwnd = state.lock().map_err(|e| e.to_string())?.main_hwnd;
    let win = create_browser_window(&app, main_hwnd, &browser_id, &bounds, &url_str)?;

    let mut s = state.lock().map_err(|e| e.to_string())?;
    let mgr = s.browsers.entry(browser_id).or_insert_with(crate::browser::BrowserManager::new);
    let tab_id = mgr.add_tab(&url_str);
    mgr.set_active(&tab_id);
    mgr.window = Some(win);
    let _ = s.context.add_history(&url_str, None);
    Ok(tab_id)
}

/// Close a browser tab. If the tab was active, recreate the window for the next tab.
#[tauri::command]
pub async fn close_browser_tab(
    app: AppHandle,
    state: State<'_, Mutex<AppState>>,
    browser_id: String,
    tab_id: String,
    bounds: PaneBounds,
) -> Result<Vec<crate::browser::BrowserTabInfo>, String> {
    let (was_active, next_url, main_hwnd) = {
        let mut s = state.lock().map_err(|e| e.to_string())?;
        let mgr = s.browsers.get_mut(&browser_id).ok_or("browser not found")?;
        let was_active = mgr.close_tab(&tab_id);
        if was_active {
            if let Some(w) = mgr.take_window() { let _ = w.destroy(); }
        }
        let next_url = mgr.active_url().map(|u| u.to_string());
        (was_active, next_url, s.main_hwnd)
    };

    if was_active {
        if let Some(url) = next_url {
            let win = create_browser_window(&app, main_hwnd, &browser_id, &bounds, &url)?;
            if let Some(mgr) = state.lock().map_err(|e| e.to_string())?.browsers.get_mut(&browser_id) {
                mgr.window = Some(win);
            }
        }
    }

    Ok(state.lock().map_err(|e| e.to_string())?
        .browsers.get(&browser_id)
        .map(|m| m.list_tabs())
        .unwrap_or_default())
}

/// Switch to a different tab within a browser pane.
#[tauri::command]
pub async fn switch_browser_tab(
    app: AppHandle,
    state: State<'_, Mutex<AppState>>,
    browser_id: String,
    tab_id: String,
    bounds: PaneBounds,
) -> Result<(), String> {
    let (url, main_hwnd) = {
        let mut s = state.lock().map_err(|e| e.to_string())?;
        let mgr = s.browsers.get_mut(&browser_id).ok_or("browser not found")?;
        if let Some(w) = mgr.take_window() { let _ = w.destroy(); }
        mgr.set_active(&tab_id);
        let url = mgr.active_url().map(|u| u.to_string()).ok_or("tab not found")?;
        (url, s.main_hwnd)
    };

    let win = create_browser_window(&app, main_hwnd, &browser_id, &bounds, &url)?;
    if let Some(mgr) = state.lock().map_err(|e| e.to_string())?.browsers.get_mut(&browser_id) {
        mgr.window = Some(win);
    }
    Ok(())
}

#[tauri::command]
pub fn list_browser_tabs(state: State<'_, Mutex<AppState>>, browser_id: String) -> Result<Vec<crate::browser::BrowserTabInfo>, String> {
    Ok(state.lock().map_err(|e| e.to_string())?
        .browsers.get(&browser_id)
        .map(|m| m.list_tabs())
        .unwrap_or_default())
}

#[tauri::command]
pub async fn set_browser_bounds(
    state: State<'_, Mutex<AppState>>,
    browser_id: String,
    bounds: PaneBounds,
) -> Result<(), String> {
    let s = state.lock().map_err(|e| e.to_string())?;
    let (main_hwnd, win) = (s.main_hwnd, s.browsers.get(&browser_id).and_then(|m| m.window.as_ref()).cloned());
    if let Some(win) = win {
        let (sx, sy) = unsafe {
            use windows::Win32::Foundation::*;
            use windows::Win32::Graphics::Gdi::ClientToScreen;
            let hwnd = HWND(main_hwnd as *mut _);
            let mut pt = POINT { x: bounds.x, y: bounds.y };
            let _ = ClientToScreen(hwnd, &mut pt);
            (pt.x, pt.y)
        };
        let _ = win.set_position(tauri::PhysicalPosition::new(sx, sy));
        let _ = win.set_size(tauri::PhysicalSize::new(bounds.width as u32, bounds.height as u32));
    }
    Ok(())
}

#[tauri::command]
pub async fn browser_navigate(
    state: State<'_, Mutex<AppState>>,
    browser_id: String,
    url: String,
) -> Result<(), String> {
    let mut s = state.lock().map_err(|e| e.to_string())?;
    let _ = s.context.add_history(&url, None);
    if let Some(mgr) = s.browsers.get_mut(&browser_id) {
        if let Some(active_id) = mgr.active_tab_id.clone() {
            mgr.update_url(&active_id, &url);
        }
        if let Some(win) = &mgr.window {
            let parsed = tauri::Url::parse(&url).map_err(|e| e.to_string())?;
            win.navigate(parsed).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

#[tauri::command]
pub async fn browser_back(state: State<'_, Mutex<AppState>>, browser_id: String) -> Result<(), String> {
    if let Some(win) = state.lock().map_err(|e| e.to_string())?.browsers.get(&browser_id).and_then(|m| m.window.as_ref()).cloned() {
        win.eval("window.history.back()").map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[tauri::command]
pub async fn browser_forward(state: State<'_, Mutex<AppState>>, browser_id: String) -> Result<(), String> {
    if let Some(win) = state.lock().map_err(|e| e.to_string())?.browsers.get(&browser_id).and_then(|m| m.window.as_ref()).cloned() {
        win.eval("window.history.forward()").map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[tauri::command]
pub async fn browser_reload(state: State<'_, Mutex<AppState>>, browser_id: String) -> Result<(), String> {
    if let Some(win) = state.lock().map_err(|e| e.to_string())?.browsers.get(&browser_id).and_then(|m| m.window.as_ref()).cloned() {
        win.eval("location.reload()").map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[tauri::command]
pub async fn browser_evaluate(
    state: State<'_, Mutex<AppState>>,
    browser_id: String,
    js: String,
    call_id: String,
) -> Result<(), String> {
    if let Some(win) = state.lock().map_err(|e| e.to_string())?.browsers.get(&browser_id).and_then(|m| m.window.as_ref()).cloned() {
        let script = format!(
            r#"(async () => {{ try {{ const __r = await (async () => {{ {js} }})(); window.__TAURI_INTERNALS__?.emit('browser:eval-result', {{ id: '{call_id}', ok: true, value: JSON.stringify(__r) }}); }} catch(e) {{ window.__TAURI_INTERNALS__?.emit('browser:eval-result', {{ id: '{call_id}', ok: false, value: e.message }}); }} }})()"#
        );
        win.eval(&script).map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[tauri::command]
pub async fn browser_get_source(
    state: State<'_, Mutex<AppState>>,
    browser_id: String,
    call_id: String,
) -> Result<(), String> {
    if let Some(win) = state.lock().map_err(|e| e.to_string())?.browsers.get(&browser_id).and_then(|m| m.window.as_ref()).cloned() {
        win.eval(&format!(r#"window.__TAURI_INTERNALS__?.emit('browser:source', {{ id: '{call_id}', html: document.documentElement.outerHTML }})"#))
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[tauri::command]
pub async fn show_browser(state: State<'_, Mutex<AppState>>, browser_id: String) -> Result<(), String> {
    if let Some(win) = state.lock().map_err(|e| e.to_string())?.browsers.get(&browser_id).and_then(|m| m.window.as_ref()).cloned() {
        win.show().map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[tauri::command]
pub async fn hide_browser(state: State<'_, Mutex<AppState>>, browser_id: String) -> Result<(), String> {
    if let Some(win) = state.lock().map_err(|e| e.to_string())?.browsers.get(&browser_id).and_then(|m| m.window.as_ref()).cloned() {
        let _ = win.hide();
    }
    Ok(())
}

#[tauri::command]
pub fn close_browser(state: State<'_, Mutex<AppState>>, browser_id: String) -> Result<(), String> {
    let mut s = state.lock().map_err(|e| e.to_string())?;
    if let Some(mgr) = s.browsers.get_mut(&browser_id) {
        if let Some(w) = mgr.take_window() { let _ = w.destroy(); }
    }
    s.browsers.remove(&browser_id);
    Ok(())
}

#[tauri::command]
pub fn browser_open_devtools(state: State<'_, Mutex<AppState>>, browser_id: String) -> Result<(), String> {
    if let Some(win) = state.lock().map_err(|e| e.to_string())?.browsers.get(&browser_id).and_then(|m| m.window.as_ref()).cloned() {
        win.open_devtools();
    }
    Ok(())
}

/// Return the current URL of the active tab in a browser pane.
#[tauri::command]
pub fn get_browser_url(state: State<'_, Mutex<AppState>>, browser_id: String) -> Result<String, String> {
    Ok(state.lock().map_err(|e| e.to_string())?
        .browsers.get(&browser_id)
        .and_then(|m| m.active_url().map(|u| u.to_string()))
        .unwrap_or_default())
}

/// Get the URL of the first available browser pane (used by vmuxctl).
#[tauri::command]
pub fn get_any_browser_url(state: State<'_, Mutex<AppState>>) -> Result<String, String> {
    Ok(state.lock().map_err(|e| e.to_string())?
        .browsers.values()
        .find_map(|m| m.active_url().map(|u| u.to_string()))
        .unwrap_or_default())
}

/// Capture a screenshot of a browser pane and save it as PNG.
/// Returns the file path of the saved screenshot.
///
/// NOTE: Requires Win32 PrintWindow + GDI capture; not yet wired up.
#[tauri::command]
pub fn screenshot_browser(
    _app: AppHandle,
    state: State<'_, Mutex<AppState>>,
    browser_id: Option<String>,
    _output_path: Option<String>,
) -> Result<String, String> {
    let _win = {
        let s = state.lock().map_err(|e| e.to_string())?;
        if let Some(id) = &browser_id {
            s.browsers.get(id).and_then(|m| m.window.clone())
        } else {
            s.browsers.values().find_map(|m| m.window.clone())
        }
    }.ok_or_else(|| "no browser pane open".to_string())?;

    // TODO: capture via Win32 PrintWindow API.
    // Tauri 2.10.3 / wry 0.54.4 do not expose capture_image() yet.
    Err("Screenshot not yet supported in this build (requires Win32 PrintWindow implementation)".to_string())
}

#[tauri::command]
pub fn browser_history(state: State<'_, Mutex<AppState>>, limit: Option<usize>) -> Result<Vec<crate::context_store::BrowserHistoryEntry>, String> {
    state.lock().map_err(|e| e.to_string())?
        .context.list_history(limit.unwrap_or(50)).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn clear_browser_history(state: State<'_, Mutex<AppState>>) -> Result<(), String> {
    state.lock().map_err(|e| e.to_string())?
        .context.clear_history().map_err(|e| e.to_string())
}

// ─── Settings ─────────────────────────────────────────────────────────────────

#[tauri::command]
pub fn get_settings(state: State<'_, Mutex<AppState>>) -> Result<crate::settings::Settings, String> {
    Ok(state.lock().map_err(|e| e.to_string())?.settings.clone())
}

/// Convert a single-letter prefix key setting (e.g. "g") to its uppercase
/// virtual-key code. Falls back to VK_A if the setting isn't a plain letter.
pub fn prefix_key_to_vk(key: &str) -> u16 {
    key.chars().next()
        .map(|c| c.to_ascii_uppercase() as u16)
        .filter(|vk| (0x41..=0x5A).contains(vk))
        .unwrap_or(0x41)
}

/// Persist new settings and apply theme/font-size changes live to every
/// currently open terminal pane, and the prefix key to the native WndProc.
#[tauri::command]
pub fn update_settings(
    state: State<'_, Mutex<AppState>>,
    settings: crate::settings::Settings,
) -> Result<(), String> {
    let mut s = state.lock().map_err(|e| e.to_string())?;

    crate::settings::save(&s.context_db_path, &settings).map_err(|e| e.to_string())?;

    let theme = crate::settings::theme_from_name(&settings.theme_name);
    s.terminals.set_theme_all(&theme);
    s.terminals.set_font_size_all(settings.font_size);
    crate::terminal::window::set_prefix_vk(prefix_key_to_vk(&settings.prefix_key));

    s.theme = theme;
    s.settings = settings;
    Ok(())
}

// ─── Terminal scrollback persistence ─────────────────────────────────────────

/// Save a terminal's capture buffer to SQLite, keyed by pane_id.
/// Declared async so Tauri v2 dispatches it on the tokio thread pool instead
/// of the WebView2 main thread, keeping the Win32 message pump free.
#[tauri::command]
pub async fn save_terminal_scrollback(
    state: State<'_, Mutex<AppState>>,
    pane_id: String,
    terminal_id: String,
) -> Result<(), String> {
    let s = state.lock().map_err(|e| e.to_string())?;
    if let Some(pane) = s.terminals.panes.get(&terminal_id) {
        if let Some(ref buf) = pane.capture_buf {
            if let Ok(captured) = buf.lock() {
                // Keep last 128KB to avoid bloating the database.
                // Floor to a char boundary so we never slice mid-codepoint.
                let bytes = if captured.len() > 128_000 {
                    let start = captured.floor_char_boundary(captured.len() - 128_000);
                    captured[start..].as_bytes()
                } else {
                    captured.as_bytes()
                };
                s.context.save_scrollback(&pane_id, bytes)
                    .map_err(|e| e.to_string())?;
            }
        }
    }
    Ok(())
}

/// Replay saved scrollback into a terminal's VT grid (called after restore).
#[tauri::command]
pub async fn restore_terminal_scrollback(
    state: State<'_, Mutex<AppState>>,
    pane_id: String,
    terminal_id: String,
) -> Result<bool, String> {
    let s = state.lock().map_err(|e| e.to_string())?;
    let content = s.context.load_scrollback(&pane_id)
        .map_err(|e| e.to_string())?;
    if let Some(bytes) = content {
        if let Some(pane) = s.terminals.panes.get(&terminal_id) {
            // Feed saved output into the VT grid so it appears as scrollback
            pane.feed_grid(&bytes);
            // Also populate the capture buffer so future saves include this content
            if let Some(ref buf) = pane.capture_buf {
                if let Ok(mut b) = buf.lock() {
                    if let Ok(text) = std::str::from_utf8(&bytes) {
                        b.push_str(text);
                    }
                }
            }
            return Ok(true);
        }
    }
    Ok(false)
}

// ─── Git worktree commands ───────────────────────────────────────────────────

#[tauri::command]
pub fn create_worktree(repo_path: String, branch: String) -> Result<String, String> {
    crate::worktree::create_worktree(&repo_path, &branch).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn list_worktrees(repo_path: String) -> Result<Vec<crate::worktree::WorktreeInfo>, String> {
    crate::worktree::list_worktrees(&repo_path).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn delete_worktree(repo_path: String, branch: String) -> Result<(), String> {
    crate::worktree::delete_worktree(&repo_path, &branch).map_err(|e| e.to_string())
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

#[derive(serde::Serialize)]
pub struct DetectedAgentFile {
    pub name: String,
    pub path: String,
    pub content: String,
}

/// Look for CLAUDE.md / AGENTS.md already sitting in a project's working
/// directory, so vmux's own agent_config store can be seeded from (or kept
/// in sync with) whatever the project already has on disk.
#[tauri::command]
pub fn detect_agent_files(project_path: String) -> Vec<DetectedAgentFile> {
    const CANDIDATES: [&str; 2] = ["CLAUDE.md", "AGENTS.md"];
    CANDIDATES.iter()
        .filter_map(|name| {
            let path = std::path::Path::new(&project_path).join(name);
            let content = std::fs::read_to_string(&path).ok()?;
            Some(DetectedAgentFile { name: name.to_string(), path: path.to_string_lossy().to_string(), content })
        })
        .collect()
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
    crate::rag::run_search(db_path, config, query, project_id, top_k.unwrap_or(10)).await
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

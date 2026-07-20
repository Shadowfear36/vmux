pub mod pty;
pub mod grid;
pub mod font;
pub mod renderer;
pub mod window;
pub mod input;
pub mod shell;
pub mod agents;
pub mod cwd;
pub mod daemon_client;

use std::collections::HashMap;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;
use uuid::Uuid;
use parking_lot::Mutex;
use std::sync::Arc;
use tauri::{AppHandle, Emitter};

use self::pty::PtySession;
use self::daemon_client::DaemonPtySession;
use self::grid::{TermGrid, TermEvent};
use self::window::{TerminalWindow, WindowMessage};
use self::renderer::GpuRenderer;
use self::shell::ShellProfile;
use self::agents::AgentProfile;
use crate::theme::Theme;
use crate::osc::OscParser;

pub type TerminalId = String;

/// Convert client-area pixel coordinates to a 0-indexed grid cell, clamping
/// negative coords (e.g. a drag that overshoots above/left of the pane) to 0.
fn pixel_to_cell(x: i32, y: i32, cell_w: f32, cell_h: f32) -> (usize, usize) {
    let col = (x.max(0) as f32 / cell_w.max(1.0)) as usize;
    let row = (y.max(0) as f32 / cell_h.max(1.0)) as usize;
    (col, row)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TerminalInfo {
    pub id: TerminalId,
    pub title: String,
    pub shell_id: String,
    pub shell_name: String,
    pub working_dir: Option<String>,
    pub has_notification: bool,
    pub notification_message: Option<String>,
    pub pid: Option<u32>,
    pub is_agent: bool,
    pub agent_id: Option<String>,
    pub claude_session_id: Option<String>,
    /// Path to the notify side-channel file (for Claude hook events)
    #[serde(skip)]
    pub notify_file: Option<String>,
    /// Set when this pane's PTY is owned by the vmuxd daemon (Phase 3 of
    /// docs/session-reattach-design.md). None for local (PtySession) panes.
    #[serde(default)]
    pub daemon_session_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PaneBounds {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

impl PaneBounds {
    pub fn cols(&self, cell_w: f32) -> u16 {
        ((self.width as f32 / cell_w).floor() as u16).max(2)
    }
    pub fn rows(&self, cell_h: f32) -> u16 {
        ((self.height as f32 / cell_h).floor() as u16).max(1)
    }
}

// ─── TerminalPane ─────────────────────────────────────────────────────────────

/// One terminal pane: PTY + VT state machine + Win32 window + GPU renderer.
///
/// Creation is split into two phases to keep the UI responsive:
///   Phase 1 (sync, <5ms):  PTY spawned, ID returned to frontend immediately.
///   Phase 2 (async, lazy): Called when the frontend first reports bounds.
///                           Creates the Win32 HWND + initialises wgpu.
/// Which backend owns the actual PTY. `Daemon` is Phase 2 of
/// docs/session-reattach-design.md, gated behind VMUX_DAEMON_TERMINALS —
/// only `TerminalPane::spawn` (plain shells) uses it so far.
enum PtyBackend {
    Local(PtySession),
    Daemon(DaemonPtySession),
}

impl PtyBackend {
    /// For daemon-backed panes, tell the daemon to kill the process and
    /// drop the session from its registry — without this, closing a
    /// daemon-backed pane would leak the session forever. Local sessions
    /// need no equivalent: their process dies when PtySession is dropped.
    fn kill_if_daemon(&self) {
        if let PtyBackend::Daemon(p) = self {
            if let Err(e) = p.kill() {
                log::error!("failed to kill daemon session: {e}");
            }
        }
    }

    fn write(&self, data: &[u8]) -> Result<()> {
        match self {
            PtyBackend::Local(p) => p.write(data),
            PtyBackend::Daemon(p) => p.write(data),
        }
    }
    fn writer_handle(&self) -> Arc<std::sync::Mutex<Box<dyn std::io::Write + Send>>> {
        match self {
            PtyBackend::Local(p) => p.writer_handle(),
            PtyBackend::Daemon(p) => p.writer_handle(),
        }
    }
    fn resize(&self, cols: u16, rows: u16) -> Result<()> {
        match self {
            PtyBackend::Local(p) => p.resize(cols, rows),
            PtyBackend::Daemon(p) => p.resize(cols, rows),
        }
    }
}

pub struct TerminalPane {
    pub info: TerminalInfo,
    pty: PtyBackend,
    grid: Arc<Mutex<TermGrid>>,
    /// None until init_renderer() is called with the first real bounds.
    win: Option<TerminalWindow>,
    renderer: Option<Arc<tokio::sync::Mutex<GpuRenderer>>>,
    /// VT events from alacritty (title changes, bell) — consumed once in init_renderer.
    events_rx: Option<mpsc::UnboundedReceiver<TermEvent>>,
    /// Last PTY column/row size — used to skip no-op resizes.
    last_cols: u16,
    last_rows: u16,
    /// Captured terminal output for agent sessions (shared with PTY output task).
    pub capture_buf: Option<Arc<std::sync::Mutex<String>>>,
}

impl TerminalPane {
    // ── Phase 1: fast PTY-only creation ──────────────────────────────────────

    pub fn spawn(working_dir: Option<String>, shell: &ShellProfile) -> Result<(Self, mpsc::UnboundedReceiver<Vec<u8>>)> {
        let id = Uuid::new_v4().to_string();
        // Resolve working_dir so TerminalInfo always has a value for file tree / sidebar
        let effective_dir = working_dir.or_else(|| {
            std::env::var("USERPROFILE").or_else(|_| std::env::var("HOME")).ok()
        });
        // Start with a sensible default size; will be resized on first bounds report
        let (pty, pty_rx) = PtySession::spawn(80, 24, effective_dir.as_deref(), shell)?;
        let (grid, events_rx) = TermGrid::new(80, 24, pty.writer_handle());

        let info = TerminalInfo {
            id,
            title: shell.name.clone(),
            shell_id: shell.id.clone(),
            shell_name: shell.name.clone(),
            working_dir: effective_dir,
            has_notification: false,
            notification_message: None,
            pid: pty.pid,
            is_agent: false,
            agent_id: None,
            claude_session_id: None,
            notify_file: None,
            daemon_session_id: None,
        };

        let pane = TerminalPane {
            info,
            pty: PtyBackend::Local(pty),
            grid: Arc::new(Mutex::new(grid)),
            win: None,
            renderer: None,
            events_rx: Some(events_rx),
            last_cols: 80,
            last_rows: 24,
            capture_buf: Some(Arc::new(std::sync::Mutex::new(String::new()))),
        };
        Ok((pane, pty_rx))
    }

    /// Whether daemon-backed terminals (Phase 2 of the session-reattach
    /// design doc) are enabled. Only `spawn_maybe_daemon` (plain shells
    /// created via `create_terminal`) checks this — agent terminals and
    /// workspace restore always use the local `PtySession` path via `spawn`.
    pub fn daemon_terminals_enabled() -> bool {
        std::env::var("VMUX_DAEMON_TERMINALS").is_ok()
    }

    /// Like `spawn`, but routes through the `vmuxd` daemon when
    /// `VMUX_DAEMON_TERMINALS` is set. Async because the daemon path needs
    /// to connect over a named pipe; the local path just wraps `spawn`
    /// with no real await, so callers should invoke this without holding
    /// `Mutex<AppState>` across the `.await` (see `commands::create_terminal`).
    pub async fn spawn_maybe_daemon(working_dir: Option<String>, shell: &ShellProfile) -> Result<(Self, mpsc::UnboundedReceiver<Vec<u8>>)> {
        if !Self::daemon_terminals_enabled() {
            return Self::spawn(working_dir, shell);
        }

        let id = Uuid::new_v4().to_string();
        let effective_dir = working_dir.or_else(|| {
            std::env::var("USERPROFILE").or_else(|_| std::env::var("HOME")).ok()
        });

        let (pty, pty_rx) = DaemonPtySession::spawn(
            &shell.path, &shell.args, effective_dir.as_deref(), 80, 24,
        ).await?;
        let (grid, events_rx) = TermGrid::new(80, 24, pty.writer_handle());

        let info = TerminalInfo {
            id,
            title: shell.name.clone(),
            shell_id: shell.id.clone(),
            shell_name: shell.name.clone(),
            working_dir: effective_dir,
            has_notification: false,
            notification_message: None,
            pid: pty.pid,
            is_agent: false,
            agent_id: None,
            claude_session_id: None,
            notify_file: None,
            daemon_session_id: Some(pty.session_id.clone()),
        };

        let pane = TerminalPane {
            info,
            pty: PtyBackend::Daemon(pty),
            grid: Arc::new(Mutex::new(grid)),
            win: None,
            renderer: None,
            events_rx: Some(events_rx),
            last_cols: 80,
            last_rows: 24,
            capture_buf: Some(Arc::new(std::sync::Mutex::new(String::new()))),
        };
        Ok((pane, pty_rx))
    }

    /// Reattach to a daemon session persisted from a previous vmux run
    /// (Phase 3 of the session-reattach design doc). Returns an error if
    /// the daemon is unreachable or the session no longer exists — the
    /// caller (workspace restore) should fall back to `spawn` in that case.
    pub async fn attach_daemon(session_id: &str, working_dir: Option<String>, shell: &ShellProfile) -> Result<(Self, mpsc::UnboundedReceiver<Vec<u8>>)> {
        let id = Uuid::new_v4().to_string();
        let (pty, pty_rx) = DaemonPtySession::attach(session_id, 80, 24).await?;
        let (grid, events_rx) = TermGrid::new(80, 24, pty.writer_handle());

        let info = TerminalInfo {
            id,
            title: shell.name.clone(),
            shell_id: shell.id.clone(),
            shell_name: shell.name.clone(),
            working_dir,
            has_notification: false,
            notification_message: None,
            pid: pty.pid,
            is_agent: false,
            agent_id: None,
            claude_session_id: None,
            notify_file: None,
            daemon_session_id: Some(pty.session_id.clone()),
        };

        let pane = TerminalPane {
            info,
            pty: PtyBackend::Daemon(pty),
            grid: Arc::new(Mutex::new(grid)),
            win: None,
            renderer: None,
            events_rx: Some(events_rx),
            last_cols: 80,
            last_rows: 24,
            capture_buf: Some(Arc::new(std::sync::Mutex::new(String::new()))),
        };
        Ok((pane, pty_rx))
    }

    // ── Phase 1b: agent PTY creation ────────────────────────────────────────

    pub fn spawn_agent(
        working_dir: Option<String>,
        agent: &AgentProfile,
        session_name: Option<String>,
        resume_session: Option<String>,
        continue_session: bool,
    ) -> Result<(Self, mpsc::UnboundedReceiver<Vec<u8>>)> {
        let id = Uuid::new_v4().to_string();
        let effective_dir = working_dir.or_else(|| {
            std::env::var("USERPROFILE").or_else(|_| std::env::var("HOME")).ok()
        });

        // Build args and env, with Claude-specific enhancements
        let mut args = agent.args.clone();
        let mut env = agent.env.clone();
        let mut notify_file: Option<String> = None;

        if agent.id == "claude" {
            env.push(("VMUX".into(), "1".into()));

            let notify_dir = std::env::temp_dir().join("vmux");
            let _ = std::fs::create_dir_all(&notify_dir);
            let notify_path = notify_dir.join(format!("{}.notify", &id));
            let _ = std::fs::File::create(&notify_path);
            let path_str = notify_path.to_string_lossy().to_string();
            env.push(("VMUX_NOTIFY_FILE".into(), path_str.clone()));
            notify_file = Some(path_str);

            if let Some(name) = &session_name {
                args.push("--name".into());
                args.push(name.clone());
            }

            // Session persistence: --continue resumes last session in CWD,
            // --resume <id> resumes a specific session
            if continue_session {
                args.push("--continue".into());
            } else if let Some(sid) = &resume_session {
                args.push("--resume".into());
                args.push(sid.clone());
            }
        }

        let (pty, pty_rx) = PtySession::spawn_command(
            80, 24, effective_dir.as_deref(),
            &agent.command, &args, &env,
        )?;
        let (grid, events_rx) = TermGrid::new(80, 24, pty.writer_handle());

        let info = TerminalInfo {
            id,
            title: agent.name.clone(),
            shell_id: agent.id.clone(),
            shell_name: agent.name.clone(),
            working_dir: effective_dir,
            has_notification: false,
            notification_message: None,
            pid: pty.pid,
            is_agent: true,
            agent_id: Some(agent.id.clone()),
            claude_session_id: None,
            notify_file,
            daemon_session_id: None,
        };

        // Agent terminals capture output for context store
        let capture_buf = Some(Arc::new(std::sync::Mutex::new(String::new())));

        let pane = TerminalPane {
            info,
            pty: PtyBackend::Local(pty),
            grid: Arc::new(Mutex::new(grid)),
            win: None,
            renderer: None,
            events_rx: Some(events_rx),
            last_cols: 80,
            last_rows: 24,
            capture_buf,
        };
        Ok((pane, pty_rx))
    }

    // ── Phase 2: renderer + window init (called once, from async context) ────

    pub async fn init_renderer(
        &mut self,
        app: &AppHandle,
        parent_hwnd: isize,
        bounds: PaneBounds,
        notification_tx: mpsc::UnboundedSender<(TerminalId, String)>,
        theme: Theme,
        font_size: f32,
        pty_rx: mpsc::UnboundedReceiver<Vec<u8>>,
    ) -> Result<()> {
        // Create Win32 child window on the main thread, passing PTY writer
        // so it can handle keyboard input natively in the WndProc.
        let (win, mut win_rx) = TerminalWindow::create_on_main_thread(
            app, parent_hwnd,
            bounds.x, bounds.y, bounds.width, bounds.height,
            self.pty.writer_handle(),
        ).await?;
        let hwnd = win.hwnd_isize();

        // Clamp bounds to reasonable minimums for init — transient 16px-wide
        // bounds from Allotment layout cause 2-column PTY and garbled text.
        let init_w = (bounds.width as u32).max(200);
        let init_h = (bounds.height as u32).max(100);

        // Initialise wgpu renderer (async, ~200ms)
        let renderer = GpuRenderer::new(hwnd, init_w, init_h, theme, font_size).await?;

        // Use the renderer's actual font metrics for PTY/grid sizing.
        let cell_w = renderer.font.cell_width;
        let cell_h = renderer.font.cell_height;
        let init_bounds = PaneBounds { x: bounds.x, y: bounds.y, width: init_w as i32, height: init_h as i32 };
        let cols = init_bounds.cols(cell_w);
        let rows = init_bounds.rows(cell_h);
        self.pty.resize(cols, rows)?;
        self.grid.lock().resize(cols, rows);

        let renderer       = Arc::new(tokio::sync::Mutex::new(renderer));
        let renderer_pty   = renderer.clone();   // PTY output task
        let renderer_inp   = renderer.clone();   // input task (scroll)
        let renderer_blink = renderer.clone();   // cursor blink task

        self.win      = Some(win);
        self.renderer = Some(renderer);

        // ── Task: PTY output → VT state machine → render ─────────────────────
        let id = self.info.id.clone();
        let grid_pty = self.grid.clone();
        let notif_tx = notification_tx;
        let app_osc = app.clone();
        let capture = self.capture_buf.clone();
        tokio::spawn(async move {
            use crate::osc::OscAction;
            let mut osc = OscParser::new();
            let mut rx  = pty_rx;
            while let Some(bytes) = rx.recv().await {
                let text = String::from_utf8_lossy(&bytes);

                // Capture output for agent terminals (for context store)
                if let Some(ref buf) = capture {
                    if let Ok(mut b) = buf.lock() {
                        // Cap at 2MB to prevent unbounded growth
                        if b.len() < 2_000_000 {
                            b.push_str(&text);
                        }
                    }
                }

                // Parse all OSC actions (notifications + browser + cwd)
                for action in osc.parse_actions(&text) {
                    match action {
                        OscAction::Notification(msg) => {
                            let _ = notif_tx.send((id.clone(), msg));
                        }
                        OscAction::CwdChanged(path) => {
                            let _ = app_osc.emit("terminal:cwd",
                                serde_json::json!({ "terminalId": id, "cwd": path }));
                        }
                        OscAction::BrowserOpen(url) => {
                            let _ = app_osc.emit("agent:browser-open",
                                serde_json::json!({ "terminalId": id, "url": url }));
                        }
                        OscAction::BrowserNavigate(url) => {
                            let _ = app_osc.emit("agent:browser-navigate",
                                serde_json::json!({ "terminalId": id, "url": url }));
                        }
                        OscAction::BrowserClose => {
                            let _ = app_osc.emit("agent:browser-close",
                                serde_json::json!({ "terminalId": id }));
                        }
                        OscAction::BrowserEval(js) => {
                            let _ = app_osc.emit("agent:browser-eval",
                                serde_json::json!({ "terminalId": id, "js": js }));
                        }
                    }
                }

                // Drain any additional buffered chunks before rendering
                // to batch multiple rapid outputs into one render call.
                grid_pty.lock().process(&bytes);
                while let Ok(more) = rx.try_recv() {
                    grid_pty.lock().process(&more);
                }
                let snap = grid_pty.lock().snapshot();
                if let Ok(mut r) = renderer_pty.try_lock() {
                    let _ = r.render(&snap);
                }
            }
        });

        // ── Task: VT title events → frontend ─────────────────────────────────
        if let Some(events_rx) = self.events_rx.take() {
            let id_t  = self.info.id.clone();
            let app_t = app.clone();
            tokio::spawn(async move {
                let mut rx = events_rx;
                while let Some(ev) = rx.recv().await {
                    if let TermEvent::TitleChanged(title) = ev {
                        let _ = app_t.emit("terminal:title",
                            serde_json::json!({ "terminalId": id_t, "title": title }));
                    }
                }
            });
        }

        // ── Task: Cursor blink (~530ms period) ───────────────────────────────
        let grid_blink = self.grid.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(
                std::time::Duration::from_millis(530)
            );
            loop {
                interval.tick().await;
                // Use try_lock — yield to PTY output which is higher priority.
                if let Ok(mut r) = renderer_blink.try_lock() {
                    r.toggle_cursor_blink();
                    let snap = grid_blink.lock().snapshot();
                    let _ = r.render(&snap);
                }
            }
        });

        // ── Task: CWD polling (fallback for shells that don't emit OSC 7) ────
        if let Some(pid) = self.info.pid {
            let id_cwd  = self.info.id.clone();
            let app_cwd = app.clone();
            tokio::spawn(async move {
                let mut last_cwd = String::new();
                let mut interval = tokio::time::interval(
                    std::time::Duration::from_secs(2)
                );
                loop {
                    interval.tick().await;
                    if let Some(cwd) = cwd::get_process_cwd(pid) {
                        if cwd != last_cwd {
                            last_cwd = cwd.clone();
                            let _ = app_cwd.emit("terminal:cwd",
                                serde_json::json!({ "terminalId": id_cwd, "cwd": cwd }));
                        }
                    }
                }
            });
        }

        // ── Task: Win32 mouse events → scroll / click notification ───────────
        let grid_inp   = self.grid.clone();
        let id_click   = self.info.id.clone();
        let app_click  = app.clone();

        tokio::spawn(async move {
            use crate::terminal::input::InputEvent;
            while let Some(msg) = win_rx.recv().await {
                match msg {
                    // Scroll wheel: move the view, re-render (no PTY write)
                    WindowMessage::Input(InputEvent::Scroll(delta)) => {
                        grid_inp.lock().scroll(delta);
                        let snap = grid_inp.lock().snapshot();
                        if let Ok(mut r) = renderer_inp.try_lock() {
                            let _ = r.render(&snap);
                        }
                    }
                    // Text selection: convert client pixel coords to a grid
                    // cell using the renderer's actual font metrics.
                    WindowMessage::SelectionStart(x, y) => {
                        if let Ok(r) = renderer_inp.try_lock() {
                            let (col, row) = pixel_to_cell(x, y, r.font.cell_width, r.font.cell_height);
                            drop(r);
                            grid_inp.lock().start_selection(col, row);
                            let snap = grid_inp.lock().snapshot();
                            if let Ok(mut r) = renderer_inp.try_lock() {
                                let _ = r.render(&snap);
                            }
                        }
                    }
                    WindowMessage::SelectionUpdate(x, y) => {
                        if let Ok(r) = renderer_inp.try_lock() {
                            let (col, row) = pixel_to_cell(x, y, r.font.cell_width, r.font.cell_height);
                            drop(r);
                            grid_inp.lock().update_selection(col, row);
                            let snap = grid_inp.lock().snapshot();
                            if let Ok(mut r) = renderer_inp.try_lock() {
                                let _ = r.render(&snap);
                            }
                        }
                    }
                    WindowMessage::CopySelection => {
                        if let Some(text) = grid_inp.lock().selection_text() {
                            unsafe {
                                let win_hwnd = windows::Win32::Foundation::HWND(hwnd as *mut _);
                                crate::terminal::window::write_clipboard(win_hwnd, &text);
                            }
                        }
                    }
                    // Click: tell the frontend which terminal has focus
                    WindowMessage::Clicked => {
                        let _ = app_click.emit("terminal:clicked",
                            serde_json::json!({ "terminalId": id_click }));
                    }
                    // Prefix system events → frontend
                    WindowMessage::PrefixActivated => {
                        let _ = app_click.emit("prefix:activated",
                            serde_json::json!({ "terminalId": id_click }));
                    }
                    WindowMessage::PrefixDeactivated => {
                        let _ = app_click.emit("prefix:deactivated",
                            serde_json::json!({}));
                    }
                    WindowMessage::PrefixCommand(key) => {
                        let _ = app_click.emit("prefix:command",
                            serde_json::json!({ "key": key }));
                    }
                    WindowMessage::Resize(_, _) => {
                        // Handled in set_bounds via ResizeObserver
                    }
                    WindowMessage::Close => break,
                    // Input/Paste are now handled entirely in React
                    _ => {}
                }
            }
        });

        Ok(())
    }

    pub fn write_input(&mut self, data: &[u8]) -> Result<()> {
        self.pty.write(data)
    }

    pub fn set_bounds(&mut self, bounds: &PaneBounds) {
        // Always reposition the window (even if tiny — keeps it in sync with layout).
        if let Some(win) = &self.win {
            win.set_bounds(bounds.x, bounds.y, bounds.width, bounds.height);
        }

        // Skip PTY/grid/renderer resize for transient tiny bounds (e.g. during
        // Allotment layout animation). Resizing the PTY to 2 cols causes cmd.exe
        // to re-wrap all text, creating the "vertical waterfall" glitch.
        if bounds.width < 64 || bounds.height < 32 {
            return;
        }

        // Use actual font metrics from renderer; fall back to defaults before init.
        let (cell_w, cell_h) = if let Some(r) = &self.renderer {
            if let Ok(r) = r.try_lock() { (r.font.cell_width, r.font.cell_height) }
            else { (8.0f32, 16.0f32) }
        } else { (8.0f32, 16.0f32) };

        let cols = bounds.cols(cell_w);
        let rows = bounds.rows(cell_h);

        // Resize the wgpu surface to match the new pixel dimensions.
        if let Some(r) = &self.renderer {
            if let Ok(mut renderer) = r.try_lock() {
                renderer.resize(bounds.width as u32, bounds.height as u32);
            }
        }

        // Only resize PTY/grid when the cell dimensions actually change.
        // This avoids flooding cmd.exe with resize events on every pixel change
        // during window drag, which causes text re-wrapping glitches.
        if cols != self.last_cols || rows != self.last_rows {
            self.last_cols = cols;
            self.last_rows = rows;
            let _ = self.pty.resize(cols, rows);
            self.grid.lock().resize(cols, rows);
        }

        // Re-render with the new surface size so the display isn't stale.
        if let Some(r) = &self.renderer {
            if let Ok(mut renderer) = r.try_lock() {
                let snap = self.grid.lock().snapshot();
                let _ = renderer.render(&snap);
            }
        }
    }

    pub fn focus(&self) {
        if let Some(win) = &self.win { win.focus(); }
    }

    pub fn show(&self) {
        if let Some(win) = &self.win { win.show(); }
    }

    pub fn hide(&self) {
        if let Some(win) = &self.win { win.hide(); }
    }

    #[allow(dead_code)]
    pub fn is_renderer_ready(&self) -> bool {
        self.win.is_some()
    }

    pub fn set_notification(&mut self, message: Option<String>) {
        self.info.has_notification = message.is_some();
        self.info.notification_message = message;
    }

    /// Feed raw bytes into the VT grid (used to replay saved scrollback).
    pub fn feed_grid(&self, data: &[u8]) {
        self.grid.lock().process(data);
    }

    /// Apply a new theme live — colors are read at render time, so this just
    /// swaps the renderer's theme and re-renders the current grid snapshot.
    pub fn set_theme(&mut self, theme: Theme) {
        if let Some(r) = &self.renderer {
            if let Ok(mut renderer) = r.try_lock() {
                renderer.theme = theme;
                let snap = self.grid.lock().snapshot();
                let _ = renderer.render(&snap);
            }
        }
    }

    /// Apply a new font size live: rebuilds the renderer's font/atlas, then
    /// re-derives cols/rows from the new cell metrics and resizes the
    /// PTY/grid if they changed (mirrors the resize logic in `set_bounds`).
    pub fn set_font_size(&mut self, font_size: f32) {
        if let Some(r) = &self.renderer {
            if let Ok(mut renderer) = r.try_lock() {
                renderer.set_font_size(font_size);
                let cell_w = renderer.font.cell_width;
                let cell_h = renderer.font.cell_height;
                let cols = ((renderer.width as f32 / cell_w).floor() as u16).max(2);
                let rows = ((renderer.height as f32 / cell_h).floor() as u16).max(1);

                if cols != self.last_cols || rows != self.last_rows {
                    self.last_cols = cols;
                    self.last_rows = rows;
                    let _ = self.pty.resize(cols, rows);
                    self.grid.lock().resize(cols, rows);
                }

                let snap = self.grid.lock().snapshot();
                let _ = renderer.render(&snap);
            }
        }
    }
}

// ─── TerminalManager ──────────────────────────────────────────────────────────

/// Holds all terminal panes and their pending PTY readers (before renderer init).
pub struct TerminalManager {
    pub panes: HashMap<TerminalId, TerminalPane>,
    /// Pending PTY readers for panes that haven't had init_renderer called yet.
    pub pending_rx: HashMap<TerminalId, mpsc::UnboundedReceiver<Vec<u8>>>,
}

impl TerminalManager {
    pub fn new() -> Self {
        TerminalManager { panes: HashMap::new(), pending_rx: HashMap::new() }
    }

    /// Returns true if this terminal still needs renderer init (first bounds call).
    pub fn panes_needing_init(&self, id: &str) -> bool {
        self.pending_rx.contains_key(id)
    }

    /// Phase 1: spawn PTY, return ID immediately (sync, <5ms).
    pub fn spawn(&mut self, working_dir: Option<String>, shell: &ShellProfile) -> Result<TerminalInfo> {
        let (pane, pty_rx) = TerminalPane::spawn(working_dir, shell)?;
        let info = pane.info.clone();
        let id = info.id.clone();
        self.panes.insert(id.clone(), pane);
        self.pending_rx.insert(id, pty_rx);
        Ok(info)
    }

    /// Phase 1: spawn agent CLI in a PTY, return ID immediately.
    pub fn spawn_agent(
        &mut self,
        working_dir: Option<String>,
        agent: &AgentProfile,
        session_name: Option<String>,
        resume_session: Option<String>,
        continue_session: bool,
    ) -> Result<TerminalInfo> {
        let (pane, pty_rx) = TerminalPane::spawn_agent(working_dir, agent, session_name, resume_session, continue_session)?;
        let info = pane.info.clone();
        let id = info.id.clone();
        self.panes.insert(id.clone(), pane);
        self.pending_rx.insert(id, pty_rx);
        Ok(info)
    }

    /// Register a pane constructed outside the manager (e.g. via
    /// `TerminalPane::spawn_maybe_daemon`, which must run without holding
    /// `Mutex<AppState>` across its `.await`) along with its pending PTY
    /// output receiver, exactly as `spawn`/`spawn_agent` do internally.
    pub fn insert(&mut self, pane: TerminalPane, pty_rx: mpsc::UnboundedReceiver<Vec<u8>>) -> TerminalInfo {
        let info = pane.info.clone();
        let id = info.id.clone();
        self.panes.insert(id.clone(), pane);
        self.pending_rx.insert(id, pty_rx);
        info
    }

    pub fn write(&mut self, id: &str, data: &[u8]) -> Result<()> {
        self.panes.get_mut(id)
            .ok_or_else(|| anyhow::anyhow!("terminal not found: {id}"))?
            .write_input(data)
    }

    pub fn set_bounds(&mut self, id: &str, bounds: &PaneBounds) {
        if let Some(pane) = self.panes.get_mut(id) {
            pane.set_bounds(bounds);
        }
    }

    /// Apply a theme change to every currently open pane.
    pub fn set_theme_all(&mut self, theme: &Theme) {
        for pane in self.panes.values_mut() {
            pane.set_theme(theme.clone());
        }
    }

    /// Apply a font size change to every currently open pane.
    pub fn set_font_size_all(&mut self, font_size: f32) {
        for pane in self.panes.values_mut() {
            pane.set_font_size(font_size);
        }
    }

    pub fn focus(&self, id: &str) {
        if let Some(pane) = self.panes.get(id) { pane.focus(); }
    }

    pub fn show(&self, id: &str) {
        if let Some(pane) = self.panes.get(id) { pane.show(); }
    }

    pub fn hide(&self, id: &str) {
        if let Some(pane) = self.panes.get(id) { pane.hide(); }
    }

    pub fn close(&mut self, id: &str) {
        if let Some(pane) = self.panes.get(id) {
            pane.pty.kill_if_daemon();
        }
        self.panes.remove(id);
        self.pending_rx.remove(id);
    }

    pub fn list(&self) -> Vec<TerminalInfo> {
        self.panes.values().map(|p| p.info.clone()).collect()
    }

    pub fn set_notification(&mut self, id: &str, message: Option<String>) {
        if let Some(p) = self.panes.get_mut(id) { p.set_notification(message); }
    }
}

unsafe impl Send for TerminalManager {}
unsafe impl Sync for TerminalManager {}


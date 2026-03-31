pub mod pty;
pub mod grid;
pub mod font;
pub mod renderer;
pub mod window;
pub mod input;
pub mod shell;
pub mod agents;

use std::collections::HashMap;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;
use uuid::Uuid;
use parking_lot::Mutex;
use std::sync::Arc;
use tauri::{AppHandle, Emitter};

use self::pty::PtySession;
use self::grid::{TermGrid, TermEvent};
use self::window::{TerminalWindow, WindowMessage};
use self::renderer::GpuRenderer;
use self::shell::ShellProfile;
use self::agents::AgentProfile;
use crate::theme::Theme;
use crate::osc::OscParser;

pub type TerminalId = String;

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
pub struct TerminalPane {
    pub info: TerminalInfo,
    pty: PtySession,
    grid: Arc<Mutex<TermGrid>>,
    /// None until init_renderer() is called with the first real bounds.
    win: Option<TerminalWindow>,
    renderer: Option<Arc<tokio::sync::Mutex<GpuRenderer>>>,
    /// VT events from alacritty (title changes, bell) — consumed once in init_renderer.
    events_rx: Option<mpsc::UnboundedReceiver<TermEvent>>,
    /// Last PTY column/row size — used to skip no-op resizes.
    last_cols: u16,
    last_rows: u16,
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
        };

        let pane = TerminalPane {
            info,
            pty,
            grid: Arc::new(Mutex::new(grid)),
            win: None,
            renderer: None,
            events_rx: Some(events_rx),
            last_cols: 80,
            last_rows: 24,
        };
        Ok((pane, pty_rx))
    }

    // ── Phase 1b: agent PTY creation ────────────────────────────────────────

    pub fn spawn_agent(working_dir: Option<String>, agent: &AgentProfile) -> Result<(Self, mpsc::UnboundedReceiver<Vec<u8>>)> {
        let id = Uuid::new_v4().to_string();
        let effective_dir = working_dir.or_else(|| {
            std::env::var("USERPROFILE").or_else(|_| std::env::var("HOME")).ok()
        });
        let (pty, pty_rx) = PtySession::spawn_command(
            80, 24, effective_dir.as_deref(),
            &agent.command, &agent.args, &agent.env,
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
        };

        let pane = TerminalPane {
            info,
            pty,
            grid: Arc::new(Mutex::new(grid)),
            win: None,
            renderer: None,
            events_rx: Some(events_rx),
            last_cols: 80,
            last_rows: 24,
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
        let renderer = GpuRenderer::new(hwnd, init_w, init_h, theme).await?;

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
        tokio::spawn(async move {
            use crate::osc::OscAction;
            let mut osc = OscParser::new();
            let mut rx  = pty_rx;
            while let Some(bytes) = rx.recv().await {
                let text = String::from_utf8_lossy(&bytes);

                // Parse all OSC actions (notifications + browser commands)
                for action in osc.parse_actions(&text) {
                    match action {
                        OscAction::Notification(msg) => {
                            let _ = notif_tx.send((id.clone(), msg));
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
                    WindowMessage::PrefixCommand(ch) => {
                        let _ = app_click.emit("prefix:command",
                            serde_json::json!({ "key": ch.to_string() }));
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
    pub fn spawn_agent(&mut self, working_dir: Option<String>, agent: &AgentProfile) -> Result<TerminalInfo> {
        let (pane, pty_rx) = TerminalPane::spawn_agent(working_dir, agent)?;
        let info = pane.info.clone();
        let id = info.id.clone();
        self.panes.insert(id.clone(), pane);
        self.pending_rx.insert(id, pty_rx);
        Ok(info)
    }

    #[allow(dead_code)]
    pub fn insert(&mut self, pane: TerminalPane) -> TerminalInfo {
        let info = pane.info.clone();
        self.panes.insert(info.id.clone(), pane);
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


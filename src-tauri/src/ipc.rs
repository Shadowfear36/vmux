/// vmux IPC server — named pipe control interface.
///
/// Allows external tools (the `vmux` CLI, scripts) to query and control a
/// running vmux instance without being inside a terminal pane.
///
/// Pipe name: `\\.\pipe\vmux-ipc`
///
/// Protocol: length-prefixed JSON (u32-LE length then UTF-8 JSON body),
/// identical framing to the vmuxd daemon. One request → one response →
/// connection closes.

use std::sync::Mutex;

use anyhow::Result;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager};
use tokio::net::windows::named_pipe::ServerOptions;

use crate::state::AppState;
use crate::terminal::daemon_client::{read_framed, write_framed};

pub const PIPE_NAME: &str = r"\\.\pipe\vmux-ipc";

// ─── Protocol types ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum IpcRequest {
    Ping,
    /// Emit a notification on the first open terminal (or `__global__` if
    /// no terminals are open). Triggers the same sidebar badge as an OSC 9
    /// notification emitted from inside the terminal.
    Notify { message: String },
    /// Return a summary of every open terminal pane.
    List,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TerminalSummary {
    pub id: String,
    pub title: String,
    pub is_agent: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum IpcResponse {
    Pong,
    Ok,
    Terminals(Vec<TerminalSummary>),
    Error(String),
}

// ─── Server ────────────────────────────────────────────────────────────────────

/// Start the IPC pipe server in a background thread with its own Tokio runtime.
///
/// Only one vmux instance should hold the pipe at a time; if the pipe creation
/// fails (e.g. another instance is running) the function returns without error
/// and the IPC feature is simply unavailable for this instance.
pub fn start_ipc_server(app: AppHandle) {
    std::thread::spawn(move || {
        let rt = match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(rt) => rt,
            Err(e) => {
                log::error!("ipc: failed to create runtime: {e}");
                return;
            }
        };
        rt.block_on(serve(app));
    });
}

async fn serve(app: AppHandle) {
    loop {
        let server = match ServerOptions::new().create(PIPE_NAME) {
            Ok(s) => s,
            Err(e) => {
                // Another vmux instance already owns the pipe — not an error.
                log::debug!("ipc: could not create named pipe (another instance?): {e}");
                return;
            }
        };
        match server.connect().await {
            Ok(()) => {}
            Err(e) => {
                log::debug!("ipc: pipe connect error: {e}");
                continue;
            }
        }
        let app = app.clone();
        tokio::spawn(async move {
            if let Err(e) = handle_client(server, app).await {
                log::debug!("ipc: client error: {e}");
            }
        });
    }
}

async fn handle_client(
    pipe: tokio::net::windows::named_pipe::NamedPipeServer,
    app: AppHandle,
) -> Result<()> {
    let (mut r, mut w) = tokio::io::split(pipe);

    let response = match read_framed::<IpcRequest, _>(&mut r).await? {
        None => return Ok(()),
        Some(IpcRequest::Ping) => IpcResponse::Pong,
        Some(IpcRequest::Notify { message }) => {
            let terminal_id = {
                let state = app.state::<Mutex<AppState>>();
                let s = state.lock().unwrap();
                s.terminals
                    .list()
                    .into_iter()
                    .next()
                    .map(|info| info.id)
                    .unwrap_or_else(|| "__global__".to_string())
            };
            let _ = app.emit(
                "terminal:notification",
                serde_json::json!({
                    "terminalId": terminal_id,
                    "message": message,
                }),
            );
            IpcResponse::Ok
        }
        Some(IpcRequest::List) => {
            let state = app.state::<Mutex<AppState>>();
            let s = state.lock().unwrap();
            let summaries = s
                .terminals
                .list()
                .into_iter()
                .map(|info| TerminalSummary {
                    id: info.id,
                    title: info.title,
                    is_agent: info.is_agent,
                })
                .collect();
            IpcResponse::Terminals(summaries)
        }
    };

    write_framed(&mut w, &response).await?;
    Ok(())
}

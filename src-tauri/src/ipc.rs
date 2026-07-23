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
use std::time::Duration;

use anyhow::Result;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Listener, Manager};
use tokio::net::windows::named_pipe::ServerOptions;

use crate::state::AppState;
use crate::terminal::daemon_client::{read_framed, write_framed};

// For screenshot encoding (png) and temp-path generation (uuid)
extern crate png;

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
    /// Navigate the first available browser pane to a URL.
    BrowserNavigate { url: String },
    /// Capture a screenshot of the first available browser pane.
    /// `output_path` is optional; if None a temp file is used and the path is returned.
    BrowserScreenshot { output_path: Option<String> },
    /// Return the current URL of the first available browser pane.
    BrowserGetUrl,
    /// Evaluate JavaScript in the first available browser pane and wait for
    /// its return value (unlike the OSC `browser-eval` action, which is
    /// fire-and-forget). The script's last expression's value is captured,
    /// JSON-encoded, and sent back over the pipe.
    BrowserEval { js: String },
    /// Semantic search across all imported conversation history + notes.
    ContextSearch { query: String, top_k: Option<usize> },
    /// Fetch every chunk of a specific conversation, in order — for pulling
    /// the full thread once ContextSearch has found the relevant one.
    ContextGetConversation { conversation_id: String },
    /// Backs the `vmux [dir]` CLI: find-or-create a workspace for `path`
    /// and make it active, then bring the main window to front. `path:
    /// None` (bare `vmux` with no directory) just focuses the window —
    /// this is also how `vmux`/`vmux <dir>` enforce single-instance: `main.rs`
    /// tries this request first, and only launches a new GUI process if
    /// no instance is reachable to send it to.
    OpenPath { path: Option<String> },
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
    /// Path to the saved screenshot file.
    Screenshot { path: String },
    /// Current browser URL.
    BrowserUrl { url: String },
    /// Result of a BrowserEval request. `ok` is false if the script threw;
    /// `value` is the JSON-encoded return value (or the error message).
    BrowserEvalResult { ok: bool, value: String },
    /// Results of a ContextSearch request, most relevant first.
    ContextSearchResults(Vec<ContextSearchHit>),
    /// Full contents of a conversation, in order.
    ContextConversation { title: Option<String>, project_name: String, chunks: Vec<ContextChunkOut> },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextSearchHit {
    pub conversation_id: String,
    pub conversation_title: Option<String>,
    pub project_name: String,
    pub role: String,
    pub score: f32,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextChunkOut {
    pub role: String,
    pub content: String,
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
        Some(IpcRequest::BrowserNavigate { url }) => {
            // Emit an event; the Tauri frontend handles navigation so we don't
            // need to call the command directly from the IPC thread.
            let _ = app.emit("ipc:browser-navigate", serde_json::json!({ "url": url }));
            IpcResponse::Ok
        }
        Some(IpcRequest::BrowserGetUrl) => {
            let url = {
                let state = app.state::<Mutex<AppState>>();
                let s = state.lock().unwrap();
                s.browsers.values()
                    .find_map(|m| m.active_url().map(|u| u.to_string()))
                    .unwrap_or_default()
            };
            IpcResponse::BrowserUrl { url }
        }
        Some(IpcRequest::BrowserScreenshot { .. }) => {
            // TODO: implement via Win32 PrintWindow.
            // Tauri 2.10.3 / wry 0.54.4 do not expose capture_image() yet.
            IpcResponse::Error("Screenshot not yet supported in this build".to_string())
        }
        Some(IpcRequest::BrowserEval { js }) => browser_eval(&app, &js).await,
        Some(IpcRequest::ContextSearch { query, top_k }) => context_search(&app, query, top_k).await,
        Some(IpcRequest::ContextGetConversation { conversation_id }) => context_get_conversation(&app, &conversation_id),
        Some(IpcRequest::OpenPath { path }) => {
            let result = if let Some(path) = &path {
                let state = app.state::<Mutex<AppState>>();
                let mut s = state.lock().unwrap();
                s.open_path_as_workspace(path)
            } else {
                Ok(String::new())
            };
            match result {
                Ok(workspace_id) => {
                    if !workspace_id.is_empty() {
                        let _ = app.emit("vmux:workspace-opened", serde_json::json!({ "workspaceId": workspace_id }));
                    }
                    if let Some(window) = app.get_webview_window("main") {
                        let _ = window.unminimize();
                        let _ = window.show();
                        let _ = window.set_focus();
                    }
                    IpcResponse::Ok
                }
                Err(e) => IpcResponse::Error(e.to_string()),
            }
        }
    };

    write_framed(&mut w, &response).await?;
    Ok(())
}

/// Evaluate JavaScript in the first available browser pane and wait (with a
/// timeout) for its result. The eval script wraps the caller's JS the same
/// way `commands::browser_evaluate` does, emitting a `browser:eval-result`
/// event with a matching call ID; we register a one-shot listener for that
/// event here so the IPC caller (unlike the fire-and-forget OSC path) gets
/// an actual return value back over the pipe.
async fn browser_eval(app: &AppHandle, js: &str) -> IpcResponse {
    let win = {
        let state = app.state::<Mutex<AppState>>();
        let s = state.lock().unwrap();
        s.browsers.values().find_map(|m| m.window.clone())
    };
    let Some(win) = win else {
        return IpcResponse::Error("no browser pane open".to_string());
    };

    let call_id = uuid::Uuid::new_v4().to_string();
    let script = format!(
        r#"(async () => {{ try {{ const __r = await (async () => {{ {js} }})(); window.__TAURI_INTERNALS__?.emit('browser:eval-result', {{ id: '{call_id}', ok: true, value: JSON.stringify(__r) }}); }} catch(e) {{ window.__TAURI_INTERNALS__?.emit('browser:eval-result', {{ id: '{call_id}', ok: false, value: e.message }}); }} }})()"#
    );

    let (tx, rx) = tokio::sync::oneshot::channel();
    let tx = std::sync::Mutex::new(Some(tx));
    let cid = call_id.clone();
    let handler_id = app.listen_any("browser:eval-result", move |event| {
        let Ok(payload) = serde_json::from_str::<serde_json::Value>(event.payload()) else { return };
        if payload.get("id").and_then(|v| v.as_str()) != Some(cid.as_str()) {
            return;
        }
        if let Some(tx) = tx.lock().unwrap().take() {
            let _ = tx.send(payload);
        }
    });

    if let Err(e) = win.eval(&script) {
        app.unlisten(handler_id);
        return IpcResponse::Error(e.to_string());
    }

    let result = tokio::time::timeout(Duration::from_secs(10), rx).await;
    app.unlisten(handler_id);

    match result {
        Ok(Ok(payload)) => IpcResponse::BrowserEvalResult {
            ok: payload.get("ok").and_then(|v| v.as_bool()).unwrap_or(false),
            value: payload.get("value").and_then(|v| v.as_str()).unwrap_or("").to_string(),
        },
        _ => IpcResponse::Error("browser eval timed out after 10s".to_string()),
    }
}

/// Semantic search across all imported conversation history, via the same
/// embed-then-search flow the Search tab uses (`rag::run_search`).
async fn context_search(app: &AppHandle, query: String, top_k: Option<usize>) -> IpcResponse {
    let (db_path, config) = {
        let state = app.state::<Mutex<AppState>>();
        let s = state.lock().unwrap();
        (s.context_db_path.clone(), s.embedding_config.clone())
    };
    match crate::rag::run_search(db_path, config, query, None, top_k.unwrap_or(10)).await {
        Ok(results) => IpcResponse::ContextSearchResults(
            results.into_iter().map(|r| ContextSearchHit {
                conversation_id: r.chunk.conversation_id,
                conversation_title: r.conversation_title,
                project_name: r.project_name,
                role: r.chunk.role,
                score: r.score,
                content: r.chunk.content,
            }).collect()
        ),
        Err(e) => IpcResponse::Error(e),
    }
}

/// Fetch every chunk of a conversation, in order, so an agent can pull the
/// full thread once ContextSearch has found the relevant one.
fn context_get_conversation(app: &AppHandle, conversation_id: &str) -> IpcResponse {
    let state = app.state::<Mutex<AppState>>();
    let db_path = state.lock().unwrap().context_db_path.clone();
    let store = match crate::context_store::ContextStore::new(&db_path) {
        Ok(s) => s,
        Err(e) => return IpcResponse::Error(e.to_string()),
    };
    let meta = match store.get_conversation_meta(conversation_id) {
        Ok(Some(m)) => m,
        Ok(None) => return IpcResponse::Error("conversation not found".to_string()),
        Err(e) => return IpcResponse::Error(e.to_string()),
    };
    let chunks = match store.get_chunks(conversation_id) {
        Ok(c) => c,
        Err(e) => return IpcResponse::Error(e.to_string()),
    };
    IpcResponse::ContextConversation {
        title: if meta.0.is_empty() { None } else { Some(meta.0) },
        project_name: meta.1,
        chunks: chunks.into_iter().map(|c| ContextChunkOut { role: c.role, content: c.content }).collect(),
    }
}

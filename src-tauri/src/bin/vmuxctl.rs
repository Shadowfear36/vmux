//! vmuxctl — control a running vmux instance or send terminal notifications.
//!
//! Usage:
//!   vmuxctl notify <message>   Emit a notification in the current terminal pane
//!                              (prints an OSC 9 escape sequence; works inside
//!                              any terminal that understands OSC 9, including vmux).
//!
//!   vmuxctl ping               Check whether a vmux instance is running and
//!                              reachable via the IPC pipe.
//!
//!   vmuxctl list               Print a table of all open terminal panes.
//!
//! The `notify` subcommand is intentionally OSC-based: no IPC is needed
//! because vmux's VT parser already recognises the escape sequence from any
//! output that appears in a terminal pane. `ping` and `list` use the named
//! pipe (\\.\pipe\vmux-ipc) for queries that need to reach vmux from outside
//! a terminal.

use std::io::Write;

use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::windows::named_pipe::ClientOptions;

const PIPE_NAME: &str = r"\\.\pipe\vmux-ipc";

// ─── Protocol mirror ───────────────────────────────────────────────────────────
// Duplicated from src/ipc.rs so this binary stays free of the heavy vmux_lib
// dep chain (wgpu, cosmic-text, etc.).

#[derive(Debug, Serialize, Deserialize)]
enum IpcRequest {
    Ping,
    Notify { message: String },
    List,
    BrowserNavigate { url: String },
    BrowserScreenshot { output_path: Option<String> },
    BrowserGetUrl,
    BrowserEval { js: String },
    ContextSearch { query: String, top_k: Option<usize> },
    ContextGetConversation { conversation_id: String },
}

#[derive(Debug, Deserialize)]
struct TerminalSummary {
    id: String,
    title: String,
    is_agent: bool,
}

#[derive(Debug, Deserialize)]
struct ContextSearchHit {
    conversation_id: String,
    conversation_title: Option<String>,
    project_name: String,
    role: String,
    score: f32,
    content: String,
}

#[derive(Debug, Deserialize)]
struct ContextChunkOut {
    role: String,
    content: String,
}

#[derive(Debug, Deserialize)]
enum IpcResponse {
    Pong,
    Ok,
    Terminals(Vec<TerminalSummary>),
    Error(String),
    Screenshot { path: String },
    BrowserUrl { url: String },
    BrowserEvalResult { ok: bool, value: String },
    ContextSearchResults(Vec<ContextSearchHit>),
    ContextConversation { title: Option<String>, project_name: String, chunks: Vec<ContextChunkOut> },
}

// ─── Framing (mirrors daemon_client::{write_framed, read_framed}) ──────────────

async fn write_framed<T: Serialize, W: tokio::io::AsyncWrite + Unpin>(
    w: &mut W,
    msg: &T,
) -> Result<()> {
    let bytes = serde_json::to_vec(msg)?;
    w.write_u32_le(bytes.len() as u32).await?;
    w.write_all(&bytes).await?;
    w.flush().await?;
    Ok(())
}

async fn read_framed<T: for<'de> Deserialize<'de>, R: tokio::io::AsyncRead + Unpin>(
    r: &mut R,
) -> Result<T> {
    let len = r.read_u32_le().await?;
    let mut buf = vec![0u8; len as usize];
    r.read_exact(&mut buf).await?;
    Ok(serde_json::from_slice(&buf)?)
}

// ─── IPC helper ────────────────────────────────────────────────────────────────

async fn ipc_call(req: IpcRequest) -> Result<IpcResponse> {
    let pipe = ClientOptions::new()
        .open(PIPE_NAME)
        .map_err(|e| anyhow::anyhow!("vmux is not running (could not connect to {PIPE_NAME}): {e}"))?;
    let (mut r, mut w) = tokio::io::split(pipe);
    write_framed(&mut w, &req).await?;
    Ok(read_framed(&mut r).await?)
}

// ─── Subcommands ───────────────────────────────────────────────────────────────

/// vmux notify <message>
///
/// Prints an OSC 9 escape sequence to stdout. When run inside a vmux terminal
/// pane the VT parser intercepts it and triggers the notification badge in the
/// sidebar — no IPC required.
fn cmd_notify(message: &str) {
    // OSC 9: \x1b]9;<message>\x07
    // Use write! directly so the escape bytes reach stdout even if the shell
    // has line-buffering enabled.
    let mut out = std::io::stdout().lock();
    let _ = write!(out, "\x1b]9;{}\x07", message);
}

/// vmux ping
async fn cmd_ping() -> Result<()> {
    match ipc_call(IpcRequest::Ping).await? {
        IpcResponse::Pong => {
            println!("vmux is running");
            Ok(())
        }
        other => bail!("unexpected response: {other:?}"),
    }
}

/// vmux list
async fn cmd_list() -> Result<()> {
    match ipc_call(IpcRequest::List).await? {
        IpcResponse::Terminals(terminals) => {
            if terminals.is_empty() {
                println!("no terminals open");
                return Ok(());
            }
            println!("{:<38}  {:<6}  {}", "ID", "AGENT", "TITLE");
            println!("{}", "-".repeat(70));
            for t in terminals {
                println!(
                    "{:<38}  {:<6}  {}",
                    t.id,
                    if t.is_agent { "yes" } else { "no" },
                    t.title,
                );
            }
            Ok(())
        }
        IpcResponse::Error(e) => bail!("vmux error: {e}"),
        other => bail!("unexpected response: {other:?}"),
    }
}

/// vmux browser navigate <url>
async fn cmd_browser_navigate(url: &str) -> Result<()> {
    match ipc_call(IpcRequest::BrowserNavigate { url: url.to_string() }).await? {
        IpcResponse::Ok => Ok(()),
        IpcResponse::Error(e) => bail!("vmux error: {e}"),
        other => bail!("unexpected response: {other:?}"),
    }
}

/// vmux browser screenshot [path]
async fn cmd_browser_screenshot(output_path: Option<String>) -> Result<()> {
    match ipc_call(IpcRequest::BrowserScreenshot { output_path }).await? {
        IpcResponse::Screenshot { path } => {
            println!("{path}");
            Ok(())
        }
        IpcResponse::Error(e) => bail!("vmux error: {e}"),
        other => bail!("unexpected response: {other:?}"),
    }
}

/// vmux browser eval <js>
///
/// Unlike printing a `browser-eval` OSC escape sequence (fire-and-forget),
/// this waits for the script's return value and prints it — the return
/// value is whatever the JS expression evaluates to, JSON-encoded.
async fn cmd_browser_eval(js: &str) -> Result<()> {
    match ipc_call(IpcRequest::BrowserEval { js: js.to_string() }).await? {
        IpcResponse::BrowserEvalResult { ok: true, value } => {
            println!("{value}");
            Ok(())
        }
        IpcResponse::BrowserEvalResult { ok: false, value } => {
            bail!("script threw: {value}")
        }
        IpcResponse::Error(e) => bail!("vmux error: {e}"),
        other => bail!("unexpected response: {other:?}"),
    }
}

/// vmux context search <query> [--top-k N]
async fn cmd_context_search(query: &str, top_k: Option<usize>) -> Result<()> {
    match ipc_call(IpcRequest::ContextSearch { query: query.to_string(), top_k }).await? {
        IpcResponse::ContextSearchResults(hits) => {
            if hits.is_empty() {
                println!("no matches");
                return Ok(());
            }
            for (i, h) in hits.iter().enumerate() {
                println!(
                    "[{}] {:.0}%  {}  ({})  conversation_id={}",
                    i + 1,
                    h.score * 100.0,
                    h.conversation_title.as_deref().unwrap_or("Untitled"),
                    h.project_name,
                    h.conversation_id,
                );
                println!("    role: {}", h.role);
                println!("{}", h.content);
                println!("---");
            }
            Ok(())
        }
        IpcResponse::Error(e) => bail!("vmux error: {e}"),
        other => bail!("unexpected response: {other:?}"),
    }
}

/// vmux context get <conversation_id>
async fn cmd_context_get(conversation_id: &str) -> Result<()> {
    match ipc_call(IpcRequest::ContextGetConversation { conversation_id: conversation_id.to_string() }).await? {
        IpcResponse::ContextConversation { title, project_name, chunks } => {
            println!("{} ({})", title.as_deref().unwrap_or("Untitled"), project_name);
            println!("{}", "=".repeat(40));
            for chunk in chunks {
                println!("[{}]", chunk.role);
                println!("{}", chunk.content);
                println!("---");
            }
            Ok(())
        }
        IpcResponse::Error(e) => bail!("vmux error: {e}"),
        other => bail!("unexpected response: {other:?}"),
    }
}

/// vmux browser url
async fn cmd_browser_url() -> Result<()> {
    match ipc_call(IpcRequest::BrowserGetUrl).await? {
        IpcResponse::BrowserUrl { url } => {
            println!("{url}");
            Ok(())
        }
        IpcResponse::Error(e) => bail!("vmux error: {e}"),
        other => bail!("unexpected response: {other:?}"),
    }
}

// ─── Entry point ───────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();

    match args.get(1).map(String::as_str) {
        Some("notify") => {
            let message = args[2..].join(" ");
            if message.is_empty() {
                bail!("usage: vmuxctl notify <message>");
            }
            cmd_notify(&message);
            Ok(())
        }
        Some("ping") => cmd_ping().await,
        Some("list") => cmd_list().await,
        Some("context") => {
            match args.get(2).map(String::as_str) {
                Some("search") => {
                    // Supports a trailing `--top-k N` flag anywhere after the query.
                    let mut rest = args[3..].to_vec();
                    let mut top_k = None;
                    if let Some(pos) = rest.iter().position(|a| a == "--top-k") {
                        if let Some(n) = rest.get(pos + 1).and_then(|s| s.parse::<usize>().ok()) {
                            top_k = Some(n);
                        }
                        rest.drain(pos..(pos + 2).min(rest.len()));
                    }
                    let query = rest.join(" ");
                    if query.is_empty() {
                        bail!("usage: vmuxctl context search <query> [--top-k N]");
                    }
                    cmd_context_search(&query, top_k).await
                }
                Some("get") => {
                    let id = args.get(3).ok_or_else(|| anyhow::anyhow!("usage: vmuxctl context get <conversation_id>"))?;
                    cmd_context_get(id).await
                }
                _ => {
                    eprintln!("vmuxctl context — search + retrieve conversation history and notes\n");
                    eprintln!("usage:");
                    eprintln!("  vmuxctl context search <query> [--top-k N]   semantic search across all history");
                    eprintln!("  vmuxctl context get <conversation_id>         print a full conversation, in order");
                    std::process::exit(1);
                }
            }
        }
        Some("browser") => {
            match args.get(2).map(String::as_str) {
                Some("navigate") => {
                    let url = args.get(3).ok_or_else(|| anyhow::anyhow!("usage: vmuxctl browser navigate <url>"))?;
                    cmd_browser_navigate(url).await
                }
                Some("screenshot") => {
                    let output_path = args.get(3).cloned();
                    cmd_browser_screenshot(output_path).await
                }
                Some("url") => cmd_browser_url().await,
                Some("eval") => {
                    let js = args[3..].join(" ");
                    if js.is_empty() {
                        bail!("usage: vmuxctl browser eval <js>");
                    }
                    cmd_browser_eval(&js).await
                }
                _ => {
                    eprintln!("vmuxctl browser — browser pane control\n");
                    eprintln!("usage:");
                    eprintln!("  vmuxctl browser navigate <url>       navigate the browser to a URL");
                    eprintln!("  vmuxctl browser screenshot [path]     screenshot the browser (prints path)");
                    eprintln!("  vmuxctl browser url                   print the current URL");
                    eprintln!("  vmuxctl browser eval <js>              run JS, print its return value");
                    std::process::exit(1);
                }
            }
        }
        _ => {
            eprintln!("vmuxctl — terminal multiplexer control\n");
            eprintln!("usage:");
            eprintln!("  vmuxctl notify <message>   send a notification in the current terminal pane");
            eprintln!("  vmuxctl ping               check whether vmux is running");
            eprintln!("  vmuxctl list               list open terminal panes");
            eprintln!("  vmuxctl browser <cmd>      control the browser pane");
            eprintln!("  vmuxctl context <cmd>      search + retrieve conversation history and notes");
            std::process::exit(1);
        }
    }
}

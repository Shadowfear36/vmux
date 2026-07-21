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
}

#[derive(Debug, Deserialize)]
struct TerminalSummary {
    id: String,
    title: String,
    is_agent: bool,
}

#[derive(Debug, Deserialize)]
enum IpcResponse {
    Pong,
    Ok,
    Terminals(Vec<TerminalSummary>),
    Error(String),
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
        _ => {
            eprintln!("vmuxctl — terminal multiplexer control\n");
            eprintln!("usage:");
            eprintln!("  vmuxctl notify <message>   send a notification in the current terminal pane");
            eprintln!("  vmuxctl ping               check whether vmux is running");
            eprintln!("  vmuxctl list               list open terminal panes");
            std::process::exit(1);
        }
    }
}

//! Client for the `vmuxd` background daemon (Phase 2 of
//! docs/session-reattach-design.md). Gated behind the VMUX_DAEMON_TERMINALS
//! env var — see `TerminalPane::spawn`. Only plain shell terminals
//! (`create_terminal`) use this path so far; agent terminals and workspace
//! restore are untouched, keeping the existing in-process path as default.
//!
//! Protocol types here are shared with `src/bin/vmuxd.rs` (the daemon
//! binary) via this crate's `pub` visibility — no duplicated definitions.

use std::io::Write;
use std::sync::{Arc, Mutex};

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::windows::named_pipe::ClientOptions;
use tokio::sync::mpsc as tmpsc;
use tokio::sync::mpsc::UnboundedReceiver;

pub const PIPE_NAME: &str = r"\\.\pipe\vmux-daemon";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Request {
    Spawn { shell_path: String, args: Vec<String>, cwd: Option<String>, cols: u16, rows: u16 },
    Attach { session_id: String },
    Write(Vec<u8>),
    Resize { cols: u16, rows: u16 },
    /// Terminate the session's process and remove it from the daemon's
    /// registry. Without this, closing a daemon-backed pane in vmux would
    /// leave its shell/agent process running forever as an unreachable
    /// (but never cleaned up) daemon session.
    Kill { session_id: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Event {
    Spawned { session_id: String, pid: Option<u32> },
    Replay(Vec<u8>),
    Output(Vec<u8>),
    Exited,
    Error(String),
}

/// Length-prefixed JSON framing, used for both directions of the protocol —
/// the daemon binary (`src/bin/vmuxd.rs`) reuses these directly since it
/// sends `Event`s and receives `Request`s (the mirror image of the client).
pub async fn write_framed<T: Serialize, W: tokio::io::AsyncWrite + Unpin>(w: &mut W, msg: &T) -> Result<()> {
    let bytes = serde_json::to_vec(msg)?;
    w.write_u32_le(bytes.len() as u32).await?;
    w.write_all(&bytes).await?;
    w.flush().await?;
    Ok(())
}

pub async fn read_framed<T: for<'de> Deserialize<'de>, R: tokio::io::AsyncRead + Unpin>(r: &mut R) -> Result<Option<T>> {
    let len = match r.read_u32_le().await {
        Ok(l) => l,
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e.into()),
    };
    let mut buf = vec![0u8; len as usize];
    r.read_exact(&mut buf).await?;
    Ok(Some(serde_json::from_slice(&buf)?))
}

pub async fn send_request<W: tokio::io::AsyncWrite + Unpin>(w: &mut W, req: &Request) -> Result<()> {
    write_framed(w, req).await
}

pub async fn recv_event<R: tokio::io::AsyncRead + Unpin>(r: &mut R) -> Result<Option<Event>> {
    read_framed(r).await
}

/// A PTY session owned by the `vmuxd` daemon, accessed over a named pipe.
/// Exposes roughly the same surface as `pty::PtySession` so `TerminalPane`
/// can hold either behind one `PtyBackend` enum.
pub struct DaemonPtySession {
    pub session_id: String,
    pub pid: Option<u32>,
    req_tx: tmpsc::UnboundedSender<Request>,
}

struct DaemonWriter {
    tx: tmpsc::UnboundedSender<Request>,
}

impl Write for DaemonWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.tx.send(Request::Write(buf.to_vec()))
            .map_err(|_| std::io::Error::new(std::io::ErrorKind::BrokenPipe, "daemon connection closed"))?;
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> { Ok(()) }
}

impl DaemonPtySession {
    /// Ensure `vmuxd` is running (launching it detached if not), spawn a new
    /// shell session on it, and return a session handle plus a receiver of
    /// output bytes — matching `PtySession::spawn`'s output shape so the
    /// rest of the VT/render pipeline needs no changes.
    pub async fn spawn(
        shell_path: &str,
        args: &[String],
        cwd: Option<&str>,
        cols: u16,
        rows: u16,
    ) -> Result<(Self, UnboundedReceiver<Vec<u8>>)> {
        ensure_daemon_running().await?;
        let client = connect_with_retry(20).await?;
        let (read_half, mut write_half) = tokio::io::split(client);
        let mut read_half = read_half;

        send_request(&mut write_half, &Request::Spawn {
            shell_path: shell_path.to_string(),
            args: args.to_vec(),
            cwd: cwd.map(|s| s.to_string()),
            cols, rows,
        }).await?;

        let (session_id, pid) = match recv_event(&mut read_half).await? {
            Some(Event::Spawned { session_id, pid }) => (session_id, pid),
            Some(Event::Error(msg)) => return Err(anyhow!("daemon spawn error: {msg}")),
            other => return Err(anyhow!("unexpected daemon response to Spawn: {other:?}")),
        };
        log::info!("daemon session spawned: id={session_id} pid={pid:?}");

        Ok(Self::finish_connection(session_id, pid, read_half, write_half))
    }

    /// Reattach to an existing daemon session by ID — e.g. after vmux
    /// restarts and finds a persisted `daemon_session_id` on a saved pane.
    /// Falling back is the caller's responsibility: if the daemon is
    /// unreachable or the session no longer exists, this returns an error
    /// and the caller should spawn a fresh session instead.
    pub async fn attach(session_id: &str, cols: u16, rows: u16) -> Result<(Self, UnboundedReceiver<Vec<u8>>)> {
        let client = connect_with_retry(5).await?;
        let (read_half, mut write_half) = tokio::io::split(client);
        let mut read_half = read_half;

        send_request(&mut write_half, &Request::Attach { session_id: session_id.to_string() }).await?;
        send_request(&mut write_half, &Request::Resize { cols, rows }).await?;

        // Attach's first event is Replay directly (no Spawned handshake —
        // there's nothing new to report beyond the session already existing).
        let replay = match recv_event(&mut read_half).await? {
            Some(Event::Replay(data)) => data,
            Some(Event::Error(msg)) => return Err(anyhow!("daemon attach error: {msg}")),
            other => return Err(anyhow!("unexpected daemon response to Attach: {other:?}")),
        };
        log::info!("daemon session attached: id={session_id}, {} bytes of replay", replay.len());

        let (session, out_rx) = Self::finish_connection(session_id.to_string(), None, read_half, write_half);
        // finish_connection's forwarding task will also deliver this same
        // Replay once its loop starts — but we already consumed it above to
        // check for Error, so re-inject it as the first thing the caller sees.
        Ok((session, prepend_replay(replay, out_rx)))
    }

    /// Shared post-handshake plumbing for both `spawn` and `attach`: spawn
    /// the two forwarding tasks (outgoing requests, incoming output) that
    /// make a `DaemonPtySession` behave like `PtySession` to the rest of
    /// the app.
    fn finish_connection(
        session_id: String,
        pid: Option<u32>,
        mut read_half: tokio::io::ReadHalf<tokio::net::windows::named_pipe::NamedPipeClient>,
        mut write_half: tokio::io::WriteHalf<tokio::net::windows::named_pipe::NamedPipeClient>,
    ) -> (Self, UnboundedReceiver<Vec<u8>>) {
        let (out_tx, out_rx) = tmpsc::unbounded_channel::<Vec<u8>>();
        let (req_tx, mut req_rx) = tmpsc::unbounded_channel::<Request>();

        // Forward outgoing requests (Write/Resize/Kill) to the daemon.
        tokio::spawn(async move {
            while let Some(req) = req_rx.recv().await {
                if send_request(&mut write_half, &req).await.is_err() { break; }
            }
        });

        // Forward incoming replay/output to the same receiver PtySession would produce.
        tokio::spawn(async move {
            loop {
                match recv_event(&mut read_half).await {
                    Ok(Some(Event::Replay(data))) | Ok(Some(Event::Output(data))) => {
                        if !data.is_empty() && out_tx.send(data).is_err() { break; }
                    }
                    Ok(Some(Event::Exited)) | Ok(None) => break,
                    Ok(Some(_)) => {} // Spawned/Error only expected pre-handshake
                    Err(e) => { log::error!("daemon client recv error: {e}"); break; }
                }
            }
        });

        (DaemonPtySession { session_id, pid, req_tx }, out_rx)
    }

    pub fn write(&self, data: &[u8]) -> Result<()> {
        self.req_tx.send(Request::Write(data.to_vec()))
            .map_err(|e| anyhow!("{e}"))
    }

    pub fn writer_handle(&self) -> Arc<Mutex<Box<dyn Write + Send>>> {
        Arc::new(Mutex::new(Box::new(DaemonWriter { tx: self.req_tx.clone() }) as Box<dyn Write + Send>))
    }

    pub fn resize(&self, cols: u16, rows: u16) -> Result<()> {
        self.req_tx.send(Request::Resize { cols, rows })
            .map_err(|e| anyhow!("{e}"))
    }

    /// Terminate this session on the daemon (kills the process, removes it
    /// from the registry). Call this from `close_terminal` — without it,
    /// closing a daemon-backed pane leaks the session forever.
    pub fn kill(&self) -> Result<()> {
        self.req_tx.send(Request::Kill { session_id: self.session_id.clone() })
            .map_err(|e| anyhow!("{e}"))
    }
}

/// Re-deliver an already-consumed Replay event as the first item on a fresh
/// receiver, so `attach`'s caller sees it even though `attach` had to read
/// it early (to check for an Error response) before the forwarding task
/// that owns the receiver even started.
fn prepend_replay(replay: Vec<u8>, mut rx: UnboundedReceiver<Vec<u8>>) -> UnboundedReceiver<Vec<u8>> {
    if replay.is_empty() { return rx; }
    let (tx, new_rx) = tmpsc::unbounded_channel::<Vec<u8>>();
    let _ = tx.send(replay);
    tokio::spawn(async move {
        while let Some(data) = rx.recv().await {
            if tx.send(data).is_err() { break; }
        }
    });
    new_rx
}

async fn connect_with_retry(attempts: u32) -> Result<tokio::net::windows::named_pipe::NamedPipeClient> {
    for i in 0..attempts {
        match ClientOptions::new().open(PIPE_NAME) {
            Ok(c) => return Ok(c),
            Err(e) if e.raw_os_error() == Some(231) => {
                // ERROR_PIPE_BUSY — daemon mid-accepting another client.
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound && i < attempts - 1 => {
                // Daemon may still be starting up.
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            }
            Err(e) => return Err(e.into()),
        }
    }
    Err(anyhow!("could not connect to vmuxd at {PIPE_NAME} after {attempts} attempts"))
}

/// If the daemon isn't reachable, launch it detached (independent of vmux's
/// own process/job object) so its lifetime doesn't tie to the UI.
async fn ensure_daemon_running() -> Result<()> {
    if ClientOptions::new().open(PIPE_NAME).is_ok() {
        return Ok(());
    }

    let exe = std::env::current_exe()?;
    let daemon_path = exe.with_file_name("vmuxd.exe");
    log::info!("vmuxd not reachable, launching {}", daemon_path.display());

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x00000200;
        const DETACHED_PROCESS: u32 = 0x00000008;
        std::process::Command::new(&daemon_path)
            .creation_flags(CREATE_NEW_PROCESS_GROUP | DETACHED_PROCESS)
            .spawn()?;
    }

    // Give it a moment to bind the pipe before the caller tries to connect.
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    Ok(())
}

//! Client for the `vmuxd` background daemon (Phase 2/3 of
//! docs/session-reattach-design.md). Gated behind the VMUX_DAEMON_TERMINALS
//! env var — see `TerminalPane::spawn`. Both plain shells (`create_terminal`)
//! and agent terminals (`create_agent_terminal`) can use this path; the
//! in-process `PtySession` path remains the default when the flag is unset.
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

/// Bump whenever `Request`/`Event` change in a way that isn't purely
/// additive. Checked by the `Hello` handshake so a stale `vmuxd.exe` left
/// running across an app update gets detected (and self-healed via
/// `Request::Shutdown`) instead of silently misbehaving — see §16/§17 of
/// docs/session-reattach-design.md.
pub const PROTOCOL_VERSION: u32 = 1;

/// Marker prefix on the error string returned when a connected daemon
/// reports a different `PROTOCOL_VERSION`, so callers can distinguish "stale
/// daemon, needs a restart" from an ordinary connection failure.
pub const VERSION_MISMATCH_PREFIX: &str = "VMUXD_VERSION_MISMATCH";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Request {
    /// Must be the very first message on every connection. The daemon
    /// replies `HelloAck` if versions match, or `Error` (prefixed with
    /// `VERSION_MISMATCH_PREFIX`) if not — in which case the only other
    /// request it will still honor on that connection is `Shutdown`.
    Hello { version: u32 },
    Spawn { shell_path: String, args: Vec<String>, env: Vec<(String, String)>, cwd: Option<String>, cols: u16, rows: u16 },
    Attach { session_id: String },
    Write(Vec<u8>),
    Resize { cols: u16, rows: u16 },
    /// Terminate the session's process and remove it from the daemon's
    /// registry. Without this, closing a daemon-backed pane in vmux would
    /// leave its shell/agent process running forever as an unreachable
    /// (but never cleaned up) daemon session.
    Kill { session_id: String },
    /// One-shot control-plane request: list all currently registered
    /// sessions (attached or not).
    ListSessions,
    /// One-shot control-plane request: list processes found alive-but-
    /// orphaned at daemon startup (left over from a previous `vmuxd` crash).
    ListOrphans,
    /// One-shot control-plane request: forcibly terminate an orphaned
    /// process by PID (there's no session/registry entry for it to route
    /// through, since orphans are by definition outside the daemon's
    /// control — this just calls into the OS directly).
    KillOrphan { pid: u32 },
    /// Gracefully terminate the daemon itself: kills every registered
    /// session's process, then exits. Used to replace a stale (version-
    /// mismatched) daemon — a real UX tradeoff (it ends every running
    /// session), only ever sent automatically right after a version
    /// mismatch is detected, never casually.
    Shutdown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Event {
    HelloAck,
    Spawned { session_id: String, pid: Option<u32> },
    Replay(Vec<u8>),
    Output(Vec<u8>),
    Exited,
    Error(String),
    SessionList(Vec<SessionMeta>),
    OrphanList(Vec<OrphanInfo>),
    Ok,
}

/// Metadata for a live daemon-owned session — used by `ListSessions` to
/// power a "daemon sessions" view even for sessions with no attached pane.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionMeta {
    pub session_id: String,
    pub pid: Option<u32>,
    /// The command that was spawned (e.g. a shell path or agent binary) —
    /// enough for a human to recognize which session is which.
    pub label: String,
    pub attached_clients: u32,
}

/// A process left alive-but-unreachable by a previous `vmuxd` crash (see
/// §2/§12 of the design doc: killing the daemon doesn't kill its children,
/// but their anonymous pipe handles die with it, so they can never be
/// reattached — only recognized and, if desired, killed).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrphanInfo {
    pub pid: u32,
    pub label: String,
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
        env: &[(String, String)],
        cwd: Option<&str>,
        cols: u16,
        rows: u16,
    ) -> Result<(Self, UnboundedReceiver<Vec<u8>>)> {
        ensure_daemon_running().await?;
        let client = connect_with_retry(20).await?;
        let (read_half, mut write_half) = tokio::io::split(client);
        let mut read_half = read_half;

        hello_handshake(&mut read_half, &mut write_half).await?;

        send_request(&mut write_half, &Request::Spawn {
            shell_path: shell_path.to_string(),
            args: args.to_vec(),
            env: env.to_vec(),
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

        hello_handshake(&mut read_half, &mut write_half).await?;

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

/// Sends `Hello` and expects `HelloAck` back. Every connection must do this
/// before anything else — see `Request::Hello`'s doc comment. Returns a
/// `VERSION_MISMATCH_PREFIX`-prefixed error on a version mismatch so callers
/// (specifically `ensure_daemon_running`) can react to it specially.
async fn hello_handshake<R, W>(read_half: &mut R, write_half: &mut W) -> Result<()>
where
    R: tokio::io::AsyncRead + Unpin,
    W: tokio::io::AsyncWrite + Unpin,
{
    send_request(write_half, &Request::Hello { version: PROTOCOL_VERSION }).await?;
    match recv_event(read_half).await? {
        Some(Event::HelloAck) => Ok(()),
        Some(Event::Error(msg)) => Err(anyhow!("{msg}")),
        other => Err(anyhow!("unexpected daemon response to Hello: {other:?}")),
    }
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
/// own process/job object) so its lifetime doesn't tie to the UI. If it *is*
/// reachable but running a stale (mismatched) protocol version — e.g. left
/// over from before an app update — asks it to shut down and launches a
/// fresh one instead of silently proceeding against an incompatible daemon.
async fn ensure_daemon_running() -> Result<()> {
    if let Ok(client) = ClientOptions::new().open(PIPE_NAME) {
        let (read_half, mut write_half) = tokio::io::split(client);
        let mut read_half = read_half;
        match hello_handshake(&mut read_half, &mut write_half).await {
            Ok(()) => return Ok(()),
            Err(e) if e.to_string().contains(VERSION_MISMATCH_PREFIX) => {
                log::warn!("stale vmuxd detected ({e}), asking it to shut down and relaunching");
                let _ = send_request(&mut write_half, &Request::Shutdown).await;
                // Wait for the old daemon to actually exit (pipe becomes unreachable)
                // before launching a replacement — otherwise we could race it.
                for _ in 0..40 {
                    if ClientOptions::new().open(PIPE_NAME).is_err() { break; }
                    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                }
            }
            Err(e) => return Err(e),
        }
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

/// Opens a short-lived control connection: connect, `Hello`, send one
/// request, read one reply, return it (connection is dropped after).
async fn control_request(req: Request) -> Result<Event> {
    let client = connect_with_retry(5).await?;
    let (read_half, mut write_half) = tokio::io::split(client);
    let mut read_half = read_half;
    hello_handshake(&mut read_half, &mut write_half).await?;
    send_request(&mut write_half, &req).await?;
    recv_event(&mut read_half).await?.ok_or_else(|| anyhow!("daemon closed connection without replying"))
}

/// Whether `vmuxd` is currently reachable and speaking a compatible
/// protocol version. Does not launch it if not — purely a status check
/// (e.g. for a Settings-panel "daemon not running" display).
pub async fn is_daemon_running() -> bool {
    let Ok(client) = ClientOptions::new().open(PIPE_NAME) else { return false };
    let (read_half, mut write_half) = tokio::io::split(client);
    let mut read_half = read_half;
    hello_handshake(&mut read_half, &mut write_half).await.is_ok()
}

/// List every session currently registered on the daemon, whether or not
/// any pane is attached to it — surfaces sessions that would otherwise be
/// invisible (e.g. after their owning tab/workspace was deleted without
/// explicitly killing the session).
pub async fn list_sessions() -> Result<Vec<SessionMeta>> {
    match control_request(Request::ListSessions).await? {
        Event::SessionList(sessions) => Ok(sessions),
        Event::Error(msg) => Err(anyhow!("{msg}")),
        other => Err(anyhow!("unexpected daemon response to ListSessions: {other:?}")),
    }
}

/// Kill a daemon session by ID without needing a live `DaemonPtySession`
/// handle to it — used by the Settings panel to kill sessions with no
/// attached pane, where `DaemonPtySession::kill` isn't available.
pub async fn kill_session(session_id: &str) -> Result<()> {
    match control_request(Request::Kill { session_id: session_id.to_string() }).await? {
        Event::Ok => Ok(()),
        Event::Error(msg) => Err(anyhow!("{msg}")),
        other => Err(anyhow!("unexpected daemon response to Kill: {other:?}")),
    }
}

/// List processes found alive-but-orphaned at the current daemon's
/// startup — leftovers from a previous `vmuxd` crash (see `OrphanInfo`).
pub async fn list_orphans() -> Result<Vec<OrphanInfo>> {
    match control_request(Request::ListOrphans).await? {
        Event::OrphanList(orphans) => Ok(orphans),
        Event::Error(msg) => Err(anyhow!("{msg}")),
        other => Err(anyhow!("unexpected daemon response to ListOrphans: {other:?}")),
    }
}

/// Forcibly terminate an orphaned process by PID.
pub async fn kill_orphan(pid: u32) -> Result<()> {
    match control_request(Request::KillOrphan { pid }).await? {
        Event::Ok => Ok(()),
        Event::Error(msg) => Err(anyhow!("{msg}")),
        other => Err(anyhow!("unexpected daemon response to KillOrphan: {other:?}")),
    }
}

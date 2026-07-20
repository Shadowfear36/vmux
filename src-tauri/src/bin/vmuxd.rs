//! Phase 2 of docs/session-reattach-design.md: the real (multi-session)
//! `vmuxd` background daemon. Owns PTY sessions independent of any
//! connected client (see the Phase 1 prototype in vmuxd_proto.rs, which
//! validated the core assumption with one hardcoded session).
//!
//! Launched automatically by `vmux_lib::terminal::daemon_client` when
//! VMUX_DAEMON_TERMINALS is set and no daemon is reachable yet. Not started
//! by default — the in-process PtySession path remains the default.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::sync::{Arc, Mutex};

use portable_pty::{native_pty_system, CommandBuilder, MasterPty, PtySize};
use tokio::net::windows::named_pipe::{NamedPipeServer, ServerOptions};
use tokio::sync::broadcast;

use vmux_lib::terminal::daemon_client::{read_framed, write_framed, Event, Request, PIPE_NAME};

const SCROLLBACK_CAP: usize = 256 * 1024;

struct Session {
    writer: Mutex<Box<dyn Write + Send>>,
    master: Mutex<Box<dyn MasterPty + Send>>,
    scrollback: Arc<Mutex<Vec<u8>>>,
    tx: broadcast::Sender<Vec<u8>>,
    pid: Option<u32>,
}

type Registry = Arc<Mutex<HashMap<String, Arc<Session>>>>;

fn spawn_session(shell_path: &str, args: &[String], cwd: Option<&str>, cols: u16, rows: u16) -> anyhow::Result<Session> {
    let pty_system = native_pty_system();
    let pair = pty_system.openpty(PtySize { rows, cols, pixel_width: 0, pixel_height: 0 })?;

    let mut cmd = CommandBuilder::new(shell_path);
    for a in args { cmd.arg(a); }
    if let Some(dir) = cwd { cmd.cwd(dir); }
    let child = pair.slave.spawn_command(cmd)?;
    let pid = child.process_id();
    println!("[vmuxd] spawned {shell_path}, pid={pid:?}");

    let mut writer = pair.master.take_writer()?;
    let mut reader = pair.master.try_clone_reader()?;

    // No real VT parser here (that's alacritty_terminal's job in the real
    // app once the pane is attached) — reply to the shell's initial cursor
    // position query so it doesn't block waiting for a response.
    let _ = writer.write_all(b"\x1b[24;1R");

    let scrollback: Arc<Mutex<Vec<u8>>> = Arc::new(Mutex::new(Vec::new()));
    let (tx, _rx) = broadcast::channel::<Vec<u8>>(1024);

    let scrollback_rd = scrollback.clone();
    let tx_rd = tx.clone();
    std::thread::spawn(move || {
        let mut buf = [0u8; 8192];
        loop {
            match reader.read(&mut buf) {
                Ok(0) => { let _ = tx_rd.send(Vec::new()); break; }
                Ok(n) => {
                    let data = buf[..n].to_vec();
                    {
                        let mut sb = scrollback_rd.lock().unwrap();
                        sb.extend_from_slice(&data);
                        if sb.len() > SCROLLBACK_CAP {
                            let excess = sb.len() - SCROLLBACK_CAP;
                            sb.drain(0..excess);
                        }
                    }
                    let _ = tx_rd.send(data);
                }
                Err(_) => break,
            }
        }
    });

    Ok(Session {
        writer: Mutex::new(writer),
        master: Mutex::new(pair.master),
        scrollback,
        tx,
        pid,
    })
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    println!("[vmuxd] listening on {PIPE_NAME}");
    let registry: Registry = Arc::new(Mutex::new(HashMap::new()));

    loop {
        let server = ServerOptions::new().create(PIPE_NAME)?;
        server.connect().await?;
        let registry = registry.clone();
        tokio::spawn(async move {
            if let Err(e) = handle_client(server, registry).await {
                println!("[vmuxd] client handler error: {e}");
            }
        });
    }
}

async fn handle_client(pipe: NamedPipeServer, registry: Registry) -> anyhow::Result<()> {
    let (mut read_half, mut write_half) = tokio::io::split(pipe);

    // First message must be Spawn (new session) or Attach (existing one).
    let session: Arc<Session> = match read_framed::<Request, _>(&mut read_half).await? {
        Some(Request::Spawn { shell_path, args, cwd, cols, rows }) => {
            match spawn_session(&shell_path, &args, cwd.as_deref(), cols, rows) {
                Ok(s) => {
                    let session = Arc::new(s);
                    let session_id = uuid::Uuid::new_v4().to_string();
                    registry.lock().unwrap().insert(session_id.clone(), session.clone());
                    write_framed(&mut write_half, &Event::Spawned { session_id, pid: session.pid }).await?;
                    session
                }
                Err(e) => {
                    write_framed(&mut write_half, &Event::Error(e.to_string())).await?;
                    return Ok(());
                }
            }
        }
        Some(Request::Attach { session_id }) => {
            let found = registry.lock().unwrap().get(&session_id).cloned();
            match found {
                Some(session) => session,
                None => {
                    write_framed(&mut write_half, &Event::Error(format!("no such session: {session_id}"))).await?;
                    return Ok(());
                }
            }
        }
        other => {
            write_framed(&mut write_half, &Event::Error(format!("expected Spawn or Attach, got {other:?}"))).await?;
            return Ok(());
        }
    };

    // Immediate replay, then subscribe for live output.
    let replay = session.scrollback.lock().unwrap().clone();
    write_framed(&mut write_half, &Event::Replay(replay)).await?;

    let mut rx = session.tx.subscribe();
    let output_task = tokio::spawn(async move {
        loop {
            match rx.recv().await {
                Ok(data) if !data.is_empty() => {
                    if write_framed(&mut write_half, &Event::Output(data)).await.is_err() { break; }
                }
                Ok(_) => { let _ = write_framed(&mut write_half, &Event::Exited).await; break; }
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(broadcast::error::RecvError::Closed) => break,
            }
        }
    });

    loop {
        match read_framed::<Request, _>(&mut read_half).await {
            Ok(Some(Request::Write(data))) => {
                let _ = session.writer.lock().unwrap().write_all(&data);
            }
            Ok(Some(Request::Resize { cols, rows })) => {
                let _ = session.master.lock().unwrap()
                    .resize(PtySize { rows, cols, pixel_width: 0, pixel_height: 0 });
            }
            Ok(Some(_)) => {} // Spawn/Attach only valid as the first message
            Ok(None) => break,
            Err(e) => { println!("[vmuxd] recv error: {e}"); break; }
        }
    }

    output_task.abort();
    // Client disconnected — session (and its registry entry) live on.
    Ok(())
}

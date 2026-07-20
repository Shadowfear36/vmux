//! Phase 1 prototype for docs/session-reattach-design.md.
//!
//! Standalone daemon that owns ONE hardcoded ConPTY session (cmd.exe),
//! independent of any connected client. Validates the core assumption:
//! a session keeps running and buffering output while zero clients are
//! attached, and a newly-connecting client gets an immediate replay plus
//! the live stream from then on.
//!
//! NOT wired into the Tauri app — run standalone via:
//!   cargo run --manifest-path src-tauri/Cargo.toml --bin vmuxd_proto

use std::io::{Read, Write};
use std::sync::{Arc, Mutex};

use portable_pty::{native_pty_system, CommandBuilder, MasterPty, PtySize};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::windows::named_pipe::{NamedPipeServer, ServerOptions};
use tokio::sync::broadcast;

const PIPE_NAME: &str = r"\\.\pipe\vmux-proto";
const SCROLLBACK_CAP: usize = 64 * 1024;

#[derive(Debug, Serialize, Deserialize)]
enum Request {
    Write(Vec<u8>),
    Resize { cols: u16, rows: u16 },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
enum Event {
    Replay(Vec<u8>),
    Output(Vec<u8>),
    Exited,
}

struct Session {
    writer: Mutex<Box<dyn Write + Send>>,
    master: Mutex<Box<dyn MasterPty + Send>>,
    scrollback: Arc<Mutex<Vec<u8>>>,
    tx: broadcast::Sender<Vec<u8>>,
}

fn spawn_session() -> anyhow::Result<Session> {
    let pty_system = native_pty_system();
    let pair = pty_system.openpty(PtySize { rows: 30, cols: 100, pixel_width: 0, pixel_height: 0 })?;
    let cmd = CommandBuilder::new("cmd.exe");
    let child = pair.slave.spawn_command(cmd)?;
    println!("[vmuxd] spawned cmd.exe, pid={:?}", child.process_id());

    let mut writer = pair.master.take_writer()?;
    let mut reader = pair.master.try_clone_reader()?;

    // This prototype doesn't run a real VT parser (that's alacritty_terminal's
    // job in the real app — see grid.rs's EventProxy::PtyWrite handling),
    // so cmd.exe's initial cursor-position query (\x1b[6n) would otherwise
    // go unanswered and cmd.exe blocks waiting for a reply before processing
    // anything else. Send a canned reply so it unblocks.
    let _ = writer.write_all(b"\x1b[24;1R");

    let scrollback: Arc<Mutex<Vec<u8>>> = Arc::new(Mutex::new(Vec::new()));
    let (tx, _rx) = broadcast::channel::<Vec<u8>>(1024);

    let scrollback_rd = scrollback.clone();
    let tx_rd = tx.clone();
    std::thread::spawn(move || {
        let mut buf = [0u8; 8192];
        loop {
            match reader.read(&mut buf) {
                Ok(0) => { println!("[vmuxd] session PTY EOF — process exited"); let _ = tx_rd.send(Vec::new()); break; }
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
                    // Ignore send errors — fine if nobody's attached right now.
                    let _ = tx_rd.send(data);
                }
                Err(e) => { println!("[vmuxd] PTY read error: {e}"); break; }
            }
        }
    });

    Ok(Session {
        writer: Mutex::new(writer),
        master: Mutex::new(pair.master),
        scrollback,
        tx,
    })
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    println!("[vmuxd] starting — spawning the one prototype session...");
    let session = Arc::new(spawn_session()?);
    println!("[vmuxd] session is running, independent of any client.");
    println!("[vmuxd] listening on {PIPE_NAME}");
    println!("[vmuxd] close/reopen the test client any number of times — the session keeps going.");

    loop {
        let server = ServerOptions::new().create(PIPE_NAME)?;
        server.connect().await?;
        println!("[vmuxd] client connected");
        let session = session.clone();
        tokio::spawn(async move {
            if let Err(e) = handle_client(server, session).await {
                println!("[vmuxd] client handler error: {e}");
            }
            println!("[vmuxd] client disconnected — session keeps running");
        });
    }
}

async fn handle_client(pipe: NamedPipeServer, session: Arc<Session>) -> anyhow::Result<()> {
    let (mut read_half, mut write_half) = tokio::io::split(pipe);

    // Immediate replay of whatever happened while nobody was attached.
    let replay = session.scrollback.lock().unwrap().clone();
    send_event(&mut write_half, &Event::Replay(replay)).await?;

    let mut rx = session.tx.subscribe();
    let output_task = tokio::spawn(async move {
        loop {
            match rx.recv().await {
                Ok(data) if !data.is_empty() => {
                    if send_event(&mut write_half, &Event::Output(data)).await.is_err() { break; }
                }
                Ok(_) => {
                    let _ = send_event(&mut write_half, &Event::Exited).await;
                    break;
                }
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(broadcast::error::RecvError::Closed) => break,
            }
        }
    });

    loop {
        match recv_request(&mut read_half).await {
            Ok(Some(Request::Write(data))) => {
                let _ = session.writer.lock().unwrap().write_all(&data);
            }
            Ok(Some(Request::Resize { cols, rows })) => {
                let _ = session.master.lock().unwrap()
                    .resize(PtySize { rows, cols, pixel_width: 0, pixel_height: 0 });
            }
            Ok(None) => break,
            Err(e) => { println!("[vmuxd] recv error: {e}"); break; }
        }
    }

    output_task.abort();
    Ok(())
}

async fn send_event<W: tokio::io::AsyncWrite + Unpin>(w: &mut W, event: &Event) -> anyhow::Result<()> {
    let bytes = serde_json::to_vec(event)?;
    w.write_u32_le(bytes.len() as u32).await?;
    w.write_all(&bytes).await?;
    w.flush().await?;
    Ok(())
}

async fn recv_request<R: tokio::io::AsyncRead + Unpin>(r: &mut R) -> anyhow::Result<Option<Request>> {
    let len = match r.read_u32_le().await {
        Ok(l) => l,
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e.into()),
    };
    let mut buf = vec![0u8; len as usize];
    r.read_exact(&mut buf).await?;
    Ok(Some(serde_json::from_slice(&buf)?))
}

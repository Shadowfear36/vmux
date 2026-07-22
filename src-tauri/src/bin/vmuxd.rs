//! Phase 2 of docs/session-reattach-design.md: the real (multi-session)
//! `vmuxd` background daemon. Owns PTY sessions independent of any
//! connected client (see the Phase 1 prototype in vmuxd_proto.rs, which
//! validated the core assumption with one hardcoded session).
//!
//! Launched automatically by `vmux_lib::terminal::daemon_client` when
//! VMUX_DAEMON_TERMINALS is set and no daemon is reachable yet. Not started
//! by default — the in-process PtySession path remains the default.
//!
//! Phase 5 (lifecycle hardening, see docs/session-reattach-design.md §17)
//! added: a version handshake so a stale post-update daemon self-replaces
//! instead of silently misbehaving; an idle-shutdown timer so the daemon
//! doesn't run forever with zero sessions; a persisted session sidecar file
//! so orphaned processes from a crash can at least be recognized and killed
//! (never reattached — see docs §2/§12 for why that's impossible); and
//! one-shot control-plane requests (ListSessions/ListOrphans/KillOrphan/
//! Shutdown) so the UI can surface/manage sessions with no attached pane.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use portable_pty::{native_pty_system, CommandBuilder, MasterPty, PtySize};
use serde::{Deserialize, Serialize};
use sysinfo::{Pid, System};
use tokio::net::windows::named_pipe::{NamedPipeServer, ServerOptions};
use tokio::sync::broadcast;

use vmux_lib::terminal::daemon_client::{
    read_framed, write_framed, Event, OrphanInfo, Request, SessionMeta, PIPE_NAME,
    PROTOCOL_VERSION, VERSION_MISMATCH_PREFIX,
};

const SCROLLBACK_CAP: usize = 256 * 1024;
/// How often the idle-shutdown task checks the registry.
const IDLE_CHECK_INTERVAL: std::time::Duration = std::time::Duration::from_secs(30);
/// Consecutive empty checks before shutting down (~10 minutes at the
/// interval above) — matches the grace period suggested in design doc §5.
const IDLE_TICKS_BEFORE_EXIT: u32 = 20;

struct Session {
    writer: Mutex<Box<dyn Write + Send>>,
    master: Mutex<Box<dyn MasterPty + Send>>,
    child: Mutex<Box<dyn portable_pty::Child + Send>>,
    #[allow(dead_code)]
    scrollback: Arc<Mutex<Vec<u8>>>,
    tx: broadcast::Sender<Vec<u8>>,
    pid: Option<u32>,
    /// The command that was spawned — shown to the user in the
    /// sessions/orphans list so they can tell sessions apart.
    label: String,
}

type Registry = Arc<Mutex<HashMap<String, Arc<Session>>>>;
type Orphans = Arc<Mutex<Vec<OrphanInfo>>>;

/// Mirrors `SessionMeta` but without derived fields (attached client count)
/// that only make sense while the daemon holding the live registry is
/// running — this is what gets written to disk so a *future* `vmuxd`
/// instance (after a crash) can recognize what used to be running.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct PersistedSession {
    pid: Option<u32>,
    label: String,
}

fn sessions_file_path() -> PathBuf {
    std::env::temp_dir().join("vmux").join("vmuxd-sessions.json")
}

/// Best-effort snapshot of the current registry to disk, so a future
/// `vmuxd` instance (started after this one crashes) can tell which PIDs
/// used to belong to a session and check whether they're still alive.
fn persist_registry(registry: &Registry) {
    let entries: Vec<PersistedSession> = registry.lock().unwrap().values()
        .map(|s| PersistedSession { pid: s.pid, label: s.label.clone() })
        .collect();
    let path = sessions_file_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(json) = serde_json::to_string(&entries) {
        let _ = std::fs::write(&path, json);
    }
}

/// Reads whatever sidecar file a previous (now-not-running, by construction
/// — see `daemon_client::ensure_daemon_running`) `vmuxd` instance left
/// behind, and checks which of its PIDs are still alive. Those are real
/// orphans: unreachable (their anonymous pipe handles died with the old
/// daemon) but still running, per docs §2/§12. This can only recognize
/// them, never reattach — recognition + a kill button beats requiring
/// Task Manager.
fn detect_orphans() -> Vec<OrphanInfo> {
    let path = sessions_file_path();
    let Ok(data) = std::fs::read_to_string(&path) else { return Vec::new() };
    let Ok(entries) = serde_json::from_str::<Vec<PersistedSession>>(&data) else { return Vec::new() };

    let mut sys = System::new_all();
    sys.refresh_all();

    entries.into_iter()
        .filter_map(|e| {
            let pid = e.pid?;
            sys.process(Pid::from_u32(pid)).map(|_| OrphanInfo { pid, label: e.label })
        })
        .collect()
}

fn spawn_session(
    session_id: String,
    shell_path: &str,
    args: &[String],
    env: &[(String, String)],
    cwd: Option<&str>,
    cols: u16,
    rows: u16,
    registry: Registry,
) -> anyhow::Result<Arc<Session>> {
    let pty_system = native_pty_system();
    let pair = pty_system.openpty(PtySize { rows, cols, pixel_width: 0, pixel_height: 0 })?;

    let mut cmd = CommandBuilder::new(shell_path);
    for a in args { cmd.arg(a); }
    for (k, v) in env { cmd.env(k, v); }
    if let Some(dir) = cwd { cmd.cwd(dir); }
    let child = pair.slave.spawn_command(cmd)?;
    let pid = child.process_id();
    println!("[vmuxd] spawned {shell_path}, pid={pid:?}, session={session_id}");

    let mut writer = pair.master.take_writer()?;
    let mut reader = pair.master.try_clone_reader()?;

    // No real VT parser here (that's alacritty_terminal's job in the real
    // app once the pane is attached) — reply to the shell's initial cursor
    // position query so it doesn't block waiting for a response.
    let _ = writer.write_all(b"\x1b[24;1R");

    let scrollback: Arc<Mutex<Vec<u8>>> = Arc::new(Mutex::new(Vec::new()));
    let (tx, _rx) = broadcast::channel::<Vec<u8>>(1024);

    let session = Arc::new(Session {
        writer: Mutex::new(writer),
        master: Mutex::new(pair.master),
        child: Mutex::new(child),
        scrollback: scrollback.clone(),
        tx: tx.clone(),
        pid,
        label: shell_path.to_string(),
    });

    registry.lock().unwrap().insert(session_id.clone(), session.clone());
    persist_registry(&registry);

    std::thread::spawn(move || {
        let mut buf = [0u8; 8192];
        loop {
            match reader.read(&mut buf) {
                Ok(0) => { let _ = tx.send(Vec::new()); break; }
                Ok(n) => {
                    let data = buf[..n].to_vec();
                    {
                        let mut sb = scrollback.lock().unwrap();
                        sb.extend_from_slice(&data);
                        if sb.len() > SCROLLBACK_CAP {
                            let excess = sb.len() - SCROLLBACK_CAP;
                            sb.drain(0..excess);
                        }
                    }
                    let _ = tx.send(data);
                }
                Err(_) => break,
            }
        }
        // Process exited on its own (not via an explicit Kill request) —
        // remove it from the registry so it doesn't linger as a zombie
        // entry ListSessions would report, and so idle-shutdown can
        // actually observe an empty registry. Kill also removes the entry;
        // whichever happens first wins, the other is a harmless no-op.
        registry.lock().unwrap().remove(&session_id);
        persist_registry(&registry);
        println!("[vmuxd] session {session_id} exited");
    });

    Ok(session)
}

/// Kills every currently-registered session's process — used both for an
/// explicit `Shutdown` request and for replacing a stale (version-
/// mismatched) daemon. A real UX tradeoff (every running session ends),
/// only ever triggered deliberately, never silently.
fn shutdown_all(registry: &Registry) {
    let sessions: Vec<Arc<Session>> = registry.lock().unwrap().drain().map(|(_, s)| s).collect();
    for s in &sessions {
        let _ = s.child.lock().unwrap().kill();
    }
    persist_registry(registry);
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    println!("[vmuxd] listening on {PIPE_NAME}, protocol v{PROTOCOL_VERSION}");
    let registry: Registry = Arc::new(Mutex::new(HashMap::new()));

    let orphan_list = detect_orphans();
    if !orphan_list.is_empty() {
        println!("[vmuxd] found {} orphaned process(es) from a previous crash: {orphan_list:?}", orphan_list.len());
    }
    let orphans: Orphans = Arc::new(Mutex::new(orphan_list));
    // Fresh (empty) registry overwrites whatever sidecar file the previous,
    // now-dead instance left behind.
    persist_registry(&registry);

    {
        let registry = registry.clone();
        tokio::spawn(async move {
            let mut idle_ticks = 0u32;
            loop {
                tokio::time::sleep(IDLE_CHECK_INTERVAL).await;
                if registry.lock().unwrap().is_empty() {
                    idle_ticks += 1;
                    if idle_ticks >= IDLE_TICKS_BEFORE_EXIT {
                        println!("[vmuxd] no sessions for {} checks, shutting down", idle_ticks);
                        std::process::exit(0);
                    }
                } else {
                    idle_ticks = 0;
                }
            }
        });
    }

    loop {
        let server = ServerOptions::new().create(PIPE_NAME)?;
        server.connect().await?;
        let registry = registry.clone();
        let orphans = orphans.clone();
        tokio::spawn(async move {
            if let Err(e) = handle_client(server, registry, orphans).await {
                println!("[vmuxd] client handler error: {e}");
            }
        });
    }
}

async fn handle_client(pipe: NamedPipeServer, registry: Registry, orphans: Orphans) -> anyhow::Result<()> {
    let (mut read_half, mut write_half) = tokio::io::split(pipe);

    // Every connection starts with Hello. A mismatch only ever accepts a
    // follow-up Shutdown (used to replace a stale daemon after an app
    // update) — anything else on a mismatched connection is refused.
    match read_framed::<Request, _>(&mut read_half).await? {
        Some(Request::Hello { version }) if version == PROTOCOL_VERSION => {
            write_framed(&mut write_half, &Event::HelloAck).await?;
        }
        Some(Request::Hello { version }) => {
            write_framed(&mut write_half, &Event::Error(
                format!("{VERSION_MISMATCH_PREFIX}: client v{version}, daemon v{PROTOCOL_VERSION}")
            )).await?;
            if let Ok(Some(Request::Shutdown)) = read_framed::<Request, _>(&mut read_half).await {
                println!("[vmuxd] shutting down (stale version, replacement requested)");
                shutdown_all(&registry);
                let _ = write_framed(&mut write_half, &Event::Ok).await;
                std::process::exit(0);
            }
            return Ok(());
        }
        other => {
            write_framed(&mut write_half, &Event::Error(format!("expected Hello, got {other:?}"))).await?;
            return Ok(());
        }
    }

    // Next message is either a session request (Spawn/Attach) or a one-shot
    // control-plane request — the latter reply and close, no session loop.
    let (session_id, session): (String, Arc<Session>) = match read_framed::<Request, _>(&mut read_half).await? {
        Some(Request::Spawn { shell_path, args, env, cwd, cols, rows }) => {
            let session_id = uuid::Uuid::new_v4().to_string();
            match spawn_session(session_id.clone(), &shell_path, &args, &env, cwd.as_deref(), cols, rows, registry.clone()) {
                Ok(session) => {
                    write_framed(&mut write_half, &Event::Spawned { session_id: session_id.clone(), pid: session.pid }).await?;
                    (session_id, session)
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
                Some(session) => (session_id, session),
                None => {
                    write_framed(&mut write_half, &Event::Error(format!("no such session: {session_id}"))).await?;
                    return Ok(());
                }
            }
        }
        Some(Request::Kill { session_id }) => {
            // Standalone kill — e.g. from the Settings panel, for a session
            // with no attached pane to route this through. Session-scoped
            // Kill (sent by an already-attached client, mid-session) is
            // handled further down in the per-session loop instead.
            let found = registry.lock().unwrap().remove(&session_id);
            persist_registry(&registry);
            let evt = match found {
                Some(session) => {
                    let _ = session.child.lock().unwrap().kill();
                    println!("[vmuxd] killed session {session_id} (standalone request)");
                    Event::Ok
                }
                None => Event::Error(format!("no such session: {session_id}")),
            };
            write_framed(&mut write_half, &evt).await?;
            return Ok(());
        }
        Some(Request::ListSessions) => {
            let sessions: Vec<SessionMeta> = registry.lock().unwrap().iter()
                .map(|(id, s)| SessionMeta {
                    session_id: id.clone(),
                    pid: s.pid,
                    label: s.label.clone(),
                    attached_clients: s.tx.receiver_count() as u32,
                })
                .collect();
            write_framed(&mut write_half, &Event::SessionList(sessions)).await?;
            return Ok(());
        }
        Some(Request::ListOrphans) => {
            let list = orphans.lock().unwrap().clone();
            write_framed(&mut write_half, &Event::OrphanList(list)).await?;
            return Ok(());
        }
        Some(Request::KillOrphan { pid }) => {
            let mut sys = System::new_all();
            sys.refresh_all();
            let killed = sys.process(Pid::from_u32(pid)).map(|p| p.kill()).unwrap_or(false);
            orphans.lock().unwrap().retain(|o| o.pid != pid);
            let evt = if killed { Event::Ok } else { Event::Error(format!("could not kill pid {pid}")) };
            write_framed(&mut write_half, &evt).await?;
            return Ok(());
        }
        Some(Request::Shutdown) => {
            println!("[vmuxd] shutting down (requested)");
            shutdown_all(&registry);
            let _ = write_framed(&mut write_half, &Event::Ok).await;
            std::process::exit(0);
        }
        other => {
            write_framed(&mut write_half, &Event::Error(format!("expected Spawn/Attach/ListSessions/ListOrphans/KillOrphan/Shutdown, got {other:?}"))).await?;
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
            Ok(Some(Request::Kill { session_id })) => {
                registry.lock().unwrap().remove(&session_id);
                persist_registry(&registry);
                let _ = session.child.lock().unwrap().kill();
                println!("[vmuxd] killed session {session_id}");
                break;
            }
            Ok(Some(_)) => {} // Hello/Spawn/Attach/etc. only valid earlier in the connection
            Ok(None) => break,
            Err(e) => { println!("[vmuxd] recv error: {e}"); break; }
        }
    }

    output_task.abort();
    let _ = session_id; // silence unused warning on the plain-disconnect path
    // Client disconnected (without Kill) — session (and its registry entry) live on.
    Ok(())
}

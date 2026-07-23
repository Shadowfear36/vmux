// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use vmux_lib::ipc::{IpcRequest, IpcResponse, PIPE_NAME};
use vmux_lib::terminal::daemon_client::{read_framed, write_framed};

fn main() {
    // `vmux` (no args) or `vmux <dir>` — resolve a directory argument (if
    // any) to an absolute path up front, before deciding whether to attach
    // to an already-running instance or launch fresh.
    let path = std::env::args().nth(1).map(|p| {
        std::fs::canonicalize(&p)
            .map(|abs| abs.to_string_lossy().trim_start_matches(r"\\?\").to_string())
            .unwrap_or(p)
    });

    // Single-instance enforcement: if vmux is already running, send it the
    // requested path (or nothing, for bare `vmux`) over the same IPC pipe
    // `vmuxctl` uses, and exit without ever starting a second GUI process —
    // it just opens/focuses a workspace for that directory and brings the
    // window to front. Only launch fresh if no instance is reachable.
    if try_attach(path.clone()) {
        return;
    }

    vmux_lib::run(path);
}

/// Returns true if an existing vmux instance was reached and handled the
/// request (in which case this process should exit immediately).
fn try_attach(path: Option<String>) -> bool {
    let rt = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
        Ok(rt) => rt,
        Err(_) => return false,
    };
    rt.block_on(async {
        let mut client = match tokio::net::windows::named_pipe::ClientOptions::new().open(PIPE_NAME) {
            Ok(c) => c,
            Err(_) => return false, // No instance running — proceed with a normal launch.
        };
        if write_framed(&mut client, &IpcRequest::OpenPath { path }).await.is_err() {
            return false;
        }
        matches!(read_framed::<IpcResponse, _>(&mut client).await, Ok(Some(_)))
    })
}

//! Manual integration smoke test for Phases 2 and 3 of
//! docs/session-reattach-design.md. Unlike vmuxd_proto.rs (which validated
//! the daemon/protocol idea in isolation), this exercises the REAL
//! integration code path — `TerminalPane::spawn_maybe_daemon`,
//! `TerminalPane::attach_daemon`, and `TerminalPane::write_input` — proving
//! the daemon-backed `PtyBackend` and reattachment work through the actual
//! app plumbing, not a reimplementation.
//!
//! Run:
//!   cargo run --manifest-path src-tauri/Cargo.toml --bin vmuxd_integration_check

use vmux_lib::terminal::shell::ShellProfile;
use vmux_lib::terminal::TerminalPane;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    std::env::set_var("VMUX_DAEMON_TERMINALS", "1");

    let shell = ShellProfile {
        id: "cmd".to_string(),
        name: "Command Prompt".to_string(),
        path: "cmd.exe".to_string(),
        args: vec![],
        env: vec![],
    };

    println!("[check] spawning via the real TerminalPane::spawn_maybe_daemon (daemon-backed)...");
    let (mut pane, mut rx) = TerminalPane::spawn_maybe_daemon(None, &shell).await?;
    println!("[check] pane created: id={} pid={:?}", pane.info.id, pane.info.pid);

    let mut got_output = false;
    for _ in 0..20 {
        match tokio::time::timeout(std::time::Duration::from_millis(300), rx.recv()).await {
            Ok(Some(data)) => { got_output = true; print!("{}", String::from_utf8_lossy(&data)); }
            Ok(None) => break,
            Err(_) => if got_output { break } else { continue },
        }
    }
    if !got_output {
        anyhow::bail!("FAIL: no output received from the daemon-backed session");
    }

    println!("\n[check] writing a command via TerminalPane::write_input...");
    pane.write_input(b"echo INTEGRATION_CHECK_OK\r\n")?;

    let mut saw_marker = false;
    for _ in 0..20 {
        match tokio::time::timeout(std::time::Duration::from_millis(300), rx.recv()).await {
            Ok(Some(data)) => {
                let text = String::from_utf8_lossy(&data);
                print!("{text}");
                if text.contains("INTEGRATION_CHECK_OK") { saw_marker = true; }
            }
            Ok(None) => break,
            Err(_) => if saw_marker { break } else { continue },
        }
    }

    if !saw_marker {
        anyhow::bail!("FAIL: did not see the echoed marker — daemon-backed pane path is broken");
    }
    println!("\n[check] PASS (Phase 2): daemon-backed TerminalPane round-tripped a real command correctly.");

    // ── Phase 3: reattachment ────────────────────────────────────────────
    let session_id = pane.info.daemon_session_id.clone().expect("daemon session id should be set");
    println!("\n[check] dropping the pane (simulating vmux closing) — session_id={session_id}");
    drop(pane);
    drop(rx);
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;

    println!("[check] reattaching via the real TerminalPane::attach_daemon...");
    let (mut pane2, mut rx2) = TerminalPane::attach_daemon(&session_id, None, &shell).await?;
    println!("[check] reattached: id={} (fresh pane id, same daemon session)", pane2.info.id);

    let mut replay_text = String::new();
    for _ in 0..20 {
        match tokio::time::timeout(std::time::Duration::from_millis(300), rx2.recv()).await {
            Ok(Some(data)) => { replay_text.push_str(&String::from_utf8_lossy(&data)); }
            Ok(None) => break,
            Err(_) => if !replay_text.is_empty() { break } else { continue },
        }
    }
    print!("{replay_text}");
    if !replay_text.contains("INTEGRATION_CHECK_OK") {
        anyhow::bail!("FAIL: replay after reattach is missing prior session history");
    }

    println!("\n[check] writing a new command through the reattached pane...");
    pane2.write_input(b"echo REATTACH_CHECK_OK\r\n")?;
    let mut saw_reattach_marker = false;
    for _ in 0..20 {
        match tokio::time::timeout(std::time::Duration::from_millis(300), rx2.recv()).await {
            Ok(Some(data)) => {
                let text = String::from_utf8_lossy(&data);
                print!("{text}");
                if text.contains("REATTACH_CHECK_OK") { saw_reattach_marker = true; }
            }
            Ok(None) => break,
            Err(_) => if saw_reattach_marker { break } else { continue },
        }
    }

    if saw_reattach_marker {
        println!("\n[check] PASS (Phase 3): reattached pane saw prior history AND handled new input — same session throughout.");
        Ok(())
    } else {
        anyhow::bail!("FAIL: reattached pane did not respond to new input")
    }
}

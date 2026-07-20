//! Manual integration smoke test for Phase 2 of
//! docs/session-reattach-design.md. Unlike vmuxd_proto.rs (which validated
//! the daemon/protocol idea in isolation), this exercises the REAL
//! integration code path — `vmux_lib::terminal::TerminalPane::spawn_maybe_daemon`
//! and `TerminalPane::write_input` — proving the daemon-backed `PtyBackend`
//! works through the actual app plumbing, not a reimplementation.
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

    if saw_marker {
        println!("\n[check] PASS: daemon-backed TerminalPane round-tripped a real command correctly.");
        Ok(())
    } else {
        anyhow::bail!("FAIL: did not see the echoed marker — daemon-backed pane path is broken");
    }
}

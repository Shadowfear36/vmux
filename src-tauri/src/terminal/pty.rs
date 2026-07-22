use anyhow::Result;
use portable_pty::{native_pty_system, CommandBuilder, MasterPty, PtySize};
use std::io::{Read, Write};
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;

use super::shell::ShellProfile;

/// Prepend the directory containing the running vmux executable to a PATH
/// value. vmuxctl.exe is built alongside vmux.exe (both land in the same
/// target/debug or target/release output dir in dev, and would sit next to
/// each other in a properly-bundled install too) but nothing else puts that
/// directory on PATH — a spawned shell only inherits vmux's own PATH, which
/// is whatever launched `npm run tauri dev`/the installed app, not anywhere
/// vmuxctl.exe lives. Without this, agents can never find `vmuxctl` on PATH.
fn prepend_vmuxctl_dir(existing_path: &str) -> String {
    match std::env::current_exe().ok().and_then(|p| p.parent().map(|p| p.to_path_buf())) {
        Some(exe_dir) => format!("{};{}", exe_dir.display(), existing_path),
        None => existing_path.to_string(),
    }
}

/// Apply the PATH-augmentation to a CommandBuilder, respecting any PATH the
/// caller already set explicitly (e.g. via a shell profile's env overrides).
fn augment_path(cmd: &mut CommandBuilder) {
    let base = cmd.get_env("PATH")
        .map(|v| v.to_string_lossy().to_string())
        .unwrap_or_else(|| std::env::var("PATH").unwrap_or_default());
    cmd.env("PATH", prepend_vmuxctl_dir(&base));
}

/// A running PTY session backed by ConPTY on Windows.
pub struct PtySession {
    _master: Box<dyn MasterPty + Send>,
    /// Shared writer — cloned into the input task and used by write_input().
    writer: Arc<Mutex<Box<dyn Write + Send>>>,
    pub pid: Option<u32>,
}

impl PtySession {
    pub fn spawn(
        cols: u16,
        rows: u16,
        working_dir: Option<&str>,
        shell: &ShellProfile,
    ) -> Result<(Self, mpsc::UnboundedReceiver<Vec<u8>>)> {
        let pty_system = native_pty_system();
        let pair = pty_system.openpty(PtySize {
            rows, cols, pixel_width: 0, pixel_height: 0,
        })?;

        let mut cmd = CommandBuilder::new(&shell.path);
        for arg in &shell.args {
            cmd.arg(arg);
        }
        for (key, val) in &shell.env {
            cmd.env(key, val);
        }
        augment_path(&mut cmd);
        if let Some(dir) = working_dir {
            cmd.cwd(dir);
        } else {
            // Default to the user's home directory
            if let Ok(home) = std::env::var("USERPROFILE").or_else(|_| std::env::var("HOME")) {
                cmd.cwd(&home);
            }
        }

        let child = pair.slave.spawn_command(cmd)?;
        let pid = child.process_id().map(|p| p as u32);

        let writer = Arc::new(Mutex::new(pair.master.take_writer()?));
        let mut reader = pair.master.try_clone_reader()?;

        let (tx, rx) = mpsc::unbounded_channel::<Vec<u8>>();
        std::thread::spawn(move || {
            log::debug!("PTY reader thread started");
            let mut buf = [0u8; 8192];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) => { log::debug!("PTY reader: EOF"); break; }
                    Err(e) => { log::error!("PTY reader error: {e}"); break; }
                    Ok(n) => {
                        log::trace!("PTY reader: {n} bytes read");
                        let _ = tx.send(buf[..n].to_vec());
                    }
                }
            }
        });

        Ok((PtySession { _master: pair.master, writer, pid }, rx))
    }

    /// Spawn an arbitrary command in a PTY (used for agent CLIs).
    pub fn spawn_command(
        cols: u16,
        rows: u16,
        working_dir: Option<&str>,
        command: &str,
        args: &[String],
        env: &[(String, String)],
    ) -> Result<(Self, mpsc::UnboundedReceiver<Vec<u8>>)> {
        let pty_system = native_pty_system();
        let pair = pty_system.openpty(PtySize {
            rows, cols, pixel_width: 0, pixel_height: 0,
        })?;

        let mut cmd = CommandBuilder::new(command);
        for arg in args {
            cmd.arg(arg);
        }
        for (key, val) in env {
            cmd.env(key, val);
        }
        augment_path(&mut cmd);
        if let Some(dir) = working_dir {
            cmd.cwd(dir);
        } else if let Ok(home) = std::env::var("USERPROFILE").or_else(|_| std::env::var("HOME")) {
            cmd.cwd(&home);
        }

        let child = pair.slave.spawn_command(cmd)?;
        let pid = child.process_id().map(|p| p as u32);

        let writer = Arc::new(Mutex::new(pair.master.take_writer()?));
        let mut reader = pair.master.try_clone_reader()?;

        let (tx, rx) = mpsc::unbounded_channel::<Vec<u8>>();
        std::thread::spawn(move || {
            log::debug!("PTY reader thread started (command)");
            let mut buf = [0u8; 8192];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) => { log::debug!("PTY reader: EOF"); break; }
                    Err(e) => { log::error!("PTY reader error: {e}"); break; }
                    Ok(n) => { let _ = tx.send(buf[..n].to_vec()); }
                }
            }
        });

        Ok((PtySession { _master: pair.master, writer, pid }, rx))
    }

    pub fn write(&self, data: &[u8]) -> Result<()> {
        self.writer.lock().map_err(|e| anyhow::anyhow!("{e}"))?
            .write_all(data)?;
        Ok(())
    }

    /// Clone the writer handle so it can be used from other tasks (e.g. Win32 input task).
    pub fn writer_handle(&self) -> Arc<Mutex<Box<dyn Write + Send>>> {
        self.writer.clone()
    }

    pub fn resize(&self, cols: u16, rows: u16) -> Result<()> {
        self._master.resize(PtySize { rows, cols, pixel_width: 0, pixel_height: 0 })?;
        Ok(())
    }
}

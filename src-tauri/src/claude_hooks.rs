/// Claude Code hook integration for vmux.
///
/// Two responsibilities:
/// 1. Install vmux-aware hooks into ~/.claude/settings.json so Claude emits
///    lifecycle events (Stop, Notification, SessionStart, TaskCompleted) to a
///    side-channel file that vmux watches.
/// 2. Watch the notify file for events and emit Tauri events to the frontend.

use anyhow::Result;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::fs;
use std::io::{BufRead, BufReader, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tauri::{AppHandle, Emitter};

// ─── Hook Installation ──────────────────────────────────────────────────────

/// Marker string used to identify vmux-installed hooks.
const VMUX_HOOK_MARKER: &str = "VMUX_NOTIFY_FILE";

fn claude_settings_path() -> Option<PathBuf> {
    let home = std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .ok()?;
    Some(PathBuf::from(home).join(".claude").join("settings.json"))
}

/// Check if vmux hooks are already installed in Claude's settings.
pub fn has_vmux_hooks() -> bool {
    let Some(path) = claude_settings_path() else { return false };
    let Ok(content) = fs::read_to_string(&path) else { return false };
    content.contains(VMUX_HOOK_MARKER)
}

/// Build a single vmux hook entry that writes to the notify file.
fn vmux_hook(event_tag: &str) -> Value {
    // The hook command reads JSON from stdin (Claude provides hook input),
    // then writes "event_tag\t<json>" to the notify file.
    // Guarded by [ -n "$VMUX_NOTIFY_FILE" ] so it's a no-op outside vmux.
    let command = format!(
        r#"[ -n "$VMUX_NOTIFY_FILE" ] && {{ read -r L; echo "{}\t$L" >> "$VMUX_NOTIFY_FILE"; }} || true"#,
        event_tag
    );
    json!({
        "matcher": "",
        "hooks": [{
            "type": "command",
            "command": command,
            "shell": "bash",
            "async": true
        }]
    })
}

/// Install vmux hooks into ~/.claude/settings.json, preserving existing hooks.
/// Returns Ok(true) if hooks were newly installed, Ok(false) if already present.
pub fn ensure_vmux_hooks() -> Result<bool> {
    if has_vmux_hooks() {
        return Ok(false);
    }

    let path = claude_settings_path()
        .ok_or_else(|| anyhow::anyhow!("cannot determine Claude settings path"))?;

    // Ensure parent directory exists
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    // Read existing settings or start with empty object
    let mut settings: Value = if path.exists() {
        let content = fs::read_to_string(&path)?;
        serde_json::from_str(&content).unwrap_or_else(|_| json!({}))
    } else {
        json!({})
    };

    // Get or create the hooks object
    let hooks = settings
        .as_object_mut()
        .ok_or_else(|| anyhow::anyhow!("settings is not an object"))?
        .entry("hooks")
        .or_insert_with(|| json!({}));

    let hooks_obj = hooks
        .as_object_mut()
        .ok_or_else(|| anyhow::anyhow!("hooks is not an object"))?;

    // For each event, append our vmux hook to the existing array (or create one)
    let events = [
        ("Stop", "stop"),
        ("Notification", "notification"),
        ("SessionStart", "session-start"),
        ("TaskCompleted", "task-done"),
    ];

    for (event_key, tag) in &events {
        let entry = hooks_obj
            .entry(*event_key)
            .or_insert_with(|| json!([]));
        if let Some(arr) = entry.as_array_mut() {
            arr.push(vmux_hook(tag));
        }
    }

    // Write back with pretty formatting
    let content = serde_json::to_string_pretty(&settings)?;
    fs::write(&path, content)?;

    eprintln!("[vmux] Installed Claude hooks in {}", path.display());
    Ok(true)
}

// ─── Notify File Watcher ────────────────────────────────────────────────────

/// Shared registry of active watchers so we can stop them on terminal close.
static WATCHERS: std::sync::LazyLock<std::sync::Mutex<HashMap<String, Arc<AtomicBool>>>> =
    std::sync::LazyLock::new(|| std::sync::Mutex::new(HashMap::new()));

/// Start watching a notify file for Claude hook events.
/// Spawns a background thread that polls the file every 500ms.
/// Events are emitted as `claude:event` Tauri events.
pub fn start_notify_watcher(terminal_id: String, notify_path: String, app: AppHandle) {
    let stop = Arc::new(AtomicBool::new(false));

    // Register the stop flag
    if let Ok(mut watchers) = WATCHERS.lock() {
        watchers.insert(terminal_id.clone(), stop.clone());
    }

    let tid = terminal_id.clone();
    std::thread::spawn(move || {
        eprintln!("[vmux] notify watcher started for {tid}: {notify_path}");

        // Wait briefly for the file to be created
        let path = Path::new(&notify_path);
        for _ in 0..10 {
            if path.exists() { break; }
            std::thread::sleep(std::time::Duration::from_millis(200));
        }

        let file = match fs::File::open(path) {
            Ok(f) => f,
            Err(e) => {
                eprintln!("[vmux] notify watcher: failed to open {notify_path}: {e}");
                return;
            }
        };
        let mut reader = BufReader::new(file);
        // Seek to end — we only want new events
        let _ = reader.seek(SeekFrom::End(0));

        let mut line = String::new();
        while !stop.load(Ordering::Relaxed) {
            line.clear();
            match reader.read_line(&mut line) {
                Ok(0) => {
                    // No new data — sleep and retry
                    std::thread::sleep(std::time::Duration::from_millis(500));
                }
                Ok(_) => {
                    let trimmed = line.trim();
                    if trimmed.is_empty() { continue; }
                    if let Some((event, data_str)) = trimmed.split_once('\t') {
                        // Try to parse the JSON data
                        let data: Value = serde_json::from_str(data_str)
                            .unwrap_or_else(|_| json!(data_str));
                        let _ = app.emit("claude:event", json!({
                            "terminalId": tid,
                            "event": event,
                            "data": data,
                        }));
                        eprintln!("[vmux] claude event: {event} for {tid}");
                    }
                }
                Err(e) => {
                    eprintln!("[vmux] notify watcher read error: {e}");
                    std::thread::sleep(std::time::Duration::from_millis(1000));
                }
            }
        }

        // Cleanup: remove the notify file
        let _ = fs::remove_file(path);
        eprintln!("[vmux] notify watcher stopped for {tid}");
    });
}

/// Stop the notify watcher for a terminal and clean up.
pub fn stop_notify_watcher(terminal_id: &str) {
    if let Ok(mut watchers) = WATCHERS.lock() {
        if let Some(stop) = watchers.remove(terminal_id) {
            stop.store(true, Ordering::Relaxed);
        }
    }
}

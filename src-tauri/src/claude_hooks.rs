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
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Emitter, Manager};

use crate::state::AppState;

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

/// Merge vmux's hook entries into a Claude settings JSON value in place,
/// preserving any existing `hooks` entries for the same event.
fn merge_vmux_hooks_into(settings: &mut Value) -> Result<()> {
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

    Ok(())
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

    merge_vmux_hooks_into(&mut settings)?;

    // Write back with pretty formatting
    let content = serde_json::to_string_pretty(&settings)?;
    fs::write(&path, content)?;

    log::info!("Installed Claude hooks in {}", path.display());
    Ok(true)
}

// ─── Notify File Watcher ────────────────────────────────────────────────────

/// Shared registry of active watchers so we can stop them on terminal close.
static WATCHERS: std::sync::LazyLock<std::sync::Mutex<HashMap<String, Arc<AtomicBool>>>> =
    std::sync::LazyLock::new(|| std::sync::Mutex::new(HashMap::new()));

/// Start watching a notify file for Claude hook events.
/// Spawns a background thread that polls the file every 500ms.
/// Events are emitted as `claude:event` Tauri events. If `working_dir` is
/// given, each event also triggers a scoped re-import of that project's
/// Claude transcripts, so History picks up new turns without the user ever
/// clicking "Import Claude".
pub fn start_notify_watcher(terminal_id: String, notify_path: String, working_dir: Option<String>, app: AppHandle) {
    let stop = Arc::new(AtomicBool::new(false));

    // Register the stop flag
    if let Ok(mut watchers) = WATCHERS.lock() {
        watchers.insert(terminal_id.clone(), stop.clone());
    }

    let tid = terminal_id.clone();
    std::thread::spawn(move || {
        log::debug!("notify watcher started for {tid}: {notify_path}");

        // Wait briefly for the file to be created
        let path = Path::new(&notify_path);
        for _ in 0..10 {
            if path.exists() { break; }
            std::thread::sleep(std::time::Duration::from_millis(200));
        }

        let file = match fs::File::open(path) {
            Ok(f) => f,
            Err(e) => {
                log::error!("notify watcher: failed to open {notify_path}: {e}");
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
                        log::debug!("claude event: {event} for {tid}");

                        if let Some(dir) = &working_dir {
                            let state = app.state::<Mutex<AppState>>();
                            let db_path = state.lock().unwrap().context_db_path.clone();
                            match crate::context_store::ContextStore::new(&db_path) {
                                Ok(store) => match crate::transcript::import_all_transcripts_for_project(&store, dir) {
                                    Ok(n) if n > 0 => log::debug!("auto-import after claude event: {n} new chunks"),
                                    Ok(_) => {}
                                    Err(e) => log::warn!("auto-import after claude event failed: {e}"),
                                },
                                Err(e) => log::warn!("auto-import: could not open context store: {e}"),
                            }
                        }
                    }
                }
                Err(e) => {
                    log::error!("notify watcher read error: {e}");
                    std::thread::sleep(std::time::Duration::from_millis(1000));
                }
            }
        }

        // Cleanup: remove the notify file
        let _ = fs::remove_file(path);
        log::debug!("notify watcher stopped for {tid}");
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merge_into_empty_settings_creates_hooks_for_all_events() {
        let mut settings = json!({});
        merge_vmux_hooks_into(&mut settings).unwrap();

        let hooks = &settings["hooks"];
        for event in ["Stop", "Notification", "SessionStart", "TaskCompleted"] {
            let arr = hooks[event].as_array().expect("hook array");
            assert_eq!(arr.len(), 1);
            assert!(arr[0]["hooks"][0]["command"]
                .as_str()
                .unwrap()
                .contains(VMUX_HOOK_MARKER));
        }
    }

    #[test]
    fn merge_preserves_existing_third_party_hooks() {
        let mut settings = json!({
            "hooks": {
                "Stop": [
                    { "matcher": "", "hooks": [{ "type": "command", "command": "echo existing" }] }
                ]
            },
            "otherSetting": true,
        });

        merge_vmux_hooks_into(&mut settings).unwrap();

        let stop_hooks = settings["hooks"]["Stop"].as_array().unwrap();
        assert_eq!(stop_hooks.len(), 2, "existing Stop hook must be preserved, vmux's appended");
        assert_eq!(stop_hooks[0]["hooks"][0]["command"], "echo existing");
        assert!(stop_hooks[1]["hooks"][0]["command"].as_str().unwrap().contains(VMUX_HOOK_MARKER));

        // Unrelated top-level settings must survive untouched.
        assert_eq!(settings["otherSetting"], true);
    }

    #[test]
    fn has_vmux_hooks_detects_marker_in_content() {
        // has_vmux_hooks reads a real file path, but the marker-detection logic
        // itself is a simple substring check — verify it directly here since
        // ensure_vmux_hooks's output is what would be scanned.
        let mut settings = json!({});
        merge_vmux_hooks_into(&mut settings).unwrap();
        let content = serde_json::to_string(&settings).unwrap();
        assert!(content.contains(VMUX_HOOK_MARKER));
    }
}

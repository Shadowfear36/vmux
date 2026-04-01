/// Claude Code transcript importer.
/// Reads JSONL session files from ~/.claude/projects/ and imports them
/// as conversations + chunks into the vmux context store.

use anyhow::Result;
use serde_json::Value;
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

use crate::context_store::ContextStore;

/// Sanitize a directory path the same way Claude Code does:
/// replace non-alphanumeric chars with hyphens.
fn sanitize_path(path: &str) -> String {
    path.chars()
        .map(|c| if c.is_alphanumeric() { c } else { '-' })
        .collect()
}

/// Get the Claude projects directory (~/.claude/projects/).
fn claude_projects_dir() -> Option<PathBuf> {
    let home = std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .ok()?;
    Some(PathBuf::from(home).join(".claude").join("projects"))
}

/// Find all Claude session directories and their JSONL files.
pub fn discover_claude_sessions() -> Vec<(String, PathBuf)> {
    let Some(projects_dir) = claude_projects_dir() else { return vec![] };
    let mut sessions = Vec::new();

    let Ok(entries) = fs::read_dir(&projects_dir) else { return vec![] };
    for entry in entries.flatten() {
        let project_dir = entry.path();
        if !project_dir.is_dir() { continue; }

        let Ok(files) = fs::read_dir(&project_dir) else { continue };
        for file in files.flatten() {
            let path = file.path();
            if path.extension().map_or(true, |e| e != "jsonl") { continue; }
            if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                // stem is the session UUID
                sessions.push((stem.to_string(), path));
            }
        }
    }
    sessions
}

/// Reverse the sanitized project dir name back to a real path.
/// e.g. "C--Users-Dylan-Code-vmux" → "C:/Users/Dylan/Code/vmux"
fn unsanitize_project_dir(dir_name: &str) -> String {
    // Claude Code's sanitization: non-alphanumeric → '-'
    // On Windows: "C:\Users\Dylan\Code\vmux" → "C--Users-Dylan-Code-vmux"
    // We reverse by: split on '-', rejoin with '/', fix drive letter
    let parts: Vec<&str> = dir_name.split('-').filter(|s| !s.is_empty()).collect();
    if parts.is_empty() { return dir_name.to_string(); }

    // First part is drive letter (e.g., "C"), rest are path segments
    let drive = parts[0];
    if parts.len() > 1 {
        format!("{}:/{}", drive, parts[1..].join("/"))
    } else {
        drive.to_string()
    }
}

/// Extract the project directory path from a JSONL file's parent directory name.
fn project_path_from_jsonl(jsonl_path: &Path) -> Option<String> {
    let project_dir = jsonl_path.parent()?;
    let dir_name = project_dir.file_name()?.to_str()?;
    Some(unsanitize_project_dir(dir_name))
}

/// Extract the project name (last path segment) from the project path.
fn project_name_from_path(path: &str) -> String {
    path.split(['/', '\\']).filter(|s| !s.is_empty()).last()
        .unwrap_or("unknown")
        .to_string()
}

/// Extract text content from a Claude message's content field.
/// Handles both string content and array-of-blocks content.
fn extract_text(content: &Value) -> String {
    match content {
        Value::String(s) => s.clone(),
        Value::Array(blocks) => {
            blocks.iter()
                .filter_map(|block| {
                    if block.get("type")?.as_str()? == "text" {
                        block.get("text")?.as_str().map(|s| s.to_string())
                    } else {
                        None
                    }
                })
                .collect::<Vec<_>>()
                .join("\n")
        }
        _ => String::new(),
    }
}

/// Import a single Claude JSONL transcript file into the context store.
/// Returns the number of chunks imported, or 0 if already imported.
pub fn import_transcript(store: &ContextStore, jsonl_path: &Path) -> Result<usize> {
    let session_id = jsonl_path.file_stem()
        .and_then(|s| s.to_str())
        .ok_or_else(|| anyhow::anyhow!("invalid jsonl filename"))?;

    // Skip if already imported
    if store.conversation_exists_by_session(session_id)? {
        return Ok(0);
    }

    let project_path = project_path_from_jsonl(jsonl_path)
        .ok_or_else(|| anyhow::anyhow!("cannot determine project path"))?;
    let project_name = project_name_from_path(&project_path);

    // Ensure project exists
    let project = store.ensure_project(&project_path, &project_name)?;

    // Read and parse JSONL
    let file = fs::File::open(jsonl_path)?;
    let reader = BufReader::new(file);

    let mut chunks: Vec<(String, String)> = Vec::new(); // (role, content)
    let mut title: Option<String> = None;
    let mut first_timestamp: Option<i64> = None;

    for line in reader.lines() {
        let line = line?;
        if line.trim().is_empty() { continue; }

        let entry: Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(_) => continue,
        };

        let entry_type = entry.get("type").and_then(|t| t.as_str()).unwrap_or("");

        match entry_type {
            "user" | "assistant" => {
                if let Some(message) = entry.get("message") {
                    let role = message.get("role").and_then(|r| r.as_str()).unwrap_or(entry_type);
                    let content = message.get("content").map(extract_text).unwrap_or_default();
                    if !content.trim().is_empty() {
                        // Capture first user message as title
                        if title.is_none() && role == "user" {
                            title = Some(content.chars().take(80).collect());
                        }
                        chunks.push((role.to_string(), content));
                    }
                }
                // Capture timestamp
                if first_timestamp.is_none() {
                    if let Some(ts) = entry.get("timestamp").and_then(|t| t.as_str()) {
                        // Parse ISO 8601
                        if let Ok(dt) = ts.parse::<chrono_lite::DateTime>() {
                            first_timestamp = Some(dt.timestamp());
                        }
                    }
                }
            }
            "summary" => {
                // Use AI-generated title if available
                if let Some(t) = entry.get("title").and_then(|t| t.as_str()) {
                    title = Some(t.to_string());
                }
            }
            "custom-title" | "ai-title" => {
                if let Some(t) = entry.get("title").and_then(|t| t.as_str()) {
                    title = Some(t.to_string());
                }
            }
            _ => {} // Skip metadata entries
        }
    }

    if chunks.is_empty() {
        return Ok(0);
    }

    // Create conversation
    let conv = store.create_conversation(
        &project.id,
        "claude",
        Some(session_id),
        title.as_deref(),
        "transcript",
    )?;

    // Insert chunks
    for (i, (role, content)) in chunks.iter().enumerate() {
        store.add_chunk(&conv.id, i as i32, role, content)?;
    }

    let count = chunks.len();
    eprintln!("[vmux] imported transcript {session_id}: {count} chunks, title={:?}", title);
    Ok(count)
}

/// Import all Claude transcripts for a given project directory.
pub fn import_all_transcripts_for_project(store: &ContextStore, project_path: &str) -> Result<usize> {
    let Some(projects_dir) = claude_projects_dir() else {
        return Ok(0);
    };

    let sanitized = sanitize_path(project_path);
    let session_dir = projects_dir.join(&sanitized);

    if !session_dir.exists() {
        return Ok(0);
    }

    let mut total = 0;
    let Ok(files) = fs::read_dir(&session_dir) else { return Ok(0) };
    for entry in files.flatten() {
        let path = entry.path();
        if path.extension().map_or(true, |e| e != "jsonl") { continue; }
        match import_transcript(store, &path) {
            Ok(n) => total += n,
            Err(e) => eprintln!("[vmux] transcript import error: {e}"),
        }
    }
    Ok(total)
}

/// Import all Claude transcripts from all project directories.
pub fn import_all_transcripts(store: &ContextStore) -> Result<usize> {
    let Some(projects_dir) = claude_projects_dir() else {
        return Ok(0);
    };

    let mut total = 0;
    let Ok(entries) = fs::read_dir(&projects_dir) else { return Ok(0) };
    for entry in entries.flatten() {
        let project_dir = entry.path();
        if !project_dir.is_dir() { continue; }

        let Ok(files) = fs::read_dir(&project_dir) else { continue };
        for file in files.flatten() {
            let path = file.path();
            if path.extension().map_or(true, |e| e != "jsonl") { continue; }
            match import_transcript(store, &path) {
                Ok(n) => total += n,
                Err(e) => eprintln!("[vmux] transcript import error: {e}"),
            }
        }
    }
    Ok(total)
}

// Simple ISO 8601 timestamp parser (avoids chrono dependency)
mod chrono_lite {
    pub struct DateTime {
        secs: i64,
    }

    impl DateTime {
        pub fn timestamp(&self) -> i64 { self.secs }
    }

    impl std::str::FromStr for DateTime {
        type Err = ();
        fn from_str(s: &str) -> Result<Self, ()> {
            // Parse "2026-03-31T03:26:18.667Z" format
            let s = s.trim_end_matches('Z');
            let parts: Vec<&str> = s.splitn(2, 'T').collect();
            if parts.len() != 2 { return Err(()); }
            let date_parts: Vec<&str> = parts[0].split('-').collect();
            let time_str = parts[1].split('.').next().unwrap_or(parts[1]);
            let time_parts: Vec<&str> = time_str.split(':').collect();

            if date_parts.len() != 3 || time_parts.len() != 3 { return Err(()); }

            let year: i64 = date_parts[0].parse().map_err(|_| ())?;
            let month: i64 = date_parts[1].parse().map_err(|_| ())?;
            let day: i64 = date_parts[2].parse().map_err(|_| ())?;
            let hour: i64 = time_parts[0].parse().map_err(|_| ())?;
            let min: i64 = time_parts[1].parse().map_err(|_| ())?;
            let sec: i64 = time_parts[2].parse().map_err(|_| ())?;

            // Rough Unix timestamp (good enough for ordering)
            let days = (year - 1970) * 365 + (year - 1969) / 4 + month_days(month) + day - 1;
            let secs = days * 86400 + hour * 3600 + min * 60 + sec;
            Ok(DateTime { secs })
        }
    }

    fn month_days(month: i64) -> i64 {
        match month {
            1 => 0, 2 => 31, 3 => 59, 4 => 90, 5 => 120, 6 => 151,
            7 => 181, 8 => 212, 9 => 243, 10 => 273, 11 => 304, 12 => 334,
            _ => 0,
        }
    }
}

/// Agent CLI profile detection for Windows.
/// Discovers AI coding agent CLIs: Claude Code, Gemini, Codex, Aider, etc.

use serde::{Deserialize, Serialize};
use std::path::Path;

use super::shell::which;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentProfile {
    pub id: String,
    pub name: String,
    /// Absolute path or command name
    pub command: String,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
    /// Emoji icon for sidebar display
    pub icon: String,
}

impl AgentProfile {
    fn new(id: &str, name: &str, command: &str, args: &[&str], icon: &str) -> Self {
        AgentProfile {
            id: id.into(),
            name: name.into(),
            command: command.into(),
            args: args.iter().map(|s| s.to_string()).collect(),
            env: vec![],
            icon: icon.into(),
        }
    }
}

/// Return all detected AI agent CLI profiles.
pub fn detect_agents() -> Vec<AgentProfile> {
    let mut agents = Vec::new();

    // ── Claude Code ──────────────────────────────────────────────────────────
    if let Some(path) = find_agent("claude") {
        agents.push(AgentProfile::new(
            "claude", "Claude Code", &path, &[], "\u{1F7E3}", // 🟣
        ));
    }

    // ── Gemini CLI ───────────────────────────────────────────────────────────
    if let Some(path) = find_agent("gemini") {
        agents.push(AgentProfile::new(
            "gemini", "Gemini", &path, &[], "\u{1F535}", // 🔵
        ));
    }

    // ── OpenAI Codex ─────────────────────────────────────────────────────────
    if let Some(path) = find_agent("codex") {
        agents.push(AgentProfile::new(
            "codex", "Codex", &path, &[], "\u{1F7E2}", // 🟢
        ));
    }

    // ── Aider ────────────────────────────────────────────────────────────────
    if let Some(path) = find_agent("aider") {
        agents.push(AgentProfile::new(
            "aider", "Aider", &path, &[], "\u{1F7E0}", // 🟠
        ));
    }

    // ── Amazon Q / CodeWhisperer ─────────────────────────────────────────────
    if let Some(path) = find_agent("q") {
        agents.push(AgentProfile::new(
            "amazon-q", "Amazon Q", &path, &["chat"], "\u{1F7E1}", // 🟡
        ));
    }

    agents
}

/// Search for an agent CLI executable across PATH and common install locations.
fn find_agent(name: &str) -> Option<String> {
    let exe = format!("{name}.exe");

    // Check PATH first
    if let Some(path) = which(&exe) {
        return Some(path);
    }
    // Also check without .exe (e.g. batch files, shell scripts)
    if let Some(path) = which(name) {
        return Some(path);
    }

    // Check common install locations
    let home = std::env::var("USERPROFILE").ok()?;
    let candidates = [
        // npm global installs
        format!(r"{home}\AppData\Roaming\npm\{exe}"),
        format!(r"{home}\AppData\Roaming\npm\{name}.cmd"),
        // cargo installs
        format!(r"{home}\.cargo\bin\{exe}"),
        // pip/pipx installs
        format!(r"{home}\.local\bin\{exe}"),
        format!(r"{home}\AppData\Roaming\Python\Scripts\{exe}"),
        // Scoop
        format!(r"{home}\scoop\shims\{exe}"),
    ];

    for path in &candidates {
        if Path::new(path).exists() {
            return Some(path.clone());
        }
    }

    None
}

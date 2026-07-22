/// Installs portable Claude Code skills describing how to control vmux from
/// inside a terminal pane — the in-app browser (see `src/osc.rs` for the
/// escape-sequence protocol) and the context/RAG search store (see
/// `src/ipc.rs`'s ContextSearch/ContextGetConversation).
///
/// Unlike claude_hooks.rs, these aren't tied to any one project — each is
/// written to `~/.claude/skills/<id>/SKILL.md` so it's available to any
/// Claude Code session running inside any vmux terminal pane, regardless of
/// which repo that session is working in. Like the hooks installer, this
/// mutates the user's real, shared `~/.claude` config and must only run
/// after explicit user consent (see the Settings panel in the frontend).
use anyhow::Result;
use std::fs;
use std::path::PathBuf;

const BROWSER_SKILL_CONTENT: &str = include_str!("skills/vmux_browser_skill.md");
const CONTEXT_SKILL_CONTENT: &str = include_str!("skills/vmux_context_skill.md");

fn skill_path(id: &str) -> Option<PathBuf> {
    let home = std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .ok()?;
    Some(PathBuf::from(home).join(".claude").join("skills").join(id).join("SKILL.md"))
}

/// Check if a skill file already exists, without modifying anything.
fn has_skill(id: &str) -> bool {
    skill_path(id).is_some_and(|p| p.exists())
}

/// Write (or overwrite) a skill file with the version bundled in this build,
/// so re-running after a vmux upgrade picks up any changes.
fn install_skill(id: &str, content: &str) -> Result<()> {
    let path = skill_path(id).ok_or_else(|| anyhow::anyhow!("cannot determine home directory"))?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&path, content)?;
    log::info!("Installed {id} skill at {}", path.display());
    Ok(())
}

pub fn has_vmux_browser_skill() -> bool {
    has_skill("vmux-browser")
}

pub fn install_vmux_browser_skill() -> Result<()> {
    install_skill("vmux-browser", BROWSER_SKILL_CONTENT)
}

pub fn has_vmux_context_skill() -> bool {
    has_skill("vmux-context")
}

pub fn install_vmux_context_skill() -> Result<()> {
    install_skill("vmux-context", CONTEXT_SKILL_CONTENT)
}

use serde::{Deserialize, Serialize};
use std::path::Path;
use anyhow::Result;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct GitMeta {
    pub branch: Option<String>,
    pub is_dirty: bool,
    pub ahead: u32,
    pub behind: u32,
    pub staged: u32,
    pub unstaged: u32,
}

/// Get git metadata for a working directory using git2.
pub fn get_git_meta(dir: &str) -> Result<GitMeta> {
    let repo = git2::Repository::discover(Path::new(dir))?;

    let head = repo.head()?;
    let branch = head.shorthand().map(|s| s.to_string());

    let mut opts = git2::StatusOptions::new();
    opts.include_untracked(true);
    let statuses = repo.statuses(Some(&mut opts))?;

    let mut staged = 0u32;
    let mut unstaged = 0u32;

    for entry in statuses.iter() {
        let s = entry.status();
        if s.intersects(
            git2::Status::INDEX_NEW
                | git2::Status::INDEX_MODIFIED
                | git2::Status::INDEX_DELETED
                | git2::Status::INDEX_RENAMED
                | git2::Status::INDEX_TYPECHANGE,
        ) {
            staged += 1;
        }
        if s.intersects(
            git2::Status::WT_MODIFIED
                | git2::Status::WT_DELETED
                | git2::Status::WT_RENAMED
                | git2::Status::WT_TYPECHANGE
                | git2::Status::WT_NEW,
        ) {
            unstaged += 1;
        }
    }

    let is_dirty = staged > 0 || unstaged > 0;

    // Count ahead/behind vs upstream
    let (ahead, behind) = match get_ahead_behind(&repo) {
        Ok(ab) => ab,
        Err(_) => (0, 0),
    };

    Ok(GitMeta {
        branch,
        is_dirty,
        ahead,
        behind,
        staged,
        unstaged,
    })
}

fn get_ahead_behind(repo: &git2::Repository) -> Result<(u32, u32)> {
    let head = repo.head()?;
    let local = head.target().ok_or_else(|| anyhow::anyhow!("no target"))?;

    let upstream = repo
        .branch_upstream_name(head.name().unwrap_or(""))
        .ok()
        .and_then(|n| n.as_str().map(|s| s.to_string()))
        .and_then(|n| repo.find_reference(&n).ok())
        .and_then(|r| r.target());

    if let Some(remote) = upstream {
        let (ahead, behind) = repo.graph_ahead_behind(local, remote)?;
        Ok((ahead as u32, behind as u32))
    } else {
        Ok((0, 0))
    }
}

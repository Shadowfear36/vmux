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

/// Changed file with its diff text, for the git diff panel.
#[derive(Debug, Clone, Serialize)]
pub struct ChangedFile {
    pub path: String,
    pub status: String,
    pub diff: String,
}

/// Get list of changed files with their diffs.
pub fn get_changed_files(dir: &str) -> Result<Vec<ChangedFile>> {
    let repo = git2::Repository::discover(Path::new(dir))?;

    let mut opts = git2::StatusOptions::new();
    opts.include_untracked(true);
    let statuses = repo.statuses(Some(&mut opts))?;

    let mut files = Vec::new();

    for entry in statuses.iter() {
        let path = entry.path().unwrap_or("").to_string();
        let s = entry.status();

        let status = if s.intersects(git2::Status::INDEX_NEW | git2::Status::WT_NEW) {
            "Added"
        } else if s.intersects(git2::Status::INDEX_DELETED | git2::Status::WT_DELETED) {
            "Deleted"
        } else if s.intersects(git2::Status::INDEX_RENAMED | git2::Status::WT_RENAMED) {
            "Renamed"
        } else if s.intersects(git2::Status::INDEX_MODIFIED | git2::Status::WT_MODIFIED) {
            "Modified"
        } else {
            "Changed"
        };

        // Get diff for this file
        let diff_text = get_file_diff(&repo, &path).unwrap_or_default();

        files.push(ChangedFile {
            path,
            status: status.to_string(),
            diff: diff_text,
        });
    }

    Ok(files)
}

/// Get the diff text for a single file (working tree vs HEAD).
fn get_file_diff(repo: &git2::Repository, file_path: &str) -> Result<String> {
    let mut diff_opts = git2::DiffOptions::new();
    diff_opts.pathspec(file_path);

    // Diff working tree against HEAD (or index for staged)
    let head_tree = repo.head().ok()
        .and_then(|h| h.peel_to_tree().ok());

    let diff = repo.diff_tree_to_workdir_with_index(
        head_tree.as_ref(),
        Some(&mut diff_opts),
    )?;

    let mut text = String::new();
    diff.print(git2::DiffFormat::Patch, |_delta, _hunk, line| {
        let origin = line.origin();
        match origin {
            '+' => text.push_str(&format!("+{}", std::str::from_utf8(line.content()).unwrap_or(""))),
            '-' => text.push_str(&format!("-{}", std::str::from_utf8(line.content()).unwrap_or(""))),
            ' ' => text.push_str(&format!(" {}", std::str::from_utf8(line.content()).unwrap_or(""))),
            'H' => text.push_str(std::str::from_utf8(line.content()).unwrap_or("")),
            _ => text.push_str(std::str::from_utf8(line.content()).unwrap_or("")),
        }
        true
    })?;

    Ok(text)
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

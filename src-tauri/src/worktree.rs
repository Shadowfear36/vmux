/// Git worktree management.
/// Allows running multiple agents on different branches of the same repo
/// by creating isolated git worktrees.

use anyhow::Result;
use serde::{Serialize, Deserialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorktreeInfo {
    pub path: String,
    pub branch: String,
    pub is_main: bool,
}

/// Create a new git worktree for the given branch.
/// Places it in `<repo>/.worktrees/<branch>`.
/// Returns the absolute path to the worktree.
pub fn create_worktree(repo_path: &str, branch: &str) -> Result<String> {
    let repo = git2::Repository::discover(repo_path)?;
    let repo_root = repo.workdir()
        .ok_or_else(|| anyhow::anyhow!("bare repository"))?;

    let wt_dir = repo_root.join(".worktrees").join(branch);
    std::fs::create_dir_all(wt_dir.parent().unwrap_or(Path::new(".")))?;

    let wt_path_str = wt_dir.to_string_lossy().to_string();

    // Check if branch exists
    let branch_exists = repo.find_branch(branch, git2::BranchType::Local).is_ok();

    if branch_exists {
        // Worktree from existing branch
        let reference = repo.find_branch(branch, git2::BranchType::Local)?;
        let reference = reference.into_reference();
        repo.worktree(branch, &wt_dir, Some(
            git2::WorktreeAddOptions::new()
                .reference(Some(&reference)),
        ))?;
    } else {
        // Create new branch from HEAD, then create worktree
        let head = repo.head()?;
        let commit = head.peel_to_commit()?;
        repo.branch(branch, &commit, false)?;
        let reference = repo.find_branch(branch, git2::BranchType::Local)?;
        let reference = reference.into_reference();
        repo.worktree(branch, &wt_dir, Some(
            git2::WorktreeAddOptions::new()
                .reference(Some(&reference)),
        ))?;
    }

    eprintln!("[vmux] created worktree: {wt_path_str} on branch {branch}");
    Ok(wt_path_str)
}

/// List all worktrees for the repo at the given path.
pub fn list_worktrees(repo_path: &str) -> Result<Vec<WorktreeInfo>> {
    let repo = git2::Repository::discover(repo_path)?;
    let wt_names = repo.worktrees()?;

    let mut results = Vec::new();

    // Add main worktree
    if let Some(main_dir) = repo.workdir() {
        let branch = repo.head().ok()
            .and_then(|h| h.shorthand().map(|s| s.to_string()))
            .unwrap_or_else(|| "HEAD".to_string());
        results.push(WorktreeInfo {
            path: main_dir.to_string_lossy().to_string(),
            branch,
            is_main: true,
        });
    }

    // Add other worktrees
    for name in wt_names.iter() {
        if let Some(name) = name {
            if let Ok(wt) = repo.find_worktree(name) {
                let path = wt.path().to_string_lossy().to_string();
                results.push(WorktreeInfo {
                    path,
                    branch: name.to_string(),
                    is_main: false,
                });
            }
        }
    }

    Ok(results)
}

/// Delete a worktree by its branch name.
/// Removes the worktree directory and the git reference.
pub fn delete_worktree(repo_path: &str, branch: &str) -> Result<()> {
    let repo = git2::Repository::discover(repo_path)?;
    let repo_root = repo.workdir()
        .ok_or_else(|| anyhow::anyhow!("bare repository"))?;

    // Remove the worktree directory
    let wt_dir = repo_root.join(".worktrees").join(branch);
    if wt_dir.exists() {
        std::fs::remove_dir_all(&wt_dir)?;
    }

    // Remove the git worktree metadata
    let git_dir = repo.path(); // .git directory
    let wt_ref = git_dir.join("worktrees").join(branch);
    if wt_ref.exists() {
        std::fs::remove_dir_all(&wt_ref)?;
    }

    eprintln!("[vmux] deleted worktree: {branch}");
    Ok(())
}

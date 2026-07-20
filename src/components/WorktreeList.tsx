import React, { useEffect, useState } from 'react';
import { useStore } from '../store';
import type { WorktreeInfo } from '../types';
import './WorktreeList.css';

interface Props {
  onClose: () => void;
}

export function WorktreeList({ onClose }: Props) {
  const listWorktreesForActiveRepo = useStore(s => s.listWorktreesForActiveRepo);
  const deleteWorktree = useStore(s => s.deleteWorktree);
  const [repoPath, setRepoPath] = useState<string | null>(null);
  const [worktrees, setWorktrees] = useState<WorktreeInfo[]>([]);
  const [loading, setLoading] = useState(true);

  const refresh = async () => {
    setLoading(true);
    const result = await listWorktreesForActiveRepo();
    setRepoPath(result.repoPath);
    setWorktrees(result.worktrees);
    setLoading(false);
  };

  useEffect(() => { refresh(); }, []);

  const handleDelete = async (branch: string) => {
    if (!repoPath) return;
    if (!confirm(`Delete worktree for branch "${branch}"? This removes its directory and git metadata.`)) return;
    try {
      await deleteWorktree(repoPath, branch);
      await refresh();
    } catch (e) {
      alert(`Failed to delete worktree: ${e}`);
    }
  };

  return (
    <div className="wt-list-overlay" onClick={onClose}>
      <div className="wt-list-panel" onClick={e => e.stopPropagation()}>
        <div className="wt-list-header">
          <span>Git Worktrees</span>
          <button className="wt-list-close" onClick={onClose}>x</button>
        </div>
        <div className="wt-list-body">
          {loading && <div className="wt-list-empty">Loading...</div>}
          {!loading && !repoPath && (
            <div className="wt-list-empty">No repo detected for the active workspace/terminal.</div>
          )}
          {!loading && repoPath && worktrees.length === 0 && (
            <div className="wt-list-empty">No worktrees found.</div>
          )}
          {!loading && worktrees.map(wt => (
            <div key={wt.path} className="wt-list-row">
              <div className="wt-list-info">
                <div className="wt-list-branch">
                  {wt.branch}
                  {wt.is_main && <span className="wt-list-main-badge">main</span>}
                </div>
                <div className="wt-list-path">{wt.path}</div>
              </div>
              {!wt.is_main && (
                <button className="wt-list-delete" onClick={() => handleDelete(wt.branch)}>Delete</button>
              )}
            </div>
          ))}
        </div>
        <div className="wt-list-footer">
          <kbd>Ctrl-A w n</kbd> creates a new worktree tab
        </div>
      </div>
    </div>
  );
}

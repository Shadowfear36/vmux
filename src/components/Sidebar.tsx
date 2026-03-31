import React, { useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { useStore } from '../store';
import type { GitMeta } from '../types';
import './Sidebar.css';

interface SidebarProps {
  onShowHelp: () => void;
}

export function Sidebar({ onShowHelp }: SidebarProps) {
  const {
    workspaces, activeWorkspaceId, activeTabId,
    terminals, agents, focusedTerminalId,
    addTab, closeTerminal, focusTerminal,
    createTerminalInTab, createAgentTerminalInTab,
    shells, defaultShellId, setDefaultShell,
    showFileTree, toggleFileTree,
  } = useStore();

  const activeWorkspace = workspaces.find(w => w.id === activeWorkspaceId);

  const activeTab = activeWorkspace?.tabs.find(t => t.id === activeTabId);

  // Get all terminal panes in the active tab
  const paneTerminals = (activeTab?.panes ?? [])
    .filter((p: any) => p.kind?.type === 'terminal')
    .map((p: any) => ({ pane: p, terminal: terminals[p.kind.terminal_id] }))
    .filter(({ terminal }: any) => terminal);

  const addNewPane = (shellId?: string) => {
    if (!activeWorkspaceId || !activeTabId) return;
    const sid = shellId ?? defaultShellId;
    if (sid) setDefaultShell(sid);
    createTerminalInTab(activeWorkspaceId, activeTabId, undefined, sid ?? undefined);
  };

  const addAgentPane = (agentId: string) => {
    if (!activeWorkspaceId || !activeTabId) return;
    // Launch agent in the focused terminal's working directory
    const focusedDir = focusedTerminalId
      ? terminals[focusedTerminalId]?.working_dir ?? undefined
      : undefined;
    createAgentTerminalInTab(activeWorkspaceId, activeTabId, agentId, focusedDir);
  };

  return (
    <div className="sidebar">
      {/* Workspace selector */}
      <div className="sidebar-header">
        <span className="sidebar-logo">vmux</span>
        <WorkspaceSelector />
      </div>

      {/* Panes in workspace */}
      <div className="sidebar-section">
        <div className="sidebar-section-label">
          PANES
          <ShellPickerButton shells={shells} onPick={addNewPane} />
        </div>

        {paneTerminals.map(({ pane, terminal }: any, i: number) => {
          const isFocused = focusedTerminalId === terminal.id;
          const icon = terminal.is_agent
            ? agentIcon(terminal.agent_id ?? '')
            : shellIcon(terminal.shell_id ?? '');
          return (
            <div
              key={pane.id}
              className={`tab-row ${isFocused ? 'tab-row-active' : ''} ${terminal.has_notification ? 'tab-row-notified' : ''}`}
              onClick={() => focusTerminal(terminal.id)}
            >
              <span className="tab-icon">{icon}</span>
              <span className="tab-name">{terminal.title || `Pane ${i + 1}`}</span>
              {terminal.has_notification && (
                <span className="tab-notification-dot" title="Agent notification" />
              )}
              <button
                className="tab-close-btn"
                onClick={e => { e.stopPropagation(); closeTerminal(terminal.id); }}
              >x</button>
            </div>
          );
        })}
      </div>

      {/* Agent launchers */}
      {agents.length > 0 && (
        <div className="sidebar-section">
          <div className="sidebar-section-label">AGENTS</div>
          {agents.map(agent => (
            <button
              key={agent.id}
              className="agent-launch-btn"
              onClick={() => addAgentPane(agent.id)}
              title={`Launch ${agent.name}`}
            >
              <span className="agent-launch-icon">{agent.icon}</span>
              {agent.name}
            </button>
          ))}
        </div>
      )}

      {/* Footer buttons */}
      <div className="sidebar-footer">
        <div className="sidebar-footer-row">
          <button
            className={`sidebar-footer-btn ${showFileTree ? 'active' : ''}`}
            onClick={toggleFileTree}
            title="File tree (Ctrl-A f)"
          >
            Files
          </button>
          <ContextShortcut />
        </div>
        <button
          className="sidebar-footer-btn sidebar-help-btn"
          onClick={onShowHelp}
          title="Keyboard shortcuts (Ctrl-A ?)"
        >
          ? Shortcuts
        </button>
      </div>
    </div>
  );
}

interface ShellPickerButtonProps {
  shells: import('../types').ShellProfile[];
  onPick: (shellId?: string) => void;
}

function ShellPickerButton({ shells, onPick }: ShellPickerButtonProps) {
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!open) return;
    const handler = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) setOpen(false);
    };
    document.addEventListener('mousedown', handler);
    return () => document.removeEventListener('mousedown', handler);
  }, [open]);

  if (shells.length <= 1) {
    return (
      <button
        className="sidebar-add-btn"
        onClick={() => onPick()}
        title="New terminal (Ctrl-A c)"
      >+</button>
    );
  }

  return (
    <div ref={ref} className="shell-picker-wrap">
      <button
        className="sidebar-add-btn"
        onClick={() => onPick()}
        title="New terminal (Ctrl-A c)"
      >+</button>
      <button
        className="sidebar-add-btn shell-picker-arrow"
        onClick={() => setOpen(o => !o)}
        title="Pick shell"
      >v</button>
      {open && (
        <div className="shell-picker-menu">
          {shells.map(s => (
            <button
              key={s.id}
              className="shell-picker-item"
              onClick={() => { onPick(s.id); setOpen(false); }}
            >
              <span className="shell-picker-icon">{shellIcon(s.id)}</span>
              {s.name}
            </button>
          ))}
        </div>
      )}
    </div>
  );
}

function shellIcon(id: string): string {
  switch (id) {
    case 'cmd':        return '>';
    case 'powershell': return '$';
    case 'pwsh':       return '$';
    case 'gitbash':    return '#';
    default:           return '>';
  }
}

function agentIcon(id: string): string {
  switch (id) {
    case 'claude':   return 'C';
    case 'gemini':   return 'G';
    case 'codex':    return 'X';
    case 'aider':    return 'A';
    case 'amazon-q': return 'Q';
    default:         return '*';
  }
}

function WorkspaceSelector() {
  const { workspaces, activeWorkspaceId, setActiveWorkspace, createWorkspace, renameWorkspace, deleteWorkspace } = useStore();
  const [menuOpen, setMenuOpen] = useState(false);
  const [renaming, setRenaming] = useState(false);
  const [renameValue, setRenameValue] = useState('');
  const ref = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!menuOpen) return;
    const handler = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) {
        setMenuOpen(false);
        setRenaming(false);
      }
    };
    document.addEventListener('mousedown', handler);
    return () => document.removeEventListener('mousedown', handler);
  }, [menuOpen]);

  const activeWs = workspaces.find(w => w.id === activeWorkspaceId);

  const handleRename = () => {
    if (renameValue.trim() && activeWorkspaceId) {
      renameWorkspace(activeWorkspaceId, renameValue.trim());
      setRenaming(false);
      setMenuOpen(false);
    }
  };

  const handleDelete = () => {
    if (activeWorkspaceId && workspaces.length > 1) {
      deleteWorkspace(activeWorkspaceId);
      setMenuOpen(false);
    }
  };

  return (
    <div ref={ref} className="workspace-selector">
      <select
        value={activeWorkspaceId ?? ''}
        onChange={e => setActiveWorkspace(e.target.value)}
        className="workspace-select"
      >
        {workspaces.map(ws => (
          <option key={ws.id} value={ws.id}>{ws.name}</option>
        ))}
      </select>
      <button
        className="sidebar-add-btn"
        onClick={() => setMenuOpen(o => !o)}
        title="Workspace options"
      >...</button>
      <button
        className="sidebar-add-btn"
        onClick={() => createWorkspace(`Project ${workspaces.length + 1}`)}
        title="New workspace (Ctrl-A w)"
      >+</button>

      {menuOpen && (
        <div className="workspace-menu">
          {renaming ? (
            <div className="workspace-menu-rename">
              <input
                className="workspace-rename-input"
                value={renameValue}
                onChange={e => setRenameValue(e.target.value)}
                onKeyDown={e => e.key === 'Enter' && handleRename()}
                autoFocus
                placeholder="Workspace name"
              />
              <button className="workspace-menu-item" onClick={handleRename}>Save</button>
            </div>
          ) : (
            <>
              <button
                className="workspace-menu-item"
                onClick={() => { setRenameValue(activeWs?.name ?? ''); setRenaming(true); }}
              >
                Rename
              </button>
              {workspaces.length > 1 && (
                <button
                  className="workspace-menu-item workspace-menu-delete"
                  onClick={handleDelete}
                >
                  Delete
                </button>
              )}
            </>
          )}
        </div>
      )}
    </div>
  );
}

/** Shows per-terminal metadata: git branch, notification ring */
export function TerminalMetaBar({ terminalId }: { terminalId: string }) {
  const terminal = useStore(s => s.terminals[terminalId]);
  const clearNotification = useStore(s => s.clearNotification);
  const [git, setGit] = useState<GitMeta | null>(null);

  useEffect(() => {
    if (terminal?.working_dir) {
      invoke<GitMeta>('get_git_metadata', { dir: terminal.working_dir })
        .then(setGit)
        .catch(() => setGit(null));
    }
    const id = setInterval(() => {
      if (terminal?.working_dir) {
        invoke<GitMeta>('get_git_metadata', { dir: terminal.working_dir })
          .then(setGit).catch(() => {});
      }
    }, 3000);
    return () => clearInterval(id);
  }, [terminal?.working_dir]);

  if (!terminal) return null;

  return (
    <div className={`terminal-meta-bar ${terminal.has_notification ? 'terminal-meta-notified' : ''}`}>
      {terminal.has_notification && (
        <button
          className="notification-badge"
          onClick={() => clearNotification(terminalId)}
          title={terminal.notification_message ?? 'Agent notification'}
        >
          * {terminal.notification_message ?? 'notification'}
        </button>
      )}
      {git?.branch && (
        <span className="git-branch">
          {git.branch}
          {git.is_dirty && <span className="git-dirty">*</span>}
          {git.ahead > 0 && <span className="git-ahead">+{git.ahead}</span>}
          {git.behind > 0 && <span className="git-behind">-{git.behind}</span>}
        </span>
      )}
      {terminal.working_dir && (
        <span className="working-dir" title={terminal.working_dir}>
          {terminal.working_dir.split(/[\\/]/).slice(-2).join('/')}
        </span>
      )}
    </div>
  );
}

function ContextShortcut() {
  const { toggleContext, showContext } = useStore();
  return (
    <button
      className={`sidebar-footer-btn ${showContext ? 'active' : ''}`}
      onClick={toggleContext}
      title="Context manager (Ctrl-A x)"
    >
      Context
    </button>
  );
}

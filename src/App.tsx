import React, { useEffect, useCallback, useState } from 'react';
import { createPortal } from 'react-dom';
import { listen } from '@tauri-apps/api/event';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { Allotment } from 'allotment';
import 'allotment/dist/style.css';
import { useStore } from './store';
import { Sidebar } from './components/Sidebar';
import { TabView } from './components/TabView';
import { BrowserPane } from './components/BrowserPane';
import { ContextPanel } from './components/ContextPanel';
import { KeyboardHelp } from './components/KeyboardHelp';
import { FileTree } from './components/FileTree';
import './App.css';

/**
 * vmux prefix key system: Ctrl-A (like tmux)
 *
 * Single keys:
 *   c / | / %   → split horizontal (default shell)
 *   - / "       → split vertical (default shell)
 *   t           → new tab
 *   n / p       → next / prev tab
 *   d           → close focused pane
 *   x / b / f / g → toggle panels
 *   ?           → keyboard help
 *
 * Chords (two keys):
 *   s → 1-9     → open shell #N (horizontal split)
 *   s → - → 1-9 → open shell #N (vertical split)
 *   a → 1-9     → open agent #N (horizontal split)
 *   a → - → 1-9 → open agent #N (vertical split)
 *   w → n       → new worktree (prompt for branch)
 *   w → l       → list worktrees
 *   w → +       → new workspace
 */
const PREFIX_KEY = 'a';

type ChordState = null | 'shell' | 'shell-vert' | 'agent' | 'agent-vert' | 'worktree';

let chordState: ChordState = null;

function handlePrefixCommand(key: string, setShowHelp: (fn: (h: boolean) => boolean) => void, setChordLabel: (l: string | null) => void): boolean {
  const store = useStore.getState();
  const ws = store.workspaces.find(w => w.id === store.activeWorkspaceId);

  // ── Chord mode: waiting for second key ──────────────────────────────────
  if (chordState) {
    const state = chordState;
    chordState = null;
    setChordLabel(null);

    // Sub-direction modifier: s/a → - means switch to vertical before picking
    if ((state === 'shell' || state === 'agent') && (key === '-' || key === '"')) {
      chordState = state === 'shell' ? 'shell-vert' : 'agent-vert';
      setChordLabel(state === 'shell' ? 'SHELL ->' : 'AGENT ->');
      return true; // stay in prefix mode
    }

    const direction: 'horizontal' | 'vertical' = (state === 'shell-vert' || state === 'agent-vert') ? 'vertical' : 'horizontal';
    const num = parseInt(key);

    if ((state === 'shell' || state === 'shell-vert') && num >= 1 && num <= 9) {
      const shell = store.shells[num - 1];
      if (shell && ws && store.activeTabId) {
        store.splitFocusedPaneWith('shell', shell.id, direction);
      }
      return false;
    }

    if ((state === 'agent' || state === 'agent-vert') && num >= 1 && num <= 9) {
      const agent = store.agents[num - 1];
      if (agent && ws && store.activeTabId) {
        store.splitFocusedPaneWith('agent', agent.id, direction);
      }
      return false;
    }

    if (state === 'worktree') {
      if (key === 'n') {
        // New worktree — prompt for branch name
        const branch = prompt('Branch name for new worktree:');
        if (branch && branch.trim()) {
          store.createWorktreeTab(branch.trim());
        }
      } else if (key === '+') {
        const count = store.workspaces.length;
        store.createWorkspace(`Project ${count + 1}`);
      }
      return false;
    }

    return false;
  }

  // ── First key after Ctrl-A ──────────────────────────────────────────────
  switch (key) {
    case 'c': case '|': case '%': {
      if (!ws) break;
      if (ws.tabs.length === 0) { store.addTab(ws.id, 'Terminal'); }
      else if (store.focusedTerminalId) { store.splitFocusedPane('horizontal'); }
      else { const tab = ws.tabs.find(t => t.id === store.activeTabId); if (tab && store.activeWorkspaceId) store.createTerminalInTab(store.activeWorkspaceId, tab.id); }
      break;
    }
    case '-': case '"': {
      if (!ws) break;
      if (store.focusedTerminalId) { store.splitFocusedPane('vertical'); }
      else { const tab = ws.tabs.find(t => t.id === store.activeTabId); if (tab && store.activeWorkspaceId) store.createTerminalInTab(store.activeWorkspaceId, tab.id); }
      break;
    }
    case 's': // Enter shell chord
      chordState = 'shell';
      setChordLabel('SHELL');
      return true; // stay in prefix mode
    case 'a': // Enter agent chord
      chordState = 'agent';
      setChordLabel('AGENT');
      return true;
    case 'w': // Enter worktree chord
      chordState = 'worktree';
      setChordLabel('WORKTREE');
      return true;
    case 't': if (ws) store.addTab(ws.id, `Tab ${ws.tabs.length + 1}`); break;
    case 'n': store.cycleTab('next'); break;
    case 'p': store.cycleTab('prev'); break;
    case 'x': store.toggleContext(); break;
    case 'b': store.toggleBrowser(); break;
    case 'f': store.toggleFileTree(); break;
    case 'g': store.toggleGitDiff(); break;
    case 'd': if (store.focusedTerminalId) store.closeTerminal(store.focusedTerminalId); break;
  }
  return false;
}

export default function App() {
  const {
    workspaces, activeWorkspaceId, activeTabId,
    loadWorkspaces, loadShells, addTab, setActiveTab,
    showBrowser, showContext, showFileTree, showGitDiff,
  } = useStore();

  const [prefixActive, setPrefixActive] = useState(false);
  const [chordLabel, setChordLabel] = useState<string | null>(null);
  const [showHelp, setShowHelp] = useState(false);

  const activeWorkspace = workspaces.find(w => w.id === activeWorkspaceId);
  const activeTab = activeWorkspace?.tabs.find(t => t.id === activeTabId);

  // ── Load initial state ────────────────────────────────────────────────────
  useEffect(() => {
    (async () => {
      await loadWorkspaces();
      await loadShells();
      useStore.getState().loadAgents();

      const store = useStore.getState();
      if (store.activeWorkspaceId) {
        store.loadContext(store.activeWorkspaceId);
      }

      if (store.activeWorkspaceId) {
        const ws = store.workspaces.find(w => w.id === store.activeWorkspaceId);
        if (ws && ws.tabs.some(t => t.panes.length > 0)) {
          await store.restoreWorkspacePanes(store.activeWorkspaceId);
        }
      }
    })();
  }, []);

  // ── Auto-save workspace state periodically + on close ─────────────────
  useEffect(() => {
    const interval = setInterval(() => {
      useStore.getState().saveWorkspaceState();
    }, 5000); // Save every 5 seconds
    // Tauri window close event (beforeunload doesn't fire reliably in Tauri)
    const unlistenClose = getCurrentWindow().onCloseRequested(() => {
      useStore.getState().saveWorkspaceState();
    });
    return () => { clearInterval(interval); unlistenClose.then(f => f()); };
  }, []);

  // ── Event listeners ──────────────────────────────────────────────────────
  useEffect(() => {
    const unsub = listen<{ terminalId: string; message: string }>(
      'terminal:notification',
      ({ payload }) => { useStore.getState().setNotification(payload.terminalId, payload.message); }
    );
    return () => { unsub.then(f => f()); };
  }, []);

  useEffect(() => {
    const unsub = listen<{ terminalId: string; cwd: string }>(
      'terminal:cwd',
      ({ payload }) => { useStore.getState().setTerminalCwd(payload.terminalId, payload.cwd); }
    );
    return () => { unsub.then(f => f()); };
  }, []);

  useEffect(() => {
    const unsub = listen<{ terminalId: string; title: string }>(
      'terminal:title',
      ({ payload }) => { useStore.getState().setTerminalTitle(payload.terminalId, payload.title); }
    );
    return () => { unsub.then(f => f()); };
  }, []);

  // ── Claude Code hook events (via notify file watcher) ───────────────────
  useEffect(() => {
    const unsub = listen<{ terminalId: string; event: string; data: any }>(
      'claude:event',
      ({ payload }) => {
        const store = useStore.getState();
        switch (payload.event) {
          case 'stop':
            store.setNotification(payload.terminalId, 'Claude finished');
            break;
          case 'notification':
            store.setNotification(
              payload.terminalId,
              payload.data?.message ?? 'Claude notification',
            );
            break;
          case 'session-start':
            if (payload.data?.session_id) {
              store.setClaudeSessionId(payload.terminalId, payload.data.session_id);
            }
            break;
          case 'task-done':
            store.setNotification(payload.terminalId, 'Task completed');
            break;
        }
      }
    );
    return () => { unsub.then(f => f()); };
  }, []);

  useEffect(() => {
    const unsubs: Promise<() => void>[] = [];
    unsubs.push(listen<{ terminalId: string; url: string }>('agent:browser-open', ({ payload }) => {
      const store = useStore.getState();
      if (!store.showBrowser) store.toggleBrowser();
      setTimeout(() => store.browserNavigate(payload.url), 300);
    }));
    unsubs.push(listen<{ terminalId: string; url: string }>('agent:browser-navigate', ({ payload }) => {
      useStore.getState().browserNavigate(payload.url);
    }));
    unsubs.push(listen<{ terminalId: string }>('agent:browser-close', () => {
      const store = useStore.getState();
      if (store.showBrowser) store.toggleBrowser();
    }));
    unsubs.push(listen<{ terminalId: string; js: string }>('agent:browser-eval', ({ payload }) => {
      useStore.getState().browserEvaluate(payload.js);
    }));
    return () => { unsubs.forEach(u => u.then(f => f())); };
  }, []);

  // ── Keyboard prefix ──────────────────────────────────────────────────────
  useEffect(() => {
    let jsPrefixPending = false;
    const handleKeyDown = (e: KeyboardEvent) => {
      const tag = (e.target as HTMLElement).tagName;
      if (tag === 'INPUT' || tag === 'TEXTAREA' || tag === 'SELECT') return;
      if (!jsPrefixPending && e.ctrlKey && e.key === 'a') {
        e.preventDefault(); jsPrefixPending = true; setPrefixActive(true); return;
      }
      if (jsPrefixPending) {
        e.preventDefault();
        if (e.key === '?') { jsPrefixPending = false; setPrefixActive(false); setShowHelp(h => !h); return; }
        const stayInPrefix = handlePrefixCommand(e.key, setShowHelp, setChordLabel);
        if (!stayInPrefix) { jsPrefixPending = false; setPrefixActive(false); setChordLabel(null); }
      }
    };
    document.addEventListener('keydown', handleKeyDown);
    return () => document.removeEventListener('keydown', handleKeyDown);
  }, []);

  useEffect(() => {
    const unsub1 = listen<{ terminalId: string }>('prefix:activated', () => setPrefixActive(true));
    const unsub2 = listen<{}>('prefix:deactivated', () => { setPrefixActive(false); chordState = null; setChordLabel(null); });
    const unsub3 = listen<{ key: string }>('prefix:command', ({ payload }) => {
      if (payload.key === '?') { setPrefixActive(false); setShowHelp(h => !h); return; }
      const stay = handlePrefixCommand(payload.key, setShowHelp, setChordLabel);
      if (!stay) { setPrefixActive(false); setChordLabel(null); }
    });
    return () => { unsub1.then(f => f()); unsub2.then(f => f()); unsub3.then(f => f()); };
  }, []);

  const tabs = activeWorkspace?.tabs ?? [];

  return (
    <div className="app">
      <Allotment proportionalLayout={false}>
        {/* Sidebar — resizable */}
        <Allotment.Pane minSize={140} preferredSize={220} maxSize={400}>
          <Sidebar onShowHelp={() => setShowHelp(h => !h)} />
        </Allotment.Pane>

        {/* File tree — resizable, toggleable */}
        <Allotment.Pane minSize={120} preferredSize={200} maxSize={400} visible={showFileTree}>
          <FileTree />
        </Allotment.Pane>

        {/* Main content area */}
        <Allotment.Pane minSize={200}>
          <div className="app-main">
            {tabs.length > 1 && (
              <div className="tabbar">
                {tabs.map((tab, i) => (
                  <div
                    key={tab.id}
                    className={`tabbar-item ${tab.id === activeTabId ? 'tabbar-item-active' : ''}`}
                    onClick={() => setActiveTab(tab.id)}
                  >
                    <span className="tabbar-index">{i + 1}</span>
                    {tab.name}
                  </div>
                ))}
              </div>
            )}

            <div className="app-content">
              {/* Terminal + Browser — resizable split */}
              <Allotment>
                <Allotment.Pane minSize={200}>
                  {activeTab ? (
                    <TabView tab={activeTab} />
                  ) : (
                    <WelcomeScreen onStart={() => activeWorkspaceId && addTab(activeWorkspaceId, 'Workspace')} />
                  )}
                </Allotment.Pane>
                <Allotment.Pane minSize={200} preferredSize={500} visible={showBrowser}>
                  <BrowserPane />
                </Allotment.Pane>
              </Allotment>
            </div>
          </div>
        </Allotment.Pane>

        {/* Git diff panel — toggleable */}
        <Allotment.Pane minSize={200} preferredSize={320} maxSize={600} visible={showGitDiff}>
          <GitDiffPanel />
        </Allotment.Pane>

        {/* Context panel — resizable, toggleable */}
        <Allotment.Pane minSize={200} preferredSize={320} maxSize={500} visible={showContext}>
          <ContextPanel />
        </Allotment.Pane>
      </Allotment>

      {/* Portals — render overlays into document.body to escape Allotment stacking context */}
      {prefixActive && createPortal(<div className="prefix-indicator">{chordLabel ? `PREFIX ${chordLabel}` : 'PREFIX'}</div>, document.body)}
      {showHelp && createPortal(<KeyboardHelp onClose={() => setShowHelp(false)} />, document.body)}
    </div>
  );
}

function WelcomeScreen({ onStart }: { onStart: () => void }) {
  return (
    <div className="welcome">
      <div className="welcome-content">
        <div className="welcome-logo">vmux</div>
        <div className="welcome-subtitle">Windows terminal multiplexer for AI agents</div>
        <button className="welcome-btn" onClick={onStart}>New Terminal</button>
        <div className="welcome-keys">
          <div className="key-row"><kbd>Ctrl-A c</kbd> Split horizontal</div>
          <div className="key-row"><kbd>Ctrl-A -</kbd> Split vertical</div>
          <div className="key-row"><kbd>Ctrl-A s #</kbd> Shell picker</div>
          <div className="key-row"><kbd>Ctrl-A a #</kbd> Agent picker</div>
          <div className="key-row"><kbd>Ctrl-A w n</kbd> New worktree</div>
          <div className="key-row"><kbd>Ctrl-A t/n/p</kbd> Tab: new/next/prev</div>
          <div className="key-row"><kbd>Ctrl-A d</kbd> Close pane</div>
          <div className="key-row"><kbd>Ctrl-A b/x/f/g</kbd> Panels</div>
          <div className="key-row"><kbd>Ctrl-A ?</kbd> All shortcuts</div>
        </div>
      </div>
    </div>
  );
}

// ── Git Diff Panel ─────────────────────────────────────────────────────────

import { invoke } from '@tauri-apps/api/core';

interface GitChangedFile {
  path: string;
  status: string;
  diff: string;
}

function GitDiffPanel() {
  const { toggleGitDiff, focusedTerminalId, terminals } = useStore();
  const [files, setFiles] = useState<GitChangedFile[]>([]);
  const [selected, setSelected] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);

  const cwd = focusedTerminalId ? terminals[focusedTerminalId]?.working_dir : null;

  useEffect(() => {
    if (!cwd) { setFiles([]); return; }
    setLoading(true);
    invoke<GitChangedFile[]>('git_changed_files', { dir: cwd })
      .then(setFiles)
      .catch(() => setFiles([]))
      .finally(() => setLoading(false));
  }, [cwd]);

  const refresh = () => {
    if (!cwd) return;
    setLoading(true);
    invoke<GitChangedFile[]>('git_changed_files', { dir: cwd })
      .then(setFiles)
      .catch(() => setFiles([]))
      .finally(() => setLoading(false));
  };

  const selectedFile = files.find(f => f.path === selected);

  return (
    <div className="git-diff-panel">
      <div className="git-diff-header">
        <span>Git Changes</span>
        <div style={{ display: 'flex', gap: 4 }}>
          <button className="git-diff-btn" onClick={refresh} title="Refresh">R</button>
          <button className="git-diff-btn" onClick={toggleGitDiff} title="Close">x</button>
        </div>
      </div>
      {loading && <div className="git-diff-loading">Loading...</div>}
      {!loading && files.length === 0 && (
        <div className="git-diff-empty">{cwd ? 'No changes' : 'Focus a terminal'}</div>
      )}
      <div className="git-diff-files">
        {files.map(f => (
          <div
            key={f.path}
            className={`git-diff-file ${f.path === selected ? 'git-diff-file-active' : ''}`}
            onClick={() => setSelected(f.path === selected ? null : f.path)}
          >
            <span className={`git-diff-status git-diff-status-${f.status.toLowerCase()}`}>
              {f.status.charAt(0)}
            </span>
            <span className="git-diff-path">{f.path}</span>
          </div>
        ))}
      </div>
      {selectedFile && (
        <div className="git-diff-content">
          <pre className="git-diff-pre">{selectedFile.diff || '(no diff)'}</pre>
        </div>
      )}
    </div>
  );
}

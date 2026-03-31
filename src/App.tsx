import React, { useEffect, useCallback, useState } from 'react';
import { listen } from '@tauri-apps/api/event';
import { useStore } from './store';
import { Sidebar } from './components/Sidebar';
import { TabView } from './components/TabView';
import { BrowserPane } from './components/BrowserPane';
import { ContextPanel } from './components/ContextPanel';
import { KeyboardHelp } from './components/KeyboardHelp';
import { FileTree } from './components/FileTree';
import './App.css';

/**
 * vmux prefix key: Ctrl-A (like screen/tmux-style)
 * After pressing prefix, the next key triggers a command:
 *
 *   Ctrl-A c     → split new pane
 *   Ctrl-A t     → new tab
 *   Ctrl-A n     → next tab
 *   Ctrl-A p     → previous tab
 *   Ctrl-A d     → close focused pane
 *   Ctrl-A x     → toggle context panel
 *   Ctrl-A b     → toggle browser pane
 *   Ctrl-A f     → toggle file tree
 *   Ctrl-A w     → new workspace
 *   Ctrl-A ?     → toggle keyboard help
 */
const PREFIX_KEY = 'a'; // i.e. Ctrl-A

function handlePrefixCommand(key: string) {
  const store = useStore.getState();
  const ws = store.workspaces.find(w => w.id === store.activeWorkspaceId);

  switch (key) {
    case 'c': {
      if (!ws) break;
      if (ws.tabs.length === 0) {
        store.addTab(ws.id, 'Terminal');
      } else {
        const tab = ws.tabs.find(t => t.id === store.activeTabId);
        if (tab && store.activeWorkspaceId) {
          setTimeout(() => store.createTerminalInTab(store.activeWorkspaceId!, tab.id), 0);
        }
      }
      break;
    }
    case 't': {
      if (ws) store.addTab(ws.id, `Tab ${ws.tabs.length + 1}`);
      break;
    }
    case 'n': store.cycleTab('next'); break;
    case 'p': store.cycleTab('prev'); break;
    case 'x': store.toggleContext(); break;
    case 'b': store.toggleBrowser(); break;
    case 'f': store.toggleFileTree(); break;
    case 'w': {
      const count = store.workspaces.length;
      store.createWorkspace(`Project ${count + 1}`);
      break;
    }
    case 'd': {
      if (store.focusedTerminalId) {
        store.closeTerminal(store.focusedTerminalId);
      }
      break;
    }
    // '?' is handled in the component since it's local state
  }
}

export default function App() {
  const {
    workspaces, activeWorkspaceId, activeTabId,
    loadWorkspaces, loadShells, addTab, setActiveTab,
    toggleContext, toggleBrowser, showBrowser, showContext, showFileTree,
  } = useStore();

  const [prefixActive, setPrefixActive] = useState(false);
  const [showHelp, setShowHelp] = useState(false);

  const activeWorkspace = workspaces.find(w => w.id === activeWorkspaceId);
  const activeTab = activeWorkspace?.tabs.find(t => t.id === activeTabId);

  // ── Load initial state ────────────────────────────────────────────────────
  useEffect(() => {
    (async () => {
      await loadWorkspaces();
      await loadShells();
      useStore.getState().loadAgents();

      // Load context entries
      const store = useStore.getState();
      if (store.activeWorkspaceId) {
        store.loadContext(store.activeWorkspaceId);
      }

      // Restore terminals for persisted panes
      if (store.activeWorkspaceId) {
        const ws = store.workspaces.find(w => w.id === store.activeWorkspaceId);
        if (ws && ws.tabs.some(t => t.panes.length > 0)) {
          await store.restoreWorkspacePanes(store.activeWorkspaceId);
        }
      }
    })();
  }, []);

  // ── Agent notification listener ───────────────────────────────────────────
  useEffect(() => {
    const unsub = listen<{ terminalId: string; message: string }>(
      'terminal:notification',
      ({ payload }) => {
        useStore.getState().setNotification(payload.terminalId, payload.message);
      }
    );
    return () => { unsub.then(f => f()); };
  }, []);

  // ── Terminal title updates (OSC 0/2) ──────────────────────────────────────
  useEffect(() => {
    const unsub = listen<{ terminalId: string; title: string }>(
      'terminal:title',
      ({ payload }) => {
        useStore.getState().setTerminalTitle(payload.terminalId, payload.title);
      }
    );
    return () => { unsub.then(f => f()); };
  }, []);

  // ── Agent browser control (OSC vmux;browser-*) ───────────────────────────
  useEffect(() => {
    const unsubs: Promise<() => void>[] = [];

    unsubs.push(listen<{ terminalId: string; url: string }>('agent:browser-open', ({ payload }) => {
      const store = useStore.getState();
      if (!store.showBrowser) {
        store.toggleBrowser();
      }
      // Small delay to let the browser pane mount
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

  // ── JS keyboard prefix fallback ──────────────────────────────────────────
  useEffect(() => {
    let jsPrefixPending = false;

    const handleKeyDown = (e: KeyboardEvent) => {
      const tag = (e.target as HTMLElement).tagName;
      if (tag === 'INPUT' || tag === 'TEXTAREA' || tag === 'SELECT') return;

      if (!jsPrefixPending && e.ctrlKey && e.key === 'a') {
        e.preventDefault();
        jsPrefixPending = true;
        setPrefixActive(true);
        return;
      }

      if (jsPrefixPending) {
        e.preventDefault();
        jsPrefixPending = false;
        setPrefixActive(false);

        if (e.key === '?') {
          setShowHelp(h => !h);
        } else {
          handlePrefixCommand(e.key);
        }
      }
    };

    document.addEventListener('keydown', handleKeyDown);
    return () => document.removeEventListener('keydown', handleKeyDown);
  }, []);

  // ── Prefix system (driven by native HWND via Tauri events) ───────────────
  useEffect(() => {
    const unsub1 = listen<{ terminalId: string }>('prefix:activated', () => {
      setPrefixActive(true);
    });
    const unsub2 = listen<{}>('prefix:deactivated', () => {
      setPrefixActive(false);
    });
    const unsub3 = listen<{ key: string }>('prefix:command', ({ payload }) => {
      setPrefixActive(false);
      if (payload.key === '?') {
        setShowHelp(h => !h);
      } else {
        handlePrefixCommand(payload.key);
      }
    });
    return () => {
      unsub1.then(f => f());
      unsub2.then(f => f());
      unsub3.then(f => f());
    };
  }, []);

  // ── Tab bar for multi-tab workspaces ─────────────────────────────────────
  const tabs = activeWorkspace?.tabs ?? [];

  return (
    <div className="app">
      {prefixActive && <div className="prefix-indicator">PREFIX</div>}
      {showHelp && <KeyboardHelp onClose={() => setShowHelp(false)} />}

      <div className="app-sidebar">
        <Sidebar onShowHelp={() => setShowHelp(h => !h)} />
      </div>

      {showFileTree && (
        <div className="app-file-tree">
          <FileTree />
        </div>
      )}

      <div className="app-main">
        {/* Tab bar */}
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
          <div className={`app-terminal-area ${showBrowser ? 'app-terminal-area-split' : ''}`}>
            {activeTab ? (
              <TabView tab={activeTab} />
            ) : (
              <WelcomeScreen onStart={() => activeWorkspaceId && addTab(activeWorkspaceId, 'Workspace')} />
            )}
          </div>
          {showBrowser && (
            <div className="app-browser-area">
              <BrowserPane />
            </div>
          )}
        </div>
      </div>

      {showContext && (
        <div className="app-context-panel">
          <ContextPanel />
        </div>
      )}
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
          <div className="key-row"><kbd>Ctrl-A c</kbd> Split new pane</div>
          <div className="key-row"><kbd>Ctrl-A t</kbd> New tab</div>
          <div className="key-row"><kbd>Ctrl-A n/p</kbd> Next/prev tab</div>
          <div className="key-row"><kbd>Ctrl-A d</kbd> Close pane</div>
          <div className="key-row"><kbd>Ctrl-A x</kbd> Context panel</div>
          <div className="key-row"><kbd>Ctrl-A b</kbd> Browser pane</div>
          <div className="key-row"><kbd>Ctrl-A f</kbd> File tree</div>
          <div className="key-row"><kbd>Ctrl-A ?</kbd> Keyboard help</div>
        </div>
      </div>
    </div>
  );
}

import { create } from 'zustand';
import { invoke } from '@tauri-apps/api/core';
import type { TerminalInfo, ShellProfile, AgentProfile, Workspace, Tab, Pane, PaneKind, PaneBounds, ContextEntry, BrowserTabInfo } from './types';
import { removeNode, splitNode, type SplitNode } from './components/SplitTree';

interface AppStore {
  // Shells
  shells: ShellProfile[];
  defaultShellId: string | null;
  loadShells: () => Promise<void>;
  setDefaultShell: (id: string) => void;

  // Agents
  agents: AgentProfile[];
  loadAgents: () => Promise<void>;
  createAgentTerminalInTab: (workspaceId: string, tabId: string, agentId: string, workingDir?: string) => Promise<void>;

  // Workspaces
  workspaces: Workspace[];
  activeWorkspaceId: string | null;
  activeTabId: string | null;

  // Terminals
  terminals: Record<string, TerminalInfo>;
  focusedTerminalId: string | null;

  // Context
  contextEntries: ContextEntry[];

  // Browser
  showBrowser: boolean;
  browserUrl: string;
  browserTabs: BrowserTabInfo[];
  activeBrowserTabId: string | null;

  // UI state
  sidebarWidth: number;
  showContext: boolean;

  // Split tree state per tab (tabId → SplitNode)
  splitTrees: Record<string, import('./components/SplitTree').SplitNode>;
  setSplitTree: (tabId: string, tree: import('./components/SplitTree').SplitNode) => void;
  splitFocusedPane: (direction: 'horizontal' | 'vertical') => Promise<void>;
  splitFocusedPaneWith: (kind: 'shell' | 'agent', id: string, direction: 'horizontal' | 'vertical') => Promise<void>;
  createWorktreeTab: (branch: string) => Promise<void>;
  saveWorkspaceState: () => Promise<void>;

  // Actions
  loadWorkspaces: () => Promise<void>;
  restoreWorkspacePanes: (workspaceId: string) => Promise<void>;
  createWorkspace: (name: string) => Promise<Workspace>;
  setActiveWorkspace: (id: string) => Promise<void>;
  addTab: (workspaceId: string, name: string) => Promise<Tab>;
  closeTab: (workspaceId: string, tabId: string) => Promise<void>;
  setActiveTab: (tabId: string) => void;

  createTerminal: (bounds: PaneBounds, workingDir?: string) => Promise<TerminalInfo>;
  createTerminalInTab: (workspaceId: string, tabId: string, workingDir?: string, shellId?: string) => Promise<void>;
  setTerminalBounds: (terminalId: string, bounds: PaneBounds) => Promise<void>;
  focusTerminal: (terminalId: string) => void;
  showTerminal: (terminalId: string) => void;
  hideTerminal: (terminalId: string) => void;
  closeTerminal: (terminalId: string) => Promise<void>;
  setNotification: (terminalId: string, message: string) => void;
  clearNotification: (terminalId: string) => Promise<void>;
  setTerminalCwd: (terminalId: string, cwd: string) => void;
  setTerminalTitle: (terminalId: string, title: string) => void;
  setClaudeSessionId: (terminalId: string, sessionId: string) => void;

  // Claude session resume: last session ID per working directory
  claudeSessionsByDir: Record<string, string>;

  loadContext: (workspaceId?: string) => Promise<void>;

  openBrowser: (bounds: PaneBounds, url?: string) => Promise<void>;
  openBrowserTab: (bounds: PaneBounds, url?: string) => Promise<void>;
  closeBrowserTab: (tabId: string, bounds: PaneBounds) => Promise<void>;
  switchBrowserTab: (tabId: string, bounds: PaneBounds) => Promise<void>;
  loadBrowserTabs: () => Promise<void>;
  setBrowserBounds: (bounds: PaneBounds) => Promise<void>;
  browserNavigate: (url: string) => void;
  browserBack: () => void;
  browserForward: () => void;
  browserReload: () => void;
  browserEvaluate: (js: string) => Promise<string>;
  browserGetSource: () => Promise<string>;
  browserOpenDevtools: () => void;
  showBrowserPane: () => void;
  hideBrowserPane: () => void;
  closeBrowser: () => Promise<void>;
  toggleBrowser: () => void;
  setBrowserUrl: (url: string) => void;

  // Workspace management
  renameWorkspace: (workspaceId: string, name: string) => Promise<void>;
  setWorkspaceDirectory: (workspaceId: string, directory: string | null) => Promise<void>;
  deleteWorkspace: (workspaceId: string) => Promise<void>;
  cycleTab: (direction: 'next' | 'prev') => void;

  // Pane layout
  reorderPanes: (workspaceId: string, tabId: string, paneIds: string[]) => Promise<void>;
  setTabDirection: (workspaceId: string, tabId: string, direction: 'horizontal' | 'vertical') => Promise<void>;

  // Context CRUD
  createContext: (title: string, content: string, tags?: string[]) => Promise<ContextEntry>;
  updateContextEntry: (id: string, title?: string, content?: string, tags?: string[]) => Promise<void>;
  deleteContext: (id: string) => Promise<void>;

  // File tree
  showFileTree: boolean;
  toggleFileTree: () => void;

  // Git diff
  showGitDiff: boolean;
  toggleGitDiff: () => void;

  setSidebarWidth: (w: number) => void;
  toggleContext: () => void;
}

export const useStore = create<AppStore>((set, get) => ({
  shells: [],
  defaultShellId: null,

  loadShells: async () => {
    const shells: ShellProfile[] = await invoke('list_shells');
    set(s => ({ shells, defaultShellId: s.defaultShellId ?? shells[0]?.id ?? null }));
  },

  setDefaultShell: (id) => set({ defaultShellId: id }),

  agents: [],

  loadAgents: async () => {
    const agents: AgentProfile[] = await invoke('list_agents');
    set({ agents });
  },

  createAgentTerminalInTab: async (workspaceId, tabId, agentId, workingDir) => {
    const state = get();
    const ws = state.workspaces.find(w => w.id === workspaceId);
    const tab = ws?.tabs.find(t => t.id === tabId);
    const sessionName = `${ws?.name ?? 'vmux'}-${tab?.name ?? 'tab'}`;

    // Directory priority: explicit workingDir > workspace directory > focused terminal CWD
    const effectiveDir = workingDir
      ?? ws?.directory
      ?? (state.focusedTerminalId ? state.terminals[state.focusedTerminalId]?.working_dir : null)
      ?? null;

    const continueSession = agentId === 'claude';

    const info: TerminalInfo = await invoke('create_agent_terminal', {
      agentId,
      workingDir: effectiveDir,
      sessionName: agentId === 'claude' ? sessionName : null,
      resumeSession: null,
      continueSession,
    });
    set(s => ({ terminals: { ...s.terminals, [info.id]: info } }));

    const pane = await invoke('add_pane', {
      workspaceId,
      tabId,
      kind: { type: 'terminal', terminal_id: info.id },
    });

    set(s => ({
      workspaces: s.workspaces.map(ws =>
        ws.id === workspaceId
          ? {
              ...ws,
              tabs: ws.tabs.map(t =>
                t.id === tabId ? { ...t, panes: [...t.panes, pane as any] } : t
              ),
            }
          : ws
      ),
    }));

    set({ focusedTerminalId: info.id });
  },

  workspaces: [],
  activeWorkspaceId: null,
  activeTabId: null,
  terminals: {},
  focusedTerminalId: null,
  contextEntries: [],
  showBrowser: false,
  browserUrl: 'about:blank',
  browserTabs: [],
  activeBrowserTabId: null,
  sidebarWidth: 240,
  showContext: false,
  showFileTree: false,
  showGitDiff: false,
  splitTrees: {},
  claudeSessionsByDir: {},

  loadWorkspaces: async () => {
    const workspaces: Workspace[] = await invoke('list_workspaces');
    const active = workspaces[0];
    set({
      workspaces,
      activeWorkspaceId: active?.id ?? null,
      activeTabId: active?.active_tab_id ?? null,
    });
  },

  restoreWorkspacePanes: async (workspaceId) => {
    // Capture old terminal IDs per pane BEFORE restore (order matters)
    const oldWs = get().workspaces.find(w => w.id === workspaceId);
    const oldPaneTerminalIds: string[] = [];
    if (oldWs) {
      for (const tab of oldWs.tabs) {
        for (const pane of tab.panes) {
          if (pane.kind.type === 'terminal') {
            oldPaneTerminalIds.push(pane.kind.terminal_id);
          }
        }
      }
    }

    const infos: TerminalInfo[] = await invoke('restore_workspace_terminals', { workspaceId });
    if (infos.length === 0) return;

    const newTerminals: Record<string, TerminalInfo> = {};
    for (const info of infos) {
      newTerminals[info.id] = info;
    }
    set(s => ({ terminals: { ...s.terminals, ...newTerminals } }));

    // Re-load workspaces to get updated pane terminal_ids
    const workspaces: Workspace[] = await invoke('list_workspaces');

    // Build old→new terminal ID mapping (panes are in the same order)
    const idMap: Record<string, string> = {};
    let newIdx = 0;
    const newWs = workspaces.find(w => w.id === workspaceId);
    if (newWs) {
      for (const tab of newWs.tabs) {
        for (const pane of tab.panes) {
          if (pane.kind.type === 'terminal' && newIdx < oldPaneTerminalIds.length) {
            idMap[oldPaneTerminalIds[newIdx]] = pane.kind.terminal_id;
            newIdx++;
          }
        }
      }
    }

    // Remap saved split tree layouts with new terminal IDs
    const splitTrees: Record<string, SplitNode> = {};
    if (newWs) {
      for (const tab of newWs.tabs) {
        if (tab.layout) {
          try {
            const tree = JSON.parse(tab.layout) as SplitNode;
            const remapped = remapTerminalIds(tree, idMap);
            splitTrees[tab.id] = remapped;
          } catch { /* invalid layout, will rebuild */ }
        }
      }
    }

    set(s => ({
      workspaces,
      activeTabId: workspaces.find(w => w.id === s.activeWorkspaceId)?.active_tab_id ?? s.activeTabId,
      splitTrees: { ...s.splitTrees, ...splitTrees },
    }));

    // Restore scrollback for each terminal (pane_id is stable across restores)
    if (newWs) {
      for (const tab of newWs.tabs) {
        for (const pane of tab.panes) {
          if (pane.kind.type === 'terminal') {
            invoke('restore_terminal_scrollback', {
              paneId: pane.id,
              terminalId: pane.kind.terminal_id,
            }).catch(() => {});
          }
        }
      }
    }
  },

  createWorkspace: async (name) => {
    const ws: Workspace = await invoke('create_workspace', { name });
    set(s => ({ workspaces: [...s.workspaces, ws] }));

    // Switch to the new workspace and create an initial tab
    await invoke('set_active_workspace', { workspaceId: ws.id });

    // Hide all current terminals
    const state = get();
    for (const id of Object.keys(state.terminals)) {
      invoke('hide_terminal', { terminalId: id }).catch(() => {});
    }

    const tab: Tab = await invoke('add_tab', { workspaceId: ws.id, name: 'Terminal' });
    set(s => ({
      workspaces: s.workspaces.map(w =>
        w.id === ws.id ? { ...w, tabs: [tab], active_tab_id: tab.id } : w
      ),
      activeWorkspaceId: ws.id,
      activeTabId: tab.id,
      focusedTerminalId: null,
    }));
    return ws;
  },

  setActiveWorkspace: async (id) => {
    const prev = get();

    // Hide all terminals belonging to the current workspace
    if (prev.activeWorkspaceId) {
      const oldWs = prev.workspaces.find(w => w.id === prev.activeWorkspaceId);
      if (oldWs) {
        for (const tab of oldWs.tabs) {
          for (const pane of tab.panes) {
            if (pane.kind.type === 'terminal') {
              invoke('hide_terminal', { terminalId: pane.kind.terminal_id }).catch(() => {});
            }
          }
        }
      }
    }

    await invoke('set_active_workspace', { workspaceId: id });
    const ws = get().workspaces.find(w => w.id === id);
    set({ activeWorkspaceId: id, activeTabId: ws?.active_tab_id ?? null, focusedTerminalId: null });

    // Restore/show terminals for the new workspace
    if (ws && ws.tabs.some(t => t.panes.length > 0)) {
      // Check if these terminals already exist in our store
      const firstTerminalId = ws.tabs.flatMap(t => t.panes)
        .find(p => p.kind.type === 'terminal')?.kind;
      const alreadyLoaded = firstTerminalId?.type === 'terminal'
        && prev.terminals[firstTerminalId.terminal_id];

      if (alreadyLoaded) {
        // Just show them
        for (const tab of ws.tabs) {
          if (tab.id !== ws.active_tab_id) continue;
          for (const pane of tab.panes) {
            if (pane.kind.type === 'terminal') {
              invoke('show_terminal', { terminalId: pane.kind.terminal_id }).catch(() => {});
            }
          }
        }
        // Focus first terminal in active tab
        const activeTab = ws.tabs.find(t => t.id === ws.active_tab_id);
        const first = activeTab?.panes.find(p => p.kind.type === 'terminal');
        if (first?.kind.type === 'terminal') {
          set({ focusedTerminalId: first.kind.terminal_id });
        }
      } else {
        // Need to spawn fresh PTYs
        await get().restoreWorkspacePanes(id);
      }
    }
  },

  addTab: async (workspaceId, name) => {
    const tab: Tab = await invoke('add_tab', { workspaceId, name });
    set(s => ({
      workspaces: s.workspaces.map(ws =>
        ws.id === workspaceId
          ? { ...ws, tabs: [...ws.tabs, tab], active_tab_id: tab.id }
          : ws
      ),
      activeTabId: tab.id,
    }));
    return tab;
  },

  closeTab: async (workspaceId, tabId) => {
    await invoke('close_tab', { workspaceId, tabId });
    set(s => ({
      workspaces: s.workspaces.map(ws =>
        ws.id === workspaceId
          ? { ...ws, tabs: ws.tabs.filter(t => t.id !== tabId) }
          : ws
      ),
    }));
  },

  setActiveTab: (tabId) => {
    const ws = get().workspaces.find(w => w.id === get().activeWorkspaceId);
    const tab = ws?.tabs.find(t => t.id === tabId);
    const firstTerminal = tab?.panes.find(p => p.kind.type === 'terminal');
    const newFocusedId = firstTerminal?.kind.type === 'terminal'
      ? firstTerminal.kind.terminal_id
      : get().focusedTerminalId;
    set({ activeTabId: tabId, focusedTerminalId: newFocusedId ?? null });
  },

  createTerminal: async (_bounds, workingDir) => {
    const info: TerminalInfo = await invoke('create_terminal', {
      workingDir: workingDir ?? null,
    });
    set(s => ({ terminals: { ...s.terminals, [info.id]: info } }));
    return info;
  },

  createTerminalInTab: async (workspaceId, tabId, workingDir, shellId) => {
    const state = get();
    const ws = state.workspaces.find(w => w.id === workspaceId);
    // Directory priority: explicit > workspace directory > null (backend defaults to USERPROFILE)
    const effectiveDir = workingDir ?? ws?.directory ?? null;
    const effectiveShellId = shellId ?? state.defaultShellId ?? null;
    const info: TerminalInfo = await invoke('create_terminal', {
      workingDir: effectiveDir,
      shellId: effectiveShellId,
    });
    set(s => ({ terminals: { ...s.terminals, [info.id]: info } }));

    // Register pane in workspace so tab.panes gets populated
    const pane = await invoke('add_pane', {
      workspaceId,
      tabId,
      kind: { type: 'terminal', terminal_id: info.id, shell_id: effectiveShellId },
    });

    // Update local workspace state so TabView re-renders with the new pane
    set(s => ({
      workspaces: s.workspaces.map(ws =>
        ws.id === workspaceId
          ? {
              ...ws,
              tabs: ws.tabs.map(t =>
                t.id === tabId ? { ...t, panes: [...t.panes, pane as any] } : t
              ),
            }
          : ws
      ),
    }));

    set({ focusedTerminalId: info.id });
  },

  setTerminalBounds: async (terminalId, bounds) => {
    // Phase 2 on first call: creates Win32 HWND + wgpu (~200ms, non-blocking UI)
    // Subsequent calls: just repositions the native window (<1ms)
    try {
      await invoke('set_terminal_bounds', { terminalId, bounds });
    } catch (e) {
      console.error('[vmux] set_terminal_bounds failed:', terminalId, bounds, e);
    }
  },

  focusTerminal: (terminalId) => {
    set({ focusedTerminalId: terminalId });
    // Set Win32 keyboard focus on the terminal HWND so it receives keystrokes.
    // All keyboard input is handled natively in the WndProc, not in JS.
    invoke('focus_terminal', { terminalId }).catch(() => {});
  },

  showTerminal: (terminalId) => { invoke('show_terminal', { terminalId }); },
  hideTerminal: (terminalId) => { invoke('hide_terminal', { terminalId }); },

  closeTerminal: async (terminalId) => {
    await invoke('close_terminal', { terminalId });

    // Find the pane to remove so we can persist the removal to SQLite
    const state = get();
    for (const ws of state.workspaces) {
      for (const tab of ws.tabs) {
        const pane = tab.panes.find(p =>
          p.kind.type === 'terminal' && p.kind.terminal_id === terminalId
        );
        if (pane) {
          invoke('remove_pane', { workspaceId: ws.id, tabId: tab.id, paneId: pane.id })
            .catch(e => console.error('[vmux] remove_pane failed:', e));
          break;
        }
      }
    }

    set(s => {
      const { [terminalId]: _, ...rest } = s.terminals;
      const workspaces = s.workspaces.map(ws => ({
        ...ws,
        tabs: ws.tabs.map(t => ({
          ...t,
          panes: t.panes.filter(p =>
            p.kind.type !== 'terminal' || p.kind.terminal_id !== terminalId
          ),
        })),
      }));
      // Also remove from split trees
      const splitTrees = { ...s.splitTrees };
      for (const [tabId, tree] of Object.entries(splitTrees)) {
        const updated = removeNode(tree as SplitNode, terminalId);
        if (updated) {
          splitTrees[tabId] = updated;
        } else {
          delete splitTrees[tabId];
        }
      }

      return { terminals: rest, workspaces, splitTrees };
    });
  },

  setNotification: (terminalId, message) => {
    set(s => ({
      terminals: {
        ...s.terminals,
        [terminalId]: { ...s.terminals[terminalId], has_notification: true, notification_message: message },
      },
    }));
  },

  clearNotification: async (terminalId) => {
    await invoke('clear_notification', { terminalId });
    set(s => ({
      terminals: {
        ...s.terminals,
        [terminalId]: { ...s.terminals[terminalId], has_notification: false, notification_message: null },
      },
    }));
  },

  setTerminalCwd: (terminalId, cwd) => {
    set(s => {
      const term = s.terminals[terminalId];
      if (!term || term.working_dir === cwd) return s;

      // Update pane's working_dir in workspace state for persistence
      const workspaces = s.workspaces.map(ws => ({
        ...ws,
        tabs: ws.tabs.map(t => ({
          ...t,
          panes: t.panes.map(p =>
            p.kind.type === 'terminal' && p.kind.terminal_id === terminalId
              ? { ...p, kind: { ...p.kind, working_dir: cwd } }
              : p
          ),
        })),
      }));

      return {
        terminals: { ...s.terminals, [terminalId]: { ...term, working_dir: cwd } },
        workspaces,
      };
    });
  },

  setTerminalTitle: (terminalId, title) => {
    set(s => ({
      terminals: {
        ...s.terminals,
        [terminalId]: { ...s.terminals[terminalId], title },
      },
    }));
  },

  setClaudeSessionId: (terminalId, sessionId) => {
    set(s => {
      const term = s.terminals[terminalId];
      if (!term) return s;
      const updated: Record<string, string> = { ...s.claudeSessionsByDir };
      // Store session ID indexed by working directory for future resume
      if (term.working_dir) {
        updated[term.working_dir] = sessionId;
      }
      return {
        terminals: {
          ...s.terminals,
          [terminalId]: { ...term, claude_session_id: sessionId },
        },
        claudeSessionsByDir: updated,
      };
    });
  },

  loadContext: async (workspaceId) => {
    const entries: ContextEntry[] = await invoke('list_context', {
      workspaceId: workspaceId ?? null,
    });
    set({ contextEntries: entries });
  },

  openBrowser: async (bounds, url) => {
    const tabId: string = await invoke('open_browser', { bounds, url: url ?? null });
    const urlStr = url ?? 'about:blank';
    set(s => ({
      showBrowser: true,
      browserUrl: urlStr,
      browserTabs: [...s.browserTabs, { id: tabId, url: urlStr, title: '' }],
      activeBrowserTabId: tabId,
    }));
  },

  openBrowserTab: async (bounds, url) => {
    const tabId: string = await invoke('open_browser_tab', { bounds, url: url ?? null });
    const urlStr = url ?? 'about:blank';
    set(s => ({
      browserUrl: urlStr,
      browserTabs: [...s.browserTabs, { id: tabId, url: urlStr, title: '' }],
      activeBrowserTabId: tabId,
    }));
  },

  closeBrowserTab: async (tabId, bounds) => {
    const tabs: BrowserTabInfo[] = await invoke('close_browser_tab', { tabId, bounds });
    set(s => {
      const showBrowser = tabs.length > 0 ? s.showBrowser : false;
      const activeTab = tabs.find(t => t.id === s.activeBrowserTabId) ?? tabs[tabs.length - 1];
      return {
        browserTabs: tabs,
        activeBrowserTabId: activeTab?.id ?? null,
        browserUrl: activeTab?.url ?? 'about:blank',
        showBrowser,
      };
    });
  },

  switchBrowserTab: async (tabId, bounds) => {
    await invoke('switch_browser_tab', { tabId, bounds });
    set(s => {
      const tab = s.browserTabs.find(t => t.id === tabId);
      return {
        activeBrowserTabId: tabId,
        browserUrl: tab?.url ?? s.browserUrl,
      };
    });
  },

  loadBrowserTabs: async () => {
    const tabs: BrowserTabInfo[] = await invoke('list_browser_tabs');
    set({ browserTabs: tabs });
  },

  setBrowserBounds: async (bounds) => {
    await invoke('set_browser_bounds', { bounds });
  },

  browserNavigate: (url) => {
    invoke('browser_navigate', { url });
    set({ browserUrl: url });
  },

  browserBack: () => { invoke('browser_back'); },
  browserForward: () => { invoke('browser_forward'); },
  browserReload: () => { invoke('browser_reload'); },
  browserOpenDevtools: () => { invoke('browser_open_devtools'); },

  browserEvaluate: (js) => {
    return new Promise((resolve, reject) => {
      const callId = Math.random().toString(36).slice(2);
      let unlisten: (() => void) | null = null;
      import('@tauri-apps/api/event').then(({ listen }) => {
        listen<{ id: string; ok: boolean; value: string }>('browser:eval-result', ({ payload }) => {
          if (payload.id !== callId) return;
          unlisten?.();
          if (payload.ok) resolve(payload.value);
          else reject(new Error(payload.value));
        }).then(fn => { unlisten = fn; });
      });
      invoke('browser_evaluate', { js, callId });
    });
  },

  browserGetSource: () => {
    return new Promise((resolve, reject) => {
      const callId = Math.random().toString(36).slice(2);
      let unlisten: (() => void) | null = null;
      import('@tauri-apps/api/event').then(({ listen }) => {
        listen<{ id: string; html: string }>('browser:source', ({ payload }) => {
          if (payload.id !== callId) return;
          unlisten?.();
          resolve(payload.html);
        }).then(fn => { unlisten = fn; });
      });
      invoke('browser_get_source', { callId });
    });
  },

  showBrowserPane: () => { invoke('show_browser'); },
  hideBrowserPane: () => { invoke('hide_browser'); },

  closeBrowser: async () => {
    await invoke('close_browser');
    set({ showBrowser: false, browserTabs: [], activeBrowserTabId: null });
  },

  toggleBrowser: () => {
    const showing = !get().showBrowser;
    set({ showBrowser: showing });
    // Actually show/hide the WebView2 popup window
    if (showing) {
      invoke('show_browser').catch(() => {});
    } else {
      invoke('hide_browser').catch(() => {});
    }
  },
  setBrowserUrl: (url) => set({ browserUrl: url }),

  renameWorkspace: async (workspaceId, name) => {
    await invoke('rename_workspace', { workspaceId, name });
    set(s => ({
      workspaces: s.workspaces.map(ws =>
        ws.id === workspaceId ? { ...ws, name } : ws
      ),
    }));
  },

  setWorkspaceDirectory: async (workspaceId, directory) => {
    await invoke('set_workspace_directory', { workspaceId, directory });
    set(s => ({
      workspaces: s.workspaces.map(ws =>
        ws.id === workspaceId ? { ...ws, directory } : ws
      ),
    }));
  },

  deleteWorkspace: async (workspaceId) => {
    const workspaces: Workspace[] = await invoke('delete_workspace', { workspaceId });
    const active = workspaces[0];
    set({
      workspaces,
      activeWorkspaceId: active?.id ?? null,
      activeTabId: active?.active_tab_id ?? null,
    });
  },

  cycleTab: (direction) => {
    const { workspaces, activeWorkspaceId, activeTabId } = get();
    const ws = workspaces.find(w => w.id === activeWorkspaceId);
    if (!ws || ws.tabs.length === 0) return;
    const idx = ws.tabs.findIndex(t => t.id === activeTabId);
    const next = direction === 'next'
      ? (idx + 1) % ws.tabs.length
      : (idx - 1 + ws.tabs.length) % ws.tabs.length;
    const tab = ws.tabs[next];
    const firstTerminal = tab.panes.find(p => p.kind.type === 'terminal');
    const newFocusedId = firstTerminal?.kind.type === 'terminal'
      ? firstTerminal.kind.terminal_id
      : get().focusedTerminalId;
    set({ activeTabId: tab.id, focusedTerminalId: newFocusedId ?? null });
  },

  createContext: async (title, content, tags) => {
    const workspaceId = get().activeWorkspaceId;
    const entry: ContextEntry = await invoke('create_context', {
      title,
      content,
      workspaceId: workspaceId ?? null,
      tabId: null,
      tags: tags ?? [],
    });
    set(s => ({ contextEntries: [entry, ...s.contextEntries] }));
    return entry;
  },

  updateContextEntry: async (id, title, content, tags) => {
    await invoke('update_context', {
      id,
      title: title ?? null,
      content: content ?? null,
      tags: tags ?? null,
    });
    set(s => ({
      contextEntries: s.contextEntries.map(e =>
        e.id === id
          ? { ...e, ...(title !== undefined && { title }), ...(content !== undefined && { content }), ...(tags !== undefined && { tags }) }
          : e
      ),
    }));
  },

  deleteContext: async (id) => {
    await invoke('delete_context', { id });
    set(s => ({ contextEntries: s.contextEntries.filter(e => e.id !== id) }));
  },

  reorderPanes: async (workspaceId, tabId, paneIds) => {
    await invoke('reorder_panes', { workspaceId, tabId, paneIds });
    set(s => ({
      workspaces: s.workspaces.map(ws =>
        ws.id === workspaceId
          ? {
              ...ws,
              tabs: ws.tabs.map(t => {
                if (t.id !== tabId) return t;
                const reordered = paneIds
                  .map(id => t.panes.find(p => p.id === id))
                  .filter(Boolean) as Pane[];
                return { ...t, panes: reordered };
              }),
            }
          : ws
      ),
    }));
  },

  setTabDirection: async (workspaceId, tabId, direction) => {
    await invoke('set_tab_direction', { workspaceId, tabId, direction });
    set(s => ({
      workspaces: s.workspaces.map(ws =>
        ws.id === workspaceId
          ? {
              ...ws,
              tabs: ws.tabs.map(t =>
                t.id === tabId ? { ...t, direction } : t
              ),
            }
          : ws
      ),
    }));
  },

  toggleFileTree: () => set(s => ({ showFileTree: !s.showFileTree })),
  toggleGitDiff: () => set(s => ({ showGitDiff: !s.showGitDiff })),

  setSplitTree: (tabId, tree) => {
    set(s => ({ splitTrees: { ...s.splitTrees, [tabId]: tree } }));
    // Debounced save: layout changes (drag resize) shouldn't save on every frame
    clearTimeout((globalThis as any).__vmuxSplitSaveTimer);
    (globalThis as any).__vmuxSplitSaveTimer = setTimeout(() => {
      get().saveWorkspaceState();
    }, 1000);
  },

  saveWorkspaceState: async () => {
    const state = get();
    const ws = state.workspaces.find(w => w.id === state.activeWorkspaceId);
    if (!ws) return;
    // Embed split tree layouts into tab.layout fields
    const wsWithLayouts = {
      ...ws,
      tabs: ws.tabs.map(t => ({
        ...t,
        layout: state.splitTrees[t.id] ? JSON.stringify(state.splitTrees[t.id]) : t.layout,
      })),
    };
    await invoke('save_workspace_state', {
      workspaceId: ws.id,
      workspaceJson: JSON.stringify(wsWithLayouts),
    }).catch(() => {});

    // Save terminal scrollback for each pane
    for (const tab of ws.tabs) {
      for (const pane of tab.panes) {
        if (pane.kind.type === 'terminal') {
          invoke('save_terminal_scrollback', {
            paneId: pane.id,
            terminalId: pane.kind.terminal_id,
          }).catch(() => {});
        }
      }
    }
  },

  splitFocusedPane: async (direction) => {
    const state = get();
    const { focusedTerminalId, activeWorkspaceId, activeTabId } = state;
    if (!focusedTerminalId || !activeWorkspaceId || !activeTabId) return;

    // Create a new terminal
    const ws = state.workspaces.find(w => w.id === activeWorkspaceId);
    const effectiveDir = state.terminals[focusedTerminalId]?.working_dir
      ?? ws?.directory ?? null;
    const shellId = state.defaultShellId ?? null;
    const info: TerminalInfo = await invoke('create_terminal', {
      workingDir: effectiveDir,
      shellId,
    });
    set(s => ({ terminals: { ...s.terminals, [info.id]: info } }));

    // Register pane in workspace + update split tree in a single batch
    const pane = await invoke('add_pane', {
      workspaceId: activeWorkspaceId,
      tabId: activeTabId,
      kind: { type: 'terminal', terminal_id: info.id, shell_id: shellId },
    });

    set(s => {
      const workspaces = s.workspaces.map(w =>
        w.id === activeWorkspaceId
          ? { ...w, tabs: w.tabs.map(t => t.id === activeTabId ? { ...t, panes: [...t.panes, pane as any] } : t) }
          : w
      );
      // Update split tree in the same batch to avoid sync race
      const tree = s.splitTrees[activeTabId];
      const splitTrees = tree
        ? { ...s.splitTrees, [activeTabId]: splitNode(tree, focusedTerminalId, info.id, direction) }
        : s.splitTrees;
      return { workspaces, splitTrees, focusedTerminalId: info.id };
    });
    // Save immediately after split
    get().saveWorkspaceState();
  },

  splitFocusedPaneWith: async (kind, id, direction) => {
    const state = get();
    const { focusedTerminalId, activeWorkspaceId, activeTabId } = state;
    if (!activeWorkspaceId || !activeTabId) return;

    const ws = state.workspaces.find(w => w.id === activeWorkspaceId);
    const focusedDir = focusedTerminalId
      ? state.terminals[focusedTerminalId]?.working_dir ?? ws?.directory ?? null
      : ws?.directory ?? null;

    let info: TerminalInfo;
    if (kind === 'agent') {
      info = await invoke('create_agent_terminal', {
        agentId: id,
        workingDir: focusedDir,
        sessionName: `${ws?.name ?? 'vmux'}-${id}`,
        resumeSession: null,
        continueSession: id === 'claude',
      });
    } else {
      info = await invoke('create_terminal', {
        workingDir: focusedDir,
        shellId: id,
      });
    }
    set(s => ({ terminals: { ...s.terminals, [info.id]: info } }));

    const pane = await invoke('add_pane', {
      workspaceId: activeWorkspaceId,
      tabId: activeTabId,
      kind: { type: 'terminal', terminal_id: info.id, shell_id: kind === 'shell' ? id : null },
    });

    set(s => {
      const workspaces = s.workspaces.map(w =>
        w.id === activeWorkspaceId
          ? { ...w, tabs: w.tabs.map(t => t.id === activeTabId ? { ...t, panes: [...t.panes, pane as any] } : t) }
          : w
      );
      const tree = s.splitTrees[activeTabId];
      const splitTrees = (tree && focusedTerminalId)
        ? { ...s.splitTrees, [activeTabId]: splitNode(tree, focusedTerminalId, info.id, direction) }
        : s.splitTrees;
      return { workspaces, splitTrees, focusedTerminalId: info.id };
    });
    get().saveWorkspaceState();
  },

  createWorktreeTab: async (branch) => {
    const state = get();
    const ws = state.workspaces.find(w => w.id === state.activeWorkspaceId);
    if (!ws || !state.activeWorkspaceId) return;

    // Determine repo path from workspace directory or focused terminal CWD
    const repoPath = ws.directory
      ?? (state.focusedTerminalId ? state.terminals[state.focusedTerminalId]?.working_dir : null);
    if (!repoPath) return;

    try {
      const worktreePath: string = await invoke('create_worktree', { repoPath, branch });
      // Create a new tab with the worktree path as working directory
      const tab = await get().addTab(state.activeWorkspaceId, `wt: ${branch}`);
      // Create a terminal in the new tab at the worktree path
      await get().createTerminalInTab(state.activeWorkspaceId, tab.id, worktreePath);
    } catch (e) {
      console.error('[vmux] create worktree failed:', e);
    }
  },

  setSidebarWidth: (w) => set({ sidebarWidth: w }),
  toggleContext: () => set(s => ({ showContext: !s.showContext })),
}));

/** Recursively remap terminal IDs in a split tree using an old→new ID mapping. */
function remapTerminalIds(node: SplitNode, idMap: Record<string, string>): SplitNode {
  if (node.type === 'leaf') {
    return { ...node, terminalId: idMap[node.terminalId] ?? node.terminalId };
  }
  return {
    ...node,
    children: [
      remapTerminalIds(node.children[0], idMap),
      remapTerminalIds(node.children[1], idMap),
    ],
  };
}

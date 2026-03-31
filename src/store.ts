import { create } from 'zustand';
import { invoke } from '@tauri-apps/api/core';
import type { TerminalInfo, ShellProfile, AgentProfile, Workspace, Tab, Pane, PaneKind, PaneBounds, ContextEntry, BrowserTabInfo } from './types';

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

  loadContext: (workspaceId?: string) => Promise<void>;

  openBrowser: (bounds: PaneBounds, url?: string) => Promise<string>;
  openBrowserTab: (bounds: PaneBounds, url?: string) => Promise<void>;
  closeBrowserTab: (tabId: string) => Promise<void>;
  switchBrowserTab: (tabId: string) => Promise<void>;
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
    const info: TerminalInfo = await invoke('create_agent_terminal', {
      agentId,
      workingDir: workingDir ?? null,
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
    const infos: TerminalInfo[] = await invoke('restore_workspace_terminals', { workspaceId });
    if (infos.length === 0) return;

    // Add restored terminals to store
    const newTerminals: Record<string, TerminalInfo> = {};
    for (const info of infos) {
      newTerminals[info.id] = info;
    }
    set(s => ({ terminals: { ...s.terminals, ...newTerminals } }));

    // Re-load workspaces to get updated pane terminal_ids
    const workspaces: Workspace[] = await invoke('list_workspaces');
    set(s => ({
      workspaces,
      activeTabId: workspaces.find(w => w.id === s.activeWorkspaceId)?.active_tab_id ?? s.activeTabId,
    }));
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
    // Phase 1: spawn PTY
    const effectiveShellId = shellId ?? get().defaultShellId ?? null;
    const info: TerminalInfo = await invoke('create_terminal', {
      workingDir: workingDir ?? null,
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
      return { terminals: rest, workspaces };
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
      return {
        terminals: {
          ...s.terminals,
          [terminalId]: { ...term, working_dir: cwd },
        },
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
      browserTabs: [...s.browserTabs, { id: tabId, url: urlStr }],
      activeBrowserTabId: tabId,
    }));
    return tabId;
  },

  openBrowserTab: async (bounds, url) => {
    const tabId: string = await invoke('open_browser', { bounds, url: url ?? null });
    const urlStr = url ?? 'about:blank';
    set(s => ({
      browserUrl: urlStr,
      browserTabs: [...s.browserTabs, { id: tabId, url: urlStr }],
      activeBrowserTabId: tabId,
    }));
  },

  closeBrowserTab: async (tabId) => {
    const tabs: BrowserTabInfo[] = await invoke('close_browser_tab', { tabId });
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

  switchBrowserTab: async (tabId) => {
    await invoke('switch_browser_tab', { tabId });
    set(s => {
      const tab = s.browserTabs.find(t => t.id === tabId);
      return {
        activeBrowserTabId: tabId,
        browserUrl: tab?.url ?? s.browserUrl,
      };
    });
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

  toggleBrowser: () => set(s => ({ showBrowser: !s.showBrowser })),
  setBrowserUrl: (url) => set({ browserUrl: url }),

  renameWorkspace: async (workspaceId, name) => {
    await invoke('rename_workspace', { workspaceId, name });
    set(s => ({
      workspaces: s.workspaces.map(ws =>
        ws.id === workspaceId ? { ...ws, name } : ws
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

  setSidebarWidth: (w) => set({ sidebarWidth: w }),
  toggleContext: () => set(s => ({ showContext: !s.showContext })),
}));

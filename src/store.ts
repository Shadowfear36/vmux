import { create } from 'zustand';
import { invoke } from '@tauri-apps/api/core';
import type { TerminalInfo, ShellProfile, AgentProfile, Workspace, Tab, Pane, PaneKind, PaneBounds, ContextEntry } from './types';

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
  setTerminalTitle: (terminalId: string, title: string) => void;

  loadContext: (workspaceId?: string) => Promise<void>;

  openBrowser: (bounds: PaneBounds, url?: string) => Promise<void>;
  setBrowserBounds: (bounds: PaneBounds) => Promise<void>;
  browserNavigate: (url: string) => void;
  browserBack: () => void;
  browserForward: () => void;
  browserReload: () => void;
  browserEvaluate: (js: string) => Promise<string>;
  browserGetSource: () => Promise<string>;
  showBrowserPane: () => void;
  hideBrowserPane: () => void;
  closeBrowser: () => Promise<void>;
  toggleBrowser: () => void;
  setBrowserUrl: (url: string) => void;

  // Workspace management
  renameWorkspace: (workspaceId: string, name: string) => Promise<void>;
  deleteWorkspace: (workspaceId: string) => Promise<void>;
  cycleTab: (direction: 'next' | 'prev') => void;

  // Context CRUD
  createContext: (title: string, content: string, tags?: string[]) => Promise<ContextEntry>;
  updateContextEntry: (id: string, title?: string, content?: string, tags?: string[]) => Promise<void>;
  deleteContext: (id: string) => Promise<void>;

  // File tree
  showFileTree: boolean;
  toggleFileTree: () => void;

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
  sidebarWidth: 240,
  showContext: false,
  showFileTree: false,

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
    return ws;
  },

  setActiveWorkspace: async (id) => {
    await invoke('set_active_workspace', { workspaceId: id });
    const ws = get().workspaces.find(w => w.id === id);
    set({ activeWorkspaceId: id, activeTabId: ws?.active_tab_id ?? null });
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
    // Do NOT call invoke('focus_terminal') — that calls SetForegroundWindow on
    // the HWND, stealing keyboard focus from our React WebView2 keyboard div.
    set({ focusedTerminalId: terminalId });
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
    await invoke('open_browser', { bounds, url: url ?? null });
    set({ showBrowser: true, browserUrl: url ?? 'about:blank' });
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
    set({ showBrowser: false });
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

  toggleFileTree: () => set(s => ({ showFileTree: !s.showFileTree })),

  setSidebarWidth: (w) => set({ sidebarWidth: w }),
  toggleContext: () => set(s => ({ showContext: !s.showContext })),
}));

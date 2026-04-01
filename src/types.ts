export interface ShellProfile {
  id: string;
  name: string;
  path: string;
  args: string[];
  env: [string, string][];
}

export interface TerminalInfo {
  id: string;
  title: string;
  shell_id: string;
  shell_name: string;
  working_dir: string | null;
  has_notification: boolean;
  notification_message: string | null;
  pid: number | null;
  is_agent: boolean;
  agent_id: string | null;
  claude_session_id: string | null;
}

export interface AgentProfile {
  id: string;
  name: string;
  command: string;
  args: string[];
  env: [string, string][];
  icon: string;
}

export interface PaneBounds {
  x: number;
  y: number;
  width: number;
  height: number;
}

export type PaneKind =
  | { type: 'terminal'; terminal_id: string; shell_id?: string }
  | { type: 'context' }
  | { type: 'browser'; url: string };

export interface Pane {
  id: string;
  kind: PaneKind;
}

export interface Tab {
  id: string;
  name: string;
  panes: Pane[];
  layout: string | null;
  direction: 'horizontal' | 'vertical';
}

export interface Workspace {
  id: string;
  name: string;
  tabs: Tab[];
  active_tab_id: string | null;
}

export interface ContextEntry {
  id: string;
  title: string;
  content: string;
  workspace_id: string | null;
  tab_id: string | null;
  tags: string[];
  created_at: number;
  updated_at: number;
}

export interface BrowserTabInfo {
  id: string;
  url: string;
}

export interface GitMeta {
  branch: string | null;
  is_dirty: boolean;
  ahead: number;
  behind: number;
  staged: number;
  unstaged: number;
}

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
  daemon_session_id: string | null;
  notify_file: string | null;
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
  | { type: 'terminal'; terminal_id: string; shell_id?: string | null; agent_id?: string | null; working_dir?: string; daemon_session_id?: string | null; notify_file?: string | null }
  | { type: 'context' }
  | { type: 'browser'; url: string };

export interface DaemonSessionMeta {
  session_id: string;
  pid: number | null;
  label: string;
  attached_clients: number;
}

export interface DaemonOrphanInfo {
  pid: number;
  label: string;
}

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
  directory: string | null;
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
  title: string;
}

export interface BrowserHistoryEntry {
  id: string;
  url: string;
  title: string | null;
  visited_at: number;
}

// ─── Context Manager types ───────────────────────────────────────────────────

export interface Project {
  id: string;
  name: string;
  path: string;
  created_at: number;
  updated_at: number;
}

export interface Conversation {
  id: string;
  project_id: string;
  agent_type: string;
  session_id: string | null;
  title: string | null;
  started_at: number;
  ended_at: number | null;
  source: string;
  metadata: string;
}

export interface ConversationChunk {
  id: string;
  conversation_id: string;
  chunk_index: number;
  role: string;
  content: string;
  has_embedding: boolean;
  created_at: number;
}

export interface DetectedAgentFile {
  name: string;
  path: string;
  content: string;
}

export interface AgentConfig {
  id: string;
  project_id: string;
  name: string;
  content: string;
  auto_generated: boolean;
  created_at: number;
  updated_at: number;
}

export interface WorktreeInfo {
  path: string;
  branch: string;
  is_main: boolean;
}

export interface Settings {
  theme_name: string;
  default_shell_id: string | null;
  font_size: number;
  prefix_key: string;
  /** Shell command to open files from the file tree. %f is replaced by the path.
   *  Default (null) falls back to "vim". */
  open_file_command: string | null;
}

export interface GitMeta {
  branch: string | null;
  is_dirty: boolean;
  ahead: number;
  behind: number;
  staged: number;
  unstaged: number;
}

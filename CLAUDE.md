# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Commands

```bash
# Development (runs Vite dev server + Tauri hot-reload)
npm run tauri dev

# Production build
npm run tauri build

# Frontend only (for UI iteration without Rust compile)
npm run dev

# Rust type-check only (fast)
cargo check --manifest-path src-tauri/Cargo.toml

# TypeScript type-check
npx tsc --noEmit

# Verbose Rust-side logs (default level is error-only via env_logger)
$env:RUST_LOG="vmux=debug"; npm run tauri dev
```

## Architecture

vmux is a Windows terminal multiplexer for AI coding agents. It follows a **hybrid rendering model**:

- **Tauri (WebView2)** handles the UI chrome: sidebar, tab bar, browser pane, context panel
- **Native Win32 child HWNDs** host the actual terminal rendering (GPU-accelerated, not WebView)
- The React frontend renders **transparent placeholder divs** where terminals appear and reports their pixel positions to Rust via `set_terminal_bounds`

### Frontend → Backend communication

All IPC goes through `invoke()` / `listen()` from `@tauri-apps/api`. The Zustand store (`src/store.ts`) wraps all Tauri commands. Never call `invoke()` directly from components — go through the store.

### Rust backend modules (`src-tauri/src/`)

| Module | Purpose |
|--------|---------|
| `terminal/` | Native terminal engine: PTY + VT state machine + GPU renderer + Win32 window |
| `terminal/pty.rs` | ConPTY via `portable-pty`. Spawns `cmd.exe`, reads output on a background thread |
| `terminal/grid.rs` | VT state machine using `alacritty_terminal`. Feed bytes via `parser.advance(&mut term, bytes)` |
| `terminal/renderer.rs` | `wgpu` GPU renderer. Has a wgpu `Surface` per HWND, renders grid snapshots each frame |
| `terminal/window.rs` | Win32 child HWND creation. WndProc handles keyboard input → `InputEvent` channel |
| `terminal/font.rs` | `cosmic-text` font shaping/rasterization. Handles ligatures, CJK, emoji |
| `terminal/cwd.rs` | Reads a process's current working directory via `NtQueryInformationProcess` (PEB walk) |
| `terminal/daemon_client.rs` | Client for the experimental `vmuxd` background daemon (see `docs/session-reattach-design.md`). Gated behind `VMUX_DAEMON_TERMINALS`; only `create_terminal` (plain shells) can use it, off by default. `bin/vmuxd.rs` is the daemon binary, `bin/vmuxd_proto.rs`/`vmux_proto_client.rs` are the earlier standalone prototype, `bin/vmuxd_integration_check.rs` is a manual smoke test |
| `window_tracking.rs` | Subclasses the main HWND to intercept `WM_MOVE` and reposition terminal child HWNDs immediately, avoiding debounced-IPC lag |
| `workspace.rs` | Workspace/tab/pane layout state, persisted to SQLite |
| `worktree.rs` | Git worktree create/list/delete (`git2`) so multiple agents can run on isolated branches of the same repo |
| `context_store.rs` | Agent context entries, projects, conversations, and conversation chunks — SQLite-backed |
| `transcript.rs` | Imports Claude Code's own JSONL session transcripts (`~/.claude/projects/`) into the context store as conversations/chunks |
| `embeddings.rs` | Pluggable embedding providers (Voyage AI, OpenAI-compatible, local hash/TF-IDF fallback) for semantic search |
| `rag.rs` | Cosine-similarity search over embedded conversation chunks |
| `claude_hooks.rs` | Installs/watches Claude Code lifecycle hooks (Stop/Notification/SessionStart/TaskCompleted) via a side-channel notify file. **Installing hooks mutates the user's real `~/.claude/settings.json` and requires explicit consent — never call `ensure_vmux_hooks`/`install_claude_hooks` without a user-facing prompt first** (see `has_vmux_hooks`/`install_claude_hooks` commands and `ensureClaudeHooksConsent` in `store.ts`) |
| `git_meta.rs` | Git branch + status via `git2` for sidebar metadata |
| `osc.rs` | OSC 9/99/777 escape sequence parser for agent `notify` signals |
| `theme.rs` | Color themes (Tokyo Night, Catppuccin Mocha). Passed to `GpuRenderer` |
| `browser.rs` | In-app browser tab management; only the active tab gets a live `WebviewWindow` |
| `commands.rs` | All Tauri IPC command handlers |
| `state.rs` | `AppState` — shared state behind `Mutex<AppState>`. `embedding_config` (may hold an API key) lives here in memory only and is never persisted to SQLite or disk |

### Key constraint: async commands must not hold `Mutex<AppState>` across `.await`

In `commands.rs`, async commands that need to do async GPU work (like `create_terminal`) follow this pattern:
1. Lock state, extract params, drop lock
2. Do async work without holding lock (e.g. `TerminalPane::create().await`)
3. Re-lock state, insert result

### Frontend structure (`src/`)

| File/Dir | Purpose |
|----------|---------|
| `store.ts` | Zustand store — all Tauri invoke calls go here |
| `types.ts` | Shared TypeScript types matching Rust structs |
| `App.tsx` | Root layout + keyboard prefix system (`Ctrl-A` prefix, like tmux) |
| `components/Sidebar.tsx` | Workspace/tab list, git metadata, notification indicators |
| `components/TerminalPane.tsx` | Transparent placeholder div; reports bounds via ResizeObserver |
| `components/TabView.tsx` | Split-pane layout using `allotment` |

### Keyboard prefix system

`Ctrl-A` activates prefix mode (shown as "PREFIX" badge). Next keypress:
- `c` → new tab, `n/p` → next/prev tab, `x` → context panel, `b` → browser

### Agent notifications

When a terminal process emits an OSC escape sequence (OSC 9/99/777), the Rust backend:
1. Detects it in `osc.rs`
2. Emits `terminal:notification` Tauri event with `{ terminalId, message }`
3. Frontend updates the terminal's `has_notification` flag in the store
4. Sidebar shows a blue dot on the tab; the terminal pane gets a glowing ring border

### Database

SQLite at `%APPDATA%/vmux/vmux.db`. Tables:
- `workspaces` — serialized workspace JSON
- `context_entries` — agent context/notes per workspace
- `projects`, `conversations`, `conversation_chunks` — imported Claude Code transcripts and their chunks, used by the RAG search (`rag.rs`/`embeddings.rs`); chunk embeddings are stored alongside chunks
- `agent_configs`, `browser_history`, `terminal_scrollback` — agent CLI config, in-app browser history, scrollback persistence

### Semantic search over past conversations

`transcript.rs` imports Claude Code's own JSONL transcripts (`~/.claude/projects/`) into `context_store.rs`. `embeddings.rs` embeds chunks via a pluggable provider (Voyage AI, an OpenAI-compatible endpoint, or a local hash/TF-IDF fallback needing no API key), configured at runtime via `set_embedding_config` — the resulting `EmbeddingConfig` (including any API key) lives only in in-memory `AppState`, never written to the DB. `rag.rs` does cosine-similarity search over pre-computed embeddings; the split between the async embedding call and the sync DB lookup exists so `rusqlite::Connection` is never held across an `.await`.

### Git worktrees

`worktree.rs` creates/lists/deletes git worktrees under `<repo>/.worktrees/<branch>` so multiple agents can work on isolated branches of the same repo simultaneously. Reachable via the `Ctrl-A w` chord: `n` creates one (prompts for branch, opens a new tab there), `l` opens `WorktreeList` (`src/components/WorktreeList.tsx`) to browse/delete existing ones.

### Claude Code hook integration — requires explicit consent

`claude_hooks.rs` can install `Stop`/`Notification`/`SessionStart`/`TaskCompleted` hooks into the user's real, shared `~/.claude/settings.json` so vmux can surface agent lifecycle events as `claude:event` Tauri events (via a polled notify-file side channel). **This mutates config outside the project/app sandbox, so it must never happen implicitly.** The frontend (`ensureClaudeHooksConsent` in `store.ts`) prompts the user once, via `has_vmux_hooks`/`install_claude_hooks`, before the first Claude terminal is spawned; `create_agent_terminal` itself performs no installation.

### Adding a new Tauri command

1. Add handler in `commands.rs`
2. Register in `lib.rs` `invoke_handler![]`
3. Add `invoke()` call in `store.ts`
4. Add TypeScript types in `types.ts` if needed

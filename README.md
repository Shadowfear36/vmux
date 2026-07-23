# vmux

The tmux for Windows — a GPU-accelerated terminal multiplexer built for running multiple AI coding agents side by side.

vmux gives you tmux-style terminal management with a native Windows GUI: split panes, persistent workspaces, git worktrees for running several agents on isolated branches in parallel, a built-in agent-controllable browser, semantic search over past agent conversations, and first-class support for Claude Code, Gemini CLI, Codex, and Aider.

## Installation

Download the latest installer (`vmux_x.y.z_x64-setup.exe`) from the [Releases page](../../releases). It installs per-user (no admin rights needed) and adds `vmux`/`vmuxctl` to your PATH.

The installer is currently unsigned, so Windows SmartScreen will show a warning on first run — click "More info" → "Run anyway".

## Why vmux?

Traditional terminal multiplexers (tmux, screen) don't really exist on Windows. WSL workarounds are clunky. Windows Terminal is great for basic use but lacks the multiplexing, agent awareness, and integrated tooling that running several AI coding agents at once demands.

vmux fills this gap:

- **Native Windows performance** -- GPU-rendered terminals via wgpu, not a web canvas
- **Agent-aware** -- detects Claude, Gemini, Codex, Aider, Amazon Q on PATH and provides one-click launch
- **Parallel agent workflows** -- spin up an isolated git worktree per agent so several can work on the same repo simultaneously without stepping on each other
- **Agents can act, not just talk** -- a Claude Code skill lets an agent drive the in-app browser pane itself (navigate, click, read the page) and search vmux's own history of past conversations
- **Integrated browser** -- built-in WebView2 browser with DevTools, multi-tab support, and agent-controlled navigation via OSC escape sequences
- **Persistent workspaces** -- your layout survives app restarts, backed by SQLite
- **Git integration** -- branch/status in the sidebar, full diff viewer panel
- **Experimental session reattachment** -- daemon-backed terminals/agents can survive closing vmux entirely (opt-in toggle in Settings; see `docs/session-reattach-design.md`)

## Architecture

vmux uses a hybrid rendering model:

- **Tauri (WebView2)** renders the UI chrome: sidebar, tab bar, panels
- **Native Win32 child HWNDs** host the actual terminals with GPU-accelerated rendering
- **ConPTY** provides the pseudo-terminal layer (cmd.exe, PowerShell, Git Bash)
- **alacritty_terminal** handles the VT state machine (escape sequences, colors, scrollback)
- **wgpu + cosmic-text** renders the terminal grid with proper font shaping, ligatures, and emoji

Keyboard input goes directly through the Win32 WndProc -- no JavaScript in the input path, no latency from the WebView layer.

## Keyboard Shortcuts

All shortcuts use the `Ctrl-A` prefix (like tmux/screen), rebindable to any letter in Settings. Some open a second-key "chord" (shown with a PREFIX badge) before doing anything:

| Shortcut | Action |
|----------|--------|
| `Ctrl-A c` | Split pane side-by-side |
| `Ctrl-A -` | Split pane stacked (top/bottom) |
| `Ctrl-A s` then `1-9` | Split with a specific shell (by position in your shell list); `s` `-` `1-9` splits stacked instead |
| `Ctrl-A a` then `1-9` | Split with a specific agent (Claude, Gemini, etc.); `a` `-` `1-9` splits stacked instead |
| `Ctrl-A w` then `n` | New git worktree (prompts for branch name, opens a new tab there) |
| `Ctrl-A w` then `l` | List/manage existing worktrees |
| `Ctrl-A w` then `+` | New workspace |
| `Ctrl-A` + arrow keys | Move focus between panes in that direction |
| `Ctrl-A d` | Close focused pane |
| `Ctrl-A t` | New tab |
| `Ctrl-A n` | Next tab |
| `Ctrl-A p` | Previous tab |
| `Ctrl-A b` | Split in a browser pane |
| `Ctrl-A f` | Toggle file tree |
| `Ctrl-A g` | Toggle git diff panel |
| `Ctrl-A x` | Toggle context panel |
| `Ctrl-A ?` | Keyboard shortcuts help |

`Ctrl-A Ctrl-A` sends a literal `Ctrl-A` to the terminal.

## Features

### Terminal Multiplexing
Split your terminal any way you want. Side-by-side splits for comparing output, stacked splits for monitoring. Drag panes to reorder them. Each pane is an independent ConPTY session with full VT100/xterm compatibility. Click-drag selects a range, double-click selects a word, triple-click selects a line — all extend by that same granularity as you drag, and `Ctrl+Shift+C` copies the selection.

### Workspaces
Organize your work into persistent workspaces. Each workspace has its own tabs, panes, and layout. Switch between projects instantly. Everything is saved to SQLite and restored on next launch.

### AI Agent Integration
vmux auto-detects AI coding agents on your PATH:
- **Claude Code** -- Anthropic's CLI coding agent
- **Gemini CLI** -- Google's coding agent
- **Codex** -- OpenAI's coding agent
- **Aider** -- open-source AI pair programming
- **Amazon Q** -- AWS coding agent

Click an agent in the sidebar to launch it in the focused terminal's working directory.

### Agent Notifications
When an agent emits an OSC escape sequence (OSC 9/99/777), vmux shows a notification badge on the tab and a glowing ring around the terminal pane. Never miss when an agent needs your attention.

### Built-in Browser
Split in a resizable browser pane with `Ctrl-A b`. Multiple tabs, URL bar, back/forward/reload, and full Chrome DevTools. An agent can drive the page itself — not just fire-and-forget navigation — via the bundled `vmuxctl` CLI (`vmuxctl browser eval "document.title"` returns the real value) or raw OSC sequences:

```
\x1b]vmux;browser-open;https://docs.rs\x07
\x1b]vmux;browser-navigate;https://example.com\x07
\x1b]vmux;browser-eval;document.title\x07
\x1b]vmux;browser-close\x07
```

Install the matching Claude Code skill from Settings and Claude Code picks all of this up automatically in any vmux terminal.

### Git Worktrees for Parallel Agents
`Ctrl-A w n` creates a new git worktree on its own branch and opens a tab there, so you can point multiple agents at the same repo without them colliding on the same working directory. `Ctrl-A w l` lists and manages existing worktrees.

### Context Search Across Past Agent Sessions
vmux imports Claude Code's own JSONL session transcripts and indexes them for semantic search (embeddings via Voyage AI, an OpenAI-compatible endpoint, or a local no-API-key fallback). The context panel (`Ctrl-A x`) lets you search and re-paste past conversations; the bundled `vmux-context` Claude Code skill lets an agent search its own history the same way, from any terminal.

### Agent Lifecycle Hooks
Opt-in Claude Code hook integration (Stop/Notification/SessionStart/TaskCompleted) surfaces agent lifecycle events as sidebar notifications, on top of the OSC-based notification badges above. Installing hooks writes to your real, shared `~/.claude/settings.json`, so vmux always asks for explicit consent before doing so — never silently.

### File Tree
Toggle with `Ctrl-A f`. Automatically follows the focused terminal's working directory. Updates in real-time as you `cd` around.

### Git Diff Panel
Toggle with `Ctrl-A g`. Shows all changed files in the focused terminal's git repository with full patch diffs. Color-coded status indicators for added, modified, deleted, and renamed files.

### CWD Tracking
vmux tracks each terminal's current working directory in real-time using two mechanisms:
1. **OSC 7** parsing for shells that emit it (bash, zsh, PowerShell with oh-my-posh)
2. **Windows API polling** via NtQueryInformationProcess as a fallback for cmd.exe

The file tree, git metadata, and agent launch directory all stay in sync automatically.

### Settings
Theme (Tokyo Night / Catppuccin Mocha), font size, default shell, prefix key remap, file-open command, and the experimental daemon-backed sessions toggle all live in the Settings panel, along with install/status rows for the Claude Code skills and any active daemon sessions.

### Session Reattachment (experimental)
Terminals and agents can optionally be backed by a small background daemon (`vmuxd`) instead of living entirely inside the vmux process. A daemon-backed session survives closing vmux — the shell/agent process keeps running and reattaches with full scrollback the next time you open vmux. On by default in installed builds; toggle it off in Settings if you'd rather not. See `docs/session-reattach-design.md` for the full design and current limitations.

## Tech Stack

| Layer | Technology |
|-------|-----------|
| UI Shell | Tauri v2 + WebView2 |
| Frontend | React + TypeScript + Zustand |
| Terminal Engine | alacritty_terminal 0.25 |
| PTY | portable-pty 0.9 (ConPTY) |
| GPU Rendering | wgpu 22 + cosmic-text 0.12 |
| Win32 APIs | windows 0.61 crate |
| Persistence | rusqlite (SQLite) |
| Git | git2 0.19 |
| Browser | WebView2 (via Tauri) |

## Development

```bash
# Full dev with hot reload (Vite + Tauri)
npm run tauri dev

# Frontend only (UI iteration without Rust compile)
npm run dev

# Rust type-check (fast)
cargo check --manifest-path src-tauri/Cargo.toml

# TypeScript type-check
npx tsc --noEmit

# Production build (produces the NSIS installer under
# src-tauri/target/release/bundle/nsis/)
npm run tauri build
```

`npm run tauri dev`/`build` also build the `vmuxctl`/`vmuxd` companion binaries automatically (see `scripts/build-sidecars.mjs` for the release path). CI runs on every push (`cargo check`/`cargo test`/`tsc`); pushing a `v*` tag builds and drafts a GitHub Release with the installer attached (`.github/workflows/`).

## Requirements

- Windows 10/11
- Node.js 18+
- Rust 1.70+
- WebView2 Runtime (ships with Windows 11, installable on Windows 10)

## License

MIT

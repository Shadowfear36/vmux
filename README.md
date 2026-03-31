# vmux

A GPU-accelerated terminal multiplexer for Windows, built for AI coding agents.

vmux gives you tmux-style terminal management with a native Windows GUI. Split panes, multiple workspaces, a built-in browser with DevTools, git integration, and first-class support for AI agents like Claude Code, Gemini CLI, Codex, and Aider.

## Why vmux?

Traditional terminal multiplexers (tmux, screen) don't exist on Windows. WSL workarounds are clunky. Windows Terminal is great for basic use but lacks the multiplexing, agent awareness, and integrated tooling that AI-assisted development demands.

vmux fills this gap:

- **Native Windows performance** -- GPU-rendered terminals via wgpu, not a web canvas
- **Agent-aware** -- detects Claude, Gemini, Codex, Aider, Amazon Q on PATH and provides one-click launch
- **Integrated browser** -- built-in WebView2 browser with DevTools, multi-tab support, and agent-controlled navigation via OSC escape sequences
- **Persistent workspaces** -- your layout survives app restarts, backed by SQLite
- **Git integration** -- branch/status in the sidebar, full diff viewer panel

## Architecture

vmux uses a hybrid rendering model:

- **Tauri (WebView2)** renders the UI chrome: sidebar, tab bar, panels
- **Native Win32 child HWNDs** host the actual terminals with GPU-accelerated rendering
- **ConPTY** provides the pseudo-terminal layer (cmd.exe, PowerShell, Git Bash)
- **alacritty_terminal** handles the VT state machine (escape sequences, colors, scrollback)
- **wgpu + cosmic-text** renders the terminal grid with proper font shaping, ligatures, and emoji

Keyboard input goes directly through the Win32 WndProc -- no JavaScript in the input path, no latency from the WebView layer.

## Keyboard Shortcuts

All shortcuts use the `Ctrl-A` prefix (like tmux/screen):

| Shortcut | Action |
|----------|--------|
| `Ctrl-A c` | Split pane side-by-side |
| `Ctrl-A -` | Split pane stacked (top/bottom) |
| `Ctrl-A d` | Close focused pane |
| `Ctrl-A t` | New tab |
| `Ctrl-A n` | Next tab |
| `Ctrl-A p` | Previous tab |
| `Ctrl-A w` | New workspace |
| `Ctrl-A b` | Toggle browser |
| `Ctrl-A f` | Toggle file tree |
| `Ctrl-A g` | Toggle git diff panel |
| `Ctrl-A x` | Toggle context panel |
| `Ctrl-A ?` | Keyboard shortcuts help |

`Ctrl-A Ctrl-A` sends a literal `Ctrl-A` to the terminal.

## Features

### Terminal Multiplexing
Split your terminal any way you want. Side-by-side splits for comparing output, stacked splits for monitoring. Drag panes to reorder them. Each pane is an independent ConPTY session with full VT100/xterm compatibility.

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
Toggle a resizable browser panel with `Ctrl-A b`. Multiple tabs, URL bar, back/forward/reload, and full Chrome DevTools. Agents can control the browser via custom OSC sequences:

```
\x1b]vmux;browser-open;https://docs.rs\x07
\x1b]vmux;browser-navigate;https://example.com\x07
\x1b]vmux;browser-eval;document.title\x07
\x1b]vmux;browser-close\x07
```

### File Tree
Toggle with `Ctrl-A f`. Automatically follows the focused terminal's working directory. Updates in real-time as you `cd` around.

### Git Diff Panel
Toggle with `Ctrl-A g`. Shows all changed files in the focused terminal's git repository with full patch diffs. Color-coded status indicators for added, modified, deleted, and renamed files.

### CWD Tracking
vmux tracks each terminal's current working directory in real-time using two mechanisms:
1. **OSC 7** parsing for shells that emit it (bash, zsh, PowerShell with oh-my-posh)
2. **Windows API polling** via NtQueryInformationProcess as a fallback for cmd.exe

The file tree, git metadata, and agent launch directory all stay in sync automatically.

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

# Production build
npm run tauri build
```

## Requirements

- Windows 10/11
- Node.js 18+
- Rust 1.70+
- WebView2 Runtime (ships with Windows 11, installable on Windows 10)

## License

MIT

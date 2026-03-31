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
| `workspace.rs` | Workspace/tab/pane layout state, persisted to SQLite |
| `context_store.rs` | Agent context entries (notes/files attached to sessions), SQLite-backed |
| `git_meta.rs` | Git branch + status via `git2` for sidebar metadata |
| `osc.rs` | OSC 9/99/777 escape sequence parser for agent `notify` signals |
| `theme.rs` | Color themes (Tokyo Night, Catppuccin Mocha). Passed to `GpuRenderer` |
| `commands.rs` | All Tauri IPC command handlers |
| `state.rs` | `AppState` — shared state behind `Mutex<AppState>` |

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

SQLite at `%APPDATA%/vmux/vmux.db`. Two tables:
- `workspaces` — serialized workspace JSON
- `context_entries` — agent context/notes per workspace

### Adding a new Tauri command

1. Add handler in `commands.rs`
2. Register in `lib.rs` `invoke_handler![]`
3. Add `invoke()` call in `store.ts`
4. Add TypeScript types in `types.ts` if needed

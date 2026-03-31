# vmux — Implementation Plan

## Status Legend
- [x] Done (compiles, works)
- [~] Scaffolded (compiles, not wired up)
- [ ] Not started

---

## Phase 1 — App launches and doesn't crash ✅ (current)

- [x] Tauri app boots, window opens
- [x] Sidebar renders (Tokyo Night theme)
- [x] Keyboard prefix system (Ctrl-A)
- [x] Workspace/tab persistence (SQLite)
- [x] OSC notification parsing
- [x] Git metadata

---

## Phase 2 — Terminals actually open ✅

- [x] Fix A: `CreateWindowExW` dispatched to main thread via `run_on_main_thread` + `std::sync::mpsc`
- [x] Fix B: Two-phase terminal creation (PTY sync + renderer async via ResizeObserver)
- [x] Fix C: Win32 input (WndProc `win_rx`) wired to PTY via `Arc<Mutex<writer>>`
- [x] Fix D: "Loading terminal" — `createTerminalInTab` now calls `add_pane` after PTY spawn
- [x] Zero compiler warnings
- [x] Fix E: Terminal HWNDs switched from `WS_CHILD` to `WS_POPUP` — WebView2 DirectComposition covers WS_CHILD; owned popups render above it
- [x] Fix F: Show/hide terminal popups on tab switch (`show_terminal` / `hide_terminal` commands)

---

## Phase 3 — Terminal renders text (GPU renderer) ✅

- [x] Pipelines: bg quad (solid colour) + glyph atlas (coverage mask)
- [x] Glyph atlas: 2048×2048 Rgba8Unorm, shelf-packed, rasterized via cosmic-text
- [x] Background quads: per-cell coloured rects (clears + cell backgrounds + cursor)
- [x] Glyph quads: textured quads with NDC conversion, per-vertex fg colour + alpha blend
- [x] Cursor: semi-transparent rect overlay at cursor position
- [x] Full 256-colour + NamedColor + RGB colour resolution (all theme variants)
- [x] Re-render triggered on every PTY output batch
- [ ] Verify it actually draws in the running app (needs Phase 2 HWND placement confirmed)

---

## Phase 4 — Terminal feels right (CURRENT)

- [x] Cell size consistency: use renderer's actual font metrics for PTY/grid sizing
- [x] Scrollback: mouse wheel scrolls history; any keypress snaps back to bottom
- [x] Ctrl+V paste from Win32 clipboard (UTF-16 → UTF-8, \r\n normalised)
- [x] Terminal title updates (OSC 0/2 → `terminal:title` Tauri event → tab bar)
- [ ] Text selection (click-drag, Ctrl-Shift-C to copy)
- [x] Blinking cursor animation (530ms toggle, blink task in init_renderer)
- [x] Shift+PageUp/Down scrollback keys

---

## Phase 5 — Multiplexer features

- [ ] Horizontal split (Ctrl-A |)
- [ ] Vertical split (Ctrl-A -)
- [ ] Navigate splits (Ctrl-A arrow keys)
- [ ] Close pane (Ctrl-A d)
- [ ] Layout persistence (save/restore splits per tab)
- [ ] Tab rename (double-click)

---

## Phase 6 — AI agent features

### Notifications (cmux parity)
- [~] OSC detection → frontend notification badge (implemented, needs Phase 2 done to test)
- [ ] `vmux notify <message>` CLI command (writes OSC sequence to current tty)
- [ ] Notification bell sound (optional)

### Browser pane
- [ ] Open Tauri child webview as browser pane
- [ ] URL bar + navigation
- [ ] vmux API: snapshot accessibility tree, click element, fill form (port of cmux's browser API)

### Context manager
- [~] Context store CRUD (Rust done, UI scaffolded)
- [ ] Context editor UI (markdown, tags)
- [ ] Attach context to session (paste into terminal / write to file)
- [ ] Auto-detect CLAUDE.md / AGENTS.md in working directory

---

## Phase 7 — Polish

- [ ] Catppuccin / other theme support (Rust theme structs done)
- [ ] Font picker (use cosmic-text font discovery)
- [ ] Settings panel (font size, theme, shell path, keybindings)
- [ ] vmux CLI (external process sends commands via named pipe)
- [ ] Session persistence (reattach terminals after restart using ConPTY serialization)
- [ ] Windows installer (MSI via Tauri bundler)

---

## Architecture decisions locked in
- Terminal rendering: wgpu + cosmic-text in native Win32 HWND (not xterm.js)
- Shell: cmd.exe default, configurable
- Prefix key: Ctrl-A
- Theme base: Tokyo Night
- Persistence: SQLite at %APPDATA%/vmux/vmux.db

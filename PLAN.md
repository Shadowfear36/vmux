# vmux — Implementation Plan

_Last verified against the codebase: 2026-07-20._

## Status Legend
- [x] Done (compiles, works)
- [~] Partially done / half-wired (backend exists, frontend gap or vice versa)
- [ ] Not started

---

## Phase 1 — App launches and doesn't crash ✅

- [x] Tauri app boots, window opens
- [x] Sidebar renders (Tokyo Night theme)
- [x] Keyboard prefix system (Ctrl-A)
- [x] Workspace/tab persistence (SQLite)
- [x] OSC notification parsing
- [x] Git metadata

---

## Phase 2 — Terminals actually open ✅

- [x] `CreateWindowExW` dispatched to main thread via `run_on_main_thread`
- [x] Two-phase terminal creation (PTY sync + renderer async via ResizeObserver)
- [x] Win32 input (WndProc) wired to PTY via `Arc<Mutex<writer>>`
- [x] Terminal HWNDs use `WS_POPUP` (renders above WebView2's DirectComposition layer)
- [x] Show/hide terminal popups on tab switch

---

## Phase 3 — Terminal renders text (GPU renderer) ✅

- [x] wgpu pipelines: bg quad (solid colour) + glyph atlas (coverage mask)
- [x] Glyph atlas: 2048×2048 Rgba8Unorm, shelf-packed, rasterized via cosmic-text
- [x] Full 256-colour + NamedColor + RGB colour resolution (all theme variants)
- [x] Cursor: semi-transparent rect overlay, blink animation (530ms toggle)
- [x] Concurrent-init crash risk fixed: `GPU_INIT_LOCK` mutex serializes wgpu surface/adapter/device creation across panes

---

## Phase 4 — Terminal feels right

- [x] Cell size consistency: renderer's actual font metrics drive PTY/grid sizing
- [x] Scrollback: mouse wheel scrolls history; any keypress snaps back to bottom; Shift+PageUp/Down
- [x] Ctrl+V paste from Win32 clipboard (UTF-16 → UTF-8, `\r\n` normalised)
- [x] Terminal title updates (OSC 0/2 → tab bar)
- [x] Pane-resize lag fixed: bounds reporting is rAF-throttled so the terminal tracks divider drags live instead of freezing until mouseup
- [x] Text selection (click-drag, Ctrl-Shift-C to copy) — native WndProc mouse tracking drives alacritty_terminal's own `Selection`, rendered as inverse video; no word/line select yet (single-cell-range only)

---

## Phase 5 — Multiplexer features

- [x] Horizontal split (`Ctrl-A c` / `|` / `%`)
- [x] Vertical split (`Ctrl-A -` / `"`)
- [x] Close pane (`Ctrl-A d`)
- [x] Layout persistence (split ratios saved to `tab.layout`, restored on relaunch)
- [x] Shell/agent picker chords (`Ctrl-A s #`, `Ctrl-A a #`, with `-` for vertical)
- [x] Navigate between panes via keyboard (`Ctrl-A` + arrow keys) — geometric nearest-pane selection based on the split tree's computed rects
- [x] Tab rename (double-click) — inline input in the tab bar; Enter/blur commits, Escape cancels

---

## Phase 6 — AI agent features

### Notifications
- [x] OSC 9/99/777 detection → `terminal:notification` event → sidebar badge (works for any agent CLI, not Claude-specific)
- [x] Claude Code hook integration (`Stop`/`Notification`/`SessionStart`/`TaskCompleted` via side-channel file) — gated behind an explicit one-time user consent prompt before install, never silent
- [ ] `vmux notify <message>` CLI command — **not implemented**
- [ ] Notification bell sound — **not implemented**

### Browser pane
- [x] In-app browser tab as a Tauri child webview
- [x] URL bar + navigation, tab list
- [x] Agent-driven browser control via `\x1b]vmux;...\x07` OSC commands (open/navigate/close/eval JS)
- [ ] Structured accessibility/interaction API (snapshot tree, click element, fill form) — **not implemented**; only URL nav + arbitrary JS eval exist

### Context manager & semantic search
- [x] Context store CRUD (Rust + UI)
- [x] Claude Code transcript import (`~/.claude/projects/*.jsonl` → conversations/chunks)
- [x] Semantic search (RAG) over imported conversations — pluggable embedding providers (Voyage AI, OpenAI-compatible, local hash fallback), reachable via ContextPanel's Search tab
- [x] Auto-detect `CLAUDE.md` / `AGENTS.md` in working directory — Agent.md tab shows any found in the project path with a "Load into editor" action (`detect_agent_files` command)
- [x] Attach context to session (paste into terminal) — "Paste into terminal" on both context Notes and the Agent.md editor writes straight to the focused terminal's PTY; "Export to disk" already covered write-to-file for agent.md

### Git worktrees _(not in original plan — added since)_
- [x] Create worktree + open in new tab (`Ctrl-A w n`)
- [x] List / delete worktrees UI (`Ctrl-A w l`)

---

## Phase 7 — Polish

- [x] Catppuccin Mocha theme (alongside Tokyo Night)
- [x] Settings panel — theme, font size, default shell, and prefix key, all persisted (`app_settings` SQLite table) and live-applied to every open terminal without needing to reopen it. Font-picker-as-family-choice still not implemented (cosmic-text font discovery exists but only font *size* is user-configurable, not family)
- [ ] `vmux` external CLI (named pipe control) — **not implemented**
- [~] Session persistence — plain-shell terminals *can* now genuinely survive a vmux restart via the experimental `vmuxd` daemon (see `docs/session-reattach-design.md`), gated behind `VMUX_DAEMON_TERMINALS` and off by default; agent terminals still only get scrollback-*text* replay (no process survival). Daemon lifecycle hardening (crash/orphan handling, version handshake) not done, so this isn't ready to be a default
- [ ] Windows installer (MSI via Tauri bundler) — **not implemented**

---

## Suggested next priorities

Roughly in order of "most users will hit this constantly" vs. "one-time/polish":

Everything in that tier is now done. Remaining items are the bigger lifts: true ConPTY session reattachment, `vmux` CLI, browser accessibility API, installer, word/line-select on top of the basic range selection, and remapping individual chord keys beyond the prefix key itself.

---

## Architecture decisions locked in
- Terminal rendering: wgpu + cosmic-text in native Win32 HWND (not xterm.js)
- Shell: cmd.exe default, configurable
- Prefix key: Ctrl-A
- Theme base: Tokyo Night (Catppuccin Mocha also available)
- Persistence: SQLite at `%APPDATA%/vmux/vmux.db`
- Claude Code hook installation always requires an explicit user prompt — never silent (see `CLAUDE.md`)

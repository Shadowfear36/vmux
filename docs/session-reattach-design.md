# Design: True Session Reattachment (ConPTY Daemon)

Status: **Phases 1, 2, and 3 built and validated** (§12, §13, §14). Reattachment — the actual point of this doc — now genuinely works for daemon-backed plain shells: closing vmux and relaunching it reattaches to the still-running session instead of respawning, verified end to end. Still gated behind `VMUX_DAEMON_TERMINALS`, off by default, plain shells only (`create_terminal`) — see §15 for what's left before this could default on.

## 1. Problem statement

Today, closing vmux kills every running shell/agent session. `PtySession` (`src-tauri/src/terminal/pty.rs`) holds `_master: Box<dyn MasterPty + Send>` directly inside the Tauri UI process. There's no way to "reattach" to a session after restart — `terminal_scrollback` only persists the last 128KB of *text* (see `commands.rs::save_terminal_scrollback`), replayed into a fresh VT grid on restore. The underlying process is gone.

This matters most for long-running agent sessions (Claude Code, etc.) where losing the process mid-task on an accidental window close, a crash, or an update is genuinely costly.

## 2. Why this requires a daemon (not a smaller fix)

ConPTY is not a Unix PTY. On Windows:

- Creating a pseudoconsole (`CreatePseudoConsole` / `openpty()` in `portable-pty`) spins up a hidden `conhost.exe` that manages the console session.
- The child process (`cmd.exe`, `claude.exe`, etc.) attaches to that console as its controlling terminal.
- The pseudoconsole's I/O pipes are anonymous pipes created by whichever process called `openpty()`/`CreatePseudoConsole` — not named pipes. Only that process (and anything it explicitly hands a duplicated handle to) can ever read/write them.
- **Correction from an earlier draft of this doc, based on the Phase 1 prototype (§12):** killing the owning process does *not* necessarily kill the child immediately — in testing, an abruptly-killed daemon left its `conhost.exe` and the shell it hosted running as orphans for some time. But this doesn't help: since the pipe handles were anonymous and process-local, nothing else can ever attach to that orphan again. It's "alive" but permanently unreachable — operationally identical to dead for our purposes. A *clean* shutdown (properly closing the pseudoconsole handle) may behave differently and terminate the child immediately; this wasn't tested and doesn't change the conclusion either way.
- Even setting that aside: nothing would be draining the ConPTY's output pipe while vmux is closed. Pipes have finite buffers; a starved reader eventually blocks the writer, i.e. the agent process itself would stall.

**Conclusion:** genuine reattachment requires the ConPTY handles to be owned by a single process that (a) is created once and (b) never dies for the lifetime of the session — not "any process that happens to still be running," but specifically the one holding those exact anonymous pipe handles. There is no in-process trick (job objects, `DETACHED_PROCESS`, handle inheritance tweaks) that avoids needing that process to stay alive and keep reading PTY output the whole time vmux's UI is closed.

This is architecturally the same problem tmux/screen solve on Unix (a persistent server process), just harder on Windows because ConPTY's daemon (`conhost.exe`) is not something we control directly — we control it indirectly via whichever process opened the pseudoconsole.

## 3. Proposed architecture

```
┌─────────────────┐        named pipe          ┌──────────────────────┐
│   vmux.exe       │ ───────────────────────── │   vmuxd.exe            │
│  (Tauri UI,       │   \\.\pipe\vmux-<user>     │  (background daemon)   │
│   can restart      │                            │                        │
│   freely)          │ ◄─────────────────────── │  owns every ConPTY +   │
└─────────────────┘   session events, output     │  child process         │
                                                   └──────────────────────┘
```

- **`vmuxd`**: a new, separate Rust binary (same workspace, new crate or a `--daemon` mode of the existing binary — see §7). Owns all `PtySession`s. Never spawned by Tauri directly as a child process (that would tie its lifetime to the UI again) — see §5 for startup.
- **vmux (UI)**: becomes a *client*. `TerminalPane` no longer owns a `PtySession` directly; it holds a session handle/ID and talks to `vmuxd` over a named pipe.
- **Transport**: Windows named pipes (`\\.\pipe\vmux-<sid>`), one control connection per UI instance, plus either a multiplexed stream per session or one pipe per attached session (see §6 for tradeoffs).

## 4. Protocol sketch

Illustrative only — exact wire format (length-prefixed JSON vs. bincode vs. a raw framing) is a later decision.

```rust
// Client -> daemon
enum DaemonRequest {
    ListSessions,
    SpawnSession { shell_or_agent: SpawnSpec, cwd: Option<String>, cols: u16, rows: u16 },
    Attach { session_id: SessionId },          // subscribe to output stream
    Detach { session_id: SessionId },          // stop receiving output (process keeps running)
    Write { session_id: SessionId, data: Vec<u8> },
    Resize { session_id: SessionId, cols: u16, rows: u16 },
    Kill { session_id: SessionId },
}

// Daemon -> client
enum DaemonEvent {
    SessionList(Vec<SessionMeta>),
    SessionSpawned { session_id: SessionId, pid: Option<u32> },
    Output { session_id: SessionId, data: Vec<u8> },   // live stream while attached
    ScrollbackReplay { session_id: SessionId, data: Vec<u8> },  // sent immediately on Attach
    SessionExited { session_id: SessionId, code: Option<i32> },
    Error { message: String },
}

struct SessionMeta {
    session_id: SessionId,
    title: String,
    cwd: Option<String>,
    pid: Option<u32>,
    started_at: i64,
    attached_clients: u32,   // 0 = running headless, nobody watching
}
```

Key design point: **`Attach` is idempotent and cheap.** A session keeps running whether 0 or N clients are attached. Multiple vmux windows (or vmux restarted) can attach to the same session_id and both see the same live output — this is exactly tmux's model.

## 5. Daemon lifecycle

This is the trickiest part in practice, not the IPC itself.

- **Startup**: vmux UI checks for an existing daemon (try connecting to the well-known pipe name). If not found, it spawns `vmuxd.exe` **detached** (`CREATE_NEW_PROCESS_GROUP | DETACHED_PROCESS`, not a Tauri sidecar/child in the job-object sense) so the daemon's lifetime is independent of vmux's process/job object. This must be verified carefully — Tauri itself may run under a job object that kills all descendants on exit; the daemon needs `CREATE_BREAKAWAY_FROM_JOB` or to be spawned in a way that escapes it.
- **Shutdown policy**: the daemon should NOT exit when the last client disconnects (that's the whole point). Reasonable options:
  - Exit only when explicitly told to (`vmux daemon stop` command) or when zero sessions remain for some idle grace period (e.g., 10 minutes with no sessions and no clients) — needs a decision, not a default assumption.
  - Must survive a normal Windows logoff vs. must survive a reboot are different questions — a reboot always kills it (real processes don't survive that; only genuine reattach-after-crash/close-of-vmux is in scope, not reattach-after-reboot, unless we also snapshot/restore session commands, which is out of scope here).
- **Orphan cleanup**: if the daemon crashes or is killed without cleanly closing ConPTYs, child processes may be left running with no owner. Need either a heartbeat/watchdog or acceptance that `taskkill`-style manual cleanup is sometimes required (document this rather than solve it perfectly).
- **Versioning**: the daemon and UI ship together today; once they're separate processes with independent lifetimes, an app update can leave a stale `vmuxd` (old protocol version) running when a new `vmux.exe` starts. Need a protocol version handshake and a documented "daemon restart required" story (e.g., UI detects version mismatch, offers to restart the daemon — which does kill in-flight sessions, a real UX tradeoff to surface, not hide).

## 6. Output buffering / replay

Each daemon-owned session needs a bounded ring buffer (e.g. last 1–2MB, tunable) of recent output so a newly-attaching client can repaint scrollback immediately, similar to what `terminal_scrollback` does today but live and complete rather than a periodic snapshot. This buffer should probably **be** the VT grid state (i.e. the daemon runs the `alacritty_terminal::Term` state machine, not just raw bytes) so a reattaching client gets a fully-formed grid snapshot to render immediately, not a wall of raw bytes to replay. This means `grid.rs`'s `TermGrid` moves into the daemon, not the UI process — the UI becomes purely a renderer of snapshots it receives over IPC, which is a bigger change than it first sounds: today `GpuRenderer` and `TermGrid` live in the same process and share memory directly (`Arc<Mutex<TermGrid>>`); with a daemon, grid snapshots must be serialized over the pipe on every PTY output batch. This has real bandwidth/latency implications worth prototyping before committing (a snapshot is `cols * rows` cells with color/flags each — likely fine at typical terminal sizes, but should be measured, not assumed).

## 7. Where the daemon binary lives

Two options:
- **(a) Separate crate/binary** (`src-tauri/src/bin/vmuxd.rs` or a new workspace member) sharing `terminal::pty`, `terminal::grid` code with the main app via a shared lib crate. Cleaner separation, but doubles the build/release surface (two binaries to sign/ship/update in lockstep).
- **(b) Same binary, `--daemon` flag** — `vmux.exe --daemon` runs headless as the persistent host; `vmux.exe` (no flag) is the normal UI and detects/launches the daemon mode of itself. Simpler shipping (one binary), slightly odd to reason about ("the exe is both the multiplexer and its own background service").

Leaning toward (b) for shipping simplicity, but this is a call worth revisiting once the protocol is prototyped.

## 8. What does NOT change

- `terminal/window.rs` (native Win32 HWND + WndProc keyboard handling) stays in the UI process — rendering and input capture are inherently tied to having a visible window.
- `terminal/renderer.rs` (wgpu) stays in the UI process — it renders whatever grid snapshot the daemon sends.
- Workspace/tab/pane persistence (`workspace.rs`) is unaffected — it already persists layout; it would additionally need to persist `session_id`s so relaunching vmux can `Attach` to existing daemon sessions instead of spawning new ones.

## 9. Phased rollout (if we proceed)

1. **Prototype the daemon + protocol in isolation**: spawn/attach/write/resize/kill over a named pipe, no UI integration yet, one hardcoded session, to validate the ConPTY-survives-owner-process-restart assumption end-to-end and measure snapshot-over-IPC overhead. **Done — see §12.**
2. **Wire one TerminalPane through the daemon** behind a feature flag, keep the existing in-process path as the default, so regressions are contained. **Done — see §13.**
3. **Persist `session_id` in `workspace.rs`** so relaunching vmux reattaches instead of respawning. **Not started — this is what actually delivers reattachment; Phase 2 only proves the plumbing works, `spawn_maybe_daemon` always Spawns, never Attaches.**
4. **Daemon lifecycle hardening**: version handshake, idle shutdown policy, orphan detection.
5. **Migrate all panes to the daemon path, remove the in-process `PtySession` path.**
6. **Multi-window support** (two vmux windows attached to the same daemon) — natural fallout of the architecture but worth its own test pass.

Each phase is independently shippable/revertible; step 1 alone is enough to validate or kill the whole idea before touching the rest of the app.

## 10. Alternative considered (cheaper, complementary — not a replacement)

A much smaller change — hide-to-tray instead of exiting on window close, so the existing in-process sessions simply keep running as long as the user doesn't explicitly Quit — covers the most common real-world case (accidentally closing the window, wanting the agent to keep going while the window's out of the way) at a fraction of the engineering cost, with none of the daemon/IPC/versioning complexity. It does not survive an explicit Quit, a crash, or a reboot. This is worth doing regardless of whether the daemon gets built, and could ship immediately if wanted.

## 11. Recommendation

Given the size (new binary or dual-mode binary, IPC protocol, daemon lifecycle/versioning, moving `TermGrid` out of the UI process), this is a multi-day project on its own, with real open questions (§5, §6) that need prototyping, not just implementation, before the full scope is safely estimable. Suggest: do the tray-mode quick win first (independent value, ships fast), then greenlight Phase 1 above (isolated daemon prototype) as its own scoped task before deciding whether to carry it through to full integration.

## 12. Phase 1 prototype — results

Built (not wired into the app): `src-tauri/src/bin/vmuxd_proto.rs` (daemon) and `src-tauri/src/bin/vmux_proto_client.rs` (test client), talking over `\\.\pipe\vmux-proto` with a length-prefixed JSON protocol (`Request::{Write, Resize}` / `Event::{Replay, Output, Exited}`). One hardcoded `cmd.exe` session, a 64KB scrollback ring buffer, a `tokio::sync::broadcast` channel fanning live output out to whichever client is currently attached.

**One snag found and fixed**: this prototype has no real VT parser (that's `alacritty_terminal`'s job in the real app, via `grid.rs`'s `EventProxy::PtyWrite` handling). cmd.exe's startup cursor-position query (`\x1b[6n`) went unanswered, so cmd.exe blocked forever waiting for a reply and never processed any input. Fixed by writing a canned `\x1b[24;1R` reply immediately after spawn. Not an issue for the real integration — alacritty_terminal already auto-replies to these.

**Validated, end to end**, using three separate, sequential runs of the test client against one long-running daemon:

1. Client 1 connects, runs `echo HELLO_FROM_TEST1` and `set TEST_MARKER=abc123`, exits.
2. Client 2 — a fresh process, no shared state — connects and its replay immediately shows the full history from client 1 (startup banner + both prior commands' output), then runs `echo SECOND_CLIENT_SEES_ME` live.
3. Client 3 connects, replay shows everything from clients 1 and 2, then runs `echo %TEST_MARKER%` → prints `abc123` — proving it's the *same* cmd.exe process across all three client connect/disconnect cycles, not a respawned one.

This confirms the core assumption: a session owned by a separate daemon process survives client disconnects, buffers output while unattached, and correctly replays history plus live-streams new output to whichever client attaches next. The named-pipe + broadcast-channel approach works as designed.

**Also confirmed** (via manually killing the daemon mid-test): killing the *daemon* itself, as opposed to just a client, does leave the child process (and its `conhost.exe`) running as an orphan for a bit — but as noted in the corrected §2, this doesn't help; the orphan becomes unreachable. This is exactly why §5 (daemon lifecycle: startup escaping the job object, clean shutdown policy, orphan cleanup) is real engineering work and not a footnote — the daemon dying, even briefly, has to be treated as a real outage for any sessions it owns, not something recoverable after the fact.

## 13. Phase 2 — real integration, results

Unlike Phase 1 (a standalone prototype, duplicated protocol code, no connection to the real app), Phase 2 wires the actual production code path:

- **`src-tauri/src/terminal/daemon_client.rs`** (new, `pub`): the real `DaemonPtySession`, exposing the same surface as `pty::PtySession` (`write`, `writer_handle`, `resize`) so `TerminalPane` can hold either behind one `PtyBackend` enum without the rest of the pipeline (VT grid feeding, resize logic, rendering) needing any changes. `ensure_daemon_running()` auto-launches `vmuxd.exe` detached (`CREATE_NEW_PROCESS_GROUP | DETACHED_PROCESS`) if not already reachable.
- **`src-tauri/src/bin/vmuxd.rs`** (new, the real daemon): promotes the Phase 1 prototype to a proper multi-session registry (`HashMap<SessionId, Arc<Session>>`) with a `Spawn`/`Attach` handshake as a connection's first message, reusing the exact protocol types from `daemon_client.rs` (no duplicated definitions between daemon and client, unlike Phase 1).
- **`terminal/mod.rs`**: `TerminalPane.pty` is now `PtyBackend::{Local(PtySession), Daemon(DaemonPtySession)}`. A new `TerminalPane::spawn_maybe_daemon` (async) checks `VMUX_DAEMON_TERMINALS` and branches; the original sync `TerminalPane::spawn` is untouched and still what `spawn_agent`, `TerminalManager::spawn`, and workspace-restore all use — **only `create_terminal` (plain shells, e.g. `Ctrl-A c`) can use the daemon path so far.** Agent terminals and restored terminals are unaffected regardless of the flag.
- **`commands::create_terminal`** became `async` and now follows the same "lock → extract params → drop lock → do the (possibly slow) work → re-lock → insert" pattern `set_terminal_bounds` already established for GPU init, since connecting to the daemon needs an `.await` that must not happen while holding `Mutex<AppState>`.

**A simplification from §6's assumption**: the daemon does *not* run a VT parser or own `TermGrid` — it only relays raw PTY bytes (plus a raw-byte scrollback buffer for replay), exactly like `PtySession`'s existing output channel shape. `TermGrid`/`alacritty_terminal` stay entirely in the UI process, fed via the daemon's byte stream instead of a direct in-process PTY reader thread. This sidesteps §6's "serialize a full grid snapshot over IPC every render" concern for now — nothing about Phase 2 required solving that. (It will matter once Phase 3 needs *instant* full-grid replay on reattach rather than raw bytes replayed through a fresh parser — worth revisiting then, not before.)

**Validated** via `src-tauri/src/bin/vmuxd_integration_check.rs`, a smoke test that calls `TerminalPane::spawn_maybe_daemon` and `TerminalPane::write_input` directly (the real methods, not a reimplementation): daemon auto-launches, `cmd.exe` spawns through it, output streams back through the real `PtyBackend::Daemon` path into a real `TerminalPane`, and a written command (`echo INTEGRATION_CHECK_OK`) round-trips correctly. `cargo check`/`cargo test` both clean, zero new warnings, all existing tests still pass.

**What Phase 2 does *not* do**: nothing persists a `session_id` anywhere, and `spawn_maybe_daemon` always sends `Spawn`, never `Attach` — so turning the flag on today does not give you reattachment. It proves the backend swap is viable end-to-end; Phase 3 is what actually delivers the feature this whole document is about.

## 14. Phase 3 — reattachment, results

This is the phase that actually delivers what this document is about. Built:

- **Protocol**: added `Request::Kill { session_id }` — without it, closing a daemon-backed pane would leak its session (and shell process) forever, since nothing previously told the daemon to tear one down. `vmuxd.rs`'s `Session` now holds the actual `Box<dyn portable_pty::Child>` (previously discarded right after spawn, keeping only its `pid`) so `Kill` can call `.kill()` on it for real, then remove the session from the registry.
- **`DaemonPtySession::attach(session_id, cols, rows)`**: mirrors `spawn()` but sends `Attach` instead of `Spawn` — the daemon already supported `Attach` from Phase 2's registry design, so no daemon-side changes were needed for this part. Refactored `spawn`/`attach`'s shared post-handshake plumbing (the two forwarding tasks) into one `finish_connection` helper.
- **`DaemonPtySession::kill()`**: sends `Request::Kill`; `PtyBackend::kill_if_daemon()` calls it from `TerminalManager::close`, so closing any daemon-backed pane now actually cleans up its daemon session. Local (`PtySession`) panes need no equivalent — their process already dies when the pane is dropped.
- **Persistence**: `PaneKind::Terminal` gained `daemon_session_id: Option<String>` (workspace.rs), threaded through `TerminalInfo` → `store.ts`'s pane-construction call sites (`createTerminalInTab`, `splitFocusedPane`, `splitFocusedPaneWith`) → back into the saved `PaneKind` on `add_pane`. Restore (`list_workspaces` after `restore_workspace_terminals`) picks it up automatically since the frontend re-reads full workspace state rather than reconstructing `PaneKind` locally.
- **`TerminalPane::attach_daemon(session_id, working_dir, shell)`**: the restore-side counterpart to `spawn_maybe_daemon`, used only when a saved pane has a `daemon_session_id`.
- **`commands::restore_workspace_terminals`** was restructured to the same lock/extract/drop-lock/await/re-lock/insert pattern as `create_terminal`, now across a loop: for each persisted terminal pane, if it has a `daemon_session_id`, try `attach_daemon`; on any error (daemon unreachable, session no longer exists) log a warning and fall back to the same local `spawn` restore already did before Phase 3. Panes without a saved session ID are completely unaffected — restore never *starts* new daemon sessions, it only reattaches ones that already existed, keeping this phase's blast radius to "reattachment," not "restore behavior in general."

**Validated** by extending `vmuxd_integration_check.rs`: after the Phase 2 spawn+write+verify sequence, the pane and its receiver are dropped (simulating vmux closing), then `TerminalPane::attach_daemon` reconnects to the same `session_id` — the replay correctly includes the *entire* prior session's output (the Phase 2 command and its result), and a brand-new command written through the reattached pane round-trips correctly. Same underlying `cmd.exe` process throughout, proven the same way Phase 1 proved it (an env var / marker persisting isn't tested again here since Phase 1 already nailed that down at the protocol level — this test instead confirms it end-to-end through the real `TerminalPane`/`PaneKind`-adjacent API surface). `cargo check`/`cargo test` clean, zero new warnings, all existing tests pass.

## 15. What's still missing before this could be more than experimental

- **Daemon lifecycle hardening (§5) is still unaddressed**: no version handshake between an updated `vmux.exe` and a stale running `vmuxd.exe`, no idle-shutdown policy, no watchdog for a crashed daemon leaving orphaned-but-reachable sessions around forever. Fine for a flagged, opt-in experiment; not fine as a default.
- **No UI surfacing at all**: there's no indication anywhere in the app that a pane is daemon-backed, no way to see or manage daemon sessions that have no attached pane (e.g. after a workspace/tab was deleted but the daemon session wasn't explicitly killed — this can currently happen if a pane is removed via some path other than `close_terminal`/`TerminalManager::close`, worth auditing).
- **Only plain shells** (`create_terminal`) can be daemon-backed. Agent terminals (Claude Code, etc.) — arguably the higher-value case for "don't lose my running agent" — are entirely untouched, by design, to keep each phase's scope contained. Extending to agents is mechanical (mirror what `spawn_maybe_daemon`/`attach_daemon` did for `spawn`/`spawn_agent`) but not yet done.
- **Multi-window support** (§9 step 6) untested — two vmux windows/instances attached to the same daemon session simultaneously hasn't been exercised.
- **`VMUX_DAEMON_TERMINALS` is an env var, not a Settings-panel toggle** — deliberately, since it's not yet something an end user should casually turn on (see the gaps above). Worth promoting to a real setting once the lifecycle hardening lands.

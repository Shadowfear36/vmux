---
name: vmux-browser
description: Control and interact with the live in-app browser pane in vmux (a Windows terminal multiplexer for AI coding agents) — open a URL, navigate, click/fill/read the actual page via JavaScript with real return values, or close the pane. Use when running inside a vmux terminal pane and the user wants a webpage shown, a local dev server previewed, documentation opened, or a page driven/inspected (reading text, clicking buttons, filling forms, checking element state). Has no effect and is safe to try outside vmux; use `vmuxctl ping` first if you need to confirm vmux is actually running.
---

# vmux browser control

vmux can host a live, GPU-rendered WebView2 browser pane right next to your
terminal panes. As an agent running inside a vmux terminal, you can open and
drive that browser pane without any special SDK — just print an escape
sequence to stdout, or call the bundled `vmuxctl` CLI for anything that needs
a reply.

## Is vmux actually running?

```
vmuxctl ping
```

Prints `vmux is running` and exits 0 if reachable, otherwise reports the pipe
is unavailable. If `vmuxctl` isn't on PATH at all, you're not inside vmux (or
the vmux install didn't add its bin dir to PATH) — everything below is a
no-op outside vmux, but skip it if `ping` fails so you don't waste a turn.

## Interacting with the page: `vmuxctl browser eval` (has a real return value)

This is the primary tool for actually *working with* the page — reading its
content, clicking things, filling in forms, waiting on state — not just
opening it. It runs your JavaScript in the page and prints whatever the
script's final expression evaluates to (JSON-encoded), so you can read the
result back in your own turn instead of guessing.

```
vmuxctl browser eval "document.title"
vmuxctl browser eval "document.body.innerText.slice(0, 2000)"
vmuxctl browser eval "document.querySelector('#email').value = 'a@b.com'; document.querySelector('#submit').click(); 'clicked'"
vmuxctl browser eval "[...document.querySelectorAll('a')].map(a => a.href)"
```

Practical patterns:
- **Read the page before acting on it.** Don't guess selectors — pull
  `document.documentElement.outerHTML` (or a narrower `innerText`/`querySelector`
  read) first, look at what's actually there, then write the click/fill script.
- **Return values from the script's last expression.** `document.title` returns
  the title; a `;`-separated script returns whatever the last statement
  evaluates to — end with an explicit value (like the `'clicked'` example
  above) if the action itself doesn't naturally return something useful.
- **It throws like normal JS.** If the script throws, `vmuxctl` exits non-zero
  and prints the error message — check for that instead of assuming success.
- **It waits up to 10s** for the page's JS to respond, then times out with an
  error. That's plenty for synchronous DOM work; for something that depends on
  a network response, poll with a couple of short `eval` calls rather than one
  long-running script.
- Runs against the **first open browser pane** — if several are open, there's
  no way yet to target a specific one.

## Other write actions (OSC escape sequences — no reply)

These are one-directional: print the sequence to stdout and vmux's terminal
parser acts on it immediately. Unlike `vmuxctl browser eval` above, there is
no return value — use these only for open/navigate/close, not for anything
you need a result from. If more than one browser pane is open, all of them
receive the same command.

| Action | Escape sequence | Effect |
|---|---|---|
| Open a URL | `\x1b]vmux;browser-open;<url>\x07` | Opens a browser pane (splitting the current tab if none is open yet) and navigates to `<url>` |
| Navigate | `\x1b]vmux;browser-navigate;<url>\x07` | Navigates the existing browser pane(s) to `<url>` |
| Run JavaScript (fire-and-forget) | `\x1b]vmux;browser-eval;<js>\x07` | Evaluates `<js>` in the page with no return value — prefer `vmuxctl browser eval` above instead unless you truly don't care about the result |
| Close | `\x1b]vmux;browser-close\x07` | Closes the browser pane(s) |

**Bash / zsh / Git Bash:**

```bash
printf '\033]vmux;browser-open;%s\007' "https://example.com"
printf '\033]vmux;browser-navigate;%s\007' "http://localhost:3000"
printf '\033]vmux;browser-close\007'
```

**PowerShell:**

```powershell
$e = [char]27; $bel = [char]7
Write-Host -NoNewline "$e]vmux;browser-open;https://example.com$bel"
```

**cmd.exe** can't emit raw ESC/BEL bytes cleanly — use the `vmuxctl` commands
below instead, which reach vmux over a named pipe rather than through
terminal output.

## Full `vmuxctl` command reference (reply over a named pipe)

`vmuxctl` ships alongside vmux and works from any shell, including cmd.exe,
since it talks to vmux over IPC rather than printing escape codes.

```
vmuxctl browser navigate <url>     navigate the browser pane to a URL
vmuxctl browser eval <js>          run JS in the page, print its return value
vmuxctl browser url                print the current browser URL
vmuxctl list                       list open terminal panes (id, title, is-agent)
vmuxctl notify <message>           post a notification badge on this terminal's sidebar entry
```

`vmuxctl browser screenshot` exists as a command but is **not implemented
yet** in this build — it always returns an error. Don't rely on it; there's
no way to capture the browser pane's contents as an image right now. Read the
page via `vmuxctl browser eval "document.documentElement.outerHTML"` (or a
narrower `innerText`/`querySelector` read) instead of trying to "look at" it.

## Practical guidance

- Prefer `browser-open` the first time you need the browser in a session, and
  `browser-navigate` afterward — `open` is safe to call again too (it's a
  no-op split if a pane is already there).
- To actually work with a page (read content, click, fill forms, check
  state), use `vmuxctl browser eval` — it's the only path that gives you a
  return value back.
- If the user asks you to "show them" something, `browser-open` with the
  relevant URL is almost always the right move — it's a low-friction way to
  hand a live page back to the person driving the session, not just describe
  it in text.

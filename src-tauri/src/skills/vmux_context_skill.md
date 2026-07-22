---
name: vmux-context
description: Search and retrieve past conversation history and notes stored by vmux (a Windows terminal multiplexer for AI coding agents) via the vmuxctl CLI — semantic search across every imported Claude Code session plus user notes, and pulling a full conversation once you've found the relevant one. Use when running inside a vmux terminal pane and you need to recall something from a previous session (a decision, an approach that was tried, a bug that was already diagnosed) instead of re-deriving it from scratch. Has no effect and is safe to try outside vmux; use `vmuxctl ping` first if you need to confirm vmux is actually running.
---

# vmux context search

vmux keeps a persistent, searchable store of past agent conversations (imported
from Claude Code's own session transcripts) and user-authored notes, shared
across every project and workspace. As an agent, you can search it before
assuming something hasn't been figured out before.

## Is vmux actually running?

```
vmuxctl ping
```

If `vmuxctl` isn't on PATH at all, you're not inside vmux — skip everything
below.

## Two-step workflow: search, then get the whole thing

**1. Search** — semantic search across all imported history and notes:

```
vmuxctl context search "why did we switch off the daemon terminal path"
vmuxctl context search "rate limiting approach" --top-k 5
```

Each hit prints a score, a title, a project name, the matched chunk's role
(`user`/`assistant`/`tool_use`/`tool_result`), a `conversation_id`, and the
matched chunk's full content (not truncated).

**2. Get the full conversation** — search only returns the single best-matching
chunk per hit; once you've found the right `conversation_id`, pull everything:

```
vmuxctl context get <conversation_id>
```

Prints the title, project, and every chunk in that session in order — the
whole thread, not a fragment, so you get the actual reasoning/decisions
around the snippet search found, not just the one matching line.

## What's actually searchable

- **Claude Code sessions.** Imported from `~/.claude/projects/*.jsonl` — user
  messages, assistant messages, and (as of this version) tool calls and their
  results, so you can find not just what was *said* but what was actually
  *done* in a past session.
- Import happens automatically: once at vmux startup, and again after every
  Stop/Notification/SessionStart/TaskCompleted event for any Claude terminal
  that has vmux's notification hooks installed. **If hooks aren't installed
  for this machine, auto-import doesn't fire mid-session** — the user can
  still backfill everything via the "Import Claude" button in the Context
  Manager (Ctrl-A x → History tab).
- **Only Claude Code is captured this way today.** Other agent CLIs running
  in vmux terminals aren't imported into this store — if you're not Claude
  Code, don't assume your own prior sessions are searchable here.
- User-authored notes (the Notes tab in the Context Manager) are also
  embedded and included in search results.

## Practical guidance

- Search before you re-derive something that spans more than the current
  turn — a past architectural decision, a bug that was already root-caused,
  an approach that was already tried and rejected. A quick `context search`
  is cheaper than rediscovering it.
- Scores are cosine similarity against the embedding provider currently
  configured (defaults to a local, dependency-free hash embedding if the
  user hasn't set up Voyage/OpenAI-compatible embeddings — treat scores as a
  rough ranking signal, not a precise relevance threshold, especially on the
  local provider).
- If search comes back empty, it might mean nothing's been imported yet
  (chunks also need to be embedded — the Context Manager has an "Embed all
  chunks" button) rather than that nothing relevant exists.

# The Rust core owns all state and processes; the frontend is a view

Status: accepted (2026-09-30)

The Tauri Rust core owns everything stateful:
- the single shared ACP adapter process and per-session JSON-RPC routing;
- one Worktree actor per Worktree (watcher, git status cache, file index);
- the `claude` memory monitor;
- every session transcript and the app-state file.

The SolidJS frontend only renders. It gets a streaming channel for the visible Tab (bundled to about one update per frame) and small state events for background Tabs. (Amended by ticket 28: "the visible Tab" is one per view slot, so the Tabs view has one and each column of the Columns view has its own; a session is visible while any slot shows it.) Conversations are *not* stored by the editor; Claude Code's own transcripts are the single copy, resumed through ACP. This keeps each extra Tab near-free in the webview (the ≤ 15 MB per-Tab budget in ADR 0002), survives window reloads and pop-outs, and avoids a second copy of each conversation that could drift.

## Consequences

- If the ACP adapter crashes, every session drops at once. The core must restart the adapter and resume sessions.
- A system Node ≥ 22.12 is required, because the adapter runs on Node.
- If Claude Code changes or prunes its transcript storage, resume and restore break with it.

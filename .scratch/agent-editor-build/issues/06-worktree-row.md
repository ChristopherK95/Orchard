# 06: Worktree row, discovery and context bar

**What to build:** The git service (the `git` command only, ADR 0004) lists the Workspace's Worktrees into a top tab row: branch with `⎇`, Worktree colour, ahead count, a mini state dot per session, dimmed when it has no sessions, main checkout first. Worktrees created outside the editor appear live. Selecting a Worktree shows its sessions in the row below and returns to the session last used there. A context bar shows branch · path · ahead/behind · changed count › session · state, and background Worktrees show a Needs-you badge.

**Blocked by:** 04 (Multiple sessions and Tab states).

**Status:** done (verified in the real app on Windows 11, 2026-10-01: live add/remove, last-used session per Worktree, context bar, a removed Worktree with a running session stays reachable)

- [x] Every Worktree from `git worktree list` appears; the main checkout comes first, and Worktrees without sessions are dimmed with "＋ session".
- [x] A Worktree added or removed from a terminal appears or disappears without a restart (via watching `.git/worktrees/`, and on window focus).
- [x] Selecting a Worktree restores its last-used session.
- [x] The context bar reflects the active Worktree and session.
- [x] Background Worktrees with Needs-you sessions show a badge.
- [x] Git calls use machine-readable output, with `GIT_OPTIONAL_LOCKS=0` for background calls and `GIT_TERMINAL_PROMPT=0` for all calls.
- [x] Core tests with real `git` in temporary repos: add a worktree via the CLI → a Worktree-list event is emitted.

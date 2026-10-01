# 13: File watching, file index, Ctrl+P and the Files drawer

**What to build:** Each shown Worktree gets a Worktree actor. It watches non-ignored directories (on Linux, per-directory watches from an ignore-aware walker; on Windows, one recursive watch with ignored events dropped and a rescan on overflow). If the watch limit is exhausted it falls back to 5 s polling, with a toast giving the `sysctl` fix. The actor keeps an in-memory file index, matched by frizbee for `Ctrl+P`, which also offers "New Agent session in a fresh worktree / in this worktree". The Files drawer shows the tree with changed files marked.

**Blocked by:** 06 (Worktree row).

**Status:** ready-for-agent

- [ ] Creating, deleting and renaming files updates the index and tree without a restart; ignored directories are never watched.
- [ ] `Ctrl+P` returns ranked results instantly on a large repo; typos still match, below exact matches.
- [ ] `Ctrl+P` includes the two new-session commands.
- [ ] The Files drawer is scoped to the active Worktree and marks changed files.
- [ ] Watch-limit exhaustion switches that Worktree to polling, with a toast.
- [ ] Actors stop for Worktrees that are dimmed and idle.
- [ ] The Worktree row's and context bar's ahead/behind and changed counts update live (ticket 06 refreshes them only on open, focus, Worktree-list changes and turn ends, so a terminal commit shows late).
- [ ] Core tests with real temp directories on both OSes, including the forced polling fallback.

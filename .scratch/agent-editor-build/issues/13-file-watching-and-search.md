# 13: File watching, file index, Ctrl+P and the Files drawer

**What to build:** Each shown Worktree gets a Worktree actor. It watches non-ignored directories (on Linux, per-directory watches from an ignore-aware walker; on Windows, one recursive watch with ignored events dropped and a rescan on overflow). If the watch limit is exhausted it falls back to 5 s polling, with a toast giving the `sysctl` fix. The actor keeps an in-memory file index, matched by frizbee for `Ctrl+P`, which also offers "New Agent session in a fresh worktree / in this worktree". The Files drawer shows the tree with changed files marked.

**Blocked by:** 06 (Worktree row).

**Status:** done (core tests on Windows; Arch pending)

- [x] Creating, deleting and renaming files updates the index and tree without a restart; ignored directories are never watched.
- [x] `Ctrl+P` returns ranked results instantly on a large repo; typos still match, below exact matches.
- [x] `Ctrl+P` includes the two new-session commands.
- [x] The Files drawer is scoped to the active Worktree and marks changed files.
- [x] Watch-limit exhaustion switches that Worktree to polling, with a toast.
- [x] Actors stop for Worktrees that are dimmed and idle.
- [x] The Worktree row's and context bar's ahead/behind and changed counts update live (ticket 06 refreshes them only on open, focus, Worktree-list changes and turn ends, so a terminal commit shows late).
- [ ] Core tests with real temp directories on both OSes, including the forced polling fallback.

Notes: the index comes from `git ls-files` (ignore-aware, via the git CLI per ADR 0004) rather than a separate ignore-aware walker; deleted tracked files are left out. Until the Manual editor (ticket 14), picking a file in Ctrl+P reveals it in the Files drawer. The drawer is on the right edge (story 13). Branch status is refreshed per Worktree, 200 ms after the last relevant change (its files, its git folder, the shared refs).

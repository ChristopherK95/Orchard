# 18: Git drawer: status, stage, commit, discard

**What to build:** The Git drawer shows the active Worktree's branch, ahead/behind and changed files, updating live as Agents work. It supports whole-file stage/unstage (and all at once), commit with a typed message, an "Amend last commit" option that warns if the commit is already pushed, and per-file discard with confirmation. Committing while a session in that Worktree is Working shows a warning with Commit anyway / Cancel.

**Blocked by:** 13 (File watching and search).

**Status:** done

- [x] Status updates within ~200 ms of the last change.
- [x] Stage, unstage, stage all and unstage all work on whole files.
- [x] Commit and amend work; amending a pushed commit warns first.
- [x] Discard restores the file after confirmation.
- [x] Committing while a session is Working shows the warning.
- [x] Core tests with real `git` for each operation.

**Notes (done):** The drawer shares the right edge with the Files drawer (one at a time; Ctrl+Shift+E / Ctrl+Shift+G, except when the editor uses Ctrl+Shift+G for find-previous).
- `GitStatusChanged` is sent as soon as the watcher has let a burst settle (200 ms after the last change). The drawer then reads `git status`. The test allows 700 ms from the last write of a burst; it measured well under that on Windows.
- Every operation goes through `git.rs`: `status`, `stage`/`unstage` (a rename as one; before the first commit too), `commit` (a 5-minute timeout for hooks or signing), `head_pushed`, `discard` (back to HEAD, or deleted if new).
- The core answers "ask first" for a session mid-turn (Working or Needs you) and for amending a pushed commit (`CommitOutcome`). It refuses Worktrees being removed.

Not covered yet:
- Clicking a file opens it in the Manual editor. Its diff view comes with ticket 21.
- Conflicted files can't be discarded here; ticket 20's banner handles them.

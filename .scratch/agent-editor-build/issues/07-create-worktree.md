# 07: Create a Worktree + session

**What to build:** "＋ worktree" (and the `Ctrl+P` command once it exists) opens a dialog. The branch name is prefilled `agent/<slug>` and editable. The base defaults to a freshly fetched `origin/<default>`, with a dropdown for the active branch or any branch/commit, and an "existing branch" mode. Enter creates the Worktree at `<repo>.worktrees/<slug>/` and starts an Agent session in it.

**Blocked by:** 06 (Worktree row).

**Status:** done (verified in the real app on Windows 11, 2026-10-01). Note: the prefilled name is agent/task-N; Worktree setup between creation and the first session comes with ticket 08.

- [x] Enter with defaults fetches, creates branch `agent/<slug>` from `origin/<default>` in a folder next to the repo, and opens a session Tab there.
- [x] A different base or an existing branch can be chosen.
- [x] Choosing a branch already checked out elsewhere is refused with a clear message.
- [x] Core tests with a temp repo and bare origin: the created Worktree has the expected path, branch and base.

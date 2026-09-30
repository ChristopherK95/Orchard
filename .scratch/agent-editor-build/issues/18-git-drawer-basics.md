# 18: Git drawer: status, stage, commit, discard

**What to build:** The Git drawer shows the active Worktree's branch, ahead/behind and changed files, updating live as Agents work. It supports whole-file stage/unstage (and all at once), commit with a typed message, an "Amend last commit" option that warns if the commit is already pushed, and per-file discard with confirmation. Committing while a session in that Worktree is Working shows a warning with Commit anyway / Cancel.

**Blocked by:** 13 (File watching and search).

**Status:** ready-for-agent

- [ ] Status updates within ~200 ms of the last change.
- [ ] Stage, unstage, stage all and unstage all work on whole files.
- [ ] Commit and amend work; amending a pushed commit warns first.
- [ ] Discard restores the file after confirmation.
- [ ] Committing while a session is Working shows the warning.
- [ ] Core tests with real `git` for each operation.

# 21: Changes vs base

**What to build:** A "Changes vs base" mode in the Git drawer lists the files the Worktree's branch changed since it split from its **Base** (default `origin/<default>`, changeable). Clicking a file opens its read-only diff in the Manual editor pane (unified, with a side-by-side toggle) with an "Edit file" button. Together with "Worktree from existing branch", this is the manual review flow.

**Blocked by:** 18 (Git drawer basics), 16 (Conflicts; it provides the diff view).

**Status:** ready-for-agent

- [ ] The list matches a three-dot diff against the Base, with add/modify/delete/rename status.
- [ ] The Base can be changed, and the list updates.
- [ ] Clicking a file opens its diff; Edit file opens the file itself.
- [ ] Core tests with real `git`: a branch with add, modify, delete and rename vs its base.

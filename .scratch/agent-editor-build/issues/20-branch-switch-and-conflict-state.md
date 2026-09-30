# 20: Branch switching and merge/rebase state

**What to build:** Switching a Worktree's branch from the Git drawer is blocked while any of its sessions is Working, and confirmed otherwise. Branches checked out in another Worktree are greyed out with "Go to that Worktree", and the picker offers "New Worktree from this branch". When a merge or rebase is in progress (whoever started it), the drawer shows a banner listing the conflicted files (click to open) and an Abort button.

**Blocked by:** 18 (Git drawer basics).

**Status:** ready-for-agent

- [ ] Switching is disabled while a session in the Worktree is Working.
- [ ] Branches checked out elsewhere can't be picked and link to their Worktree.
- [ ] "New Worktree from this branch" goes to the create flow.
- [ ] An in-progress merge or rebase shows the banner with the conflicted files and a working Abort.
- [ ] Sessions stay bound to the Worktree across a branch switch.
- [ ] Core tests with real `git`: switch, a blocked switch, and conflict detection and abort.

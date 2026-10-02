# 20: Branch switching and merge/rebase state

**What to build:** Switching a Worktree's branch from the Git drawer is blocked while any of its sessions is Working, and confirmed otherwise. Branches checked out in another Worktree are greyed out with "Go to that Worktree", and the picker offers "New Worktree from this branch". When a merge or rebase is in progress (whoever started it), the drawer shows a banner listing the conflicted files (click to open) and an Abort button.

**Blocked by:** 18 (Git drawer basics).

**Status:** done

- [x] Switching is disabled while a session in the Worktree is Working.
- [x] Branches checked out elsewhere can't be picked and link to their Worktree.
- [x] "New Worktree from this branch" goes to the create flow.
- [x] An in-progress merge or rebase shows the banner with the conflicted files and a working Abort.
- [x] Sessions stay bound to the Worktree across a branch switch.
- [x] Core tests with real `git`: switch, a blocked switch, and conflict detection and abort.

**Notes (done):**
- "Working" here means mid-turn: Working or Needs you. A session waiting on a permission card still has a turn running, as with commit and pull.
- `switch_branch` uses the same branch resolution as the create flow (`create_worktree::resolve_checkout`). A remote row with a local branch of the same name switches to that branch, or leads to the Worktree that has it.
- `GitStatus.operation` reads the Worktree's own git folder: merge, rebase, `git am`, cherry-pick or revert, including a multi-commit one between steps (sequencer only). The watcher treats those files as git state, so a merge started by an Agent or a terminal shows at once.
- Abort asks first. While a session there is mid-turn or an operation is in progress, the picker's rows are disabled with the reason.
- After a switch, open Manual editor tabs follow ticket 16's on-disk rules. The confirm mentions it when a file in the Worktree has unsaved changes.

Not covered yet:
- The picker's branch list duplicates the New Worktree dialog's. Extract it when ticket 21's Base picker needs a third copy.
- The mid-turn check and the switch aren't atomic (a prompt sent in that instant would start mid-switch). Commit and pull have the same small window.

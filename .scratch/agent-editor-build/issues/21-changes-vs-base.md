# 21: Changes vs base

**What to build:** A "Changes vs base" mode in the Git drawer lists the files the Worktree's branch changed since it split from its **Base** (default `origin/<default>`, changeable). Clicking a file opens its read-only diff in the Manual editor pane (unified, with a side-by-side toggle) with an "Edit file" button. Together with "Worktree from existing branch", this is the manual review flow.

**Blocked by:** 18 (Git drawer basics), 16 (Conflicts; it provides the diff view).

**Status:** done

- [x] The list matches a three-dot diff against the Base, with add/modify/delete/rename status.
- [x] The Base can be changed, and the list updates.
- [x] Clicking a file opens its diff; Edit file opens the file itself.
- [x] Core tests with real `git`: a branch with add, modify, delete and rename vs its base.

**Notes (done):**
- The core owns each Worktree's Base, kept in app state per Worktree: a new branch's start point, a Base set in the drawer, or the default. It's checked in the Worktree, so `HEAD~2` means that branch's own history.
- `changes_vs_base` returns the list and the split (merge-base). Each file's diff (`diff_vs_base`) runs from that split to HEAD, so a list and its diffs can't disagree after a fetch.
- Only committed changes are listed: "Edit file" opens the working copy, which can have more. This is recorded in CONTEXT.md under **Base**.
- Diff tabs are keyed by Worktree, file and split. The conflict diff (ticket 16) and the review diff share `DiffPanel`.

Not covered yet:
- The Base input is free text (it takes branches, tags and commits), not a picker. The shared branch list is still to extract (see ticket 20's notes).

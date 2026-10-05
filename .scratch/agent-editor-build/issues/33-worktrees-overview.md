# 33: Worktrees overview (merged branches)

**What to build:** A "Worktrees" button in the title bar opens an overview of every Worktree of the repository, saying whether each one's branch has been merged into its **Base**, so the Worktrees whose PRs are done can be removed. Each row offers "Remove…", which opens the usual Remove dialog.

**Blocked by:** 09 (Remove a Worktree safely), 21 (Changes vs base: the per-Worktree Base).

**Status:** done (Arch: core-tested and type-checked; not run in the app; Windows pending)

- [x] Opening it shows what the last fetch knew at once, then fetches (with `--prune`) and looks again. A failed fetch says so, and the list stays.
- [x] "Merged" covers each way a PR lands: a merge commit or fast-forward (the branch's commits are in the Base), and a squash or rebase (its changes are: `git merge-tree` of the branch into the Base changes nothing, or `git cherry` finds every commit).
- [x] A PR merged into another branch than the Base (a project branch) counts too: when the Base doesn't have it, the remote branches committed to since the branch's last commit are tried (the 25 newest), and the row says which one ("Merged into origin/project/x"). There only a merge commit or the changes count, not a fast-forward, so a branch stacked on this one isn't taken for where it was merged.
- [x] Which branch it was merged into is the one it arrived on first: a branch that merged the project branch in (or was made from it after the merge) has the commits too. By history (the commit they arrived with), then by how many PRs went into each since; time last.
- [x] With no Base set, a Worktree's Base is the branch git says it was made from (its reflog: created from, or last reset onto, another branch), so Worktrees made outside the editor from a project branch are measured against it.
- [x] A new branch with no commits of its own isn't "merged": it's "No commits of its own yet". A fast-forward is told apart from one by the branch's reflog (commits were made on it).
- [x] A remote branch that was deleted, but whose changes aren't found in the Base, is shown as such (the PR may have been closed without merging), not as merged.
- [x] Merged Worktrees with nothing uncommitted come first, marked; uncommitted changes and running sessions are shown on every row. The main checkout is listed, without Remove.
- [x] Clicking a row goes to that Worktree.
- [x] Each Worktree (not the main checkout) has a checkbox; the ready ones start ticked. "Remove checked" checks each first and lists what it does (branch deleted only if merged, sessions stopped, ignored files deleted), then removes them one at a time. It never discards: one that would lose work is skipped (use its own Remove…), and work that appears after the check is refused by the core.
- [x] Core tests with real `git` and a bare origin: merge commit, squash, rebase, fast-forward, fresh, unmerged, closed (remote deleted) and empty-commit branches.

**Notes (done):**
- Core: `merged.rs` (`MergeState`, `WorktreeMerge`, `detect`), `Core::merge_overview()`, the `merge_overview` command. It's measured against each Worktree's own Base (`base_of`), resolved in the main checkout.
- The removal check now uses the Worktree's Base too (it used the repo's default Base), and the same detection for `merged`. A squash- or rebase-merged branch's commits are still listed, but deleting the branch no longer needs "Discard and remove": the Base has their changes.
- `merge-tree --write-tree` writes the merged trees' objects to the object store (never refs, the index or a folder); they're unreachable and go at the next gc.

Not covered yet:
- Merges into another branch are looked for on remote branches only (where a host merges PRs), not local ones.
- No count on the title-bar button: the check runs only while the overview is open.
- A squash merge after which the Base changed the same lines isn't recognised (merge-tree conflicts, and no single commit matches); it shows as "Remote branch deleted" if the host deleted the branch.

# Git UI scope for v1

Type: grilling
Status: resolved
Blocked by: none
Part of: [map](../map.md)

## Question

What exactly can the user do from the Git drawer in v1? The git layer is the `git` command only (ADR 0004), and v1 already includes status, diff, stage, commit, push, branch switch and "Changes vs base". Settle:

- **Staging granularity**: whole files only, or hunks or lines too?
- **Commit messages**: typed by the user only, or can the active Agent session be asked to draft one?
- **Branch switching inside a Worktree**: allowed at all, given a branch can be checked out in only one Worktree? And what happens to that Worktree's Agent sessions?
- **Pull / fetch / rebase / merge**: which are in v1, and how are merge or rebase conflicts surfaced (leave them to the Agent, or show conflicted files)?
- **Stash, discard changes, amend**: in or out?
- What the Git drawer shows for a Worktree with running Agent sessions that are mid-edit.

## Answer

Resolved 2026-09-30.

- **Staging**: whole files only, plus Stage all / Unstage all. Hunk or line staging is not in v1.
- **Commit messages**: typed by the user in the drawer. No Agent "Draft" button; to have an Agent commit, ask it in chat.
- **Branch switching in a Worktree**:
  - Disabled while any session there is **Working**, and confirmed otherwise.
  - Branches checked out elsewhere are greyed out, with **Go to that Worktree**.
  - The picker also offers **New Worktree from this branch**.
  - Sessions stay bound to the Worktree (its folder), whatever branch it's on.
- **Fetch / pull / merge / rebase**:
  - **Fetch** is manual, plus automatic when the window regains focus, at most every 5 minutes, for shown Worktrees.
  - **Pull** is fast-forward only. If the branch has diverged, it says so and suggests an Agent or a terminal.
  - **No merge or rebase buttons.**
  - When a merge or rebase is in progress, whoever started it, the drawer shows a banner, the conflicted files (click to open), and **Abort**. Resolving is left to the user or an Agent.
- **Discard / amend / stash**: **discard per file** (confirmed) is in. The **Amend last commit** checkbox is in, with a warning if the commit is already pushed. **Stash is out**, since cheap Worktrees replace it.
- **Committing during Agent work**: status updates live. If Commit is pressed while a session in that Worktree is **Working**, the drawer warns that its changes may be incomplete, with **Commit anyway / Cancel**. Staging and viewing are never blocked.

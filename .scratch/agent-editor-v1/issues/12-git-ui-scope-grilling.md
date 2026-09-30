# Git UI scope for v1

Type: grilling
Status: open
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

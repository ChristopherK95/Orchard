# Worktree and Agent session lifecycle

Type: grilling
Status: open
Blocked by: none
Part of: [map](../map.md)

## Question

What are the rules for creating, discovering, and removing Worktrees and their Agent sessions? Settle:

- The "new session in a fresh worktree" flow: branch naming, base branch, and where worktree directories live on disk.
- Discovering worktrees made outside the editor.
- Removing a worktree that is dirty or has running sessions.
- What closing a Tab does to its session and its worktree.
- How Tabs are grouped and filtered by Worktree.

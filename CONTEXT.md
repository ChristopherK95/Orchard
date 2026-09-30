# Context

An agent-first source-code editor: the coding agent is the primary surface; manual editing and git tooling support it.

## Glossary

- **Workspace**: one git repository opened in an editor window. One window holds exactly one Workspace.
- **Worktree**: a git worktree belonging to the Workspace's repository. The main checkout is also a Worktree.
- **Agent session**: one running coding-agent conversation, bound to exactly one Worktree for its whole life. Shown as a **Tab**. Several Agent sessions may share a Worktree (the editor warns when they do).
- **Tab**: the UI surface of one Agent session. Hopping between Worktrees means switching or filtering Tabs, not opening another window.
- **Agent**: the coding-agent program driving an Agent session. Claude Code first; others may be supported later behind the same boundary.

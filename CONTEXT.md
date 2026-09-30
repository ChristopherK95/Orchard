# Agent-first editor

An agent-first source-code editor: the coding agent is the primary surface; manual editing and git tooling support it.

## Language

### Structure

**Workspace**:
One git repository opened in an editor window. One window holds exactly one Workspace.
_Avoid_: Project, repo window

**Worktree**:
A git worktree belonging to the Workspace's repository. The main checkout is also a Worktree.
_Avoid_: Checkout, branch (a Worktree has a branch; it is not one)

**Agent session**:
One running coding-agent conversation, bound to exactly one Worktree for its whole life. Several Agent sessions may share a Worktree (the editor warns when they do).
_Avoid_: Instance, chat, agent (the Agent is the program; the session is one conversation with it)

**Tab**:
The UI surface of one Agent session. Hopping between Worktrees means switching or filtering Tabs, not opening another window.

**Agent**:
The coding-agent program driving an Agent session. Claude Code first; others may be supported later behind the same boundary.

### Agent session states

**Working**:
The Agent is mid-turn.

**Needs you**:
The Agent is blocked on the user: a permission prompt or a question is waiting. The only state that always notifies.
_Avoid_: Blocked, waiting

**Idle**:
The turn has finished and the Agent is waiting for the next prompt.

**Suspended**:
The Agent's process has been stopped to free memory while the Tab and its conversation are kept; returning to the Tab resumes it.
_Avoid_: Paused, sleeping

**Exited**:
The Agent's process ended without being Suspended (crash, or it quit).

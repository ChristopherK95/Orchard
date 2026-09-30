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

**Base**:
The branch or commit a Worktree's branch is compared against in "Changes vs base". Defaults to `origin/<default branch>`.
_Avoid_: Upstream (that's the branch it pushes to), target

**Worktree setup**:
The per-repo list of commands run in a newly created Worktree before its first Agent session starts.
_Avoid_: Bootstrap, init script

**Recent sessions**:
The closed Agent sessions of a Worktree whose conversations can still be reopened. Closing a Tab moves its session here; it doesn't delete it.

**Edit note**:
The record of the user's saved manual changes to a file, attached to the next prompt of each Agent session on that Worktree that has read or edited the file.
_Avoid_: File-change notification, sync

**Board**:
The overview of every Agent session in the Workspace, grouped by Agent session state. It's the alternative to the Tabs view; toggle between them.
_Avoid_: Dashboard, overview

**Manual editor**:
Where the user views and hand-edits a file in a Worktree. It opens as a pane beside the chat and can be popped out into its own window.
_Avoid_: Text editor window, code view

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

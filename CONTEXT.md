# Agent-first editor

An agent-first source-code editor: the coding agent is the primary surface; manual editing and git tooling support it.

## Language

### Structure

**Workspace**:
One git repository opened in an editor window. One window holds exactly one Workspace.
_Avoid_: Project, repo window

**Workspace picker**:
Where a Workspace is chosen: at startup, or over an open Workspace to switch the window to another. Lists the Recent Workspaces and also takes a typed path or a browsed folder. (The UI calls it "Open a repository".)
_Avoid_: Project picker, welcome screen

**Recent Workspaces**:
The Workspaces opened in this editor before, most recently opened first. Removing one hides it from the list but keeps its saved Tabs and Recent sessions.

**Worktree**:
A git worktree belonging to the Workspace's repository. The main checkout is also a Worktree.
_Avoid_: Checkout, branch (a Worktree has a branch; it is not one)

**Agent session**:
One running coding-agent conversation, bound to exactly one Worktree for its whole life. Several Agent sessions may share a Worktree (the editor warns when they do).
_Avoid_: Instance, chat, agent (the Agent is the program; the session is one conversation with it)

**Tab**:
The UI surface of one Agent session. Hopping between Worktrees means switching or filtering Tabs, not opening another window.

**Base**:
The branch or commit a Worktree's branch is compared against in "Changes vs base": what it committed since it split from the Base (not uncommitted work). Defaults to the start point the Worktree was branched off (in the editor, or as git's reflog records it: the branch it was created from, or last reset onto), else `origin/<default branch>`; it can be changed, and each Worktree keeps its own.
_Avoid_: Upstream (that's the branch it pushes to), target

**Worktrees overview**:
Every Worktree of the Workspace in one list, with whether its branch is merged into its Base (or into another branch its PR targeted, such as a project branch), so finished ones can be removed. Merged counts however the PR landed: a merge commit or fast-forward (the branch's commits are in the Base), or a squash or rebase (its changes are). A remote branch that was deleted is shown, but doesn't count as merged by itself.
_Avoid_: Clean-up, prune

**Tool call**:
One action the Agent takes with a tool (reading or editing a file, running a command, …), shown as a compact row in the transcript with its status.
_Avoid_: Step, action

**Worktree setup**:
The per-repo list of commands run in a newly created Worktree before its first Agent session starts.
_Avoid_: Bootstrap, init script

**Recent sessions**:
The closed Agent sessions of a Worktree whose conversations can still be reopened. Closing a Tab moves its session here; it doesn't delete it.

**Conversations started elsewhere**:
The Agent's own conversations in a Worktree that are neither Tabs nor Recent sessions: ones started in a terminal, or closed so long ago they left the Recent sessions. Any of them can be opened in a Tab with its conversation. Listed only when asked for (the Recent menu, or the link in an empty Worktree), as listing starts the Agent.
_Avoid_: External sessions, imported sessions

**Edit note**:
The record of the user's saved manual changes to a file, attached to the next prompt of each Agent session on that Worktree that has read or edited the file. Shown as a removable chip above the composer; a removed one is never sent.
_Avoid_: File-change notification, sync

**Board**:
The overview of every Agent session in the Workspace, grouped by Agent session state. It opens over the Tabs view or the Columns view; toggle it with either.
_Avoid_: Dashboard, overview

**Columns view**:
The wide-display alternative to the Tabs view (offered from 1,600 px): each pinned Worktree side by side as a column, each column showing one of its Agent sessions at a time. More columns than fit scroll sideways.
_Avoid_: Split view, panes

**Pinned Worktree**:
A Worktree that has a column in the Columns view (choosing it in the sidebar adds one, "Close column" in the column's menu closes it). Pins are kept per Workspace across restarts; the columns follow the sidebar's order.

**Focused column**:
The one column of the Columns view that takes input: the next prompt, Y / N, the Files/Git drawer and Ctrl+P all go to its Worktree. Only it has the full composer.

**Manual editor**:
Where the user views and hand-edits a file in a Worktree (or the settings file, or an unsaved snippet copied from a chat code block). It opens as a pane beside the chat, with a file tab per file, and can be popped out into its own window. When a file it has open changes on disk, a file tab with no unsaved changes reloads; one with unsaved changes asks (show the diff, reload and drop mine, or keep mine and overwrite on the next save).
_Avoid_: Text editor window, code view

**Terminal panel**:
The user's own shell in a Worktree's folder: docked beside the chat in the Tabs view and under the column's transcript in the Columns view. Opening or closing it is per Worktree and holds in both views (Ctrl+` toggles the active Worktree's, which in the Columns view is the focused column's). Each Worktree has one shell, started the first time a panel shows that Worktree. It keeps running while out of sight (a dev server, say) and a panel comes back to its output. It stops when the user stops it, or when its Worktree is removed or the Workspace closes. It runs the repo's Windows shell setting on Windows, and `$SHELL` on Linux.
_Avoid_: Console, integrated terminal

**Action**:
A named list of commands in the repo's settings, run on demand in a Worktree (from Ctrl+P): each command is typed into a new shell of the Terminal panel, named after the Action (or after the command, when it has several), so its output stays in view and it can be stopped or rerun there. Running the Action again restarts its shells in that Worktree; one of them can be restarted alone from the panel.
_Avoid_: Task, script, run configuration

**Review**:
Going through everything a Worktree's pull request would bring, file by file (committed or not, new files too, from where its branch split from its Base), marking each viewed, and then opening the PR: what isn't committed yet is committed, the branch is pushed, and the PR is opened into the chosen branch with the user as assignee.
_Avoid_: Code review (that's what happens on the PR afterwards), diff view

**Agent**:
The coding-agent program driving an Agent session. Claude Code first; others may be supported later behind the same boundary.

### Agent session states

**Working**:
The Agent is mid-turn.

**Needs you**:
The Agent is blocked on the user: a permission prompt or a question is waiting. The only state that always notifies when you're not looking at the session.
_Avoid_: Blocked, waiting

**Idle**:
The turn has finished and the Agent is waiting for the next prompt.

**Suspended**:
The Agent's process has been stopped to free memory while the Tab and its conversation are kept; sending a message in the Tab resumes it (just looking at the Tab doesn't).
_Avoid_: Paused, sleeping

**Exited**:
The Agent's process ended without being Suspended (crash, or it quit). Resuming it brings the conversation back.

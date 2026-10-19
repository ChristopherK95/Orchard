# Agent-first editor

An agent-first source-code editor: the coding agent is the primary surface; manual editing and git tooling support it.

## Language

### Structure

**Workspace**:
One git repository opened in an editor window. One window holds exactly one Workspace.
_Avoid_: Project, repo window

**Workspace picker**:
Where a Workspace is chosen: at startup, or over an open Workspace to switch the window to another. Lists the Recent Workspaces and also takes a typed path or a browsed folder. It can also make a new repository (a named folder in a chosen location, with `git init` and an empty first commit, so Worktrees can branch from it) and open it. (The UI calls it "Open a repository".)
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
Every Worktree of the Workspace but the main checkout (which can't be removed) in one list, with whether its branch is merged into its Base (or into another branch its PR targeted, such as a project branch), so finished ones can be removed. Merged counts however the PR landed: a merge commit or fast-forward (the branch's commits are in the Base), or a squash or rebase (its changes are). A remote branch that was deleted is shown, but doesn't count as merged by itself. A Review Worktree counts once its Code review is done.
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
_Avoid_: Overview (the Dashboard is the overview of the user's work; the Board is only the sessions)

**Dashboard**:
The live status wall of the user's own work in the Workspace, and the view a Workspace opens on: the Agent sessions that need them, reviews requested from them, their open PRs by stage, Worktrees that need attention or can be cleaned up. Each item leads to where it lives (a Tab, the PR overlay, a Code review, a Worktree); a PR with failing checks or review feedback can start an Agent session on it; and, in a column beside them, statistics of the user's work over a chosen period.
_Avoid_: Home, Board (that's only the Agent sessions)

**Columns view**:
The wide-display alternative to the Tabs view (offered from 1,600 px): each pinned Worktree side by side as a column, each column showing one of its Agent sessions at a time. More columns than fit scroll sideways.
_Avoid_: Split view, panes

**Pinned Worktree**:
A Worktree that has a column in the Columns view (choosing it in the sidebar adds one, "Close column" in the column's menu closes it). Pins are kept per Workspace across restarts; the columns follow the sidebar's order.

**Focused column**:
The one column of the Columns view that takes input: the next prompt, Y / N, the Files/Git drawer and Ctrl+P all go to its Worktree. Only its composer takes the keyboard.

**Manual editor**:
Where the user views and hand-edits a file in a Worktree (or the settings file, or an unsaved snippet copied from a chat code block). It opens as a pane beside the chat, with a file tab per file, and can be popped out into its own window. When a file it has open changes on disk, a file tab with no unsaved changes reloads; one with unsaved changes asks (show the diff, reload and drop mine, or keep mine and overwrite on the next save).
_Avoid_: Text editor window, code view

**Terminal panel**:
The user's own shell in a Worktree's folder: docked beside the chat in the Tabs view and under the column's transcript in the Columns view. Opening or closing it is per Worktree and holds in both views (Ctrl+` toggles the active Worktree's, which in the Columns view is the focused column's). Each Worktree has one shell, started the first time a panel shows that Worktree. It keeps running while out of sight (a dev server, say) and a panel comes back to its output. It stops when the user stops it, or when its Worktree is removed or the Workspace closes. It runs the repo's Windows shell setting on Windows, and `$SHELL` on Linux. The sidebar marks each Worktree whose shells are running (how many, and whether one is running something), so the ones out of sight aren't forgotten; clicking the mark shows that Worktree's panel.
_Avoid_: Console, integrated terminal

**Action**:
A named list of commands in the repo's settings, run on demand in a Worktree (from Ctrl+P): each command is typed into a new shell of the Terminal panel, named after the Action (or after the command, when it has several), so its output stays in view and it can be stopped or rerun there. Running the Action again restarts its shells in that Worktree; one of them can be restarted alone from the panel.
_Avoid_: Task, script, run configuration

**Review**:
Going through everything a Worktree's pull request would bring, file by file (committed or not, new files too, from where its branch split from its Base), marking each viewed, and then opening the PR: what isn't committed yet is committed, the branch is pushed, and the PR is opened into the chosen branch with the user as assignee.
_Avoid_: Code review (that's reviewing someone else's PR), diff view

**Code review**:
Going through a pull request the user is a reviewer on, file by file, in its Review Worktree: the PR's head against its merge base with the PR's base branch (or, on a re-requested review, only what changed since the user's last review), Viewed ticks shared with GitHub, the PR's threads under their lines (with Reply), comments on lines held in the Pending review, and a verdict (Approve, Request changes or Comment) submitted with them. New commits pushed meanwhile are offered with Refresh. One review Agent session per PR, docked beside it, can review the whole PR ("Claude review") or answer a question about selected lines ("Ask Claude"); nothing it writes is posted without the user.
_Avoid_: Review (that's the user's own Worktree before its PR is opened), PR review screen

**Review Worktree**:
The Worktree a Code review is read in: `review-pr-<N>`, detached at the PR's fetched head (so a fork's PR works too), never on a branch and never the user's own Worktree on the PR's branch. Read-only: its files open read-only in the Manual editor and its Agent sessions stay in Plan mode. It carries a "Review" badge, and the Worktrees overview counts it ready to remove once its review is submitted or the PR is merged or closed.
_Avoid_: Review checkout, PR worktree

**Pending review**:
The comments on lines of a Code review not sent yet, with the lines asked Claude about, kept by the editor (they survive closing the overlay and restarts) and sent in one go with the verdict. A failed submit keeps it as it was. A comment whose line went away with new commits stays on its file as outdated, to be moved or removed.
_Avoid_: Draft review, GitHub's pending review (the editor keeps it, not GitHub)

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

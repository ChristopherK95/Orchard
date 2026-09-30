# Worktree and Agent session lifecycle

Type: grilling
Status: resolved
Blocked by: none
Part of: [map](../map.md)

## Question

What are the rules for creating, discovering, and removing Worktrees and their Agent sessions? Settle:

- The "new session in a fresh worktree" flow: branch naming, base branch, and where worktree directories live on disk.
- Discovering worktrees made outside the editor.
- Removing a worktree that is dirty or has running sessions.
- What closing a Tab does to its session and its worktree.
- How Tabs are grouped and filtered by Worktree.
- Suspend rules (from the Agent session Tab decision): when manual suspend is offered, what auto-suspend after N idle minutes means, whether to **auto-suspend the longest-idle sessions under memory pressure** (from the stack decision), and what resuming looks like.

## Answer

Resolved 2026-09-30.

**Creating a Worktree ("＋ worktree")**
- **Location**: next to the repo, at `<repo>.worktrees/<branch-slug>/` (e.g. `~/dev/myrepo.worktrees/agent-fix-login/`). Never nested inside the repo.
- **Dialog**:
  - The branch name is prefilled `agent/<slug>` and editable.
  - The base defaults to a freshly fetched `origin/<default branch>`, with a dropdown for the active Worktree's branch or any branch/commit.
  - An **existing branch** mode checks out an existing branch instead (also the future path into review mode).
  - Enter accepts the defaults.
- **Worktree setup**: a per-repo **setup command list** (e.g. `pnpm pre-config`, `pnpm install`) runs in the new Worktree, in order, stopping at the first failure.
  - The first Agent session **waits** for setup. On failure, the Tab shows the output with **Retry / Start anyway**.
  - There is no files-to-copy list; scripts handle that.
  - Commands run in `bash` on Linux and `pwsh` on Windows (Git Bash if configured). There's one shared list, with an optional per-OS override.

**Editor settings (where setup lives)**
- A hand-edited **TOML** file in the editor's app config folder (not in the repo). An "Open repo settings" command opens it in the Manual editor; there's no settings UI in v1.
- Per-repo sections are keyed by the **`origin` remote URL**, falling back to the main checkout's path when there's no remote. Other editor settings can live in the same file later.

**Discovering Worktrees**
- Every Worktree from `git worktree list` is shown, including ones created outside the editor, kept live by watching.
- The main checkout is always first. A Worktree with no sessions is shown dimmed, with "＋ session".

**Removing a Worktree**
- Running sessions are stopped first, after confirmation.
- Uncommitted changes and commits that aren't pushed or merged are listed. Removal needs an explicit **Discard and remove**; there's never a silent force.
- A "Delete branch too" checkbox is offered, ticked only if the branch is merged.
- The main checkout can't be removed.

**Closing a Tab**
- Stops the process and hides the Tab. The conversation stays resumable from a per-Worktree **Recent sessions** list, and `Ctrl+Shift+T` reopens the last closed session.
- Never touches the Worktree. Closing its last Tab just dims it.

**Suspend rules**
- Working and Needs you sessions are never auto-suspended.
- Idle auto-suspend is off by default; when enabled, it triggers after N minutes (default 30).
- **Memory pressure**: when all `claude` processes together exceed a configurable limit (default 4 GB), or the OS reports low memory, the longest-Idle sessions are suspended first until back under it. A toast says what was suspended.
- **Lazy resume**: a Suspended Tab shows its saved conversation instantly; the process restarts only when a message is sent.

**Editor restart**
- Every previously open Tab is restored as **Suspended**, in the same Worktree and order.

# Spec: Agent-first editor v1

Status: ready-for-agent
Part of: [map](map.md)
Sources: the map's Decisions so far (12 resolved tickets in `issues/`), ADRs 0001–0004 in `docs/adr/`, and the glossary in `CONTEXT.md`. Research and prototypes are on the `research/*` and `prototype/*` branches.

## Problem Statement

I do most of my coding through Claude Code now, often with several Agent sessions at once, each in its own git Worktree so they don't trample each other. My tools are built the other way round: code editors put the file editor at the centre and the agent in a side panel, and none of them handle "many sessions across many Worktrees" well. In practice that means:

- one editor window (or terminal) per Worktree;
- losing track of which session is working, finished, or waiting on me for a permission;
- manually creating and cleaning up worktrees, then re-running setup (`pnpm install`, etc.) in each;
- when I fix something by hand, the Agent doesn't know unless I tell it;
- reviewing a colleague's branch means juggling checkouts and diff tools.

It all has to run well on both machines I use, Arch Linux (Wayland, AMD GPU) and Windows 11, without eating memory when many sessions are open.

## Solution

A desktop editor where the **Agent session is the primary surface**. One window holds one Workspace (a repo). Its Worktrees and their Agent sessions are laid out as two rows of Tabs: Worktrees on top, and the active Worktree's sessions below. A **Board** view shows every session grouped by state, so "who needs me?" is one glance or one keypress.

- **Worktrees**: creating a fresh one (with its setup commands) and starting a session in it is a two-keystroke flow. Worktrees made elsewhere show up automatically, and removal is guarded against losing work.
- **Chat**: each session renders as a native chat: markdown, static highlighted code blocks, tool calls, and permission cards that show a diff *before* I approve an edit.
- **Manual editing**: a fuzzy file search opens files in a CodeMirror-based **Manual editor** pane (optional vim) beside the chat. When I save a hand edit, the sessions that have touched that file get an **Edit note** with my diff, attached to my next prompt as a visible chip.
- **Git**: a Files/Git drawer gives status, staging, committing, pushing, pulling, branch switching, and a **"Changes vs base"** view. Together with "check out an existing branch into a new Worktree", that's the manual code-review flow.
- **Memory**: idle sessions can be **Suspended** (process stopped, conversation kept) manually, after idle time, or under memory pressure. They resume lazily when I next send a message.

Under the hood:

- It's built on **Tauri 2** with a **Rust core** and a **SolidJS** frontend (ADR 0002).
- Sessions are hosted through **ACP** (`claude-agent-acp`) on my Claude subscription (ADR 0001). The core owns all processes and state; the frontend is a view (ADR 0003).
- Git goes through the `git` command only (ADR 0004).

## User Stories

### Workspace and layout

1. As a developer, I want to open a repo as a Workspace in one window, so that all of its Worktrees and Agent sessions live in one place instead of one window per Worktree.
2. As a developer, I want the Worktrees of the Workspace shown as a top row of tabs (branch name with `⎇`, Worktree colour, ahead count, a mini state dot per session), so that I can tell the Worktrees apart and see their activity at a glance.
3. As a developer, I want a second row showing only the active Worktree's Agent sessions (`✦`, state dot, name), so that Worktrees and sessions are never confused.
4. As a developer, I want the active Worktree tab visually raised and joined to the session row in its colour, so that it's always obvious which Worktree I'm in.
5. As a developer, I want a context bar under the tabs showing branch, path, ahead/behind, number of changed files, session name and session state, so that I know exactly where my next prompt goes.
6. As a developer, I want switching to a Worktree to return me to the session I last used there, so that hopping between Worktrees doesn't lose my place.
7. As a developer, I want a "Needs you" badge on background Worktree tabs, so that I notice sessions waiting on me in Worktrees I'm not looking at.
8. As a developer, I want a Tabs | Board toggle in the title bar (and `Ctrl+B`, with `Esc` returning to Tabs), so that I can switch between focused work and the overview.
9. As a developer, I want the Board to show every Agent session in the Workspace in columns by state (Needs you, Working, Idle, Suspended/Exited), filterable by Worktree, so that I can triage all my sessions in one view.
10. As a developer, I want to approve or deny a pending permission directly from a Board card, so that I can unblock sessions without opening each one.
11. As a developer, I want clicking a Board card to open that session's Tab, so that I can dive into a session from the overview.
12. As a developer, I want an "N need you" button in the title bar that jumps to the Board, so that waiting sessions are never more than one click away.
13. As a developer, I want a Files/Git drawer on the right edge, scoped to the active Worktree, so that file browsing and git are at hand without leaving the chat.
14. As a developer, I want the editor to restore all my previously open Tabs (as Suspended, in the same Worktrees and order) after a restart, so that nothing is lost and a restart costs almost no memory.

### Agent sessions and chat

15. As a developer, I want to start a new Agent session in the current Worktree ("＋ session"), so that I can run several conversations in the same checkout.
16. As a developer, I want a warning when two sessions share a Worktree, so that I remember they can edit the same files.
17. As a developer, I want each session rendered as a native chat (my messages, Agent messages as markdown, collapsed tool-call rows), so that it's easier to read than a terminal.
18. As a developer, I want code blocks in the chat highlighted with line numbers, Copy, and "Open in editor", so that I can read and reuse code quickly.
19. As a developer, I want long conversations to stay fast, so that a session with hundreds of messages doesn't slow the editor down.
20. As a developer, I want a permission request shown as an inline card with Allow / Always allow / Deny, answerable with `Y`/`N`, so that approving tool use is fast.
21. As a developer, I want Edit permission cards to show the diff before I approve, so that I review changes before they happen.
22. As a developer, I want Edit permission cards to warn when I have unsaved changes in that file, so that I don't lose work to an Agent edit.
23. As a developer, I want to switch a session's permission mode (Ask for edits / Accept edits / Plan) per Tab, defaulting to Ask for edits, so that I can loosen or tighten control per task.
24. As a developer, I want each Tab to show its Agent session state (Working, Needs you, Idle, Suspended, Exited), so that I know what every session is doing.
25. As a developer, I want an OS notification when a session I'm not looking at becomes Needs you, so that I don't leave Agents blocked.
26. As a developer, I want an optional notification when a background session finishes its turn, so that I can pick up results promptly if I choose to.
27. As a developer, I want to type while the Agent is working and have my message delivered after its current tool calls, so that I can steer it mid-turn.
28. As a developer, I want to close a Tab without destroying the conversation, and find it again under that Worktree's Recent sessions, so that closing is never destructive.
29. As a developer, I want `Ctrl+Shift+T` to reopen the last closed session, so that I can undo an accidental close.
30. As a developer, I want a session whose Agent process crashed to show as Exited with a resume action, so that I can continue the conversation.
31. As a developer, I want all sessions to recover automatically if the shared Agent adapter crashes, so that one crash doesn't cost me every conversation.

### Suspend and memory

32. As a developer, I want to suspend a session manually, so that I can free its memory (~250 MB) while keeping its conversation.
33. As a developer, I want optional auto-suspend of sessions Idle for N minutes (off by default, 30 when on), so that forgotten sessions don't hold memory.
34. As a developer, I want the editor to auto-suspend the longest-Idle sessions when all Agent processes together exceed a configurable limit (default 4 GB) or the OS reports low memory, with a toast saying what was suspended, so that my machine stays responsive.
35. As a developer, I want Working and Needs you sessions never to be auto-suspended, so that active work is never interrupted.
36. As a developer, I want a Suspended session to show its saved conversation instantly and only restart its process when I send a message, so that browsing Tabs costs no memory.
37. As a developer, I want the editor itself to stay lean (≤ 250 MB with 5 Tabs and 1 open Manual editor, ≤ 15 MB per extra Tab, near 0% CPU idle), so that the memory goes to the Agents, not the shell.

### Worktree lifecycle

38. As a developer, I want "＋ worktree" to open a small dialog with the branch name prefilled `agent/<slug>` and the base defaulting to a freshly fetched `origin/<default>`, so that the common case is two keystrokes.
39. As a developer, I want to choose a different base (the active Worktree's branch, or any branch/commit), so that I can branch off work in progress.
40. As a developer, I want to create a Worktree from an existing branch, e.g. a colleague's, so that I can review or continue it in isolation.
41. As a developer, I want new Worktrees created next to the repo (`<repo>.worktrees/<slug>/`), so that they're easy to find in a terminal and never nested inside the repo.
42. As a developer, I want per-repo Worktree setup commands (e.g. `pnpm pre-config`, `pnpm install`) to run in each new Worktree before its first Agent session starts, so that sessions don't start in a broken checkout.
43. As a developer, I want setup output shown in the Tab, stopping at the first failure with Retry / Start anyway, so that I can fix setup problems without guessing.
44. As a developer, I want one setup command list with an optional per-OS override, run in `bash` on Linux and `pwsh` on Windows, so that the same repo works on both machines.
45. As a developer, I want Worktrees created outside the editor (terminal, Agents) to appear automatically, so that the Worktree row always matches reality.
46. As a developer, I want Worktrees with no sessions shown dimmed with a "＋ session" prompt, and the main checkout always first, so that I can see every checkout I have.
47. As a developer, I want removing a Worktree to stop its sessions first (after confirmation), so that no Agent is left running in a deleted folder.
48. As a developer, I want removal to list uncommitted changes and unpushed/unmerged commits and require an explicit "Discard and remove", so that I never silently lose work.
49. As a developer, I want a "Delete branch too" option, pre-ticked only when the branch is merged, so that cleanup is safe by default.
50. As a developer, I want the main checkout to be un-removable, so that I can't delete the repo itself by accident.

### Manual editor and file search

51. As a developer, I want `Ctrl+P` fuzzy file search scoped to the active Worktree, instant even on large repos and tolerant of typos, so that I can open any file quickly.
52. As a developer, I want `Ctrl+P` to also offer "New Agent session in a fresh worktree / in this worktree", so that starting work is as fast as opening a file.
53. As a developer, I want files to open in a Manual editor pane to the right of the chat, with its own file tabs, so that code and conversation sit side by side.
54. As a developer, I want a pop-out button that moves a file into its own OS window, so that I can use a second monitor when I need to.
55. As a developer, I want syntax highlighting for common languages, loaded on demand, so that code is readable without slowing startup.
56. As a developer, I want optional vim keybindings (`editor.vim`, off by default) with `:w` saving and `:q` closing the file tab, so that I can edit the way I'm used to.
57. As a developer, I want find/replace, go to line, soft-wrap toggle, bracket matching, auto-indent and multiple cursors, so that light edits are comfortable.
58. As a developer, I want files over ~5 MB opened read-only without highlighting, binaries shown as a placeholder, and minified single-line files soft-wrapped, so that odd files never freeze the editor.
59. As a developer, I want to browse the active Worktree's files in the Files drawer, with changed files marked, so that I can find things without knowing their names.

### Edit notes and conflicts

60. As a developer, I want the sessions on a Worktree that have read or edited a file to be told when I save a manual change to it, so that Agents don't work from stale assumptions.
61. As a developer, I want sessions that never touched the file not to be told, so that unrelated conversations aren't cluttered.
62. As a developer, I want the Edit note to be a per-file diff of my changes (capped at ~200 lines, beyond which it says the file changed substantially), combining several saves, so that the Agent gets a precise but bounded update.
63. As a developer, I want the Edit note shown as a chip above the composer ("📝 You edited `status.rs` (+5 −1)"), expandable and removable, and sent with my next message, so that I always see and control what the Agent is told.
64. As a developer, I want Edit notes for Suspended sessions to wait until they resume, so that nothing is lost.
65. As a developer, I want open files with no unsaved changes to reload silently when an Agent changes them, so that I always see current code.
66. As a developer, I want open files with unsaved changes to show a "Agent changed this file" banner with Show diff / Reload / Keep mine, never reloading automatically, so that my edits are never overwritten.
67. As a developer, I want saving to warn me if the file changed on disk since I opened it, so that I don't overwrite an Agent's newer work.

### Git

68. As a developer, I want the Git drawer to show the active Worktree's branch, ahead/behind and changed files, updating live as Agents work, so that I always see the real state.
69. As a developer, I want to stage and unstage whole files (and all at once), so that I can build commits from Agent output.
70. As a developer, I want to write a commit message and commit, with an "Amend last commit" option that warns if the commit is already pushed, so that I can fix up my last commit safely.
71. As a developer, I want a warning when I commit while a session in that Worktree is Working, so that I don't commit half-finished Agent edits.
72. As a developer, I want to discard changes per file with confirmation, so that I can reject an Agent's edit to one file.
73. As a developer, I want to push, fetch, and fast-forward pull, with fetch also happening automatically on window focus (at most every 5 minutes), so that my Worktrees stay current without ceremony.
74. As a developer, I want a pull that can't fast-forward to explain that the branch has diverged and suggest an Agent or terminal, so that I know what to do.
75. As a developer, I want push and fetch to use my existing git credentials (credential helpers, SSH agent), with a clear message if git needs input, so that auth just works like my terminal.
76. As a developer, I want to switch a Worktree's branch, blocked while any of its sessions is Working, with branches checked out elsewhere greyed out and a "Go to that Worktree" link, so that switching never breaks a running Agent or confuses git.
77. As a developer, I want the branch picker to offer "New Worktree from this branch", so that the better option is always right there.
78. As a developer, I want a banner with the conflicted files and an Abort button when a merge or rebase is in progress (whoever started it), so that I can see and escape conflicts.
79. As a developer, I want a "Changes vs base" view listing the files my branch changed since it split from its Base (default `origin/<default>`, changeable), so that I can review a branch's full change set.
80. As a developer, I want clicking a file in "Changes vs base" to open its diff (unified, with a side-by-side toggle, read-only, with an "Edit file" button) in the Manual editor pane, so that reviewing a branch is fast.

### Settings and setup

81. As a developer, I want all settings in one hand-edited TOML file in the editor's config folder, with an "Open repo settings" command that opens it in the Manual editor, so that configuration is plain and versionable by me.
82. As a developer, I want per-repo sections keyed by the repo's `origin` URL (falling back to its path), so that settings follow the repo across clones.
83. As a developer, I want settings changes to apply when I save the file, without restarting, so that tweaking is quick.
84. As a developer, I want clear startup messages when git ≥ 2.55 or Node ≥ 20 is missing, so that I know exactly what to install.
85. As a developer, I want the editor to run on Arch Linux (Wayland) and Windows 11 from the same codebase with one dev command, so that I can use it on both machines.

## Implementation Decisions

### Architecture (ADRs 0002, 0003)

- **One app process**: Tauri 2 (stable 2.x) with the single-instance behaviour. A Rust **core** plus a **SolidJS** frontend in the webview. A second Workspace opens a second main window in the same process, but that's not a v1 priority, since normally only one repo is open.
- **The core owns everything stateful**: all child processes, all Agent session state and transcripts in memory, Worktree actors, settings, and persisted app state. The frontend holds no source of truth. It renders core state and sends commands.
- **Core modules** (each a deep module behind a small interface):
  - **Workspace manager**: opens a repo, owns its Worktrees and Agent sessions, restores and persists app state.
  - **ACP host**:
    - Spawns and supervises the **single shared** `claude-agent-acp` Node process, and routes JSON-RPC per Agent session.
    - Maps ACP events to Agent session updates, and permission requests to Needs you.
    - On adapter crash it restarts the adapter and resumes the sessions; sessions that were mid-turn become Exited.
    - The adapter executable path is configurable; this is the test seam.
  - **Agent session**:
    - Holds the state machine (below), the in-memory transcript, the permission mode, and the **touched-files set** (every path the session has read or edited, from ACP tool-call events).
    - Keeps the **pending Edit note queue**.
    - Handles suspend (stop the process, keep the transcript view) and lazy resume (restart via ACP resume on the next send).
  - **Worktree actor**: one per Worktree, running only while the Worktree is shown and stopping when it's dimmed and idle. It owns:
    - the file watcher;
    - the git status cache (refreshed 200 ms after the last relevant event);
    - the in-memory file index for fuzzy search;
    - fallback to 5 s status polling if the watch limit is exhausted.
  - **Git service**: the only thing that runs `git` (ADR 0004).
    - Machine-readable output throughout (`--porcelain=v2 -z`, `worktree list --porcelain -z`, `diff --name-status -z`).
    - `GIT_OPTIONAL_LOCKS=0` for background calls and `GIT_TERMINAL_PROMPT=0` for all calls.
    - One interface, so a library could replace a hot path later.
  - **Worktree lifecycle**:
    - Create: fetch, then `worktree add` next to the repo, then Worktree setup, then the first session.
    - Discover: watch `.git/worktrees/`, re-list, and refresh on window focus.
    - Remove, with guards: dirty state, unpushed/unmerged commits, merged-branch detection, never the main checkout.
  - **Document tracker**: knows which files are open in any Manual editor and whether they're dirty. It matches Worktree-actor file events against open documents (silent reload vs conflict banner) and turns saves into Edit notes for the sessions whose touched-files set contains the path.
  - **Memory monitor**: samples the memory of all `claude` processes every ~10 s and triggers memory-pressure auto-suspend.
  - **Settings**: loads, validates and watches the TOML file, and applies changes live.
- **Frontend modules**:
  - Tab strip (Worktree row + session row), context bar, Board.
  - Chat view: a virtualised transcript with static highlighted code blocks and diffs.
  - Permission card, composer with Edit note chips.
  - Manual editor pane (CodeMirror 6, vim, merge view) and pop-out window.
  - Files/Git drawer, including "Changes vs base".
  - Fuzzy palette, toasts/notifications.

### Agent session state machine

A trimmed version of the rules settled in tickets 05 and 07:

```
Working   --turn ends--------------------> Idle
Working   --permission/question pending--> Needs you
Needs you --user answers-----------------> Working
Idle      --user sends-------------------> Working
Idle      --manual / idle timeout / memory pressure--> Suspended
Suspended --user sends (lazy resume)----> Working
any live  --process ends unexpectedly----> Exited
Exited    --user resumes-----------------> Idle
(never auto-suspend from Working or Needs you; manual suspend of Working is not offered)
```

### Core ↔ frontend contract (Tauri IPC)

- **Commands** (request/response), grouped by area:
  - **Workspace**: open Workspace.
  - **Worktrees**: create Worktree (branch, base or existing branch), remove Worktree (with the explicit discard flag), select Worktree.
  - **Sessions**: new session, send prompt (with Edit note chips the user kept), answer permission, set permission mode, suspend, resume, close/reopen session, fetch transcript page.
  - **Files**: fuzzy search, read file, save file (with the expected on-disk version).
  - **Git**: stage/unstage, commit/amend, discard, push/fetch/pull, switch branch, "Changes vs base" list and per-file diff, abort merge/rebase.
  - **Settings**: open settings.
- **Streaming**: the **visible Tab** gets its session's updates over a dedicated Tauri `Channel`, batched to about one flush per frame (~16 ms). Switching Tabs re-points the channel and fetches the transcript as pages for the virtualised view.
- **Events** (small, broadcast):
  - session state changes, unread counts, Needs you;
  - Worktree list changes;
  - git status changes for shown Worktrees;
  - document changed on disk (clean → reload, dirty → conflict);
  - pending Edit notes per session;
  - setup progress/output;
  - toasts (auto-suspend, watch-limit fallback, missing prerequisites).
- **Background Tabs never stream transcript content**, which is what keeps each extra Tab at or under 15 MB.

### Agent hosting (ADR 0001)

- Claude Code first, through the ACP adapter, authenticated with the user's **own Claude subscription**. We accept the terms grey zone knowingly; if it breaks, it's fixed then (e.g. by switching to an API key).
- ACP is also the boundary for other Agents later; no other Agents in v1.
- Requires a **system Node ≥ 20**. The adapter version is pinned as a dependency the editor installs.
- The adapter must not run in a mode that disables subscription auth (e.g. Claude Code's `--bare`).
- There's no terminal fallback (escape hatch) in v1.

### Persistence

- **Conversations**: only in Claude Code's own transcripts, resumed via ACP. The editor stores no copy.
- **App state**: JSON in the app data folder. Per Workspace, it records open Tabs (session id, Worktree, order, name, permission mode), Recent sessions per Worktree, the active Worktree/session per Worktree, and window/pane layout.
- **Settings**: TOML in the app config folder, reloaded live on save. Contents in v1:
  - `editor.vim` (default false);
  - memory-pressure limit (default 4 GB);
  - idle auto-suspend (default off, 30 min when on);
  - optional background turn-finished notification;
  - default permission mode (Ask for edits);
  - keybinding overrides;
  - per-repo sections keyed by `origin` URL (or path) containing the **Worktree setup** command list, with optional `linux` / `windows` overrides and the optional Git Bash choice on Windows.

This settings list and live reload were the map's last open item. They're decided here as defaults.

### Worktree setup

- Commands run in order in the new Worktree, in `bash` on Linux and `pwsh` on Windows (Git Bash if configured), stopping at the first failure.
- Output streams to the Tab. On failure the Tab offers Retry / Start anyway.
- The first Agent session waits for setup.
- There's no files-to-copy list; the user's scripts handle that.

### Edit notes

- **Trigger**: a save. The recipients are the live sessions on that Worktree whose touched-files set contains the path; Suspended sessions queue their notes.
- **Content**: one unified diff per file against the last delivered version, combining several saves, capped at ~200 lines, with "changed substantially, re-read it" beyond that.
- **Delivery**: as extra content on the user's **next prompt** in that session, shown as a removable chip. Idle sessions aren't woken, and there's no mid-turn delivery. Claude Code's own changed-file detection is an unrelied-on bonus.

### Manual editor

- **CodeMirror 6.** Highlighting uses Lezer grammars, lazy-loaded per language (plain text if unknown).
- **Vim**: `@replit/codemirror-vim` as-is, with `:w` → save and `:q` → close file tab.
- **Diffs**: `@codemirror/merge`, unified by default with a side-by-side toggle, read-only with Edit file.
- **Chat code blocks and permission diffs are static highlighted HTML** from the same Lezer grammars, with no editor instances (chosen via prototype).
- **Pop-out uses move semantics**: text and cursor move to the new window, and undo history is lost.
- **Save always sends the expected on-disk version**; the core refuses and asks if the file on disk has changed.
- **Big files** (> ~5 MB) open read-only with no highlighting; binaries show a placeholder; minified single-line files force soft-wrap.

### Git (ADR 0004)

- `git` ≥ 2.55, checked at startup. The editor never changes repo git config (no enabling fsmonitor or untracked cache).
- **Watching**:
  - **Linux**: non-recursive watches added by an ignore-aware walker, covering non-ignored directories only, with new directories added as they appear.
  - **Windows**: one recursive watch per Worktree root. Ignored-path events are dropped, and a full rescan runs on overflow.
  - Both also watch git metadata (HEAD, index, refs).
  - If the watch limit is exhausted, that Worktree falls back to polling, with a toast giving the `sysctl` fix.
- **File search**: an ignore-aware walker builds the index, which watcher events keep current; the frizbee matcher ranks results (typo tolerance on, below exact matches).
- **Git UI scope**:
  - whole-file staging;
  - user-typed commit messages, plus amend (warning if pushed);
  - per-file discard;
  - push, fetch (manual, plus on focus at most every 5 min), fast-forward-only pull;
  - guarded branch switch;
  - merge/rebase-in-progress banner with conflicted files and Abort;
  - "Changes vs base" via three-dot diff against the Base.
- **Not in the Git UI**: merge/rebase buttons, stash, and Agent-drafted commit messages.
- **Auth**: only the user's existing credential helpers and SSH setup. If git needs input, the operation fails with the error and "run `git push` in a terminal once".

### Platforms and dev setup

- **Targets**: Arch Linux (Wayland; WebKitGTK via GTK3, AMD GPU, so the NVIDIA workarounds aren't needed) and Windows 11 (WebView2).
- **Dev prerequisites on both**: Rust stable, Node ≥ 20 with pnpm, git ≥ 2.55, the Tauri 2 system prerequisites (`webkit2gtk-4.1` and friends on Arch; WebView2 and the MSVC build tools on Windows).
- **One command runs the app in dev.** No packaging, installers or auto-update in v1.

## Testing Decisions

- **A good test** drives the system only through its public surface and asserts on observable outcomes: events emitted, state reported, files and git state on disk. It never asserts on internal structures, call order or private helpers. Tests should survive a rewrite of the internals.
- **Primary seam: the core's public API.** Tests run the core in-process and drive the same commands the frontend uses, then assert on the events and query results it produces. This one seam covers session state transitions, Edit note routing and content, conflict detection, suspend/resume, Worktree lifecycle and guards, setup, the Git UI operations, "Changes vs base", file search and settings reload.
- **Behind the seam:**
  - **Fake ACP agent**: a small scripted program speaking ACP over stdio, plugged in through the configurable adapter path. Scripts cover:
    - replies and streamed chunks;
    - file reads and edits (to populate touched-files sets);
    - permission requests;
    - delays mid-turn, crashes, resume.

    No real `claude`, no subscription usage, deterministic.
  - **Real `git` against temporary repos** (with a temporary bare "origin" where push/fetch are needed) and real temp directories, so the watcher, the watch-limit fallback (simulated by lowering the limit or forcing the fallback), status refresh, worktree operations and conflict states are exercised for real. git is deliberately not faked, because the point of ADR 0004 is terminal-identical behaviour.
  - Tests should run on **both Linux and Windows**, since watching, shells (`bash`/`pwsh`) and process handling differ.
- **The frontend is not a test seam.** There are no component or unit tests for the SolidJS views, which render core events. UI behaviour is checked manually, plus the benchmark below.
- **Memory budget benchmark**: a scripted run of the real app with the fake ACP agent. It opens 5 Tabs and 1 Manual editor, streams a long transcript and idles, measuring the editor's own processes (excluding `claude` and the adapter) as PSS on both OSes. It fails above ≤ 250 MB total, ≤ 15 MB per extra Tab, or non-trivial idle CPU. If the first build misses it, the stack decision is reopened (ADR 0002); afterwards a miss is a regression.
- **Prior art**: none in this repo yet; this spec defines the first tests. The fake-ACP-agent-plus-temp-repo pattern is the template every later core test should follow.

## Out of Scope

- **PR-integrated review mode**: listing PRs where I'm an assigned reviewer, and clicking to check out. The local "Changes vs base" and "Worktree from existing branch" flows are in; the host integration is not. The git layer must not preclude it.
- **Other Agents**: non-Claude Agents (Gemini CLI, Codex, …). The ACP boundary is kept, but there are no implementations.
- **A terminal escape hatch** to open a session in the real `claude` TUI.
- **Mid-turn Edit notes** via hooks, and manual "send selection/file to agent".
- **LSP**, autocomplete, minimap and code folding in the Manual editor.
- **Repo-wide content search** and a **plain shell terminal**.
- **Richer git UI**: hunk/line staging, Agent-drafted commit messages, merge/rebase buttons, stash.
- **A built-in credential prompt** for git.
- **Distribution polish**: AUR/MSI packaging, installers, bundled Node runtime, auto-update, settings UI.
- **Multi-Workspace polish**: a second repo window works but isn't optimised for.

## Further Notes

- **Main risk: Anthropic's terms** (ADR 0001). If subscription use through ACP is blocked, sessions stop working until the hosting changes, since there's no terminal fallback in v1. The ACP boundary keeps that change contained.
- **Adapter dependency**: `claude-agent-acp` is a third-party adapter that must keep pace with Claude Code. Pin its version and upgrade deliberately.
- **Undocumented behaviour** is used only as a bonus, never as a requirement. That covers Claude Code's own changed-file detection and its transcript format (resume goes through ACP, not by reading transcripts).
- **Rendering engines**: WebKitGTK (Linux) and WebView2 (Windows) render differently, so check UI on both.
- **Measure early**:
  - the first vertical slice should run the memory benchmark;
  - measure `git` spawn cost on Windows for status refresh; ADR 0004's interface allows swapping in a library if needed;
  - measure inotify watch counts on the user's real repos.
- **Suggested first vertical slice**: open a Workspace → one Worktree → one Agent session over ACP → send a prompt → stream to the visible Tab → answer a permission card. That exercises the ACP host, the Agent session state machine, the channel and the benchmark early.

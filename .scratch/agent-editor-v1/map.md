# Map: Agent-first editor v1

Labels: wayfinder:map

## Destination

A **v1 spec** ready to split into build issues: stack locked (Tauri vs Odin+Raylib, with an ADR), architecture outlined (how Agent sessions are hosted, the git layer, the manual editor), and the v1 feature list scoped. Must run on **Arch Linux (Wayland)** and **Windows 11**.

## Notes

- Domain vocabulary lives in `CONTEXT.md` (Workspace, Worktree, Agent session, Tab, Agent). Use it; call the `domain-modeling` skill whenever a term shifts.
- Personal daily driver first (maybe shared with a team later): no packaging, auto-update, or settings-UI polish in v1.
- **Performance and memory efficiency are the top non-functional priority**: many concurrent Agent sessions and Worktrees must not balloon memory. The user is strongest in TypeScript/web but is fine not being deep in the codebase, so language familiarity is a tiebreaker, not a driver.
- Agent: **Claude Code first**, on a **Claude subscription (not an API key)**. Keep a clean boundary so other Agents could be added later.
- v1 feature set (from charting): Tabs per Agent session; create/discover/remove Worktrees ("new session in a fresh worktree" is the core flow); fuzzy file search → open in the manual editor (viewer-first, light edits, optional non-exhaustive vim); manual edits notify the relevant Agent session; syntax highlighting; basic file tree; git UI (status, diff, stage, commit, push, branch switch).
- Resolve one ticket per session (research tickets excepted). Tracker: local markdown in `.scratch/agent-editor-v1/`.

## Decisions so far

<!-- one line per resolved ticket: [title](issues/NN-slug.md): gist -->

- [Tauri vs Odin+Raylib: facts for the stack decision](issues/01-tauri-vs-odin-raylib-research.md): Tauri 2.12 has everything v1 needs off the shelf (CM6+vim, xterm.js, multi-window, IME), but WebKitGTK risks NVIDIA/Wayland breakage and isn't lighter than Chromium. Odin+raylib is leaner but has no IME or shaping, runs via XWayland, is single-window and means building most of v1 by hand (months vs weeks).
- [Git layer and file-watching options](issues/03-git-layer-options-research.md): only the git CLI covers every worktree op, push/auth, and fast status (fsmonitor); libgit2 lacks worktree move/repair and fast status and is thin for Odin; gitoxide lacks push and worktree remove. inotify's per-directory limits make watching many Worktrees the real cost; Rust has the listing/fuzzy crates, Odin has only raw syscalls.
- [Hosting Claude Code sessions on a subscription](issues/02-hosting-claude-code-on-subscription-research.md): only the unmodified interactive `claude` in a PTY on your own `/login` is explicitly permitted; `-p` stream-json, the Agent SDK, and ACP work on a subscription but sit in a terms grey zone for third-party apps. Every option costs one `claude` process per session (~215–340 MB idle, measured). Edit notification can use Claude Code's built-in changed-file diffs, hook `additionalContext`, or the IDE-integration WebSocket.
- [What an Agent session Tab shows: terminal or native UI](issues/05-agent-session-surface-grilling.md): native chat UI through the ACP adapter on the subscription, accepting the terms risk ([ADR 0001](../../docs/adr/0001-native-chat-via-acp-on-subscription.md)); no terminal escape hatch; inline permission cards that show diffs before approval; Tab states Working / Needs you / Idle / Suspended / Exited; manual suspend plus optional auto-suspend.
- [Choose the stack: Tauri or Odin+Raylib](issues/04-choose-the-stack-grilling.md): Tauri 2.x with a Rust core and a SolidJS frontend ([ADR 0002](../../docs/adr/0002-tauri-2-with-solidjs.md)); `claude` processes dominate memory anyway and Odin would take months of UI groundwork; AMD GPU avoids the WebKitGTK breakage. A development-time memory budget (≤ 250 MB with 5 Tabs + 1 editor window, ≤ 15 MB per extra Tab, near 0% idle CPU) gates the stack.
- [Window and layout prototype](issues/08-window-and-layout-prototype.md): two-row tab strip (Worktrees, then the active Worktree's Agent sessions) plus a Worktree › session context bar; a Board view of all sessions by state, toggled with `Ctrl+B`; Files/Git in a right drawer; the manual editor opens as a pane beside the chat and can pop out into its own window.
- [Worktree and Agent session lifecycle](issues/07-worktree-and-session-lifecycle-grilling.md): worktrees go next to the repo (`<repo>.worktrees/<slug>`), created from fetched `origin/<default>` or an existing branch, then per-repo setup commands run before the first session. Settings are an app-level TOML file keyed by `origin` URL. All worktrees are auto-discovered; removal is guarded. Closing a Tab keeps the conversation resumable; auto-suspend (idle, memory pressure) never touches Working/Needs you; lazy resume on send; a restart restores Tabs as Suspended.
- [Core architecture: processes, IPC, and where state lives](issues/10-core-architecture-grilling.md): the Rust core owns all state and processes ([ADR 0003](../../docs/adr/0003-rust-core-owns-everything-frontend-is-a-view.md)): one shared ACP adapter (needs a system Node ≥ 20), Worktree actors for watcher/git/index, and a `claude` memory monitor. Only the visible Tab gets streamed updates; background Tabs get state events. Conversations live only in Claude Code's transcripts. The pop-out editor moves the buffer to the new window. One app process.
- [How manual edits reach the Agent session](issues/06-manual-edit-notification-grilling.md): on save, only sessions on that Worktree that read or edited the file get an Edit note. The note is a capped per-file diff, shown as a removable chip and sent with the next prompt. Claude Code's own change detection is a bonus. Clean buffers auto-reload when the Agent writes; dirty ones get Show diff / Reload / Keep mine; saves check for newer disk versions.
- [Git layer and file watching under Tauri](issues/11-git-layer-and-watching-grilling.md): the `git` command only, ≥ 2.55 ([ADR 0004](../../docs/adr/0004-git-cli-only.md)), with background calls that don't take git's lock and repo config left alone. Watches cover non-ignored directories per shown Worktree, falling back to polling if the watch limit is hit. Auth relies on existing credential helpers. File search is an in-memory index matched with frizbee. The "Changes vs base" branch-diff view is **in v1**.

## Not yet specified

- **Build & run on both OSes**: minimal dev/build setup for Arch+Wayland and Win11 (no distribution polish). Known prerequisites so far: git ≥ 2.55; a system Node ≥ 20 for the ACP adapter; WebKitGTK and WebView2 runtimes.
- **Config**: the settings file itself is settled (TOML in the app config folder, per-repo sections keyed by `origin` URL; see the lifecycle ticket). What else lives in it (keybindings, memory limit, auto-suspend, permission-mode defaults) and whether it reloads live is still open.

## Out of scope

- **PR-integrated review mode** (list PRs where you're an assigned reviewer, click to check out): wanted, but not in v1. The git layer should not preclude it.
- **LSP in the manual editor**: the Agent is the primary editor.
- **Repo-wide content search and a plain shell terminal**: later, not v1.
- **Non-Claude Agents**: the boundary is kept, but no implementations in v1.
- **Terminal escape hatch** (open an Agent session in the real `claude` TUI): skipped for v1 by the Agent session Tab decision.
- **Mid-turn Edit notes** (delivering manual edits to a Working session via hooks) and **manual "send to agent"** (selection/file → chat): ruled out for v1 by the manual-edit decision.
- **Distribution polish** (AUR/MSI packaging, auto-update, settings UI).

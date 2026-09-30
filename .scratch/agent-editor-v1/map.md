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

## Not yet specified

- **Session persistence**: do Agent sessions survive an editor restart (resume conversations, reopen Tabs)? ACP exposes session list/resume, so this likely rides on the same mechanism as Suspended.
- **Git UI depth**: hunk-level staging? Agent-written commit messages? Conflict handling? Sharpens once the git layer is chosen.
- **Local branch-diff view**: "check out a branch and see the files it changed vs its base". May be cheap enough to fold into v1 once the git layer exists; otherwise it joins review mode out of scope.
- **Build & run on both OSes**: minimal dev/build setup for Arch+Wayland and Win11 (no distribution polish).
- **Config**: keybindings/settings format (file-based, presumably).

## Out of scope

- **PR-integrated review mode** (list PRs where you're an assigned reviewer, click to check out): wanted, but not in v1. The git layer should not preclude it.
- **LSP in the manual editor**: the Agent is the primary editor.
- **Repo-wide content search and a plain shell terminal**: later, not v1.
- **Non-Claude Agents**: the boundary is kept, but no implementations in v1.
- **Terminal escape hatch** (open an Agent session in the real `claude` TUI): skipped for v1 by the Agent session Tab decision.
- **Distribution polish** (AUR/MSI packaging, auto-update, settings UI).

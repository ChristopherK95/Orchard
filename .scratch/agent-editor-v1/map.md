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

## Not yet specified

- **Architecture outline**: process model (one core process owning N agent processes plus per-Worktree file watchers?), frontend↔core IPC shape, where state lives. Hangs on the stack decision and the agent-hosting research.
- **Session persistence**: do Agent sessions survive an editor restart (resume conversations, reopen Tabs)? Hangs on what Claude Code exposes for resuming.
- **Git UI depth**: hunk-level staging? Agent-written commit messages? Conflict handling? Sharpens once the git layer is chosen.
- **Local branch-diff view**: "check out a branch and see the files it changed vs its base". May be cheap enough to fold into v1 once the git layer exists; otherwise it joins review mode out of scope.
- **Agent boundary**: the shape of the adapter that makes "Claude Code first, others later" real without over-building.
- **Build & run on both OSes**: minimal dev/build setup for Arch+Wayland and Win11 (no distribution polish).
- **Config**: keybindings/settings format (file-based, presumably).

## Out of scope

- **PR-integrated review mode** (list PRs where you're an assigned reviewer, click to check out): wanted, but not in v1. The git layer should not preclude it.
- **LSP in the manual editor**: the Agent is the primary editor.
- **Repo-wide content search and a plain shell terminal**: later, not v1.
- **Non-Claude Agents**: the boundary is kept, but no implementations in v1.
- **Distribution polish** (AUR/MSI packaging, auto-update, settings UI).

# Core architecture: processes, IPC, and where state lives

Type: grilling
Status: resolved
Blocked by: none
Part of: [map](../map.md)

## Question

Under Tauri 2 + SolidJS with sessions hosted through ACP (ADR 0001, ADR 0002), what is the process and responsibility layout? Settle:

- Does the **Rust core** own the ACP connection (spawning the shared `claude-agent-acp` Node process and routing JSON-RPC per session), or does the frontend talk to it?
- Which process owns the git layer, file watchers, and fuzzy file index?
- The frontend↔core IPC shape: Tauri commands vs events/channels, and how streaming chat for many Tabs is pushed without flooding the webview.
- Where Workspace, Worktree and Agent session state lives, and what's persisted to disk. The lifecycle decision requires restoring Tabs as Suspended after a restart, a per-Worktree Recent sessions list, the app-level TOML settings file, and measuring memory across all `claude` processes for the memory-pressure auto-suspend.
- How a popped-out manual editor window shares state with the main window. The layout prototype settled on one main window by default, with the manual editor as a pane and pop-out-to-window on demand; each popped-out window is another WebKit process on Linux.

## Answer

Resolved 2026-09-30. Recorded as [ADR 0003](../../../docs/adr/0003-rust-core-owns-everything-frontend-is-a-view.md).

- **Process layout**: one app process (Tauri single-instance). Each Workspace gets a main window, but a second Workspace isn't a v1 priority: the user normally works in one repo. The **Rust core** owns everything stateful. The **SolidJS frontend** is a view.
- **ACP**:
  - The core spawns **one shared `claude-agent-acp`** Node process and routes JSON-RPC per Agent session. The frontend never talks to it.
  - If the adapter crashes, the core restarts it and resumes sessions. Sessions that were mid-turn become Exited, with resume.
- **Node**: a system Node ≥ 20 is required. The adapter version is pinned as a dependency the editor installs, and the editor checks for Node at startup with a clear message. Bundling Node is distribution polish, so it's out of scope.
- **Worktree actors**:
  - One core-side task per Worktree owns its file watcher, git status cache and fuzzy file index, and pushes changes to the frontend.
  - It starts when the Worktree is first shown and stops when the Worktree is dimmed and idle.
  - A core **process monitor** samples the memory of all `claude` processes every ~10 s for memory-pressure auto-suspend.
- **Streaming to the UI**:
  - The core holds every transcript.
  - The **visible Tab** gets its session's updates over a Tauri `Channel`, bundled to about one update per frame (~16 ms).
  - **Background Tabs** get only state, unread and Needs-you events.
  - Switching Tabs fetches the transcript, which renders as a virtualised list (only what's on screen). This is what keeps each extra Tab at or under 15 MB.
- **Persistence** (one copy of each thing):
  - **Conversations** stay in Claude Code's own transcripts and are resumed through ACP; there's no editor copy.
  - **App state** is JSON in the app data folder: open Tabs (session id, Worktree, order, name, permission mode), Recent sessions and window layout, per Workspace.
  - **Settings** are the TOML file in the config folder.
- **Manual editor pop-out uses move semantics**: text and cursor move to the new window, and undo history is lost. The core tracks which files are open and unsaved, for conflict detection.

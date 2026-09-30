# Core architecture: processes, IPC, and where state lives

Type: grilling
Status: open
Blocked by: none
Part of: [map](../map.md)

## Question

Under Tauri 2 + SolidJS with sessions hosted through ACP (ADR 0001, ADR 0002), what is the process and responsibility layout? Settle:

- Does the **Rust core** own the ACP connection (spawning the shared `claude-agent-acp` Node process and routing JSON-RPC per session), or does the frontend talk to it?
- Which process owns the git layer, file watchers, and fuzzy file index?
- The frontend↔core IPC shape: Tauri commands vs events/channels, and how streaming chat for many Tabs is pushed without flooding the webview.
- Where Workspace, Worktree and Agent session state lives, and what's persisted to disk. The lifecycle decision requires restoring Tabs as Suspended after a restart, a per-Worktree Recent sessions list, the app-level TOML settings file, and measuring memory across all `claude` processes for the memory-pressure auto-suspend.
- How a popped-out manual editor window shares state with the main window. The layout prototype settled on one main window by default, with the manual editor as a pane and pop-out-to-window on demand; each popped-out window is another WebKit process on Linux.

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
- Where Workspace, Worktree and Agent session state lives, and what's persisted to disk.
- One main window vs extra OS windows for manual editors, given the Linux cost of a WebKit process per window (to be reconciled with the layout prototype).

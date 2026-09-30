# Choose the stack: Tauri or Odin+Raylib

Type: grilling
Status: resolved
Blocked by: 01, 02, 03
Part of: [map](../map.md)

## Question

Given the research on stacks, Claude Code hosting, and the git layer, which stack does v1 use: **Tauri** or **Odin + Raylib**? Weigh them against the top priority (memory and performance with many Agent sessions and Worktrees), support for Arch Wayland and Win11, and v1 effort. Agree whether to set a concrete memory budget (idle plus per session). Record the decision as an ADR.

Inputs surfaced by the stack research: which GPU the Arch machine has (the WebKitGTK/Wayland failures are NVIDIA-specific); whether the Odin option would use SDL3 instead of raylib's GLFW (it closes the Wayland, IME and multi-window gaps); and the likelihood that the Node `claude` processes, not the editor shell, dominate memory with many sessions. The hosting research measured about 215–340 MB per idle `claude` session. Per the Agent session Tab decision (ADR 0001), no terminal widget is needed. Instead, the stack must render a **rich native chat UI** (markdown, code blocks, diff/permission cards) and speak **ACP (JSON-RPC over stdio)** to a shared Node adapter process that runs regardless of stack.

## Answer

Resolved 2026-09-30. Recorded as [ADR 0002](../../../docs/adr/0002-tauri-2-with-solidjs.md).

- **Stack: Tauri 2.x (stable)**, with a Rust core and a **SolidJS** (TypeScript) frontend. Odin + Raylib and Odin + SDL3 were rejected: the memory saving is small next to the per-session `claude` processes, and the effort is months versus weeks. The Tauri 3 alpha and the CEF runtime are deferred.
- **GPU: AMD**, so the NVIDIA + Wayland WebKitGTK breakage doesn't apply.
- **Memory budget** is a development-time benchmark check, not a runtime limit. It covers the editor's own processes only (excluding `claude` and ACP), measured as PSS on both OSes: **≤ 250 MB with 5 Tabs and 1 manual editor window, ≤ 15 MB per extra Tab, near 0% idle CPU.** If the first prototype misses it, the stack decision reopens; after that, a miss is a regression.
- Reacting to memory at runtime (auto-suspending idle Agent sessions under memory pressure) is handed to the Worktree and Agent session lifecycle ticket.

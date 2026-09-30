# Choose the stack: Tauri or Odin+Raylib

Type: grilling
Status: open
Blocked by: 01, 02, 03
Part of: [map](../map.md)

## Question

Given the research on stacks, Claude Code hosting, and the git layer, which stack does v1 use: **Tauri** or **Odin + Raylib**? Weigh them against the top priority (memory and performance with many Agent sessions and Worktrees), support for Arch Wayland and Win11, and v1 effort. Agree whether to set a concrete memory budget (idle plus per session). Record the decision as an ADR.

Inputs surfaced by the stack research: which GPU the Arch machine has (the WebKitGTK/Wayland failures are NVIDIA-specific); whether the Odin option would use SDL3 instead of raylib's GLFW (it closes the Wayland, IME and multi-window gaps); and the likelihood that the Node `claude` processes, not the editor shell, dominate memory with many sessions. The hosting research measured about 215–340 MB per idle `claude` session. If the PTY route wins in ticket 05, the stack also has to render a full terminal (xterm.js vs libghostty-vt).

# Tauri vs Odin+Raylib: facts for the stack decision

Type: research
Status: open
Blocked by: none
Part of: [map](../map.md)

## Question

What are the concrete pros and cons of **Tauri (v2)** vs **Odin + Raylib** for this editor, specifically on **Arch Linux Wayland** and **Windows 11**? Surface facts; don't decide. Cover at least:

- **Memory/CPU footprint**: idle baseline and the marginal cost per extra Tab / Agent session / editor window (Tauri: WebKitGTK on Linux vs WebView2 on Windows, one webview vs many; Raylib: immediate-mode redraw cost, idle CPU).
- **Wayland reality**: known WebKitGTK-on-Wayland issues (rendering, DMABUF, fractional scaling, blank windows on NVIDIA); Raylib/GLFW Wayland support, HiDPI, clipboard, IME, window decorations.
- **Text & editor**: font rendering/shaping, Unicode, IME, selection, scrolling performance; availability of an editor component with vim mode (CodeMirror 6 + vim, Monaco + vim) vs building one in Odin; syntax highlighting (tree-sitter bindings in each).
- **Terminal rendering** (in case Agent sessions render the Claude Code TUI): xterm.js in a webview vs writing a VT emulator/renderer in Odin; PTY libraries on both OSes (ConPTY on Windows).
- **Multi-window**: separate OS windows for manual editors.
- **Ecosystem & maturity**: Odin library availability (git, fs watching, PTY, JSON, subprocess), Tauri plugin ecosystem, build tooling on both OSes.
- **Dev effort** estimate for v1 in each.

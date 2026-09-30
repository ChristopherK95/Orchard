# Tauri vs Odin+Raylib: facts for the stack decision

Type: research
Status: resolved
Blocked by: none
Part of: [map](../map.md)
Research: branch research/tauri-vs-odin-raylib, .scratch/agent-editor-v1/research/tauri-vs-odin-raylib.md

## Question

What are the concrete pros and cons of **Tauri (v2)** vs **Odin + Raylib** for this editor, specifically on **Arch Linux Wayland** and **Windows 11**? Surface facts; don't decide. Cover at least:

- **Memory/CPU footprint**: idle baseline and the marginal cost per extra Tab / Agent session / editor window (Tauri: WebKitGTK on Linux vs WebView2 on Windows, one webview vs many; Raylib: immediate-mode redraw cost, idle CPU).
- **Wayland reality**: known WebKitGTK-on-Wayland issues (rendering, DMABUF, fractional scaling, blank windows on NVIDIA); Raylib/GLFW Wayland support, HiDPI, clipboard, IME, window decorations.
- **Text & editor**: font rendering/shaping, Unicode, IME, selection, scrolling performance; availability of an editor component with vim mode (CodeMirror 6 + vim, Monaco + vim) vs building one in Odin; syntax highlighting (tree-sitter bindings in each).
- **Terminal rendering** (in case Agent sessions render the Claude Code TUI): xterm.js in a webview vs writing a VT emulator/renderer in Odin; PTY libraries on both OSes (ConPTY on Windows).
- **Multi-window**: separate OS windows for manual editors.
- **Ecosystem & maturity**: Odin library availability (git, fs watching, PTY, JSON, subprocess), Tauri plugin ecosystem, build tooling on both OSes.
- **Dev effort** estimate for v1 in each.

## Answer

Full findings, with sources: [research/tauri-vs-odin-raylib.md](../research/tauri-vs-odin-raylib.md). Facts only; the decision is ticket 04.

- **Tauri 2.12** (2026-09) is stable and first-class for multi-window, IME, text shaping, and editor/terminal components: CodeMirror 6 + codemirror-vim, Monaco + monaco-vim, and xterm.js 6 + portable-pty (ConPTY). Windows (WebView2) is robust.
- **Tauri's Linux risk is WebKitGTK**: NVIDIA + Wayland has documented blank-window, Error 71 and resize-crash issues, fixed with env vars that cost performance (tracker #9394 still open). A PSS/USS measurement found WebKitGTK memory no better than Chromium, and each extra Linux window is likely another WebProcess. The GTK4 port and Tauri 3 (alpha, with an official CEF runtime) are still pending.
- **Odin + raylib 6.0** has the lower expected footprint: one native process, and near-zero idle CPU with `EnableEventWaiting()`. However, it bundles GLFW 3.4, so there is no IME, and raylib's text rendering does no shaping. Raylib builds Wayland-off by default and the Odin prebuilt links X11, so it runs through XWayland. Raylib is also single-window only.
- **Odin ecosystem**: `core:os` gives you subprocess, JSON and net, but there is no PTY, file-watcher or git package, no ConPTY bindings, and no editor component. libghostty-vt (the Ghostling demo uses raylib) is the realistic path to a VT emulator. Odin is pre-1.0 and rewrote `core:os` in 2026.
- **Rough v1 effort**: weeks with Tauri, a few months with Odin + raylib. Most of the Odin time goes to text, editor and UI fundamentals. SDL3 (via `vendor:sdl3`) would fix raylib's Wayland, IME and multi-window gaps, but it is still build-it-yourself.

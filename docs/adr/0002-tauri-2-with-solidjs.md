# Tauri 2 with a SolidJS frontend, not Odin + Raylib

Status: accepted (2026-09-30)

v1 is built on **Tauri 2.x (stable)** with a Rust core and a **SolidJS** TypeScript frontend. The top priority is memory efficiency, and Odin + Raylib (or Odin + SDL3) would have a smaller shell footprint, so this choice needs explaining. With many Agent sessions, total memory is dominated by the per-session `claude` processes (about 215–340 MB idle each) and the shared ACP adapter, which cost the same in either stack; the shell is roughly a tenth of the total. Meanwhile Odin would mean building by hand a rich chat UI, an editor with vim, IME, text shaping and multi-window (months versus weeks), on a pre-1.0 language. Tauri's main Linux risk, WebKitGTK breaking on NVIDIA + Wayland, doesn't apply to the user's AMD machine. SolidJS was picked for fine-grained reactivity and a small runtime, since many live Tabs will stream chat at once.

## Considered Options

- **Odin + Raylib**: no IME or shaping via GLFW, XWayland only by default, single window, and everything else DIY.
- **Odin + SDL3**: fixes the windowing gaps, but the UI is still DIY.
- **Tauri 3 alpha / CEF runtime**: unstable plugin APIs. CEF would remove the difference between WebKitGTK and WebView2 but adds Chromium's footprint. Revisit when Tauri 3 is stable.

## Consequences

- **Two rendering engines**: WebKitGTK on Linux and WebView2 on Windows, so both must be tested.
- **Memory budget, as a development-time benchmark check** (not a runtime limit): the editor's own processes, excluding `claude` and ACP and measured as PSS on both OSes, must stay **≤ 250 MB with 5 Tabs and 1 manual editor window**, **≤ 15 MB per extra Tab**, and **near 0% CPU at idle**. If the first prototype misses it, this decision is reopened; after that, a miss is a regression.
- **Extra windows cost a web process on Linux**: each extra OS window (e.g. a manual editor window) likely costs another WebKit process, which weighs on the window-vs-pane layout question.

# 01: Walking skeleton: one Agent session end to end

**What to build:** Launching the app opens a repo as a Workspace and shows one Tab for an Agent session running in the main checkout. Typing a prompt sends it through the shared ACP adapter and streams the Agent's reply into the Tab as it arrives. This slice creates the Tauri 2 + SolidJS app, the Rust core with its public API, the ACP host (one shared adapter process, JSON-RPC routed per Agent session), the visible-Tab streaming channel, startup checks for git ≥ 2.55 and Node ≥ 22.12, and the test harness: a **fake ACP agent** driven through the core's public API, per the spec's Testing Decisions.

**Blocked by:** None (can start immediately).

**Status:** ready-for-agent

- [ ] One dev command starts the app on Windows 11 and Arch Linux (Wayland). _Windows 11 verified 2026-09-30 (`pnpm dev`); Arch still to run._
- [x] Opening a repo path creates a Workspace with one Agent session bound to the main checkout, shown as a Tab.
- [x] Sending a prompt streams the Agent's reply into the Tab incrementally, batched to about one flush per frame.
- [x] The core spawns exactly one ACP adapter process and routes messages per Agent session. The adapter executable is configurable (the test seam).
- [x] The Tab shows Working while a turn runs and Idle when it ends.
- [x] If git < 2.55 or Node < 22.12 (or either is missing), startup shows a clear message naming what to install.
- [x] Core tests drive the public API against the fake ACP agent: create session → send prompt → receive streamed chunks → state returns to Idle.
- [x] The fake ACP agent can script replies and streamed chunks. It's the template for later tests.

# Memory budget results

The budget is set in [ADR 0002](../adr/0002-tauri-2-with-solidjs.md); the benchmark is `pnpm bench:memory`
(`scripts/bench-memory.ps1`, ticket 05). It counts the editor's own processes only (`agent-editor` plus its
webview processes), not the fake ACP agent, Node or `claude`.

| Date | OS | Build | Scenario | 1 Tab | 5 Tabs | Per extra Tab | Idle CPU | Verdict |
|---|---|---|---|---|---|---|---|---|
| 2026-10-01 | Windows 11 (16 logical cores) | release, ticket 05 | 2,000-message transcript in Tab 1, no virtualisation yet | 80.3 MB | 93.6 MB | 3.3 MB | 0.78% of one core | Pass |
| — | Arch Linux (Wayland) | — | — | — | — | — | — | Pending: needs a Linux run (PSS) |

**Windows metric:** Windows has no PSS, so memory is each process's private working set, the closest Windows
equivalent. The full working set (346 MB with 1 Tab, 363 MB with 5) is much higher because it includes WebView2
runtime pages shared with other processes.

**First Windows run:** process breakdown at 5 Tabs, private MB:
- `agent-editor` 9.5
- WebView2 processes: 29.3, 1.6, 22.0, 6.4, 3.0, 21.8

The largest WebView2 process is the renderer. It grew from 14.2 to 21.8 MB with the 2,000-message transcript.

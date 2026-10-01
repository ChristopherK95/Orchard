# Memory budget results

The budget is set in [ADR 0002](../adr/0002-tauri-2-with-solidjs.md); the benchmark is `pnpm bench:memory`
(`scripts/bench-memory.ps1`, ticket 05). It counts the editor's own processes only (`agent-editor` plus its
webview processes), not the fake ACP agent, Node or `claude`.

| Date | OS | Build | Scenario | 1 Tab | 5 Tabs | Per extra Tab | Idle CPU | Verdict |
|---|---|---|---|---|---|---|---|---|
| 2026-10-01 | Windows 11 (16 logical cores) | release, ticket 05 (first version) | 2,000-message transcript in Tab 1, no virtualisation, **no Manual editor yet** | 80.3 MB | 93.6 MB | 3.3 MB* | 0.78% of one core (10 s) | Pass |
| 2026-10-01 | Windows 11 (16 logical cores) | release, ticket 05 (three phases) | 5 Tabs each after a short turn, then a 2,000-message transcript in Tab 1; no virtualisation, **no Manual editor yet** | 80.7 MB | 89.4 MB (94.6 MB after the transcript) | 2.2 MB | 0.39% of one core (20 s) | Pass |
| 2026-10-01 | Windows 11 (16 logical cores) | release, ticket 02 (markdown, Lezer, virtualised list, CSP) | same as above | 82.4 MB | 89.2 MB (98.7 MB after the transcript) | 1.7 MB | 0.31% of one core (20 s) | Pass |
| — | Arch Linux (Wayland) | — | — | — | — | — | — | Pending: needs a Linux run (PSS) |

\* The first version measured per-Tab cost as (5 Tabs − 1 Tab) / 4, with Tab 1's 2,000-message transcript
streamed in between, so it mixed transcript growth with Tab cost and overstated per-Tab cost. The script now
measures 5 Tabs (each after a short turn) before the long transcript.

The budget names "5 Tabs and 1 open Manual editor"; the Manual editor arrives with ticket 14, which adds it to
the scenario.

**Ticket 02:** virtualising the transcript didn't lower memory. The renderer is about 2 MB higher with the
2,000-message transcript (25.2 vs 23.1 MB). At this size the JavaScript heap (the message objects themselves, plus
markdown-it and the grammars) dominates, not the DOM. Virtualisation mainly keeps scrolling smooth. This run also
confirms the new CSP doesn't break the release build.

**Windows metric:** Windows has no PSS, so memory is each process's private working set, the closest Windows
equivalent. The full working set (346 MB with 1 Tab, 363 MB with 5) is much higher because it includes WebView2
runtime pages shared with other processes.

**First Windows run:** process breakdown at 5 Tabs, private MB:
- `agent-editor` 9.5
- WebView2 processes: 29.3, 1.6, 22.0, 6.4, 3.0, 21.8

The largest WebView2 process is the renderer. It grew from 14.2 to 21.8 MB with the 2,000-message transcript.

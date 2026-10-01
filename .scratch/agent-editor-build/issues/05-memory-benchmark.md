# 05: Memory budget benchmark (first gate)

**What to build:** A scripted benchmark runs the real app with the fake ACP agent: it opens 5 Tabs, streams a long transcript into one, switches Tabs, then idles. It measures the editor's own processes as PSS on Windows 11 and Arch Linux, excluding `claude` and the ACP adapter, and fails if the budget in ADR 0002 is exceeded. If this first run misses, the stack decision is reopened before more is built.

**Blocked by:** 04 (Multiple sessions and Tab states).

**Status:** Windows done 2026-10-01 (93.6 MB / 3.3 MB per Tab / 0.78% idle CPU, all within budget); Arch run pending

- [x] One command runs the benchmark and prints total memory, per-extra-Tab memory and idle CPU.
- [x] It fails above ≤ 250 MB total (5 Tabs), ≤ 15 MB per extra Tab, or non-trivial idle CPU.
- [x] Measurement excludes the fake agent and adapter processes and includes all webview processes.
- [ ] Results are recorded for both OSes. A miss is reported against ADR 0002. _Windows recorded and passing (`docs/benchmarks/memory.md`); Arch pending._

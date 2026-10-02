# 12: Auto-suspend (idle and memory pressure)

**What to build:** A core memory monitor samples all `claude` processes about every 10 s. When their total exceeds the configurable limit (default 4 GB), or the OS reports low memory, the longest-Idle sessions are suspended until back under the limit, with a toast naming them. Optional idle auto-suspend (off by default, 30 min when on) suspends sessions Idle longer than N minutes. Working and Needs you sessions are never auto-suspended.

**Blocked by:** 10 (Suspend, resume and crash recovery), 08 (Settings file).

**Status:** done (core tests on Windows; Arch pending)

- [x] Going over the memory limit suspends the longest-Idle sessions first until under it, with a toast.
- [x] Idle auto-suspend follows the setting and its N minutes.
- [x] Working and Needs you sessions are never auto-suspended.
- [x] Both settings take effect live.
- [x] Core tests with an injectable memory reading and clock: the selection order and exclusions are correct.

Notes: the memory counted is the adapter's direct children (the `claude` processes; resident set on Linux, working set on Windows), not the tools an Agent runs. It's split evenly across the running sessions to decide how many to suspend, then re-checked after a 20 s cooldown. Low OS memory only counts when the Agents use at least a quarter of the memory in use. A limit or idle time of 0 means off.

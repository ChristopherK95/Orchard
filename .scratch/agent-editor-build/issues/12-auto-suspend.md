# 12: Auto-suspend (idle and memory pressure)

**What to build:** A core memory monitor samples all `claude` processes about every 10 s. When their total exceeds the configurable limit (default 4 GB), or the OS reports low memory, the longest-Idle sessions are suspended until back under the limit, with a toast naming them. Optional idle auto-suspend (off by default, 30 min when on) suspends sessions Idle longer than N minutes. Working and Needs you sessions are never auto-suspended.

**Blocked by:** 10 (Suspend, resume and crash recovery), 08 (Settings file).

**Status:** ready-for-agent

- [ ] Going over the memory limit suspends the longest-Idle sessions first until under it, with a toast.
- [ ] Idle auto-suspend follows the setting and its N minutes.
- [ ] Working and Needs you sessions are never auto-suspended.
- [ ] Both settings take effect live.
- [ ] Core tests with an injectable memory reading and clock: the selection order and exclusions are correct.

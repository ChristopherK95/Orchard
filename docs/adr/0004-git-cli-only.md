# Git through the `git` command only, not libgit2 or gitoxide

Status: accepted (2026-09-30)

Every git operation runs the user's `git` (≥ 2.55) as a subprocess with machine-readable output, behind one internal interface. It's the only option that covers every worktree operation (add/list/lock/move/prune/remove/repair) and gets fast status on large repos through git's own caches and fsmonitor. It also authenticates exactly like the user's terminal (credential helpers, SSH agent, signing, hooks, LFS). libgit2 lacks worktree move/repair and fsmonitor-accelerated status, and needs its own credential callbacks. gitoxide can't push or remove worktrees. A subprocess also keeps nothing in memory between calls, which suits the memory budget in ADR 0002.

## Consequences

- Each call pays process-spawn cost, which is higher on Windows.
- If measurements show a hot path (e.g. status refresh) is too slow, a library can replace it behind the same interface. That's why the interface exists.

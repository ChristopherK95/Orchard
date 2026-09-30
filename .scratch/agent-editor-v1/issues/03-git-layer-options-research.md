# Git layer and file-watching options

Type: research
Status: resolved
Blocked by: none
Part of: [map](../map.md)
Research: branch research/git-layer, .scratch/agent-editor-v1/research/git-layer.md

## Question

What are the options for the editor's git layer, file watching, and file search, per candidate stack (Rust for Tauri, Odin for Raylib), on Arch Linux and Windows 11?

- **Git**: shelling out to the `git` CLI (`--porcelain=v2`, `-z`) vs libgit2 (git2-rs, Odin C bindings) vs gitoxide. Compare **worktree** support (add/list/remove/prune, locked worktrees), status/diff performance on large repos, branch / merge-base / diff-against-base (needed later for a branch-diff view), push and auth handling (credential helpers, SSH agent on Windows), and correctness gaps.
- **File watching** across many Worktrees at once: inotify limits on Linux, ReadDirectoryChangesW on Windows, crates like `notify`, Odin options; cost of watching N worktrees; ignoring `.gitignore`d paths.
- **Fuzzy file search**: fast repo file listing (`git ls-files`, the `ignore` crate / ripgrep's walker) and fuzzy matchers (nucleo, fzf-style) per stack.

## Answer

Full findings, sources and the comparison table: [research/git-layer.md](../research/git-layer.md). These are facts only; no option is chosen.

- **git CLI (2.56)** is the only option with the full worktree set (add/list/lock/move/prune/remove/repair) and `list --porcelain -z`. It also has the best large-repo status: untracked cache plus the built-in fsmonitor daemon, which Git 2.55 brought to Linux. Auth works exactly as in the user's terminal (credential helpers/GCM, `~/.ssh/config`, any agent). It keeps no memory between calls, but every call spawns a process. `GIT_OPTIONAL_LOCKS=0` stops background status from contending with agents for the index lock. It is available from Odin via `core:os` process APIs.
- **libgit2 1.9.7 / git2 0.21** covers push, merge-base and tree diffs, and worktree add/list/lock/prune (no move or repair). Status has no untracked cache or fsmonitor, so it always scans the whole worktree. It keeps in-process caches (256 MiB object cache cap by default). git2's vendored SSH is libssh2 only, so the app writes its own credentials callbacks. Odin gets only a small community binding (`odit`) or bindgen.
- **gitoxide (gix 0.88)** is pre-1.0 and Rust-only (nothing usable from Odin). It has status, diff, merge-base and fetch/clone. It is missing **push**, worktree remove/move/repair, prunable info, high-level checkout/switch, hooks, and fsmonitor/untracked-cache acceleration.
- **Watching**: inotify needs one watch per directory, with defaults of 8192 to 1048576 watches and 128 instances per user, shared with other tools. On Windows, ReadDirectoryChangesW uses one recursive handle per root, drops events on overflow and needs a rescan. `notify` (8.2 stable, 9.0 RC) has no ignore filter for recursive watches. Odin has raw inotify and RDCW bindings but no watcher library.
- **Search**: `git ls-files --exclude-standard` works from either stack. For listing, Rust has the `ignore` crate; for matching it has `nucleo` (no release since 2024) and `frizbee` (active, and has C bindings so Odin can use it). Odin has no gitignore or fuzzy library in core.

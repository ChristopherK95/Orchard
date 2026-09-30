# Git layer and file-watching options

Type: research
Status: open
Blocked by: none
Part of: [map](../map.md)

## Question

What are the options for the editor's git layer, file watching, and file search, per candidate stack (Rust for Tauri, Odin for Raylib), on Arch Linux and Windows 11?

- **Git**: shelling out to the `git` CLI (`--porcelain=v2`, `-z`) vs libgit2 (git2-rs, Odin C bindings) vs gitoxide. Compare **worktree** support (add/list/remove/prune, locked worktrees), status/diff performance on large repos, branch / merge-base / diff-against-base (needed later for a branch-diff view), push and auth handling (credential helpers, SSH agent on Windows), and correctness gaps.
- **File watching** across many Worktrees at once: inotify limits on Linux, ReadDirectoryChangesW on Windows, crates like `notify`, Odin options; cost of watching N worktrees; ignoring `.gitignore`d paths.
- **Fuzzy file search**: fast repo file listing (`git ls-files`, the `ignore` crate / ripgrep's walker) and fuzzy matchers (nucleo, fzf-style) per stack.

# Git layer and file watching under Tauri

Type: grilling
Status: open
Blocked by: none
Part of: [map](../map.md)

## Question

Given the git-layer research and a Rust core (ADR 0002), how does v1 do git, watching, and file search? Settle:

- `git` CLI vs git2 vs gix, or a mix: e.g. the CLI for worktree operations, push and status (fsmonitor, auth parity), and a library for hot read paths?
- The minimum git version to require.
- `GIT_OPTIONAL_LOCKS=0` for background refreshes.
- The watching strategy across many Worktrees: `notify` per Worktree, ignore filtering, inotify limit handling, and overflow rescans on Windows.
- The file listing and fuzzy matcher for search (`git ls-files` or the `ignore` crate; `nucleo` vs `frizbee`).

# 35: Several shells per Worktree

**What to build:** A Worktree can have more than one shell. The design's Terminal panel shows `zsh` and `cargo watch` tabs. The Tabs view's Terminal panel header becomes a tab list of the Worktree's shells, with a + to start another. Each tab is named after what it runs: the shell, or the command running in it.

**Blocked by:** none. The redesign (a26221e) already gives the panel header a tab list with one tab.

**Status:** not started

- [ ] The core keeps a list of shells per Worktree instead of one. `open_terminal`, `terminal_input`, `resize_terminal` and `close_terminal` take a shell id, and a view slot shows one shell at a time.
- [ ] + in the panel header starts a new shell in the Worktree's folder and shows it.
- [ ] Each tab is labelled with the shell's name, or with the foreground command if that can be found cheaply. A middle-click or × stops that shell, asking first if something is still running in it.
- [ ] A column's Terminal tab shows the same shells: its cwd strip gets the same + (see 40) and a way to switch between them.
- [ ] The green "shell running" dot (on the header's Terminal button and the column's Terminal tab) means at least one shell runs in the Worktree.
- [ ] Hiding the panel keeps every shell running, as now. Stopping the Worktree's last shell leaves the panel saying "Press Enter for a new one".
- [ ] Core tests for starting, listing and stopping several shells in one Worktree.

Notes:
- Today the core has one shell per Worktree, keyed by its path (`crates/editor-core/src/core.rs`). The frontend tracks which Worktrees have one in `src/shells.ts`.
- The running-shells signal should probably move into the core here (an event when a shell starts or exits). That also fixes the dot being off after a webview reload.

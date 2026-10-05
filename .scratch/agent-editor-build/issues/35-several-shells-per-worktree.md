# 35: Several shells per Worktree

**What to build:** A Worktree can have more than one shell. The design's Terminal panel shows `zsh` and `cargo watch` tabs. The Tabs view's Terminal panel header becomes a tab list of the Worktree's shells, with a + to start another. Each tab is named after what it runs: the shell, or the command running in it.

**Blocked by:** none. The redesign (a26221e) already gives the panel header a tab list with one tab.

**Status:** done (Windows: core-tested and type-checked; not run in the app. Arch pending, including the foreground names)

- [x] The core keeps a list of shells per Worktree instead of one. `open_terminal`, `terminal_input`, `resize_terminal` and `close_terminal` take a shell id, and a view slot shows one shell at a time.
- [x] + in the panel header starts a new shell in the Worktree's folder and shows it.
- [x] Each tab is labelled with the shell's name, or with the foreground command if that can be found cheaply. A middle-click or × stops that shell, asking first if something is still running in it.
- [x] A column's Terminal tab shows the same shells: its cwd strip gets the same + (see 40) and a way to switch between them.
- [x] The green "shell running" dot (on the header's Terminal button and the column's Terminal tab) means at least one shell runs in the Worktree.
- [x] Hiding the panel keeps every shell running, as now. Stopping the Worktree's last shell leaves the panel saying "Press Enter for a new one".
- [x] Core tests for starting, listing and stopping several shells in one Worktree.

Notes:
- Today the core has one shell per Worktree, keyed by its path (`crates/editor-core/src/core.rs`). The frontend tracks which Worktrees have one in `src/shells.ts`.
- The running-shells signal should probably move into the core here (an event when a shell starts or exits). That also fixes the dot being off after a webview reload.

**Notes (done):**
- Core: shells are kept by `TerminalId` (numbered per Workspace), each knowing its Worktree. `Core::terminals()` lists them oldest first as `TerminalInfo { id, worktree, name, busy }`; `CoreEvent::TerminalsChanged` sends the whole list whenever one starts or goes (exits, is stopped, its Worktree is removed, the Workspace closes). `open_terminal(slot, worktree, id, …)` shows `id` if it still runs there, else the Worktree's oldest, else a new one; `new_terminal` always starts one. Removing a Worktree stops all its shells at once. Tests: `tests/terminal.rs` (a new one for several shells, the old ones moved to ids).
- Names: the shell's program name (`pwsh`, `powershell`, `bash`…). On Linux, the pty's foreground process group leader's `/proc/<pid>/comm` when it isn't the shell, which also sets `busy`. There's no event for that changing: the panel asks for the list again shortly after Enter or Ctrl+C. **Windows has no foreground name**, so a shell running `cargo watch` there is still called `pwsh`, and stopping it never asks first.
- Found on the way: a shell that asks for the cursor position (ConPTY does as it starts) while no panel shows it, or just as the panel moves to another shell, hung waiting for the answer, and the replay on coming back doesn't answer. The core now answers it itself in both cases (`1;1`), and replays that output so the panel doesn't answer twice.
- Frontend: `src/shells.ts` holds the core's list (from its event, and its list at start). The panel remembers which shell each view slot last showed per Worktree. Stopping the shell on screen moves on to another of the Worktree's, if it has one. In a column, the shell tabs only appear in the cwd strip once there are two or more; with one, the strip just has the +.
- Not covered: the question before stopping a busy shell is the webview's own `confirm()`, not an app dialog.

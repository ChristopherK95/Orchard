# 28: Columns view

**What to build:** A third view for wide displays: the title bar's toggle becomes **Tabs | Columns | Board**. In Columns, each **pinned** Worktree gets a column side by side, showing its own session Tabs and transcript. One column has focus. Focus decides where the next prompt goes, what Y / N answer, and what the drawer and Manual editor show. Tabs stays the default and the whole product below 1,600 px.

**Blocked by:** 06 (Worktree row), 22 (Board view, for the view toggle). Best after 26, so the focused composer has its queued state.

**Status:** done (Windows; Arch pending)

- [x] Below 1,600 px the Columns segment is disabled, not hidden, and its hover says "Columns needs a window at least 1 600 px wide". A window that shrinks below that while in Columns goes back to Tabs.
- [x] Pinned Worktrees get a column, in Worktree-row order; pins are kept per Workspace across restarts. (Changed 2026-10-03, see notes: the Columns view has no Worktree row.)
- [x] An unfocused column's header shows the branch, its counts and a Needs-you badge; the focused column's header expands into the full context line (branch · path · counts · changed › session · state).
- [x] Only the focused column has the full composer. Every other column shows a 32 px line, "Message ⎇ agent/docs", dimmed, that focuses the column when clicked.
- [x] Y / N answer the focused column only. A pending card in an unfocused column still renders with working Allow / Deny buttons but no key caps, and a line reading "focus this column to answer with Y / N".
- [x] Prose keeps the 820 px measure, centred in each column.
- [x] The Files/Git drawer follows the focused column (its header's branch and colour say which), with a "Pin to this Worktree" control to park it. The Manual editor opens beside the columns, not inside the focused one (see notes).
- [x] Alt+←/→ move focus to the column on the left / right; Alt+↑/↓ switch to the previous / next session in the focused column.
- [x] Each column shows one session at a time; its session tabs switch between that Worktree's sessions.
- [x] Columns are at least 700 px wide; when the pinned ones don't fit, the row scrolls sideways (any number can be pinned).
- [x] The memory benchmark gets a Columns scene (4 columns, each streaming) and stays within ADR 0002's budget.

Notes:
- **The core change:** today only the visible Tab gets streamed transcript updates (ADR 0003, "only the visible Tab gets streamed updates"). Columns needs one streamed Tab per column. `show_session` becomes "these sessions are visible", and the transcript store in the view becomes per column. This is the part that can cost memory, so measure it early.
- **Decided (2026-10-03):** more pinned Worktrees than fit means horizontal scrolling (700 px floor per column), never a refused pin. A column shows one session at a time, never two stacked; you switch with its session tabs.
- New domain words (pinned Worktree, focused column) go into CONTEXT.md once settled.
- Design reference: `pen_design.pen`, "Page — Screens · Columns view" and Notes · "Decisions · the Columns view".

**Notes (done):**
- The core keeps a visible Tab per **view slot** (`show_session_in(slot, id)`, `hide_tab_in(slot)`): `"tabs"` for the Tabs view, `column:<path>` per column. A session is visible (no unread count) while any slot shows it. `show_session` is the Tabs view's slot, so nothing else changed. ADR 0003 has a line saying so.
- Pins are in the app state per Workspace (`pinned`), via `pinned_worktrees` / `set_pinned`; gone Worktrees are dropped on restore and not listed.
- The frontend's transcript state is `createTabView(slot)` (TabView.ts): the Tabs view has one, each column makes its own and hides its slot when it goes. The focused column *is* the active Worktree, so the drawer, Ctrl+P and Y / N follow it with no extra wiring.
- **Deviation:** the Manual editor stays one pane beside the columns, not inside the focused column. It holds the CodeMirror instances and unsaved buffers, and moving it between columns on every focus change would remount it.
- In the Columns view, showing a session that's in an unpinned Worktree (a notification, the palette, the Board) switches to the Tabs view.
- **Changed after review (2026-10-03):** the Columns view hides the Worktree row; the column headers are enough. Columns are added with "+ column" in the title bar (a menu of the Worktrees without one, and "New Worktree…") and closed with the × in each column header (sessions keep running). With none open, the view lists the Worktrees to open as columns. A Worktree created in the Columns view gets a column straight away.
- The composer keeps a draft per session, so typing survives focus moving between columns (and switching Tabs).
- Alt+↑/↓ also switch sessions in the Tabs view.
- Checked in the real app at 1920 px through WebView2's DevTools protocol (no OS input): 3 columns streaming, focus and Y / N per column, drafts, Alt+arrows, drawer follow and pin, Board over Columns and back, and the fallback to Tabs below 1,600 px. Core tests: `tests/columns.rs`. Benchmark: 128.7 MB with 4 columns (docs/benchmarks/memory.md).

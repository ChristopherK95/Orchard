# 28: Columns view

**What to build:** A third view for wide displays: the title bar's toggle becomes **Tabs | Columns | Board**. In Columns, each **pinned** Worktree gets a column side by side, showing its own session Tabs and transcript. One column has focus. Focus decides where the next prompt goes, what Y / N answer, and what the drawer and Manual editor show. Tabs stays the default and the whole product below 1,600 px.

**Blocked by:** 06 (Worktree row), 22 (Board view, for the view toggle). Best after 26, so the focused composer has its queued state.

**Status:** ready-for-agent

- [ ] Below 1,600 px the Columns segment is disabled, not hidden, and its hover says "Columns needs a window at least 1 600 px wide". A window that shrinks below that while in Columns goes back to Tabs.
- [ ] Each Worktree tab carries a pin control; pinned Worktrees get a column, in Worktree-row order. Pins are kept per Workspace across restarts.
- [ ] An unfocused column's header shows the branch, its counts and a Needs-you badge; the focused column's header expands into the full context line (branch · path · counts · changed › session · state).
- [ ] Only the focused column has the full composer. Every other column shows a 32 px line, "Message ⎇ agent/docs", dimmed, that focuses the column when clicked.
- [ ] Y / N answer the focused column only. A pending card in an unfocused column still renders with working Allow / Deny buttons but no key caps, and a line reading "focus this column to answer with Y / N".
- [ ] Prose keeps the 820 px measure, centred in each column.
- [ ] The Files/Git drawer follows the focused column (its header's branch and colour say which), with a "Pin to this Worktree" control to park it. The Manual editor opens inside the focused column.
- [ ] Shortcuts move focus to the column on the left / right, and pin or unpin a Worktree by its position.
- [ ] The memory benchmark gets a Columns scene (4 columns, each streaming) and stays within ADR 0002's budget.

Notes:
- **The core change:** today only the visible Tab gets streamed transcript updates (ADR 0003, "only the visible Tab gets streamed updates"). Columns needs one streamed Tab per column. `show_session` becomes "these sessions are visible", and the transcript store in the view becomes per column. This is the part that can cost memory, so measure it early.
- **Open question, decide before building:** what happens when a fifth Worktree is pinned at 1,920 px. The design assumes columns shrink to a 700 px floor and then the row scrolls sideways; the alternative is refusing the fifth pin with a reason. Also undecided: whether a column can show two sessions stacked (the design builds Tabs inside each column).
- New domain words (pinned Worktree, focused column) go into CONTEXT.md once settled.
- Design reference: `pen_design.pen`, "Page — Screens · Columns view" and Notes · "Decisions · the Columns view".

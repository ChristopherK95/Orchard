# 36: Drag to resize columns

**What to build:** In the Columns view, the gap between two columns is a resize handle (the design's grip). Dragging it moves width from one column to its neighbour. Each column keeps its share of the width.

**Blocked by:** none.

**Status:** done (Windows: core-tested and type-checked; not run in the app. Arch pending)

- [x] A handle between neighbouring columns. Its grip shows on hover and while dragging, as on the Tabs view's terminal handle.
- [x] Dragging changes only the two neighbours' widths, and neither goes below the column minimum (640 px).
- [x] Widths are kept as shares of the row, so resizing the window scales them. Adding or closing a column evens the shares out again.
- [x] Double-clicking a handle evens out its two columns.
- [x] The shares are kept per Workspace across restarts, along with the pins.

Notes:
- Columns are `flex: 1 0 640px` today (`.column` in `src/styles.css`); the shares can become `flex-grow` values.
- When more columns are pinned than fit, the row scrolls sideways. Resizing then only applies to the columns on screen.

**Notes (done):**
- Core: `column_shares()` / `set_column_shares()` (and the commands of the same names), saved in the Workspace's state next to the pins as whole numbers (1000 per column when even; a column without one counts as 1000). Only pinned Worktrees keep a share; an unknown Worktree is refused. Pinning or unpinning a Worktree clears them all (re-pinning one that's pinned changes nothing). Test: `tests/columns.rs`.
- Frontend: each column's `flex-grow` is its share, from a 0 px basis, so widths follow the shares while every column is over 640 px. A drag works out the two columns' new shares from their widths on screen and saves when let go; while dragging, only the screen changes.
- **Deviation:** the handle is the 8 px gap the columns already had (with a 3 px wider grab area each side), not 12 px: a 12 px gap looked loose next to the 8 px padding round the row.

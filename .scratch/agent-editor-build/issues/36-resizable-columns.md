# 36: Drag to resize columns

**What to build:** In the Columns view, the gap between two columns is a resize handle (the design's grip). Dragging it moves width from one column to its neighbour. Each column keeps its share of the width.

**Blocked by:** none.

**Status:** not started

- [ ] A 12 px handle between neighbouring columns. Its grip shows on hover and while dragging, as on the Tabs view's terminal handle.
- [ ] Dragging changes only the two neighbours' widths, and neither goes below the column minimum (640 px).
- [ ] Widths are kept as shares of the row, so resizing the window scales them. Adding or closing a column evens the shares out again.
- [ ] Double-clicking a handle evens out its two columns.
- [ ] The shares are kept per Workspace across restarts, along with the pins.

Notes:
- Columns are `flex: 1 0 640px` today (`.column` in `src/styles.css`); the shares can become `flex-grow` values.
- When more columns are pinned than fit, the row scrolls sideways. Resizing then only applies to the columns on screen.

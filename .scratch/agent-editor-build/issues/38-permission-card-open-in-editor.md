# 38: "Open in editor" on permission cards

**What to build:** A pending permission card for a file edit has an "Open in editor" button in its header, as in the design. It opens the proposed change in the Manual editor, so it can be read in full before answering.

**Blocked by:** none.

**Status:** not started

- [ ] Shown on pending cards that have a `file` and a `diff`. Not on commands or answered cards.
- [ ] Opens the change as a diff tab (the file as it is vs. the proposed result) in the Manual editor pane, beside the chat. Opening it again focuses that tab.
- [ ] It doesn't answer the card or take Y / N away from it: Y / N still answer the card, or the card says how to.
- [ ] Once the card is answered or the turn ends, the diff tab says it's out of date, or closes if it wasn't touched.
- [ ] In the Columns view, it opens for the card's column and focuses that column.

Notes:
- The transcript only has `onOpenSnippet` today. It needs an `onOpenDiff` (or a general open request) passed through `Transcript` to `PermissionCard`.
- The request carries the diff lines, not the whole proposed file, so the diff tab may have to apply the hunks to the file on disk.

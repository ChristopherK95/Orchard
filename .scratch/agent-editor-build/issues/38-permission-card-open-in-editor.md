# 38: "Open in editor" on permission cards

**What to build:** A pending permission card for a file edit has an "Open in editor" button in its header, as in the design. It opens the proposed change in the Manual editor, so it can be read in full before answering.

**Blocked by:** none.

**Status:** done (Windows: type-checked; not run in the app. Arch pending)

- [x] Shown on pending cards that have a `file` and a `diff`. Not on commands or answered cards.
- [x] Opens the change as a diff tab (the file as it is vs. the proposed result) in the Manual editor pane, beside the chat. Opening it again focuses that tab.
- [x] It doesn't answer the card or take Y / N away from it: Y / N still answer the card, or the card says how to.
- [x] Once the card is answered or the turn ends, the diff tab says it's out of date, or closes if it wasn't touched.
- [x] In the Columns view, it opens for the card's column and focuses that column.

Notes:
- The transcript only has `onOpenSnippet` today. It needs an `onOpenDiff` (or a general open request) passed through `Transcript` to `PermissionCard`.
- The request carries the diff lines, not the whole proposed file, so the diff tab may have to apply the hunks to the file on disk.

**Notes (done):**
- Frontend only. `src/proposals.ts` turns the card's diff into the whole file's: each hunk's old lines are found in the file on disk and replaced. The core's hunk numbers count from the snippet the Agent sent, not the file, so a hunk is looked for where its block's first hunk landed, else after the previous hunk. A new block (numbering starts again: one of several edits) is applied to the result of the one before. A lone hunk with no old lines is a whole file (Write). Then `diffWithDisk(file, proposed)`, turned round, gives the diff with the file's own line numbers.
- **Fallback:** if a hunk can't be placed, or the card's diff was cut short at 400 lines, the tab shows the card's diff as the Agent sent it and says so. A file not on disk yet shows the card's diff (it's the whole file).
- The tab is a read-only diff like the Changes ones, keyed by session and tool call, titled "name (proposed)", with "Edit file" opening the file itself. Showing it moves focus off the CodeMirror view, so Y / N keep working.
- "Touched" means scrolled or clicked in, or switched back to from another tab. A proposal settles when its card is answered (card buttons, Y / N, the Board), when a mounted card shows an outcome, or when its session leaves Needs you or closes. That last one covers the turn ending and cards that aren't on screen.

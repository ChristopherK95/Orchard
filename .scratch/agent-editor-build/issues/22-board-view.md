# 22: Board view

**What to build:** A Tabs | Board toggle in the title bar (`Ctrl+B`, with `Esc` returning to Tabs) switches to the **Board**: columns of every Agent session in the Workspace by state (Needs you, Working, Idle, Suspended/Exited), filterable by Worktree. A pending permission can be approved or denied from its card, and clicking a card opens that session's Tab. An "N need you" button in the title bar jumps to the Board.

**Blocked by:** 04 (Multiple sessions and Tab states), 03 (Permission cards), 06 (Worktree row).

**Status:** done

- [x] The toggle and shortcuts switch views without losing Tab state.
- [x] Cards sit in the correct state column and move live as states change.
- [x] The Worktree filter narrows the cards.
- [x] Allow/Deny on a card answers the permission.
- [x] Clicking a card opens its Tab; "N need you" appears only when N > 0.

**Notes (done):**
- The Board is an overlay under the title bar. The Tabs view stays mounted and laid out underneath, so nothing in it is lost, and the transcript keeps measuring. Dialogs and the palette still come on top.
- Anything that opens part of the Tabs view goes back to it: showing a session (a card, a notification, the palette), the Manual editor, the drawers.
- Cards come from the same session store as the Tabs, so they move columns as states change.
- A Needs you card reads its question from the core (`pending_permission`: the oldest open card) and answers it with the Agent's own Allow / Deny options. After answering, the next waiting question shows.
- The Worktree filter is kept between visits. Focus goes back where it was on return to Tabs.

Not covered yet:
- The active session's unread count stays 0 while the Board is open: the core still treats its Tab as the visible one.

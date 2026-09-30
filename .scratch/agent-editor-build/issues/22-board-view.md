# 22: Board view

**What to build:** A Tabs | Board toggle in the title bar (`Ctrl+B`, with `Esc` returning to Tabs) switches to the **Board**: columns of every Agent session in the Workspace by state (Needs you, Working, Idle, Suspended/Exited), filterable by Worktree. A pending permission can be approved or denied from its card, and clicking a card opens that session's Tab. An "N need you" button in the title bar jumps to the Board.

**Blocked by:** 04 (Multiple sessions and Tab states), 03 (Permission cards), 06 (Worktree row).

**Status:** ready-for-agent

- [ ] The toggle and shortcuts switch views without losing Tab state.
- [ ] Cards sit in the correct state column and move live as states change.
- [ ] The Worktree filter narrows the cards.
- [ ] Allow/Deny on a card answers the permission.
- [ ] Clicking a card opens its Tab; "N need you" appears only when N > 0.

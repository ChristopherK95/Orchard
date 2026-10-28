# 48: Worktree switcher

**What to build:** A centred overlay, opened with Ctrl+Space, showing every Worktree as a card in a grid, each with its name in full. Picking a card goes to that Worktree. The sidebar stays for now (ticket 49 removes it). Spec: `.scratch/worktree-switcher/spec.md`.

**Blocked by:** none.

**Status:** to do

- [ ] Ctrl+Space toggles the switcher, except while the Terminal panel has focus (the shell keeps the key) and in the pop-out window. Esc closes it.
- [ ] About 90% of the window. Cards are about 300 px wide, as many columns as fit, and the grid scrolls vertically.
- [ ] Cards follow the sidebar's order (`rowWorktrees()`). Needs-you cards are highlighted, not moved. The active Worktree's card has an accent outline.
- [ ] A card shows the full name wrapped, never cut with an ellipsis, plus the colour glyph, the status badge, the Review and shell badges, the sessions as clickable lines (name and state), and New session and Recent buttons.
- [ ] In the Columns view, a card shows whether it has a column, plus Give it a column / Close column.
- [ ] A removed Worktree's card is dimmed and says "folder removed". Each session line has a ×, and "Close all and forget" closes them without keeping them in Recent sessions. The card goes once no session is left.
- [ ] Typing filters by name. The arrow keys move between cards, starting on the active Worktree, and Enter picks.
- [ ] Picking a card in the Tabs view goes to the Tab last used in that Worktree, or to the empty-Worktree screen. In the Columns view it focuses the Worktree's column, giving it one first. A session line goes to that session. Picking closes the Board or Dashboard.
- [ ] A "+ New Worktree" card ends the grid. "Clean up…" in the header opens the Worktrees overview.
- [ ] `CONTEXT.md`: a **Worktree switcher** entry (_Avoid_: Worktree grid, launcher).

Notes:
- Mostly frontend: the data is what the sidebar already uses (`sessionsIn`, `rowWorktrees`, shells, setups). Find out whether "Close all and forget" needs a core call to drop sessions from Recent sessions, and if so, add one with a test in `crates/editor-core/tests/`.
- The Tabs view needs the Tab last used per Worktree. Check whether the core or `TabView` already keeps it before adding it.

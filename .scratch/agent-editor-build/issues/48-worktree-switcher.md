# 48: Worktree switcher

**What to build:** A centred overlay, opened with Ctrl+Space, showing every Worktree as a card in a grid, each with its name in full. Picking a card goes to that Worktree. The sidebar stays for now (ticket 49 removes it). Spec: `.scratch/worktree-switcher/spec.md`.

**Blocked by:** none.

**Status:** done (Linux: core-tested and type-checked; not yet tried in the app. Windows untested)

- [x] Ctrl+Space toggles the switcher, except while the Terminal panel has focus (the shell keeps the key) and in the pop-out window. Esc closes it.
- [x] About 90% of the window. Cards are about 300 px wide, as many columns as fit, and the grid scrolls vertically.
- [x] Cards follow the sidebar's order (`rowWorktrees()`). Needs-you cards are highlighted, not moved. The active Worktree's card has an accent outline.
- [x] A card shows the full name wrapped, never cut with an ellipsis, plus the colour glyph, the status badge, the Review and shell badges, the sessions as clickable lines (name and state), and New session and Recent buttons.
- [x] In the Columns view, a card shows whether it has a column, plus Give it a column / Close column.
- [x] A removed Worktree's card is dimmed and says "folder removed". Each session line has a ×, and "Close all and forget" closes them without keeping them in Recent sessions. The card goes once no session is left.
- [x] Typing filters by name. The arrow keys move between cards, starting on the active Worktree, and Enter picks.
- [x] Picking a card in the Tabs view goes to the Tab last used in that Worktree, or to the empty-Worktree screen. In the Columns view it focuses the Worktree's column, giving it one first. A session line goes to that session. Picking closes the Board or Dashboard.
- [x] A "+ New Worktree" card ends the grid. "Clean up…" in the header opens the Worktrees overview.
- [x] `CONTEXT.md`: a **Worktree switcher** entry (_Avoid_: Worktree grid, launcher).

Notes:
- Mostly frontend: the data is what the sidebar already uses (`sessionsIn`, `rowWorktrees`, shells, setups). Find out whether "Close all and forget" needs a core call to drop sessions from Recent sessions, and if so, add one with a test in `crates/editor-core/tests/`.
- The Tabs view needs the Tab last used per Worktree. Check whether the core or `TabView` already keeps it before adding it.

**Notes (done):**
- "Close all and forget" needed a core call: closing a Tab always put it in the Recent sessions (a removed Worktree's were only dropped at the next Worktree listing). `Core::close_and_forget` (command `close_and_forget`, `core.closeAndForget`) closes like `close_tab` but doesn't keep it; tested in `tests/worktree_switcher.rs`, across a restart too. The × on a removed card's session line is the usual close.
- The Tab last used per Worktree was already kept (`lastSessionIn` in `App.tsx`); picking a card uses `selectWorktree`, so with no last-used Tab it's the Worktree's first, then the empty-Worktree screen.
- `src/WorktreeSwitcher.tsx`. For now it opens from a grid button beside the sidebar's Worktrees label (ticket 49 moves it to the header). The Recent button opens a list in the card (Recent sessions, then Conversations started elsewhere), so it works in the Columns view too.
- In the Columns view, New session and a session line give the Worktree a column first, so the session shows there rather than switching to the Tabs view.
- The switcher counts as a dialog: while it's open, Ctrl+P / Ctrl+K / Ctrl+Shift+O / Ctrl+, and the other Tab shortcuts don't act. The pop-out window has no switcher (it renders only the Manual editor).
- The arrow keys always move between cards (←/→ too, not the caret in the filter).

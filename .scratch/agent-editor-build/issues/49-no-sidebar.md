# 49: No sidebar

**What to build:** Remove the sidebar. Its parts move to the header and to the Worktree switcher, and the Tabs view gets a session tab strip like the Columns view's. Spec: `.scratch/worktree-switcher/spec.md`.

**Blocked by:** 48 (Worktree switcher).

**Status:** done. Verified with `pnpm typecheck` and `cargo test -p editor-core` (no core change: it is all frontend); not run in the app.

- [x] The Worktrees button sits at the far left of the header, where the sidebar toggle was. It has a grid icon and a count of the sessions that need you in other Worktrees, and opens the Worktree switcher. Its tooltip says "Ctrl+Space".
- [x] The Workspace crumb opens the repository switcher (Ctrl+Shift+O).
- [x] Search (Ctrl+P) and a Settings gear (Ctrl+,) are in the header.
- [x] The Worktree crumb still switches the branch.
- [x] In the Tabs view, a session tab strip above the chat lists the active Worktree's sessions with a state dot, an unread count and a close ×, then New session and Recent. Reuse the Columns view's `column-tabs`.
- [x] `Sidebar`, the rail, the saved folds (`foldsIn`/`setFold`), `railIn` and their CSS are gone.
- [x] `CONTEXT.md`: the Pinned Worktree, Terminal panel and Columns view entries no longer mention the sidebar.

**Notes (done):**
- No core change: the count, the tab strip and the header buttons are frontend only, so there is no new integration test.
- The tab strip is `SessionTabs` in `src/Columns.tsx`, used by each column (with its Terminal tab as children) and by the Tabs view above the context strip. Each tab now shows a `StateDot` for every state (it was a dot for Needs you only), in both views. The shown session's model, last active and cost sit at the strip's end, as in a column; its context is in the composer's meter; the Board's cards still have all of it.
- In the Tabs view, New session and Recent are hidden for a Review Worktree and a removed one, as the sidebar did. The Recent menu now opens right-aligned under its button.
- What only the sidebar had: the shell badge was already on the switcher's cards (clicking it shows the Terminal panel); the Review badge on a card now opens its Code review (`onCodeReview`); the Worktrees overview is "Clean up…" in the switcher; "Give it a column" / "Close column" are on the cards; your open PRs and the PR board are on the Dashboard ("All PRs").
- The header lost its L-shaped join with the sidebar (`margin-left`, `::after`). The Worktrees button's label hides below 1 280 px, like Terminal's.
- Old `orchard.sidebar.folds:*` keys stay in localStorage; nothing reads them.

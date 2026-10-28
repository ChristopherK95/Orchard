# 49: No sidebar

**What to build:** Remove the sidebar. Its parts move to the header and to the Worktree switcher, and the Tabs view gets a session tab strip like the Columns view's. Spec: `.scratch/worktree-switcher/spec.md`.

**Blocked by:** 48 (Worktree switcher).

**Status:** to do

- [ ] The Worktrees button sits at the far left of the header, where the sidebar toggle was. It has a grid icon and a count of the sessions that need you in other Worktrees, and opens the Worktree switcher. Its tooltip says "Ctrl+Space".
- [ ] The Workspace crumb opens the repository switcher (Ctrl+Shift+O).
- [ ] Search (Ctrl+P) and a Settings gear (Ctrl+,) are in the header.
- [ ] The Worktree crumb still switches the branch.
- [ ] In the Tabs view, a session tab strip above the chat lists the active Worktree's sessions with a state dot, an unread count and a close ×, then New session and Recent. Reuse the Columns view's `column-tabs`.
- [ ] `Sidebar`, the rail, the saved folds (`foldsIn`/`setFold`), `railIn` and their CSS are gone.
- [ ] `CONTEXT.md`: the Pinned Worktree, Terminal panel and Columns view entries no longer mention the sidebar.

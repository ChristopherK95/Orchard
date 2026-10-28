# Worktree switcher, no sidebar, sticky Notifications

Decided in a grilling session, 2026-10-28. Tickets: 48, 49, 50 in `.scratch/agent-editor-build/issues/`.

## The problem

The sidebar is 256 px and can't be resized, and it cuts long branch names with an ellipsis
(`BM-1886-unable-to-change-screen-name-on-screen-with-no-name` is a normal name). Its tooltip shows
the path, not the branch, and the folded rail shows no names at all. With 10+ Worktrees, telling
them apart is hard. Widening the sidebar only moves the limit, so the sidebar goes.

## What replaces it

1. **Worktree switcher** (ticket 48): a centred overlay with a grid of Worktree cards that shows
   every name in full. Ctrl+Space, or the Worktrees button in the header.
2. **No sidebar** (ticket 49): what lived only in the sidebar moves to the header, and the Tabs view
   gets the same session tab strip the Columns view's columns already have.
3. **Sticky Notifications** (ticket 50): a stack of in-app cards at the bottom right for the
   sessions out of sight, so a Worktree that needs you isn't missed without the sidebar's badges.

## Worktree switcher

- **Opening:** Ctrl+Space (not while the Terminal panel has focus, since the shell can use the key,
  and not in the pop-out window) or the header's Worktrees button. Esc or picking a card closes it.
- **Size:** about 90% of the window. Cards are about 300 px wide, as many columns as fit, and the
  grid scrolls vertically.
- **Order:** stable, the order the sidebar uses today (also the Columns view's order). Cards that
  need you are highlighted, not moved. The active Worktree's card has an accent outline.
- **A card:** the full name, wrapped (never cut), the Worktree's colour glyph, its status badge
  (Needs you / Working / Idle / exited / setting up), its Review and shell badges, its sessions as
  clickable lines (name and state), and New session and Recent buttons. In the Columns view, a
  "has a column" marker and Give it a column / Close column.
- **Removed Worktrees** (folder gone, sessions still running): a dimmed card saying "folder
  removed", a × on each session line, and "Close all and forget" (it closes the sessions without
  keeping them in Recent sessions). The card goes once its last session does.
- **Keyboard:** typing filters by name, the arrow keys move between cards (the selection starts on
  the active Worktree), Enter picks.
- **Picking a card:** in the Tabs view, the Tab last used in that Worktree, or the empty-Worktree
  screen if it has none. In the Columns view, focus its column, giving it one first if needed.
  A session line goes straight to that session. Picking closes the Board or Dashboard.
- **At the end of the grid:** a "+ New Worktree" card. **In its header:** "Clean up…", which opens
  the existing Worktrees overview (that stays the place to remove Worktrees).

## No sidebar

- **Header, far left** (where the sidebar toggle was): the Worktrees button, with a grid icon, a count
  of the sessions that need you in other Worktrees, and "Ctrl+Space" in its tooltip.
- The **Workspace crumb** opens the repository switcher (Ctrl+Shift+O).
- **Search** (Ctrl+P) and a **Settings** gear (Ctrl+,) go in the header.
- The **Worktree crumb** still switches the Worktree's branch.
- **Tabs view:** a session tab strip above the chat for the active Worktree's sessions, like the
  Columns view's `column-tabs` (state dot, unread count, close ×, New session, Recent).
- The rail, the sidebar's saved folds and `railIn` go.

## Sticky Notifications

- **Events:** needs you; finished (only if the "turn finished" notification setting is on); exited
  unexpectedly; Worktree setup failed.
- **When:** only for sessions that aren't on screen (the same rule as OS notifications), even while
  the window has focus. OS notifications stay as they are.
- **One per session**, a newer event replacing the older one. **At most 10**, the oldest dropping off.
- **Layout:** bottom right, about 340 px wide, above the Terminal panel and the composer (never
  covering the send button). Newest at the bottom, nearest the corner. A new one slides in from
  the right while the others move up. A closed one fades and the rest close the gap. Animations
  respect `prefers-reduced-motion`.
- **A card:** the Worktree's glyph and full name, the session name, the event, the time, Close.
- **What clears one:** Close; clicking it (which also goes to the session); opening that session
  any other way (Ctrl+K, Alt+↑/↓, the switcher); opening the Worktree switcher clears them all.
  Never by time.
- **Where it lives:** the frontend, in memory only (not kept across restarts). It's short-lived UI
  state built from session-state events the frontend already gets.

## Vocabulary

New `CONTEXT.md` entries: **Worktree switcher** (_Avoid_: Worktree grid, launcher) and
**Notification** (the sticky in-app card; the OS notification mirrors it). Update the entries that
mention the sidebar: Pinned Worktree, Terminal panel, Columns view. "Worktrees overview" stays the
clean-up modal.

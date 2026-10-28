# 50: Sticky Notifications

**What to build:** In-app Notifications stacked at the bottom right for sessions out of sight. They stay until closed, until their session is opened, or until the Worktree switcher is opened. Spec: `.scratch/worktree-switcher/spec.md`.

**Blocked by:** 48 (Worktree switcher, which clears them). Best after 49.

**Status:** done (Linux: the rules in `src/notifications.ts` checked with a throwaway `node --test` script, type-checked and built; not yet tried in the app. Windows untested)

- [x] A Notification is raised when a session that isn't on screen needs you, finishes (only if the "turn finished" setting is on), exits unexpectedly, or its Worktree setup fails. This happens even while the window has focus. OS notifications are unchanged.
- [x] One per session: a newer event replaces the older one. At most 10, with the oldest dropping off.
- [x] Bottom right, about 340 px wide, above the Terminal panel and the composer, so the send button is never covered. The newest is at the bottom.
- [x] A new one slides in from the right while the others move up. A closed one fades and the rest close the gap. With `prefers-reduced-motion`, nothing moves.
- [x] A Notification shows the Worktree's glyph and full name, the session name, the event, the time, and Close.
- [x] One is cleared by Close, by clicking it (which goes to the session), and by opening its session any other way (Ctrl+K, Alt+↑/↓, the Worktree switcher). Opening the Worktree switcher clears them all. Nothing clears them by time.
- [x] Kept only in memory, so a restart starts with none.
- [x] `CONTEXT.md`: a **Notification** entry (the sticky in-app card; the OS notification mirrors it).

Notes:
- Frontend only: built from `onStateChanged` in `App.tsx`, which already decides when an OS notification is sent. Raise both from the same check.

**Notes (done):**
- The rules are a small pure module, `src/notifications.ts`: `eventOf` (which state change raises what), `raise` (one per key, newest last, at most 10) and `clear`. The repo has no frontend test runner, so they were checked with a `node --test` script outside the repo (Node strips the types); no test dependency was added.
- `src/Notifications.tsx` draws the stack; `App.tsx` keeps the list and raises from the same check as the OS notification (`onStateChanged`), minus the window-focus part. A setup failure (`failed` or `sessionFailed`) is keyed by its Worktree (`setup:<path>`), since no session exists yet; clicking it goes to that Worktree.
- "Exited unexpectedly" is any change to `exited`: closing a Tab is a `sessionClosed`, not an exit. A session that closes also drops its Notification.
- Clearing on show is one effect: any Notification whose session (or setup screen) is on screen, with the Board and Dashboard closed, goes. So closing the Board over a session also clears its card.
- The stack's bottom is measured from the `.composer-box`es under its 340 px strip (on resize, input, clicks, key-ups), so the send button stays clear; it sits over the Terminal panel and the Board (z-index 40), under dialogs.
- A closed card fades in place, then its row collapses (CSS grid-row animation); a card replaced by a newer event just goes, and the new one slides in at the bottom.

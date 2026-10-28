# 50: Sticky Notifications

**What to build:** In-app Notifications stacked at the bottom right for sessions out of sight. They stay until closed, until their session is opened, or until the Worktree switcher is opened. Spec: `.scratch/worktree-switcher/spec.md`.

**Blocked by:** 48 (Worktree switcher, which clears them). Best after 49.

**Status:** to do

- [ ] A Notification is raised when a session that isn't on screen needs you, finishes (only if the "turn finished" setting is on), exits unexpectedly, or its Worktree setup fails. This happens even while the window has focus. OS notifications are unchanged.
- [ ] One per session: a newer event replaces the older one. At most 10, with the oldest dropping off.
- [ ] Bottom right, about 340 px wide, above the Terminal panel and the composer, so the send button is never covered. The newest is at the bottom.
- [ ] A new one slides in from the right while the others move up. A closed one fades and the rest close the gap. With `prefers-reduced-motion`, nothing moves.
- [ ] A Notification shows the Worktree's glyph and full name, the session name, the event, the time, and Close.
- [ ] One is cleared by Close, by clicking it (which goes to the session), and by opening its session any other way (Ctrl+K, Alt+↑/↓, the Worktree switcher). Opening the Worktree switcher clears them all. Nothing clears them by time.
- [ ] Kept only in memory, so a restart starts with none.
- [ ] `CONTEXT.md`: a **Notification** entry (the sticky in-app card; the OS notification mirrors it).

Notes:
- Frontend only: built from `onStateChanged` in `App.tsx`, which already decides when an OS notification is sent. Raise both from the same check.

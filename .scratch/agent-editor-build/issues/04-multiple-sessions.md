# 04: Multiple sessions per Worktree and Tab states

**What to build:** A "＋ session" button starts additional Agent sessions in the same Worktree, shown in a session tab row with state dots (Working, Needs you, Idle, Exited). Only the visible Tab streams its transcript; background Tabs get state, unread and Needs-you events. Switching Tabs fetches that transcript. A session that becomes Needs you in the background raises an OS notification. Two sessions sharing a Worktree show a warning.

**Blocked by:** 01 (Walking skeleton), 03 (Permission cards).

**Status:** ready-for-agent

- [ ] Several Agent sessions can run at once in one Worktree, each in its own Tab.
- [ ] Each Tab shows its current state. Exited shows when a session's Agent process ends unexpectedly.
- [ ] Background Tabs receive no transcript content, only state, unread count and Needs-you events.
- [ ] Switching Tabs fetches the transcript in pages for the virtualised view.
- [ ] A session you're not looking at (another Tab is visible, or the window isn't focused) raises an OS notification when it enters Needs you (spec story 25).
- [ ] An optional notification fires when a background session finishes a turn (built, off until ticket 08's settings can switch it on).
- [ ] Opening a second session in the same Worktree shows the shared-Worktree warning.
- [ ] Core tests: two fake sessions; only the subscribed one streams; state events arrive for both.

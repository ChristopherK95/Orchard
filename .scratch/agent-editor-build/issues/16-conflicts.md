# 16: Conflicts when an Agent changes an open file

**What to build:** When a file open in a Manual editor changes on disk, a buffer with no unsaved changes reloads silently. A buffer with unsaved changes shows an "Agent changed this file" banner with Show diff / Reload (drop mine) / Keep mine, and never auto-reloads. Show diff opens a read-only diff (unified, with a side-by-side toggle). An Edit permission card for a file with unsaved changes warns about it.

**Blocked by:** 14 (Manual editor pane), 03 (Permission cards).

**Status:** ready-for-agent

- [ ] Clean buffers reload silently when the file changes on disk.
- [ ] Dirty buffers show the banner; each of the three actions behaves as named.
- [ ] The diff view renders unified by default and can toggle to side-by-side.
- [ ] Edit permission cards warn about unsaved changes in the target file.
- [ ] Core tests: the fake agent edits an open clean file vs an open dirty file, and the correct event is emitted in each case.

Note from ticket 14: open Manual editor documents (tabs, dirty state, versions) live only in the frontend so far. The spec's core document tracker belongs here: the core should learn which files are open and dirty (so it can choose silent reload vs banner, and tabs survive a window reload, per ADR 0003).

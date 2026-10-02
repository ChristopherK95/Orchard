# 16: Conflicts when an Agent changes an open file

**What to build:** When a file open in a Manual editor changes on disk, a buffer with no unsaved changes reloads silently. A buffer with unsaved changes shows an "Agent changed this file" banner with Show diff / Reload (drop mine) / Keep mine, and never auto-reloads. Show diff opens a read-only diff (unified, with a side-by-side toggle). An Edit permission card for a file with unsaved changes warns about it.

**Blocked by:** 14 (Manual editor pane), 03 (Permission cards).

**Status:** done

- [x] Clean buffers reload silently when the file changes on disk.
- [x] Dirty buffers show the banner; each of the three actions behaves as named.
- [x] The diff view renders unified by default and can toggle to side-by-side.
- [x] Edit permission cards warn about unsaved changes in the target file.
- [x] Core tests: the fake agent edits an open clean file vs an open dirty file, and the correct event is emitted in each case.

Note from ticket 14: open Manual editor documents (tabs, dirty state, versions) live only in the frontend so far. The spec's core document tracker belongs here: the core should learn which files are open and dirty (so it can choose silent reload vs banner, and tabs survive a window reload, per ADR 0003).

**Notes (done):** The core decides (ADR 0003). The file watcher reports the paths it saw written to (`ActorNews::Touched`). The core compares each open file's disk version with its editor's and sends `DocumentChangedOnDisk` (clean: reload), `DocumentConflicted` (dirty or deleted: banner) or `DocumentBackOnDisk` (undone: banner goes). It tells each window once per disk version.
- A check that raced the editor's own save, or an open, is done again. Out-of-order reads are dropped.
- Opening a file (or a pop-out collecting one) checks it at once and has its Worktree watched.
- Permission cards for edits carry the absolute `file`, and the frontend keeps the open-files list (`DocumentsChanged`) to warn.
- The diff (`diff_with_disk`, unified, with a side-by-side toggle) is a small custom view, not `@codemirror/merge`, to keep the pane light.

Not covered yet:
- The settings file (outside every Worktree) isn't checked.
- On Linux, an open file in a folder with no indexed files (an ignored folder) gets no events outside polling. Arch pending.
- Polling hashes every open file on each tick.
- The banner and "Keep mine" live in the tab, so a window reload forgets them; the core tells it again on the next change.
- Main-window tabs don't survive a reload yet (the ticket 14 note).

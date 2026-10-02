# 17: Edit notes

**What to build:** The core tracks, per Agent session, every file the session has read or edited (from ACP tool-call events). When the user saves a manual change, each live session on that Worktree whose set contains the file gets an **Edit note**: one unified diff per file against the last delivered version, combining several saves, capped at ~200 lines. It shows as a removable chip ("📝 You edited `status.rs` (+5 −1)") above the composer and is sent with the next prompt. Suspended sessions queue their notes until they resume.

**Blocked by:** 14 (Manual editor pane), 10 (Suspend, resume and crash recovery).

**Status:** done

- [x] Only sessions on the same Worktree that read or edited the file receive a note.
- [x] Several saves combine into one diff per file; beyond ~200 lines the note says the file changed substantially.
- [x] The chip is visible, expandable and removable; sending attaches the kept chips to the prompt.
- [x] Idle sessions aren't woken; Suspended sessions keep their notes until they resume.
- [x] Core tests: the fake agent reads a file → the user saves → the next prompt carries the diff; a session that never read the file gets nothing.

**Notes (done):** `edit_notes.rs` keeps each session's pending notes. Each note holds the text the Agent last saw and the text now; several saves make one diff.
- A session's files come from its read/edit/delete/move tool calls and diffs (`permissions::files_of`). They're persisted with the Tab, so a restored Tab still gets notes.
- The notes go as a text block ahead of the prompt (`[Edit notes: …]` … `[End of edit notes]`). They're shown as tags under the user's message, live and when a conversation is replayed.
- A note goes away when the session reads or edits the file itself.
- If someone else changed the file between the user's saves, the note asks the Agent to re-read the file instead of showing a diff.
- A turn that never reached the Agent puts its notes back.

Not covered yet:
- Pending notes don't survive a restart (only the files do).
- The chips' look needs the running app to check.

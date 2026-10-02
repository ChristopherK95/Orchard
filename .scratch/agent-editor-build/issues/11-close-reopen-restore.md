# 11: Close/reopen sessions and restore after restart

**What to build:** App state is persisted as JSON in the app data folder. Closing a Tab moves its session to the Worktree's **Recent sessions** list without deleting the conversation, and `Ctrl+Shift+T` reopens the last closed session. After an editor restart, every previously open Tab is restored as Suspended, in the same Worktree and order.

**Blocked by:** 10 (Suspend, resume and crash recovery), 06 (Worktree row).

**Status:** done (core tests on Windows; no manual check, per the user; Arch pending)

- [x] Closing a Tab stops its process and lists the session under Recent sessions for that Worktree.
- [x] Reopening from Recent sessions or `Ctrl+Shift+T` resumes the conversation.
- [x] Restarting the app restores all open Tabs as Suspended, with names, order, Worktree and permission mode.
- [x] Closing a Worktree's last Tab only dims the Worktree.
- [x] Core tests: the persisted state round-trips; restore yields Suspended sessions.

Notes: no conversation is stored (spec, Persistence): a restored Tab loads its conversation through ACP `session/load` the first time it's shown, then the Agent is closed again so it stays Suspended. The Tab shown at startup does this straight away, so a restart briefly starts the adapter (worth checking in the memory benchmark against the real adapter). The last shown Tab is restored; the last Tab per Worktree and window/pane layout aren't persisted yet (panes arrive with tickets 14/15). Two editors open on the same Workspace at once share the file: the last to save wins.

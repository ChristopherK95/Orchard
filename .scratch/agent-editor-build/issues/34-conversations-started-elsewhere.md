# 34: Open conversations started elsewhere (e.g. in a terminal)

**What to build:** A Worktree's Claude Code conversations that the editor didn't start (a `claude` run in a terminal in that folder, say) can be opened in a Tab, with their conversation, like a Recent session. Terminal `/resume` isn't offered over ACP (the adapter drops the CLI's terminal-only commands), so this is the editor's way in.

**Blocked by:** 11 (Close/reopen sessions and restore after restart).

**Status:** done (Arch: core-tested, type-checked, checked in the app by the user; Windows pending)

- [x] The Recent menu is always there (not only with Recent sessions) and has a "Started elsewhere" part, looked up when the menu opens: newest first, with the Agent's title and how long ago it changed.
- [x] An empty Worktree offers "Conversations started elsewhere…", which looks them up only when clicked (listing starts the adapter).
- [x] Only the Worktree's own: Claude Code lists the repository's other worktrees' conversations too, so they're filtered by `cwd`. Ones that are Tabs or Recent sessions aren't listed.
- [x] Opening one loads it (`session/load` replays it) in a new Tab named after its title (first line, 40 characters), in Ask for edits; without a title it gets the next "Session N". One that's a Tab already just shows that Tab; one that's a Recent session is reopened as that.
- [x] One whose transcript can't be loaded opens Suspended, saying why, as a Recent session does.
- [x] Core tests with the fake agent (which now answers `session/list`).

**Notes (done):**
- Core: `Core::other_conversations()`, `Core::open_conversation()`, `OtherConversation`; the `other_conversations` and `open_conversation` commands. It's ACP's `session/list` (with the Worktree as `cwd`), so it stays behind the Agent boundary rather than reading `~/.claude/projects`.
- The adapter lists headless (SDK) conversations too, so the editor's own old sessions that fell off the Recent sessions (20 per Worktree) show up here.

Not covered yet:
- A conversation still running in a terminal isn't detected: opening it too means two processes writing one conversation. The tooltip says to exit it in the terminal first; "just now" next to it is the only other hint.
- The list isn't refreshed while the menu stays open.

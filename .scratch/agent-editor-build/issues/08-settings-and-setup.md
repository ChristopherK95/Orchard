# 08: Settings file and Worktree setup

**What to build:** A hand-edited TOML settings file in the app config folder, reloaded live on save, and opened in the editor by "Open repo settings". Per-repo sections are keyed by `origin` URL, falling back to the path. Each can hold a **Worktree setup** command list with optional `linux`/`windows` overrides. After a Worktree is created, setup runs in order in `bash` (Linux) or `pwsh` (Windows, or Git Bash if configured), streaming output to the Tab. The first session waits; a failure stops the list and offers Retry / Start anyway.

**Blocked by:** 07 (Create a Worktree + session).

**Status:** ready-for-agent

- [ ] Settings load at startup and changes apply without a restart. Invalid TOML shows an error and keeps the last good settings.
- [ ] A notifications setting switches on the optional "background turn finished" notification (ticket 04 built it, off by default).
- [ ] The repo section is matched by `origin` URL, falling back to the main checkout path.
- [ ] Setup commands run in order in the new Worktree and stop at the first failure.
- [ ] Output streams into the Tab; Retry reruns from the failed command; Start anyway starts the session.
- [ ] The per-OS override replaces the shared list on that OS.
- [ ] Core tests: a succeeding and a failing setup list against a temp repo on both OSes.

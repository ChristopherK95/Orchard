# 41: Actions

**What to build:** Named commands the user runs on demand in a Worktree, e.g. a repo whose dev setup needs `cd server; pnpm start` and `cd client; pnpm start` in two terminals, to watch both for errors and restart either. Each repo's settings hold its Actions; each command of one runs in a new shell of the Terminal panel.

**Blocked by:** none (35: several shells per Worktree).

**Status:** done (Linux: core-tested and type-checked; not yet tried in the app. Windows untested)

- [x] Settings: `actions = [{ name = "Dev", run = ["cd server && pnpm start", "cd client && pnpm start"] }]` in the repo's section.
- [x] Ctrl+P lists "Run <name>" (or "Restart <name>" while it runs in this Worktree). Picking one opens the Terminal panel on its first shell.
- [x] Each command gets a shell of its own, its tab named after the Action (or after the command when there are several), with a play icon.
- [x] Running it again stops the Action's shells in that Worktree and starts them anew; other shells are left alone.
- [x] The panel's cwd strip has a restart button on an Action's shell: just that one is restarted.
- [x] The settings page's Repository section edits the list.
- [x] Core tests: running, rerunning, restarting one shell, the settings change.

Notes:
- The command is typed into the repo's interactive shell once it first prints (not run with `-c`), so the user's shell setup (nvm, aliases) applies, Ctrl+C leaves the shell and Up runs it again.
- No per-OS overrides (`run_windows`/`run_linux`) yet, unlike setup.

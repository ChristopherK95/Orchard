# 19: Push, fetch and pull

**What to build:** Push, fetch and fast-forward-only pull from the Git drawer. Fetch also happens automatically when the window regains focus (at most every 5 minutes, shown Worktrees only). A pull that can't fast-forward says the branch has diverged and suggests an Agent or a terminal. Auth uses the user's existing credential helpers and SSH setup; if git needs input, the operation fails with the error and "run `git push` in a terminal once".

**Blocked by:** 18 (Git drawer basics).

**Status:** done

- [x] Push, fetch and pull work against a remote.
- [x] Pull never merges or rebases; divergence produces the explanatory message.
- [x] Focus-triggered fetch runs at most every 5 minutes, for shown Worktrees only.
- [x] An operation that needs credentials fails fast (never hangs) with the terminal hint.
- [x] Core tests with a temp bare origin: push, fetch, fast-forward pull and diverged pull.

**Notes (done):** Every remote command goes through `git::run_remote`:
- no prompts (`core.askPass` cleared, `GIT_ASKPASS`/`SSH_ASKPASS` removed, `GCM_INTERACTIVE=never`);
- SSH in batch mode, unless the user has their own SSH command;
- a 5-minute timeout that stops git's whole process tree;
- the terminal hint on login failures.

The 401 test passes in well under its 20 s bound. Behaviour:
- Pull fetches, then fast-forwards (`--no-autostash`). It reports `Diverged` instead of merging, and asks first while a session there is mid-turn.
- Push always goes to the branch of the same name, and reports `Rejected` when the remote has commits the branch hasn't.
- One remote operation at a time per Workspace (the Worktrees share the remote refs).
- On window focus, the core fetches once for all followed Worktrees (the one looked at, and those with sessions), at most every 5 minutes. A manual fetch counts.

Behaviour change to ticket 07: new Agent branches are created `--no-track`. Before, they tracked the start point (`origin/main`), so Push failed (`push.default=simple`) or went to `main`. They now track their own name once pushed.

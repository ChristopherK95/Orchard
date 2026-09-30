# 19: Push, fetch and pull

**What to build:** Push, fetch and fast-forward-only pull from the Git drawer. Fetch also happens automatically when the window regains focus (at most every 5 minutes, shown Worktrees only). A pull that can't fast-forward says the branch has diverged and suggests an Agent or a terminal. Auth uses the user's existing credential helpers and SSH setup; if git needs input, the operation fails with the error and "run `git push` in a terminal once".

**Blocked by:** 18 (Git drawer basics).

**Status:** ready-for-agent

- [ ] Push, fetch and pull work against a remote.
- [ ] Pull never merges or rebases; divergence produces the explanatory message.
- [ ] Focus-triggered fetch runs at most every 5 minutes, for shown Worktrees only.
- [ ] An operation that needs credentials fails fast (never hangs) with the terminal hint.
- [ ] Core tests with a temp bare origin: push, fetch, fast-forward pull and diverged pull.

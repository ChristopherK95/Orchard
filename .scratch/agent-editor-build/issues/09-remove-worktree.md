# 09: Remove a Worktree safely

**What to build:** Removing a Worktree first stops its running sessions after confirmation. It lists uncommitted changes and unpushed/unmerged commits and requires an explicit "Discard and remove". It offers "Delete branch too", pre-ticked only if the branch is merged. The main checkout can't be removed.

**Blocked by:** 06 (Worktree row).

**Status:** ready-for-agent

- [ ] Removing a clean, pushed Worktree needs only a single confirmation.
- [ ] Dirty or unpushed/unmerged state is listed; removal requires "Discard and remove" and never uses a silent `--force`.
- [ ] "Delete branch too" is pre-ticked only when the branch is merged into its base.
- [ ] The main checkout offers no remove action.
- [ ] Running sessions are stopped before removal.
- [ ] A running Worktree setup is stopped with its whole process tree (a Job object on Windows, a process group on Linux). Today, aborting a setup kills only the shell, so `node`/`pnpm` under it keep running, and the folder stays locked on Windows. (Deferred from ticket 08's review.)
- [ ] Core tests with real `git`: clean, dirty, unpushed and merged cases.

# 42: Review and create PR

**What to build:** A "Review" button (header, and Ctrl+P "Review changes and create PR") for when an Agent's work looks done. It opens an overlay with every file the Worktree's PR would change, gone through one at a time like a GitHub review. Next leads to the PR step: the branch to merge into, title, description, and Create PR, which commits what isn't committed yet, pushes, and opens the PR with the user as assignee.

**Blocked by:** none (21: changes vs base, 19: push).

**Status:** done (Linux: core-tested and type-checked; not yet tried in the app. Windows untested)

- [x] The review lists what changed from the split with the Base to the files on disk: committed, uncommitted and untracked files, renames found. Files with uncommitted changes are marked.
- [x] One file's diff at a time (unified or split), a file list with Viewed ticks and progress, J / K and Next file (which marks the file viewed), V toggles viewed. A file's viewed tick drops when its diff changes.
- [x] The PR step: target branch (the remote's branches offered; the Base's by default), title (the only commit's subject, else from the branch name), description (the commits, when several), commit message for what's uncommitted, draft.
- [x] Create PR: asks first while a session there is mid-turn; commits everything left (`git add -A`), pushes as the drawer's Push does, then `gh pr create --assignee @me`. A branch with a PR already open says so and links it; a rejected push says to pull first.
- [x] Clicking a commit in the list narrows the files and diffs to what that commit changed on its own (against its first parent); clicking it again, or "All changes", goes back. Viewed ticks belong to the whole review, so they're hidden while a commit is shown, and J doesn't tick files there.
- [x] A failing hook says what it found: git puts a `pre-push` hook's own output (a linter's report) on its stdout, so pushes and commits now report stdout and stderr both (the drawer's Push and Commit too).
- [x] A failed Create PR can be sent to one of the Worktree's sessions (or a new one): the error goes with a request to fix it and commit, queued if the session is mid-turn, and the session is shown. Review again to retry.
- [x] Create PR shows a live log: each step (commit, push, open the PR) with what it printed as it came (hooks, git's push progress, `gh`); the failed step is marked. It stays under "Output" once the PR is open.
- [x] A branch whose PR is already open (`gh pr list --head`) gets Push instead of the PR step: the header links the PR, and Push goes to a step of its own that asks for a commit message if anything's uncommitted, then commits and pushes with the live log. Failures can be sent to a session as with Create PR. Without `gh` (or off GitHub) it's the PR step as before.
- [x] Core tests with real `git` and a fake `gh`.

Notes:
- `gh` runs with the login shell's environment on Unix (as Worktree setup does), so a desktop-started Orchard finds it and any `GH_TOKEN` the profile sets. `CoreConfig::gh` points tests at a fake one.
- Viewed ticks are kept per Worktree while the window is open, not across restarts.

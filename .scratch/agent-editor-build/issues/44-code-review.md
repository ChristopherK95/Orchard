# 44: Code review

**What to build:** A Code review of someone else's pull request inside Orchard: its files gone through one at a time like the Review (42), Viewed ticks shared with GitHub, comments on lines held as a pending review, and a verdict submitted.

**Blocked by:** 43 (Dashboard's review requests open it).

**Status:** not started

- [ ] Opened from a review request on the Dashboard, and from "Code review" in the PR overlay of any open PR the user didn't write.
- [ ] The PR's head is fetched (`refs/pull/N/head`, so forks work too) and diffed locally against its merge base with the PR's base branch; no Worktree. "Check out in a Worktree" makes one when the code is to be run: the Worktree on that branch if there is one, else a new one on a local branch named after the PR's (`pr-<N>` if that name is taken by another branch).
- [ ] The Review's layout: a file list with Viewed ticks and progress, one file's diff at a time (`DiffView`), Next / Previous, J / K, V.
- [ ] Viewed ticks are GitHub's (`markFileAsViewed` / `unmarkFileAsViewed`), so they match the web.
- [ ] Comments on a line or a range go into a pending review that survives closing the overlay and restarts. Submit asks for Approve, Request changes or Comment and a summary, and sends it in one `gh api` call.
- [ ] Existing threads show at their lines and can be replied to. (Resolving: later.)
- [ ] New commits pushed mid-review: a bar "N new commits — Refresh". Refreshing keeps the pending comments; one whose line is gone stays on the file as outdated. Files that changed lose their Viewed tick.
- [ ] A re-requested review opens on the changes since the user's last review, with a toggle to the full diff.
- [ ] Core tests with real `git` and a fake `gh`.

Notes:
- No Agent in v1. First follow-up: "Ask the Agent about this" on a hunk or selection.

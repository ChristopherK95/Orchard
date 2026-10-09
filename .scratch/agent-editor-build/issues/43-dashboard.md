# 43: Dashboard

**What to build:** A Dashboard: the live status wall of the user's own work in the open Workspace, and the view a Workspace opens on. Fixed sections, each item leading to where it lives; a PR with failing checks or review feedback can start an Agent session on it.

**Blocked by:** none (22: Board, 33: Worktrees overview, the PR board).

**Status:** not started

- [ ] A fourth view in the title-bar switcher, first: Dashboard | Tabs | Columns | Board. It takes the whole content area. Ctrl+D toggles it with the view that was showing before.
- [ ] It's shown every time a Workspace opens in the window: at startup once picked, on a switch, and with saved Tabs restored. `dashboard.openOnStart` (default `true`) in the settings turns that off.
- [ ] Only the open Workspace; only the user's own work, plus reviews requested from them directly (`user-review-requested:@me`, not through a team).
- [ ] Sections, in this fixed order; an empty one collapses to a single line ("No reviews requested"):
  1. **Needs you:** Agent sessions in Needs you.
  2. **Reviews requested:** open, non-draft PRs asking the user for review: title, author, how long ago they were asked, checks state, "re-requested" when they've reviewed it before.
  3. **Your PRs:** every open one, grouped by stage: Changes requested, Checks failing, Approved (ready to merge), Waiting for review, Draft. No merged or closed ones. "All PRs →" opens the PR board.
  4. **Worktrees that need attention:** any with uncommitted changes, unpushed commits, behind its upstream, no upstream but commits, or a rebase / merge in progress (the main checkout too). Every reason on the row ("3 uncommitted · 2 unpushed").
  5. **Ready to clean up:** Worktrees merged into their Base or with the remote branch deleted; opens the Worktrees overview.
  6. **Sessions:** counts of Working and Idle.
- [ ] Clicks: a session opens its Tab; a PR or review request opens the PR overlay (which links to GitHub); a Worktree switches to it with the Git drawer open; a clean-up row opens the Worktrees overview. Leaving the Dashboard lands in the last-used view (Tabs or Columns; Tabs the first time).
- [ ] "Start an Agent session" on a PR of the user's with failing checks or review feedback: in the Worktree on that branch, else a new one on it (Worktree setup runs). The composer is pre-filled, not sent:
  - failing check: its name and the last ~200 lines of the failed job's log (`gh run view --log-failed`);
  - review feedback: the bodies of reviews requesting changes and the unresolved code-comment threads with file and line.
- [ ] Refresh: local state (sessions, Worktrees) live from the core's events; GitHub data on the PR board's 5-minute poll, on window focus, on showing the Dashboard, and a refresh button. Each GitHub section says how long ago it was refreshed.
- [ ] Without `gh` (not installed, not logged in, off GitHub) the GitHub sections say so with the fix, inline; the local ones work as ever.
- [ ] A badge on the switcher's Dashboard entry while elsewhere: sessions needing the user + reviews requested + the user's PRs with changes requested or checks failing (not Worktree hygiene).
- [ ] Core tests with real `git` and a fake `gh` (review requests query, Worktree attention reasons, prompt building).

Notes:
- Review requests open the PR overlay until 44 (Code review) lands; then they open a Code review.
- The PR board stays as it is: it's the only place with merged and closed PRs.
- Jira issues are wanted on it later (45).

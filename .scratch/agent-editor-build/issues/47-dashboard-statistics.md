# 47: Statistics on the Dashboard

**What to build:** A Statistics section in a column on the left of the Dashboard (43): six read-only cards about the user's own work in the Workspace's repo, over a chosen period, each compared with the period before it where that makes sense. Also: start recording each Agent session's start and state changes (not shown yet), for 46.

**Blocked by:** none (43).

**Status:** done (Linux: core-tested and type-checked; not yet tried in the app. Windows untested)

- [x] The section is always open, in a column on the left of the Dashboard, the other sections in a column on its right (one column, the statistics last, in a narrow window). Its header has a period selector, 7 / 30 / 90 days (30 by default), remembered per Workspace.
- [x] Cards; the first four show the change against the previous period of the same length ("▲ 3 vs previous 30d"), the last two are as things stand now:
  1. **Merged PRs:** the user's PRs in this repo merged in the period, into any base branch.
  2. **Median time to merge:** from ready for review (draft time doesn't count) to merged, over the PRs merged in the period.
  3. **Median time to first review:** to the first review by someone other than the user, any verdict including comment-only, bots left out; over the user's PRs that became ready in the period and have been reviewed. The card also says "N still waiting".
  4. **Reviews given:** distinct PRs the user reviewed in the period (several rounds on one PR count once).
  5. **Oldest open PR:** the age of the user's oldest open PR.
  6. **Longest-waiting review request:** how long the oldest review request to the user has waited.
- [x] A median from fewer than 3 PRs shows a muted "(n=2)"; nothing to count shows "—".
- [x] Read-only: the cards lead nowhere.
- [x] Loaded when the Dashboard is shown, kept for about an hour, reloaded by the Dashboard's refresh. Without `gh` the section says so inline, as the other GitHub sections do.
- [x] Each Agent session's start and state changes (with times) are recorded and kept across restarts, not shown anywhere yet.
- [x] Core tests with a fake `gh` (the figures, the previous period, bots and the user's own reviews left out, draft time left out) and for the recording.

Notes:
- No sparklines in v1.
- A sprint as the period (current, maybe previous ones) once Jira is supported: see 45.

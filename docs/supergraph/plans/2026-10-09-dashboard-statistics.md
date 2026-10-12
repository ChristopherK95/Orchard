# Plan: Statistics on the Dashboard (issue 47)

Spec: `.scratch/agent-editor-build/issues/47-dashboard-statistics.md`. Glossary: `CONTEXT.md` (Dashboard, Workspace, Agent session, Agent session states). Follows the Dashboard plan (`docs/supergraph/plans/2026-10-09-dashboard.md`, issue 43).

The core works out every figure (ADR 0003) from a few `gh api graphql` searches (ADR 0004: `gh` only), as pure functions over the parsed answers so the arithmetic (periods, medians, draft time, bots and the user's own reviews) is unit-tested on its own; a fake `gh` covers the calls. The period is kept in the core's app state, per Workspace. The frontend caches the figures for an hour and renders six read-only cards at the bottom of the Dashboard. Separately, the core starts recording each Agent session's start and state changes into an append-only log (for issue 46), shown nowhere.

Decisions this plan builds on (agreed with the user):
- Periods: the current one is `(now − P, now]`, the previous one `(now − 2P, now − P]`, P = 7, 30 or 90 days (30 by default), by exact timestamps. GitHub's search qualifiers are day-granular, so the searches ask from the UTC date of `now − 2P` and Rust filters by the exact times.
- Merged PRs: the user's PRs merged in the period, any base branch.
- Time to merge: the PR's non-draft time from when it first became ready for review to `mergedAt`. "First became ready": `createdAt` if it was opened ready, else its first `READY_FOR_REVIEW` event; a later draft spell (`CONVERT_TO_DRAFT` … `READY_FOR_REVIEW`) is subtracted too, so draft time never counts. (This is the agreed "READY_FOR_REVIEW event if any, else createdAt", made exact for PRs that went back to draft.)
- Time to first review: from first ready to the earliest submitted review (any state, comment-only too) whose author isn't the user and isn't a bot (`__typename == "Bot"` or a login ending in `[bot]`), over the user's PRs that first became ready in the period and have such a review. Reviews submitted before it was first ready (on the draft) don't count. "N still waiting": those of the period's PRs that are open, not draft, with no such review yet.
- Reviews given: distinct PRs not authored by the user with at least one review by the user submitted in the period (`reviewed-by:@me -author:@me`, then the review times in Rust).
- Oldest open PR: the user's oldest open PR by `createdAt` (drafts too: it's open). Longest-waiting review request: the oldest `requested_at` among the Dashboard's review requests (Task 3 of issue 43's `REVIEW_QUERY`, so the same "asked of the user directly, open, not draft" rule).
- A median over fewer than 3 PRs shows a muted "(n=2)"; nothing to count shows "—". Changes vs the previous period are shown neutrally (no red/green: a longer time isn't always worse).
- Session log: `sessions.jsonl` in the app data folder next to `state.json` (see Task 9 for why), entries older than a year dropped on startup.

## Task 1: FakeGh remembers every call
Status: completed
Risk: low
Dependencies: none
Files:
- Modify: crates/editor-core/tests/support/mod.rs
Blast radius:
- crates/editor-core/tests/pull_request.rs, crates/editor-core/tests/dashboard.rs (use `FakeGh::args`, unchanged)
Acceptance:
- Besides overwriting `args` (the last call's, as now), the fake script writes each call's arguments, one per line, to a new file in `<dir>/calls/` made with `mktemp` (so concurrent calls, as Task 6 makes, can't interleave or overwrite).
- `FakeGh::calls() -> Vec<Vec<String>>` returns every call's arguments, in no particular order; `args()` is unchanged.
TDD:
- Behavior: test support only; covered by the existing fake-gh tests still passing and by Task 6 using `calls()`.
- Test file: crates/editor-core/tests/dashboard.rs
- Test name: review_requests_are_asked_of_gh_for_the_user_directly (existing)
- RED command: `cargo test -p editor-core --test dashboard`
- Expected RED failure: none expected; this task extends the helper only.
- Minimal GREEN change: in `FakeGh::new`, create `calls/` and prepend `f=$(mktemp '<dir>/calls/call.XXXXXX'); printf '%s\n' "$@" > "$f"` to the script; add `calls()` reading every file in `calls/`.
- Refactor candidates: none
- Mocking: none
Steps:
1. Extend `FakeGh::new` and add `calls()` (support/mod.rs ~449–481), with a doc comment saying calls come in no particular order.
2. In `failing_check_logs_are_asked_of_gh_per_failed_job` (tests/dashboard.rs), add `assert_eq!(gh.calls().len(), 3)` (pr view, run view jobs, run view log) to show it works.
3. VERIFY:
   - `cargo test -p editor-core --test dashboard`
   - `cargo test -p editor-core --test pull_request`
   - `cargo clippy -p editor-core --all-targets`
Checkpoint:
- Files: `crates/editor-core/tests/support/mod.rs crates/editor-core/tests/dashboard.rs`
- Commit: `Tests: the fake gh remembers every call, not only the last`

## Task 2: Times, periods and medians
Status: completed
Risk: low
Dependencies: none
Files:
- Create: crates/editor-core/src/statistics.rs
- Modify: crates/editor-core/src/lib.rs
- Test: crates/editor-core/src/statistics.rs
Blast radius:
- none (new module)
Acceptance:
- `parse_utc(text) -> Option<i64>` reads GitHub's `YYYY-MM-DDTHH:MM:SSZ` (fractional seconds ignored) into seconds since the Unix epoch; anything else is `None`. `utc_date(secs) -> String` gives `YYYY-MM-DD` (UTC). Civil-date arithmetic by hand (days-from-civil), no new dependency.
- `Periods { now: i64, days: u32 }` with `in_current(t)` (`now − P < t ≤ now`), `in_previous(t)` (`now − 2P < t ≤ now − P`) and `search_from() -> String` (`utc_date(now − 2P)`).
- `median(values: Vec<i64>) -> Median` where `#[derive(Serialize)] #[serde(rename_all = "camelCase")] pub struct Median { seconds: Option<i64>, count: u32 }`; even counts take the mean of the middle two (rounded down); none gives `seconds: None, count: 0`.
- `pub struct Compared<T> { now: T, before: T }` (Serialize, camelCase).
TDD:
- Behavior: a GitHub time is read exactly, the two periods split at `now − P` without overlap, and a median is the middle value.
- Test file: crates/editor-core/src/statistics.rs
- Test name: times_periods_and_medians_are_exact
- RED command: `cargo test -p editor-core --lib statistics::tests::times_periods_and_medians_are_exact`
- Expected RED failure: no module `statistics` / no function `parse_utc` (compile error).
- Minimal GREEN change: the functions and types above; `mod statistics;` in lib.rs.
- Refactor candidates: none
- Mocking: none
Steps:
1. RED: assert `parse_utc("1970-01-02T00:00:01Z") == Some(86_401)`, `parse_utc("2024-02-29T12:00:00Z")` round-trips through `utc_date` to `"2024-02-29"`, `parse_utc("2026-10-08T12:00:00.123Z")` equals the same without fraction, `parse_utc("yesterday") == None`; with `Periods { now: 100 * DAY, days: 7 }`: `in_current(93 * DAY)` false, `in_current(93 * DAY + 1)` true, `in_previous(93 * DAY)` true, `in_previous(86 * DAY)` false, `search_from()` is `utc_date(86 * DAY)`; `median(vec![5, 1, 3])` → `{3, 3}`, `median(vec![4, 1])` → `{Some(2), 2}`, `median(vec![])` → `{None, 0}`.
   Command: `cargo test -p editor-core --lib statistics::tests::times_periods_and_medians_are_exact`
   Expected: FAIL
2. GREEN: write them, with a module doc comment in the style of dashboard.rs ("Statistics on the Dashboard (issue 47): …").
   Command: same
   Expected: PASS
3. REFACTOR: none
4. VERIFY:
   - `cargo test -p editor-core --lib statistics`
   - `cargo clippy -p editor-core --all-targets`
Checkpoint:
- Files: `crates/editor-core/src/statistics.rs crates/editor-core/src/lib.rs`
- Commit: `Statistics: times, periods and medians`

## Task 3: Draft time doesn't count
Status: completed
Risk: medium
Dependencies: Task 2
Files:
- Modify: crates/editor-core/src/statistics.rs
- Test: crates/editor-core/src/statistics.rs
Blast radius:
- none
Acceptance:
- `pub(crate) struct DraftEvent { at: i64, to_draft: bool }` (from `ConvertToDraftEvent` = true, `ReadyForReviewEvent` = false).
- `first_ready(created, is_draft_now, events) -> Option<i64>`: with events (sorted by time here), it was opened as a draft iff the first one is a `ReadyForReviewEvent`, so first ready is that event's time, else `created`; with none, `created` unless it's a draft now (never ready: `None`).
- `ready_time(created, is_draft_now, events, until) -> i64`: the seconds from `first_ready` to `until` minus every draft spell in between (a `to_draft` event to the next ready one, or to `until` if none); 0 if never ready.
TDD:
- Behavior: a PR's time to merge leaves out every spell it spent as a draft.
- Test file: crates/editor-core/src/statistics.rs
- Test name: draft_spells_are_left_out_of_a_prs_time
- RED command: `cargo test -p editor-core --lib statistics::tests::draft_spells_are_left_out_of_a_prs_time`
- Expected RED failure: no function `first_ready` in `statistics`.
- Minimal GREEN change: the two functions over the sorted events.
- Refactor candidates: none
- Mocking: none
Steps:
1. RED: opened ready at 0, merged at 100, no events → first ready 0, time 100; opened as draft at 0, ready at 30, merged at 100 → 30, 70; opened ready at 0, draft at 20, ready at 50, merged at 100 → 0, 70; opened as draft and still a draft, no events → `None`, 0; events given out of order give the same answers.
   Command: `cargo test -p editor-core --lib statistics::tests::draft_spells_are_left_out_of_a_prs_time`
   Expected: FAIL
2. GREEN: as acceptance.
   Command: same
   Expected: PASS
3. REFACTOR: none
4. VERIFY:
   - `cargo test -p editor-core --lib statistics`
   - `cargo clippy -p editor-core --all-targets`
Checkpoint:
- Files: `crates/editor-core/src/statistics.rs`
- Commit: `Statistics: draft time doesn't count towards a PR's time`

## Task 4: The six figures
Status: completed
Risk: medium
Dependencies: Task 3
Files:
- Modify: crates/editor-core/src/statistics.rs
- Modify: crates/editor-core/src/lib.rs
- Test: crates/editor-core/src/statistics.rs
Blast radius:
- crates/editor-core/src/dashboard.rs (read only: `ReviewRequest`)
Acceptance:
- Inputs, as parsed in Task 5: `AuthoredPr { number, title, created_at: i64, merged_at: Option<i64>, open: bool, is_draft: bool, draft_events: Vec<DraftEvent>, reviews: Vec<Reviewed> }`, `Reviewed { at: i64, login: String, bot: bool }`, `ReviewedPr { author: String, mine: Vec<i64> }` (the times of the viewer's reviews), `Waiting { number, title, since: String }` (`since` as GitHub gave it, ISO).
- `figures(periods, viewer, authored, reviewed, oldest_open: Option<Waiting>, requests: &[ReviewRequest]) -> Statistics`, with `pub struct Statistics { days, merged: Compared<u32>, time_to_merge: Compared<Median>, time_to_first_review: Compared<Median>, still_waiting: u32, reviews_given: Compared<u32>, oldest_open: Option<Waiting>, longest_request: Option<Waiting> }` (Serialize, camelCase; `Statistics`, `Median`, `Compared`, `Waiting` re-exported from lib.rs).
- Merged and time to merge by `merged_at` in each period, the time being `ready_time(…, merged_at)`. Time to first review and still waiting as in the decisions above (bots and the viewer left out; reviews before first ready left out). Reviews given counts a PR in a period when one of `mine` falls in it (several in one period count once); a PR whose author is the viewer is left out. Longest request: the request with the smallest `requested_at` (none without one), as `Waiting`.
TDD:
- Behavior: each card's figure, for this period and the one before, leaving out bots, the user's own reviews and draft time.
- Test file: crates/editor-core/src/statistics.rs
- Test name: merged_prs_and_time_to_merge_per_period; first_review_leaves_out_bots_and_the_user; reviews_given_count_each_pr_once_per_period; the_oldest_open_pr_and_longest_request_are_found
- RED command: `cargo test -p editor-core --lib statistics::tests::merged_prs_and_time_to_merge_per_period`
- Expected RED failure: no function `figures` in `statistics`.
- Minimal GREEN change: `figures` as above.
- Refactor candidates: one helper per card if `figures` grows past a screen.
- Mocking: none
Steps:
1. RED (merged): P = 7 days; two PRs merged in the current period (one with a 1-day draft spell), one in the previous, one merged before both, one open. Assert `merged == {2, 1}`, time to merge medians and counts (draft day left out), `count` 0 / `seconds: None` for a period with none.
   Command: `cargo test -p editor-core --lib statistics::tests::merged_prs_and_time_to_merge_per_period`
   Expected: FAIL
2. GREEN: `figures` with the merged cards.
   Command: same
   Expected: PASS
3. RED/GREEN (first review): PRs readied in the current period: one reviewed first by `dependabot[bot]` then `ann` (ann's counts), one reviewed by a `__typename: Bot` and the viewer only (still waiting, open), one reviewed only while a draft and now open and not a draft (still waiting), one merged with no review (not waiting); one readied in the previous period reviewed by `bob`. Assert medians, counts and `still_waiting == 2`.
   Command: `cargo test -p editor-core --lib statistics::tests::first_review_leaves_out_bots_and_the_user`
   Expected: FAIL, then PASS
4. RED/GREEN (reviews given): a PR the viewer reviewed three times this period (counts once), one reviewed once in each period (counts in both), one authored by the viewer (left out). Assert `reviews_given == {2, 1}`.
   Command: `cargo test -p editor-core --lib statistics::tests::reviews_given_count_each_pr_once_per_period`
   Expected: FAIL, then PASS
5. RED/GREEN (now cards): `oldest_open` passed through; requests with `requested_at` `Some` at two times and one `None` → the earlier one; no requests → `None`.
   Command: `cargo test -p editor-core --lib statistics::tests::the_oldest_open_pr_and_longest_request_are_found`
   Expected: FAIL, then PASS
6. VERIFY:
   - `cargo test -p editor-core --lib statistics`
   - `cargo clippy -p editor-core --all-targets`
Checkpoint:
- Files: `crates/editor-core/src/statistics.rs crates/editor-core/src/lib.rs`
- Commit: `Statistics: the six figures, against the period before`

## Task 5: What GitHub is asked, and reading its answers
Status: completed
Risk: medium
Dependencies: Task 4
Files:
- Modify: crates/editor-core/src/statistics.rs
- Modify: crates/editor-core/src/dashboard.rs
- Test: crates/editor-core/src/statistics.rs
Blast radius:
- crates/editor-core/src/dashboard.rs (`Nodes`, `Login` and their fields `nodes`, `login` made `pub(crate)` for reuse; nothing else changes)
Acceptance:
- Query constants. `AUTHORED_QUERY` and `REVIEWED_QUERY` declare `$q: String!` and `$after: String` and ask `search(query: $q, type: ISSUE, first: 100, after: $after) { pageInfo { hasNextPage endCursor } nodes { ... on PullRequest { … } } }` and `viewer { login }`:
  - `AUTHORED_QUERY`: `number title createdAt mergedAt isDraft state timelineItems(itemTypes: [READY_FOR_REVIEW_EVENT, CONVERT_TO_DRAFT_EVENT], first: 50) { nodes { __typename ... on ReadyForReviewEvent { createdAt } ... on ConvertToDraftEvent { createdAt } } } reviews(first: 50) { nodes { submittedAt author { __typename login } } }`; search `repo:{owner}/{repo} is:pr author:@me updated:>={from}` (any PR merged or readied since `from` was updated since).
  - `REVIEWED_QUERY`: `number author { login } reviews(last: 100) { nodes { submittedAt author { login } } }`; search `repo:{owner}/{repo} is:pr reviewed-by:@me -author:@me updated:>={from}`.
  - `OPEN_QUERY` (declares only `$q`: GitHub rejects an unused variable; `search(query: $q, type: ISSUE, first: 1)`, no paging): `viewer { login }`, `number title createdAt`; search `repo:{owner}/{repo} is:pr is:open author:@me sort:created-asc`.
- Parsers: `authored_page(json) -> Result<Page<AuthoredPr>, String>`, `reviewed_page(json) -> Result<Page<ReviewedPr>, String>` (the viewer from the same answer's `viewer { login }`; only their reviews kept in `mine`), `oldest_open(json) -> Result<Option<Waiting>, String>`; `Page<T> { viewer: String, items: Vec<T>, next: Option<String> }` (`next` = `endCursor` when `hasNextPage`). Null `submittedAt` (pending review) and unparseable times are skipped; a null author (deleted user) is a non-bot other person. `gh said something unexpected: …` on bad JSON, as in dashboard.rs.
TDD:
- Behavior: GitHub's answers are read into the figures' inputs, with bots marked and pending reviews skipped.
- Test file: crates/editor-core/src/statistics.rs
- Test name: githubs_answers_are_read_into_prs_and_reviews
- RED command: `cargo test -p editor-core --lib statistics::tests::githubs_answers_are_read_into_prs_and_reviews`
- Expected RED failure: no function `authored_page` in `statistics`.
- Minimal GREEN change: the constants and serde structs/parsers, reusing `Nodes`/`Login` from dashboard.rs.
- Refactor candidates: none
- Mocking: none
Steps:
1. RED: an authored page (`hasNextPage: true`, cursor `"c1"`) with a merged PR that has a `ConvertToDraftEvent` and `ReadyForReviewEvent`, reviews by `{"__typename":"Bot","login":"ci"}`, `renovate[bot]` (User), `ann`, and a pending one; a reviewed page with the viewer's and someone else's reviews; an open answer with one PR and one with none. Assert the parsed structs, `next == Some("c1")`, `bot` flags, and `oldest_open == None` for the empty one.
   Command: `cargo test -p editor-core --lib statistics::tests::githubs_answers_are_read_into_prs_and_reviews`
   Expected: FAIL
2. GREEN: as acceptance.
   Command: same
   Expected: PASS
3. VERIFY:
   - `cargo test -p editor-core --lib statistics`
   - `cargo test -p editor-core --lib dashboard`
   - `cargo clippy -p editor-core --all-targets`
Checkpoint:
- Files: `crates/editor-core/src/statistics.rs crates/editor-core/src/dashboard.rs`
- Commit: `Statistics: what GitHub is asked, and its answers read`

## Task 6: `Core::statistics`
Status: completed
Risk: medium
Dependencies: Task 1, Task 5
Files:
- Modify: crates/editor-core/src/statistics.rs
- Modify: crates/editor-core/src/core.rs
- Modify: crates/editor-core/tests/support/mod.rs
- Create: crates/editor-core/tests/statistics.rs
Blast radius:
- crates/editor-core/src/pull_request.rs (read only: `gh_text`, `gh_program`, `not_about_creating`)
- crates/editor-core/src/core.rs (new method beside `review_requests`, ~2867)
Acceptance:
- `Core::statistics(worktree, days: u32) -> Result<Statistics, CoreError>`: `days` must be 7, 30 or 90, else `CoreError::UnknownPeriod(days)` (new variant, `#[error("statistics cover 7, 30 or 90 days, not {0}")]`). `now` is the system time in seconds.
- `statistics::read(gh, worktree, periods)` runs concurrently (`tokio::try_join!`): the authored search and the reviewed search, each paged by `search_all` (`gh api graphql -F q=… -f query=…`, plus `-f after=<cursor>` after the first page, at most 10 pages: GitHub's search stops at 1,000 results), the open search, and the review requests (`dashboard::REVIEW_SEARCH`/`REVIEW_QUERY`/`review_requests_listed`, as `Core::review_requests` asks). Then `figures`.
- A `gh` failure comes back as `CoreError::Git(not_about_creating(e))`, as for `review_requests`, so the Dashboard shows its setup hint.
TDD:
- Behavior: the statistics are read from `gh` for this period and the one before, following pages.
- Test file: crates/editor-core/tests/statistics.rs
- Test name: statistics_are_read_from_gh_for_the_period_and_the_one_before
- RED command: `cargo test -p editor-core --test statistics statistics_are_read_from_gh_for_the_period_and_the_one_before`
- Expected RED failure: no method `statistics` on `Core`.
- Minimal GREEN change: `search_all`, `read`, the Core method and the error variant.
- Refactor candidates: none
- Mocking: fake `gh` (`FakeGh`, `case "$*"` on each search's words: `*"is:open author:@me"*` first, then `*"reviewed-by:@me"*`, `*"user-review-requested:@me"*`, `*"after=c1"*` (second authored page), `*"author:@me"*`); `gh` can't reach GitHub in tests.
Steps:
1. Add `support::utc_ago(secs: u64) -> String` (`YYYY-MM-DDTHH:MM:SSZ` for now minus `secs`, by the same days-from-civil arithmetic; test-only copy, as the core's is crate-private) to tests/support/mod.rs.
2. RED: new file `tests/statistics.rs` (module doc: "Issue 47: the Dashboard's statistics, asked of a fake `gh`, and the period kept per Workspace."), each fake-`gh` test `#[cfg(unix)]` as in tests/dashboard.rs. A fake answering: authored page 1 (cursor `c1`) with a PR merged 1 day ago (ready 2 days ago, first reviewed by `ann` 1.5 days ago), page 2 with a PR merged 10 days ago and one open, ready 3 days ago, reviewed only by `bot[bot]`; reviewed page with a PR the viewer reviewed 2 days ago; open answer with PR #4 opened 40 days ago; review requests with one asked 5 days ago. `core.statistics(&root, 7)`: assert `merged == {1, 1}`, `time_to_first_review.now.count == 1`, `still_waiting == 1`, `reviews_given.now == 1`, `oldest_open.number == 4`, `longest_request` set. From `gh.calls()`: 5 calls, all `api graphql`; one has `after=c1`; the searches have `updated:>=`, `reviewed-by:@me -author:@me`, `sort:created-asc`. `core.statistics(&root, 14)` is `Err(UnknownPeriod(14))`.
   Command: `cargo test -p editor-core --test statistics statistics_are_read_from_gh_for_the_period_and_the_one_before`
   Expected: FAIL
3. GREEN: as acceptance.
   Command: same
   Expected: PASS
4. RED/GREEN: a fake that exits 1 printing `gh auth login` to stderr: the error says `gh auth login` and not "Creating a PR".
   Command: `cargo test -p editor-core --test statistics statistics_say_when_gh_cant_be_used`
   Expected: FAIL, then PASS
5. VERIFY:
   - `cargo test -p editor-core --test statistics`
   - `cargo test -p editor-core --test dashboard`
   - `cargo clippy -p editor-core --all-targets`
Checkpoint:
- Files: `crates/editor-core/src/statistics.rs crates/editor-core/src/core.rs crates/editor-core/tests/support/mod.rs crates/editor-core/tests/statistics.rs`
- Commit: `Statistics: read from gh for the period and the one before`

## Task 7: The period, remembered per Workspace
Status: pending
Risk: medium
Dependencies: Task 6
Files:
- Modify: crates/editor-core/src/app_state.rs
- Modify: crates/editor-core/src/core.rs
- Modify: crates/editor-core/tests/statistics.rs
- Test: crates/editor-core/src/app_state.rs
Blast radius:
- crates/editor-core/src/core.rs (`State`, `persist` ~4677, `restore` ~4740, beside `pinned`/`column_shares`)
- crates/editor-core/src/app_state.rs (`WorkspaceState`; older files without the field still load)
Acceptance:
- `WorkspaceState.statistics_days: Option<u32>`, `#[serde(default, skip_serializing_if = "Option::is_none")]`; `State.statistics_days` saved by `persist` and set by `restore` (so another Workspace with none goes back to the default).
- `Core::statistics_period() -> u32`: the open Workspace's, 30 when unset or none is open. `Core::set_statistics_period(days) -> Result<(), CoreError>`: 7, 30 or 90 (else `UnknownPeriod`), needs an open Workspace (`NoWorkspace`), then `persist`.
TDD:
- Behavior: the period chosen in one Workspace is the one it opens with after a restart, and another Workspace keeps its own.
- Test file: crates/editor-core/tests/statistics.rs
- Test name: the_statistics_period_is_remembered_per_workspace
- RED command: `cargo test -p editor-core --test statistics the_statistics_period_is_remembered_per_workspace`
- Expected RED failure: no method `set_statistics_period` on `Core`.
- Minimal GREEN change: the field, the two Core methods, `persist`/`restore` lines.
- Refactor candidates: none
- Mocking: none (real git, `core_with_state`)
Steps:
1. RED: two `git_repo()`s; `core_with_state`; open A: period 30; set 7; open B: 30; set 90; set 14 is `Err(UnknownPeriod(14))`. A second `core_with_state(&fake)` (the restart): open A → 7, open B → 90. (No `gh` here, so no `#[cfg(unix)]`: it runs on Windows too.)
   Command: `cargo test -p editor-core --test statistics the_statistics_period_is_remembered_per_workspace`
   Expected: FAIL
2. GREEN: as acceptance; add `statistics_days: Some(7)` to `the_state_round_trips_and_other_workspaces_are_kept` in app_state.rs.
   Command: same
   Expected: PASS
3. VERIFY:
   - `cargo test -p editor-core --test statistics`
   - `cargo test -p editor-core --lib app_state`
   - `cargo test -p editor-core --test close_reopen_restore`
   - `cargo clippy -p editor-core --all-targets`
Checkpoint:
- Files: `crates/editor-core/src/app_state.rs crates/editor-core/src/core.rs crates/editor-core/tests/statistics.rs`
- Commit: `Statistics: the period is remembered per Workspace`

## Task 8: The session log file
Status: pending
Risk: low
Dependencies: none
Files:
- Create: crates/editor-core/src/session_log.rs
- Modify: crates/editor-core/src/lib.rs
- Test: crates/editor-core/src/session_log.rs
Blast radius:
- none (new module)
Acceptance:
- `pub(crate) enum Entry` (`Serialize`, `#[serde(tag = "kind", rename_all = "camelCase")]`, fields camelCase): `Opened { at, session, workspace, worktree, new, state }` (the editor holds the session from now: `new` when the Agent just created it, else restored or reopened), `State { at, session, state }`, `Closed { at, session }` (the editor no longer holds it: Tab closed, Worktree removed, Workspace closed). `at` is ms since the Unix epoch; `session` the ACP id (stable across restarts); `state` a `SessionState`.
- `SessionLog::open(path, now_ms) -> SessionLog` drops entries older than `KEEP_FOR` (365 days) first: it reads only the first line, and only if that's older (or unreadable) rewrites the file keeping lines whose `at` is recent (bad lines dropped), through a `.partial` file and a rename as app_state.rs does. A missing file is fine.
- `SessionLog::append(&self, entry)` writes one JSON line with one `write_all` on a file opened in append mode (created with its folder if missing), under a `Mutex`; failures are `eprintln!`ed, never returned: the log must not break a session.
TDD:
- Behavior: entries are appended one JSON line each, and on opening, a year-old start is dropped while recent ones stay.
- Test file: crates/editor-core/src/session_log.rs
- Test name: entries_are_appended_as_json_lines; entries_older_than_a_year_are_dropped_on_open
- RED command: `cargo test -p editor-core --lib session_log::tests::entries_are_appended_as_json_lines`
- Expected RED failure: no module `session_log` (compile error).
- Minimal GREEN change: as acceptance.
- Refactor candidates: none
- Mocking: none (temp dir)
Steps:
1. RED: append `Opened` and `State` to a log in a temp dir (folder not yet made); read two lines, each JSON with `kind` `opened`/`state`, the ACP id and `state: "working"`.
   Command: `cargo test -p editor-core --lib session_log::tests::entries_are_appended_as_json_lines`
   Expected: FAIL
2. GREEN: `Entry`, `append`.
   Command: same
   Expected: PASS
3. RED/GREEN: a file with lines at `now − 400 days`, a bad line, `now − 10 days`, `now − 1 day`; `open(path, now)` leaves the last two. A file whose first line is recent is left byte for byte (a bad line later stays: it wasn't rewritten).
   Command: `cargo test -p editor-core --lib session_log::tests::entries_older_than_a_year_are_dropped_on_open`
   Expected: FAIL, then PASS
4. VERIFY:
   - `cargo test -p editor-core --lib session_log`
   - `cargo clippy -p editor-core --all-targets`
Checkpoint:
- Files: `crates/editor-core/src/session_log.rs crates/editor-core/src/lib.rs`
- Commit: `Session log: an append-only record, a year long`

## Task 9: Record each Agent session's start and state changes
Status: pending
Risk: high
Dependencies: Task 8
Files:
- Modify: crates/editor-core/src/core.rs
- Modify: crates/editor-core/tests/support/mod.rs
- Create: crates/editor-core/tests/session_log.rs
Blast radius:
- crates/editor-core/src/core.rs (hub: `CoreConfig`, `Inner`, `Session` struct, `register_session` ~4595, `Session::update` ~5435, `Inner::forget_session` ~4282)
- src-tauri/src/main.rs (`CoreConfig` literal uses `..CoreConfig::new`, so it still builds; the path is set in Task 10)
- every integration test (`CoreConfig::new` gets `session_log_path: None`)
Acceptance:
- `CoreConfig.session_log_path: Option<PathBuf>` ("Where each Agent session's start and state changes are recorded (issue 47, for 46); `None` records nothing."). `Core::new` opens it (`SessionLog::open(path, unix_millis())`, pruning) into `Inner.session_log: Option<Arc<SessionLog>>`.
- `register_session` gives the `Session` the log and the Workspace root, and builds `Opened { new: connection.is_some(), state }` (only `new_session_in` passes a connection: new; `reopen` and `restore` pass none). It runs under the `state` lock, so the entry is kept on the `Session` (an `opened: Option<SessionLogEntry>` taken once) and appended by its three callers after the `state` lock is released. State changes are appended in `Session::update` while it still holds the control lock (the `SessionLog`'s own mutex is the innermost lock, so the lock order stays safe), so a session's lines are written in the order its changes happened.
- `Session::update` appends `State` when `info.state` changes, written while the control lock is still held (see above), so the lines keep the order of the changes.
- `Inner::forget_session` appends `Closed`.
- Why a JSONL file of its own: `state.json` is rewritten whole on every save and shared between editor processes (each `update` re-reads and rewrites it), so a growing history there would make every save slower and race more; an append is one small write, a bad line loses one entry not the file, and `state.json` being set aside as `.bad` doesn't lose the history. Size: about 80 bytes per state change; pruning reads one line on a normal start.
TDD:
- Behavior: a session's start, each state change and its close are recorded with times, and the record survives a restart.
- Test file: crates/editor-core/tests/session_log.rs
- Test name: a_sessions_start_and_state_changes_are_recorded_across_restarts; year_old_entries_are_dropped_at_startup
- RED command: `cargo test -p editor-core --test session_log a_sessions_start_and_state_changes_are_recorded_across_restarts`
- Expected RED failure: no field `session_log_path` in `CoreConfig`.
- Minimal GREEN change: as acceptance.
- Refactor candidates: none
- Mocking: fake ACP agent (`FakeAgent`), as every session test.
Steps:
1. Add `FakeAgent::session_log_path()` (`<dir>/sessions.jsonl`) and `core_with_session_log(agent)` (state and session log both in the fake's dir, so a second one is the editor after a restart) to support/mod.rs, and a `session_log(agent) -> Vec<serde_json::Value>` reader.
2. RED: open a repo, `new_session`, one turn (`send_prompt` + `states_until(.., Idle)`); a second session also given one turn (only a Tab that was sent a prompt is saved, `persist` ~4691, so only then is it restored), left open; close the first. Lines: `opened` (new, idle) for both; for the first `state` working then idle, then `closed`; `at` never decreases per session. Drop the core; a second `core_with_session_log` opens the repo: the old lines are still first, and an `opened` (`new: false`, `suspended`) for the restored second Tab follows. (Closing a Workspace calls `forget_session` for each session, so a `closed` line for the second can come before that when the first core closes its Workspace; assert per session, not on the whole order, and don't require a `closed` line for the second: check what dropping a `Core` does first.)
   Command: `cargo test -p editor-core --test session_log a_sessions_start_and_state_changes_are_recorded_across_restarts`
   Expected: FAIL
3. GREEN: as acceptance.
   Command: same
   Expected: PASS
4. RED/GREEN: write a log with a line at `at: 1` then one at now (support has `SystemTime`), make a core: only the recent line is left.
   Command: `cargo test -p editor-core --test session_log year_old_entries_are_dropped_at_startup`
   Expected: FAIL, then PASS
5. REFACTOR: none
6. VERIFY:
   - `cargo test -p editor-core --test session_log`
   - `cargo test -p editor-core --test suspend_resume`
   - `cargo test -p editor-core --test close_reopen_restore`
   - `cargo test -p editor-core`
   - `cargo clippy -p editor-core --all-targets`
Checkpoint:
- Files: `crates/editor-core/src/core.rs crates/editor-core/tests/support/mod.rs crates/editor-core/tests/session_log.rs`
- Commit: `Session log: each Agent session's start and state changes are recorded`

## Task 10: Tauri commands and TypeScript bindings
Status: pending
Risk: low
Dependencies: Task 7, Task 9
Files:
- Modify: src-tauri/src/main.rs
- Modify: src/core.ts
Blast radius:
- src/dashboardData.ts, src/Dashboard.tsx (consume the new types later)
Acceptance:
- Commands `statistics(worktree, days)`, `statistics_period()`, `set_statistics_period(days)` beside `review_requests` (main.rs ~1078), registered in `generate_handler!` (~1573).
- `CoreConfig` in `setup` gets `session_log_path: app_data_dir/sessions.jsonl`, filtered `!bench_mode()` as `state_path` is.
- core.ts: `Median { seconds: number | null; count: number }`, `Compared<T> { now: T; before: T }`, `Waiting { number; title; since }`, `Statistics` (camelCase as serialized), `StatisticsDays = 7 | 30 | 90`, and `statistics`, `statisticsPeriod`, `setStatisticsPeriod` next to `reviewRequests`.
TDD:
- Behavior: none user-visible on its own (glue); covered by the core tests and the type check.
- Test file: none (no frontend test framework)
- Test name: none
- RED command: `pnpm typecheck`
- Expected RED failure: none expected; this task adds types and commands only.
- Minimal GREEN change: the commands and bindings.
- Refactor candidates: none
- Mocking: none
Steps:
1. Commands and the config line in main.rs.
2. Types and functions in core.ts.
3. VERIFY:
   - `cargo build -p orchard`
   - `cargo clippy --workspace --all-targets`
   - `pnpm typecheck`
Checkpoint:
- Files: `src-tauri/src/main.rs src/core.ts`
- Commit: `Statistics: commands for the frontend; the session log in the app data folder`

## Task 11: Frontend statistics data, kept an hour
Status: pending
Risk: medium
Dependencies: Task 10
Files:
- Modify: src/dashboardData.ts
- Modify: src/App.tsx
Blast radius:
- src/App.tsx (the `on(view, …)` effect ~514 that refreshes the Dashboard's GitHub data; `watchReviewRequests` wiring ~503)
Acceptance:
- `dashboardData.ts` exports `statistics = { figures, error, loading, refreshedAt, days, shown(), refresh(), setDays(d) }` and `watchStatistics(worktree, visible: () => boolean)`: on a Workspace (main Worktree) change (a non-empty one) it clears the figures, then awaits `core.statisticsPeriod()` into `days`, and only then, if `visible()` (the Dashboard is the view), calls `shown()`. It asks GitHub nothing while the Dashboard isn't showing. `shown()` does nothing until `days` has been read for the current Worktree (that read calls it afterwards), and nothing with an empty Worktree.
- `shown()` does nothing while `loading()`; it loads when there are no figures for the current Worktree and `days`, or they were read over `KEEP_MS` (1 hour) ago; `refresh()` always loads; `setDays(d)` calls `core.setStatisticsPeriod(d)`, sets `days` and loads (keeping figures per `days` in a small map, so switching back within the hour shows them without asking). Stale answers are dropped by the asked-counter as `loadRequests` does. Errors go to `error`.
- App.tsx calls `watchStatistics(mainWorktree, () => view() === "dashboard")` beside `watchReviewRequests` (so the Workspace opening on the Dashboard before its Worktrees are listed, or switching Workspace while it's up, still loads them), and the view effect calls `statistics.shown()` whenever the Dashboard becomes the view, before its `openedOnDashboard` early return (so a Workspace opening on the Dashboard loads them once). Window focus doesn't reload them.
TDD:
- Behavior: no frontend test framework; verified by type check and a run of the app (Task 13).
- Test file: none
- Test name: none
- RED command: `pnpm typecheck`
- Expected RED failure: none expected.
- Minimal GREEN change: as acceptance.
- Refactor candidates: none
- Mocking: none
Steps:
1. Write the signals and functions in dashboardData.ts (module comment updated to mention the statistics, read only when the Dashboard is shown and kept an hour).
2. Wire in App.tsx.
3. VERIFY:
   - `pnpm typecheck`
Checkpoint:
- Files: `src/dashboardData.ts src/App.tsx`
- Commit: `Statistics: read when the Dashboard is shown, kept an hour`

## Task 12: The Statistics section and its cards
Status: pending
Risk: medium
Dependencies: Task 11
Files:
- Create: src/Statistics.tsx
- Modify: src/Dashboard.tsx
- Modify: src/styles.css
Blast radius:
- src/Dashboard.tsx (`Section` gains nothing new: the period selector goes in its `action`, the refresh in `github`)
Acceptance:
- The last section of the Dashboard, after Sessions: "Statistics", always open, its head with a 7d / 30d / 90d segmented selector (the current one pressed, `aria-pressed`) calling `statistics.setDays`, and the `github` refresh ("refreshed 2m ago", refresh button → `statistics.refresh()`, error inline with the same `PR_HINT` → `GH_HINT` replacement; "Asking GitHub…" while first loading).
- `src/Statistics.tsx` exports `StatisticsCards(props: { figures: Statistics; now: number })`: six cards in a grid, in the issue's order, each a title and a big figure: Merged PRs (count); Median time to merge; Median time to first review (and "N still waiting" under it); Reviews given; Oldest open PR (age, and "#4 Title" muted); Longest-waiting review request (wait, and "#9 Title" muted).
- Durations as "3 h", "2 d", "45 min" (one helper); a median with `count` 1–2 adds a muted "(n=2)"; `count` 0 or none shows "—".
- The first four show a muted line "▲ 3 vs previous 30d" / "▼ 2 h vs previous 30d" / "same as previous 30d", and "— previous 30d" when either side has nothing; no colour for better or worse.
- Cards are plain `div`s: no click, no hover affordance, no `title` that suggests a link. Ages move on with the Dashboard's 30-second `now` tick.
- Styles beside the `.dashboard-*` rules: `.dashboard-stats` grid (`repeat(auto-fill, minmax(220px, 1fr))`), `.dashboard-stat` (card look as `.dashboard-row`, no pointer), `.dashboard-stat-figure`, `.dashboard-stat-vs`, the selector as the view switcher's segmented buttons.
TDD:
- Behavior: no frontend test framework; verified by type check and a run of the app (Task 13).
- Test file: none
- Test name: none
- RED command: `pnpm typecheck`
- Expected RED failure: none expected.
- Minimal GREEN change: as acceptance.
- Refactor candidates: none
- Mocking: none
Steps:
1. Write `Statistics.tsx` (module comment as Dashboard.tsx's, in the glossary's words).
2. Add the section to Dashboard.tsx (and update its top comment's list of sections).
3. Styles.
4. VERIFY:
   - `pnpm typecheck`
Checkpoint:
- Files: `src/Statistics.tsx src/Dashboard.tsx src/styles.css`
- Commit: `Statistics: six cards at the bottom of the Dashboard`

## Task 13: Final verification; issue 47 done
Status: pending
Risk: low
Dependencies: Task 12
Files:
- Modify: .scratch/agent-editor-build/issues/47-dashboard-statistics.md
- Modify: CONTEXT.md
Blast radius:
- none
Acceptance:
- Issue 47's checklist is ticked for what's done, with its status line saying what was and wasn't tried (as issue 43 does).
- CONTEXT.md's **Dashboard** entry ends with its statistics: "… and, at the bottom, statistics of the user's work over a chosen period."
TDD:
- Behavior: no frontend test framework; verified by a run of the app.
- Test file: none
- Test name: none
- RED command: `pnpm typecheck`
- Expected RED failure: none expected.
- Minimal GREEN change: as acceptance.
- Refactor candidates: none
- Mocking: none
Steps:
1. VERIFY:
   - `cargo test --workspace`
   - `cargo clippy --workspace --all-targets`
   - `cargo fmt --all --check`
   - `pnpm typecheck`
   - Run the app (`pnpm tauri dev`) on a repo with GitHub PRs: the Statistics section is last and open, and loads on its own when the window opens on the Dashboard and when the Workspace is switched while it's up; 30d is chosen; the six cards show figures matching GitHub for a PR or two checked by hand; 7d and 90d change them and the comparison line; the choice holds after reopening the Workspace and after a restart, and another Workspace has its own; switching away from the Dashboard and back within the hour doesn't ask GitHub again (no "refreshed" change), the refresh button does; with `gh` off `PATH` the section shows the hint inline and the other sections render; cards don't react to clicks; `sessions.jsonl` in the app data folder gets lines as a session starts, works, goes Idle and closes.
2. Update issue 47 and CONTEXT.md.
Checkpoint:
- Files: `.scratch/agent-editor-build/issues/47-dashboard-statistics.md CONTEXT.md`
- Commit: `Dashboard statistics: issue 47 done`

## Environment Context
- **Language:** Rust (edition 2021, workspace) core + Tauri 2; TypeScript with SolidJS frontend (Vite)
- **Test command:** `cargo test --workspace` (focused: `cargo test -p editor-core --test statistics <name>` / `cargo test -p editor-core --test session_log <name>` / `cargo test -p editor-core --lib statistics::tests::<name>` / `cargo test -p editor-core --lib session_log::tests::<name>`)
- **Linter command:** `cargo clippy --workspace --all-targets`
- **Formatter command:** `cargo fmt --all`
- **Build command:** `pnpm typecheck` (frontend), `cargo build -p orchard` (Tauri crate), `pnpm tauri build` (app); run with `pnpm tauri dev`
- **Branch:** dashboard (fast-forward main when done)
- **Conventional commit style:** "Area: sentence" (e.g. "Dashboard: items open where they live"), ending with the Co-Authored-By line

**Codebase conventions:** Core owns state and logic (ADR 0003); git and gh through their CLIs only (ADR 0004), `gh api graphql -F q='repo:{owner}/{repo} …' -f query=…` so `gh` fills in the repo. Errors from gh are `String`s wrapped as `CoreError::Git`, with `not_about_creating` for Dashboard sections. No date crate: times are GitHub's ISO strings, parsed by hand. Serde `rename_all = "camelCase"`, tagged enums `tag = "kind"`. App state: per-Workspace `WorkspaceState` fields with `#[serde(default, skip_serializing_if …)]`, saved by `Inner::persist`, read back in `Inner::restore`; files replaced via `.partial` + rename. Integration tests in `crates/editor-core/tests/*.rs` with `support/mod.rs` helpers (`RepoWithOrigin`, `git_repo`, `FakeAgent`, `FakeGh`, `core_with_state`, `states_until`, `eventually`), `#[tokio::test(flavor = "multi_thread")]`, `#[cfg(unix)]` for fake-`gh` tests; parser and arithmetic unit tests on JSON strings / plain values in the module. Frontend: module-level signals for shared GitHub data (`dashboardData.ts`), `Section` in Dashboard.tsx with `github`/`action` props, relative times via `ago`; comments are prose sentences in the glossary's words.

**Graph Context:**
- Blast radius: ~16 files | Hub nodes: crates/editor-core/src/core.rs (Session::update, register_session, persist/restore), src/App.tsx (graph server unavailable; from reading)
- Bridge nodes: src-tauri/src/main.rs, src/core.ts | Communities crossed: core, tauri glue, frontend, test support
- Surprising connections: the session log hooks into `Session::update`, which every state change of every session passes through (auto-suspend, resume, crashes); FakeGh's change touches every fake-gh test file.

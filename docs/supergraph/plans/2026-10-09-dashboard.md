# Plan: Dashboard (issue 43)

Spec: `.scratch/agent-editor-build/issues/43-dashboard.md`. Glossary: `CONTEXT.md` (Dashboard, Board, Worktree, Agent session, Worktrees overview, Base).

The core works out everything the Dashboard shows that isn't already in the frontend (ADR 0003): Worktree attention reasons, review requests, and the text of the "Start an Agent session" prompts. The frontend renders it and wires the view. Core tasks are test-first; the frontend has no test framework, so its tasks are verified by `pnpm typecheck` and a run of the app.

## Task 1: The `dashboard.openOnStart` setting
Status: completed
Risk: low
Dependencies: none
Files:
- Modify: crates/editor-core/src/settings.rs
- Test: crates/editor-core/src/settings.rs
Blast radius:
- crates/editor-core/src/core.rs (re-reads Settings)
- src-tauri/src/main.rs (change_setting)
- src/core.ts (Settings, SettingChange types; Task 6)
Acceptance:
- `[dashboard] open_on_start` parses; absent, it's `true`; unknown keys under `[dashboard]` are errors as in the other sections.
- `SettingChange::DashboardOpenOnStart { on }` writes `open_on_start` under `[dashboard]` keeping the file's comments and layout.
- The default settings file text has a `[dashboard]` section with `open_on_start = true`.
TDD:
- Behavior: the Dashboard opens on start unless the settings turn it off, and the toggle writes it back.
- Test file: crates/editor-core/src/settings.rs
- Test name: dashboard_opens_on_start_unless_turned_off
- RED command: `cargo test -p editor-core --lib settings::tests::dashboard_opens_on_start_unless_turned_off`
- Expected RED failure: no field `dashboard` on `Settings` / no variant `DashboardOpenOnStart` (compile error naming the missing setting, not a test-harness error).
- Minimal GREEN change: `struct Dashboard { open_on_start: bool }` with `Default` true and the same serde attributes as `Notifications`; `Settings.dashboard`; the `SettingChange` variant and its `target` arm; the default text.
- Refactor candidates: none
- Mocking: none
Steps:
1. RED: test parsing `""` (→ true), `"[dashboard]\nopen_on_start = false\n"` (→ false), and applying `DashboardOpenOnStart { on: false }` to a file with a comment, asserting the comment stays and the key is written.
   Command: `cargo test -p editor-core --lib settings::tests::dashboard_opens_on_start_unless_turned_off`
   Expected: FAIL
2. GREEN: add the struct, field, variant, `target` arm and default text, mirroring `Notifications.turn_finished` (settings.rs:30, :182, :250, ~:501).
   Command: `cargo test -p editor-core --lib settings::tests::dashboard_opens_on_start_unless_turned_off`
   Expected: PASS
3. REFACTOR: none
4. VERIFY:
   - `cargo test -p editor-core --lib settings`
   - `cargo clippy -p editor-core --all-targets`
Checkpoint:
- Files: `crates/editor-core/src/settings.rs`
- Commit: `Settings: dashboard.open_on_start`

## Task 2: Worktree attention reasons
Status: completed
Risk: medium
Dependencies: none
Files:
- Create: crates/editor-core/src/dashboard.rs
- Modify: crates/editor-core/src/lib.rs
- Modify: crates/editor-core/src/core.rs
- Create: crates/editor-core/tests/dashboard.rs
Blast radius:
- crates/editor-core/src/git_status.rs (read only: GitStatus, GitOperation)
- crates/editor-core/src/worktrees.rs (read only: WorktreeInfo)
Acceptance:
- `Core::worktree_attention() -> Result<Vec<WorktreeAttention>, CoreError>` returns, for each Worktree of the open Workspace that needs attention (the main checkout included), `{ path, branch, reasons: Vec<AttentionReason> }`.
- `AttentionReason` is `Uncommitted { files }`, `Unpushed { commits }`, `Behind { commits }`, `NoUpstream { commits }` (commits since the Base), `Operation { operation: GitOperation }`; serialized `#[serde(tag = "kind", rename_all = "camelCase")]`.
- A clean Worktree level with its upstream is not listed.
TDD:
- Behavior: a Worktree is listed with every reason it needs attention, and a clean one isn't.
- Test file: crates/editor-core/tests/dashboard.rs
- Test name: worktrees_needing_attention_are_listed_with_every_reason
- RED command: `cargo test -p editor-core --test dashboard worktrees_needing_attention_are_listed_with_every_reason`
- Expected RED failure: no method `worktree_attention` on `Core`.
- Minimal GREEN change: for each Worktree from the Worktree list, read its `GitStatus` (as `core.git_status` does) and map ahead/behind/files/operation/upstream to reasons; `NoUpstream` counts `git rev-list --count <base>..HEAD`, the Base being the Worktree's own (as "Changes vs base" resolves it); a Worktree with no upstream and 0 such commits isn't listed for it.
- Refactor candidates: share the per-Worktree status read with `git_status` if duplicated.
- Mocking: none (real git via `RepoWithOrigin`).
Steps:
1. RED: `RepoWithOrigin`; main checkout clean and pushed; Worktree A with 2 uncommitted files and 1 unpushed commit; Worktree B on a new branch with no upstream and 1 commit; Worktree C left mid-rebase (conflicting `git rebase`); Worktree D tracking a branch that origin moved 1 commit ahead of (pushed from another clone, then `git fetch`). Assert main isn't listed, A has `[Uncommitted{2}, Unpushed{1}]`, B has `[NoUpstream{1}]`, C includes `Operation{Rebase}`, D has `[Behind{1}]`.
   Command: `cargo test -p editor-core --test dashboard worktrees_needing_attention_are_listed_with_every_reason`
   Expected: FAIL
2. GREEN: add `dashboard.rs` with the types and the mapping; `Core::worktree_attention`; re-export the types from lib.rs.
   Command: `cargo test -p editor-core --test dashboard worktrees_needing_attention_are_listed_with_every_reason`
   Expected: PASS
3. REFACTOR: as above, only if duplicated.
4. VERIFY:
   - `cargo test -p editor-core --test dashboard`
   - `cargo clippy -p editor-core --all-targets`
Checkpoint:
- Files: `crates/editor-core/src/dashboard.rs crates/editor-core/src/lib.rs crates/editor-core/src/core.rs crates/editor-core/tests/dashboard.rs`
- Commit: `Dashboard: Worktrees that need attention, with why`

## Task 3: Reviews requested from the user
Status: completed
Risk: medium
Dependencies: Task 2
Files:
- Modify: crates/editor-core/src/dashboard.rs
- Modify: crates/editor-core/src/pull_request.rs
- Modify: crates/editor-core/src/core.rs
- Modify: crates/editor-core/src/lib.rs
- Modify: crates/editor-core/tests/support/mod.rs
- Modify: crates/editor-core/tests/pull_request.rs
- Test: crates/editor-core/src/dashboard.rs
- Test: crates/editor-core/tests/dashboard.rs
Blast radius:
- crates/editor-core/src/pull_request.rs (`gh_text`, `latest_checks`, `Status` made `pub(crate)`)
- crates/editor-core/tests/pull_request.rs (`FakeGh`, `core_with_gh` move to support)
Acceptance:
- `Core::review_requests(worktree) -> Result<Vec<ReviewRequest>, CoreError>` runs one `gh api graphql -F q='repo:{owner}/{repo} is:pr is:open draft:false user-review-requested:@me' -f query=…` (`-F`, so `gh` fills in `{owner}/{repo}` from the Worktree's remote). The query: `viewer { login }` and `search(query: $q, type: ISSUE, first: 50)` with, per PR, number, title, url, `author { login }`, headRefName, `latestReviews(first: 50) { nodes { author { login } } }`, `timelineItems(itemTypes: REVIEW_REQUESTED_EVENT, last: 20) { nodes { createdAt requestedReviewer { ... on User { login } } } }`, and `commits(last: 1) { nodes { commit { statusCheckRollup { contexts(first: 100) { nodes { __typename ... on CheckRun { name conclusion status detailsUrl startedAt checkSuite { workflowRun { workflow { name } } } } ... on StatusContext { context state targetUrl createdAt } } } } } } }`. Whether the viewer reviewed it before is decided in Rust from `latestReviews` and `viewer.login`.
- The GraphQL check nodes are mapped into the existing `Status` (made `pub(crate)`) so `latest_checks` turns them into `Check`s as for the PR board: `startedAt` / `createdAt` fill `Status.started_at` / `created_at` (so a check's latest attempt is picked as there), and a null `checkSuite.workflowRun` (not GitHub Actions) gives `workflow_name: None`.
- `ReviewRequest { number, title, url, author, head, checks: Vec<Check>, requested_at: Option<String>, re_requested: bool }`; `requested_at` is the latest review-request event naming the viewer; `re_requested` is true when the viewer has reviewed it before. Sorted most recently asked first.
- The repo is the main Worktree's origin, as `gh` resolves it (`gh repo view --json nameWithOwner` or `{owner}/{repo}` placeholders in `gh api`).
- `gh` missing / not logged in errors come back as for `my_pull_requests`.
TDD:
- Behavior: review requests are parsed with who asked when, their checks, and whether it's a re-request.
- Test file: crates/editor-core/src/dashboard.rs (parser), crates/editor-core/tests/dashboard.rs (fake gh)
- Test name: review_requests_say_when_the_user_was_asked_and_if_again; review_requests_are_asked_of_gh_for_the_user_directly
- RED command: `cargo test -p editor-core --lib dashboard::tests::review_requests_say_when_the_user_was_asked_and_if_again`
- Expected RED failure: no function `review_requests_listed` (the parser) in `dashboard`.
- Minimal GREEN change: the query string, a `review_requests_listed(json, viewer)` parser using `latest_checks` for checks, and the Core method calling `gh_text`.
- Refactor candidates: none
- Mocking: fake `gh` script (`FakeGh`) — the real GitHub can't be reached in tests.
Steps:
0. Move `FakeGh` and `core_with_gh` from tests/pull_request.rs into tests/support/mod.rs (unchanged); `cargo test -p editor-core --test pull_request` still passes.
1. RED (parser): a GraphQL JSON with two PRs: one asked of the viewer twice (latest event wins) and already reviewed by them, one with a team-request event (no `login`) and one of the viewer. Checks: a failed CheckRun with its workflow and a pending StatusContext. Assert order, `requested_at`, `re_requested`, author and checks.
   Command: `cargo test -p editor-core --lib dashboard::tests::review_requests_say_when_the_user_was_asked_and_if_again`
   Expected: FAIL
2. GREEN: parser.
   Command: `cargo test -p editor-core --lib dashboard::tests::review_requests_say_when_the_user_was_asked_and_if_again`
   Expected: PASS
3. RED (fake gh): `FakeGh` printing the JSON; assert `core.review_requests(main)` returns them and `fake.args()` contains `api`, `graphql`, and a query with `user-review-requested:@me` and `draft:false`.
   Command: `cargo test -p editor-core --test dashboard review_requests_are_asked_of_gh_for_the_user_directly`
   Expected: FAIL, then PASS after adding `Core::review_requests`.
4. REFACTOR: none
5. VERIFY:
   - `cargo test -p editor-core --test dashboard`
   - `cargo test -p editor-core --lib dashboard`
   - `cargo clippy -p editor-core --all-targets`
Checkpoint:
- Files: `crates/editor-core/src/dashboard.rs crates/editor-core/src/pull_request.rs crates/editor-core/src/core.rs crates/editor-core/src/lib.rs crates/editor-core/tests/dashboard.rs crates/editor-core/tests/support/mod.rs crates/editor-core/tests/pull_request.rs`
- Commit: `Dashboard: reviews requested from the user`

## Task 4: Prompt for a failing check
Status: completed
Risk: low
Dependencies: Task 3
Files:
- Modify: crates/editor-core/src/dashboard.rs
- Modify: crates/editor-core/src/core.rs
- Modify: crates/editor-core/src/pull_request.rs
- Test: crates/editor-core/src/dashboard.rs
- Test: crates/editor-core/tests/dashboard.rs
Blast radius:
- crates/editor-core/src/pull_request.rs (`ActionsJob`, the jobs lookup in `rerun`, `job_id`: lifted into a `pub(crate)` helper shared with `rerun`)
Acceptance:
- `Core::failing_check_prompt(worktree, number) -> Result<String, CoreError>` reads the PR's latest checks with `gh pr view <n> --json statusCheckRollup` and `latest_checks`; for each failed one with an Actions job, looks up the job's real id as `rerun` does (`gh run view <run> --json jobs`, then `job_id`), runs `gh run view <run> --job <id> --log-failed` and keeps its last 200 lines.
- The prompt names the PR and each failed check, with its log tail in a fenced block, and asks the Agent to find and fix the cause. A failed check without an Actions job is named with its link and no log.
TDD:
- Behavior: the prompt for a failing check carries its name and the last 200 lines of the failed job's log.
- Test file: crates/editor-core/src/dashboard.rs (prompt text), crates/editor-core/tests/dashboard.rs (fake gh)
- Test name: a_failing_checks_prompt_has_its_name_and_the_logs_last_lines; failing_check_logs_are_asked_of_gh_per_failed_job
- RED command: `cargo test -p editor-core --lib dashboard::tests::a_failing_checks_prompt_has_its_name_and_the_logs_last_lines`
- Expected RED failure: no function `failing_check_text` in `dashboard`.
- Minimal GREEN change: `failing_check_text(pr, &[(Check, Option<String>)])` keeping the tail; Core method fetching checks (`pr view --json statusCheckRollup`), the job id (helper lifted from `rerun`) and logs (via `gh_text`).
- Refactor candidates: none
- Mocking: fake `gh` (`case` on `pr view` / `run view … --json jobs` / `run view … --log-failed`).
Steps:
1. RED: a 250-line log; assert the prompt has the check name, line 51 to 250, not line 50.
   Command: `cargo test -p editor-core --lib dashboard::tests::a_failing_checks_prompt_has_its_name_and_the_logs_last_lines`
   Expected: FAIL
2. GREEN: `failing_check_text`.
   Command: same
   Expected: PASS
3. RED/GREEN (fake gh): a fake answering `pr view` with one failed Actions check, `run view --json jobs` with the job's real id (different from the link's), and `run view --log-failed` with a log; assert `--log-failed` and the looked-up id were passed and the log is in the prompt; `cargo test -p editor-core --test pull_request` still passes (the shared lookup).
   Command: `cargo test -p editor-core --test dashboard failing_check_logs_are_asked_of_gh_per_failed_job`
   Expected: FAIL, then PASS
4. VERIFY:
   - `cargo test -p editor-core --test dashboard`
   - `cargo clippy -p editor-core --all-targets`
Checkpoint:
- Files: `crates/editor-core/src/dashboard.rs crates/editor-core/src/core.rs crates/editor-core/src/pull_request.rs crates/editor-core/tests/dashboard.rs`
- Commit: `Dashboard: a prompt for a PR's failing checks`

## Task 5: Prompt for review feedback
Status: completed
Risk: medium
Dependencies: Task 4
Files:
- Modify: crates/editor-core/src/dashboard.rs
- Modify: crates/editor-core/src/core.rs
- Test: crates/editor-core/src/dashboard.rs
- Test: crates/editor-core/tests/dashboard.rs
Blast radius:
- crates/editor-core/src/pull_request.rs (read only: `SubmittedReview`)
Acceptance:
- `Core::review_feedback_prompt(worktree, number) -> Result<String, CoreError>` collects the bodies of the latest review per reviewer that requests changes (from `gh pr view --json reviews`) and the unresolved review threads (`gh api graphql`, `pullRequest(number) { reviewThreads(first: 100) { nodes { isResolved path line originalLine comments(first: 50) { nodes { author { login } body } } } } }`).
- The prompt lists each review body with its author, then each unresolved thread as `path:line` with its comments in order, and asks the Agent to address them. Resolved threads and approving or commenting reviews aren't in it.
TDD:
- Behavior: the feedback prompt has the change-requesting reviews and only the unresolved threads, with file and line.
- Test file: crates/editor-core/src/dashboard.rs, crates/editor-core/tests/dashboard.rs
- Test name: feedback_prompt_has_change_requests_and_unresolved_threads_only; review_threads_are_asked_of_gh_with_their_resolved_state
- RED command: `cargo test -p editor-core --lib dashboard::tests::feedback_prompt_has_change_requests_and_unresolved_threads_only`
- Expected RED failure: no function `review_feedback_text` in `dashboard`.
- Minimal GREEN change: a thread parser for the GraphQL JSON, `review_feedback_text(pr, reviews, threads)`, and the Core method.
- Refactor candidates: none
- Mocking: fake `gh` (`case` on `pr view` / `api graphql`).
Steps:
1. RED: reviews (one changes-requested, one approved, one changes-requested later superseded by the same reviewer's approval) and threads (one resolved, one unresolved with a reply). Assert only the live change request and the unresolved thread with `path:line` and both comments.
   Command: `cargo test -p editor-core --lib dashboard::tests::feedback_prompt_has_change_requests_and_unresolved_threads_only`
   Expected: FAIL
2. GREEN: parser and text.
   Command: same
   Expected: PASS
3. RED/GREEN (fake gh): assert the GraphQL query asks for `isResolved` and the PR number, and the prompt comes back.
   Command: `cargo test -p editor-core --test dashboard review_threads_are_asked_of_gh_with_their_resolved_state`
   Expected: FAIL, then PASS
4. VERIFY:
   - `cargo test -p editor-core --test dashboard`
   - `cargo clippy -p editor-core --all-targets`
Checkpoint:
- Files: `crates/editor-core/src/dashboard.rs crates/editor-core/src/core.rs crates/editor-core/tests/dashboard.rs`
- Commit: `Dashboard: a prompt for a PR's review feedback`

## Task 6: Tauri commands and TypeScript bindings
Status: pending
Risk: low
Dependencies: Task 1, Task 2, Task 3, Task 4, Task 5
Files:
- Modify: src-tauri/src/main.rs
- Modify: src/core.ts
Blast radius:
- src/SettingsPage.tsx, src/App.tsx (consume the new types later)
Acceptance:
- Commands `worktree_attention`, `review_requests`, `failing_check_prompt`, `review_feedback_prompt` registered in `generate_handler!` (main.rs ~1432), each forwarding to `Core` as the existing PR commands do.
- `core.ts` has `WorktreeAttention`, `AttentionReason`, `ReviewRequest` types, the four functions, `dashboard: { openOnStart: boolean }` on `Settings`, and `{ kind: "dashboardOpenOnStart"; on: boolean }` on `SettingChange`.
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
1. Add the four commands to main.rs next to `my_pull_requests`.
2. Add the types and functions to core.ts next to `myPullRequests`; extend `Settings` and `SettingChange`.
3. VERIFY:
   - `cargo build -p orchard` (the src-tauri crate; use its package name from src-tauri/Cargo.toml)
   - `pnpm typecheck`
Checkpoint:
- Files: `src-tauri/src/main.rs src/core.ts`
- Commit: `Dashboard: commands for the frontend`

## Task 7: Frontend data watching
Status: pending
Risk: medium
Dependencies: Task 6
Files:
- Modify: src/PrBoard.tsx
- Create: src/dashboardData.ts
- Modify: src/App.tsx
Blast radius:
- src/PrBoard.tsx (PR board shares `myPrs`)
Acceptance:
- `myPrs` exposes `refreshedAt: () => number | null`, set when a load succeeds; the PR board is unchanged otherwise.
- `dashboardData.ts` exports `watchReviewRequests(worktree)` (same shape as `watchMyPrs`: signals, 5-minute interval, stale-answer guard, `refreshedAt`) and `watchAttention()` (reloads `core.worktreeAttention()` on `worktreesChanged` and `gitStatusChanged` events, coalesced to one call per 300 ms).
- It also exports `watchCleanUp()`: `core.mergeOverview()` reloaded on `worktreesChanged` and by `refresh()`, keeping the rows whose merge is `merged`, `changesInBase` or `remoteDeleted`.
- App calls all three once beside `watchMyPrs(mainWorktree)`, and refreshes PRs and review requests on window focus.
TDD:
- Behavior: no frontend test framework; verified by type check and a run of the app.
- Test file: none
- Test name: none
- RED command: `pnpm typecheck`
- Expected RED failure: none expected.
- Minimal GREEN change: as acceptance.
- Refactor candidates: share the poll loop between `watchMyPrs` and `watchReviewRequests` if they end up identical.
- Mocking: none
Steps:
1. Add `refreshedAt` to `watchMyPrs`.
2. Write `dashboardData.ts`.
3. Wire in App.tsx (~482) and a `focus` listener with `onCleanup`.
4. VERIFY:
   - `pnpm typecheck`
   - Check: open the PR board; it still loads and refreshes.
Checkpoint:
- Files: `src/PrBoard.tsx src/dashboardData.ts src/App.tsx`
- Commit: `Dashboard: watch review requests and Worktrees that need attention`

## Task 8: The Dashboard view and its sections
Status: pending
Risk: medium
Dependencies: Task 7
Files:
- Create: src/Dashboard.tsx
- Modify: src/styles.css (or the stylesheet the Board uses; match its file)
Blast radius:
- none (new component)
Acceptance:
- `Dashboard` renders, in order: Needs you (sessions in Needs you), Reviews requested (title, author, "asked 3h ago", checks summary, "re-requested" tag), Your PRs (open ones grouped Changes requested → Checks failing → Approved → Waiting for review → Draft, "All PRs →"), Worktrees that need attention (branch and reasons joined with " · "), Ready to clean up (merged / remote deleted, from `core.mergeOverview()`), Sessions (Working and Idle counts).
- An empty section is one line ("No reviews requested").
- GitHub sections show "refreshed 2m ago" and a refresh button; on a `gh` error they show the error inline and the other sections render as usual. The core's `GH_HINT` speaks of creating a PR; Task 3 adds a Dashboard-neutral hint (install `gh`, `gh auth login`) used by `review_requests`, and the Dashboard shows that same hint for a `myPrs` error that carries `GH_HINT`.
- Callbacks are props (`onSession`, `onPr`, `onReviewRequest`, `onWorktree`, `onCleanUp`, `onAllPrs`, `onStartSession`); the component holds no navigation logic.
TDD:
- Behavior: no frontend test framework; verified by type check and a run of the app.
- Test file: none
- Test name: none
- RED command: `pnpm typecheck`
- Expected RED failure: none expected.
- Minimal GREEN change: as acceptance, styled like the Board's cards and the PR board's stage groups.
- Refactor candidates: reuse PR board's check-summary / stage rendering if exported.
- Mocking: none
Steps:
1. Write `Dashboard.tsx` following Board.tsx / PrBoard.tsx conventions (icons from `./icons`, `plural`, relative times as PrBoard formats them).
2. Styles beside the Board's.
3. VERIFY:
   - `pnpm typecheck`
Checkpoint:
- Files: `src/Dashboard.tsx src/styles.css`
- Commit: `Dashboard: the view and its sections`

## Task 9: Dashboard as a view of the window
Status: pending
Risk: high
Dependencies: Task 8
Files:
- Modify: src/App.tsx
- Modify: src/SettingsPage.tsx
Blast radius:
- src/App.tsx view switching (`mainView`, `boardOpen`, `chooseView`, `show`), Ctrl+B, Escape, restore of saved Tabs
- src/Board.tsx, src/PrBoard.tsx (share the board slot)
Acceptance:
- A `dashboardOpen` signal; `view()` returns `"dashboard"` first when it's set. The switcher reads Dashboard | Tabs | Columns | Board, Dashboard first, with a badge when not on it: sessions in Needs you + review requests + the user's open PRs in Changes requested or Checks failing.
- Ctrl+D toggles the Dashboard with the view that was showing before (Tabs, Columns, or the Board); choosing Tabs/Columns/Board leaves it. It's handled only when `!e.defaultPrevented` and focus isn't in the Terminal panel, so the Manual editor's Ctrl+D (next occurrence, which CodeMirror prevents) and the shell's EOF keep working; TerminalPanel does not pass Ctrl+D through.
- Each time `view()` becomes `"dashboard"`, PRs, review requests and the clean-up list are refreshed.
- `WorkspaceView` starts with the Dashboard shown when `settings.dashboard.openOnStart` is true (read after `await core.settings()` in `onMount`), on every Workspace open. Restoring saved Tabs (`show(...)` in the restore) doesn't hide it; `show()` from a user action does.
- The shortcut list in SettingsPage has `["Ctrl D", "Open or close the Dashboard"]`.
TDD:
- Behavior: no frontend test framework; verified by type check and a run of the app.
- Test file: none
- Test name: none
- RED command: `pnpm typecheck`
- Expected RED failure: none expected.
- Minimal GREEN change: as acceptance; `show(id, { quiet: true })` (or a separate restore path) for the restore so it doesn't close the Dashboard.
- Refactor candidates: none
- Mocking: none
Steps:
1. Add the signal, `view()` case, switcher entry and badge (App.tsx ~215–223, ~933–953).
2. Ctrl+D beside Ctrl+B (~720) with the guard above; Escape doesn't close the Dashboard (it's a view, not an overlay). A `createEffect(on(view, …, { defer: true }))` refreshes the GitHub data and clean-up list on showing it (deferred, so a window opening on the Dashboard doesn't ask `gh` twice).
3. Open-on-start in `onMount`; restore path that leaves it shown.
4. Shortcut list entry.
5. VERIFY:
   - `pnpm typecheck`
   - Run the app (`pnpm tauri dev`): opening a repo lands on the Dashboard, with saved Tabs too; switching Workspace lands on it; Ctrl+D toggles back to Tabs, Columns and the Board, but not from inside the Manual editor or the Terminal panel; Ctrl+B, Escape on the Board, and the Columns view still behave as before; with `open_on_start = false` it opens on Tabs.
Checkpoint:
- Files: `src/App.tsx src/SettingsPage.tsx`
- Commit: `Dashboard: a view of the window, shown when a Workspace opens`

## Task 10: Where the Dashboard's items lead
Status: pending
Risk: medium
Dependencies: Task 9
Files:
- Modify: src/App.tsx
Blast radius:
- src/App.tsx (`show`, `openMyPr`, `setPrOf`, `selectWorktree`, `setDrawer`, `setOverviewOpen`, `openBoard`)
Acceptance:
- A session opens its Tab (`show(id)`), in the last-used main view (Tabs or Columns; Tabs the first time).
- A PR of the user's opens the PR overlay via `openMyPr`; a review request opens it with `{ worktree: mainWorktree(), number }`.
- A Worktree that needs attention: leave the Dashboard, `selectWorktree(path)`, `setDrawer("git")`.
- A clean-up row and its section header open the Worktrees overview; "All PRs →" opens the PR board.
TDD:
- Behavior: no frontend test framework; verified by type check and a run of the app.
- Test file: none
- Test name: none
- RED command: `pnpm typecheck`
- Expected RED failure: none expected.
- Minimal GREEN change: pass the existing App functions as the Dashboard's props.
- Refactor candidates: none
- Mocking: none
Steps:
1. Render `<Dashboard>` for `view() === "dashboard"` with the callbacks.
2. VERIFY:
   - `pnpm typecheck`
   - Run the app: each kind of item goes where the spec says; from Columns, a session opens in Columns.
Checkpoint:
- Files: `src/App.tsx`
- Commit: `Dashboard: items open where they live`

## Task 11: Start an Agent session on a PR
Status: pending
Risk: high
Dependencies: Task 10
Files:
- Modify: src/Chat.tsx
- Modify: src/App.tsx
Blast radius:
- src/Chat.tsx (Composer drafts map)
- src/App.tsx (newSession, createWorktree flow, setupChanged handling ~1018–1031)
Acceptance:
- `Chat.tsx` exports `setDraft(session, text)` writing the drafts map before the Composer mounts.
- "Start an Agent session" on a PR in Checks failing asks `core.failingCheckPrompt`, in Changes requested `core.reviewFeedbackPrompt`; then: the Worktree on `pr.head` gets a new session; otherwise `core.createWorktree({ kind: "existingBranch", name: "origin/" + head })`, and when it has a Worktree setup, the session the setup starts (its `setupChanged` `done`) is used. The prompt is set as the session's draft, then its Tab is shown. Nothing is sent.
- A failure (prompt fetch, `BranchCheckedOut`, setup failed) is shown on the Dashboard row, not thrown away.
TDD:
- Behavior: no frontend test framework; the prompt text is covered by Tasks 4–5; verified by type check and a run of the app.
- Test file: none
- Test name: none
- RED command: `pnpm typecheck`
- Expected RED failure: none expected.
- Minimal GREEN change: as acceptance.
- Refactor candidates: share the create-Worktree-then-session flow with the sidebar's if it can be lifted out cleanly.
- Mocking: none
Steps:
1. Export `setDraft` from Chat.tsx.
2. `startSessionOn(pr)` in App.tsx; pass it as `onStartSession`.
3. VERIFY:
   - `pnpm typecheck`
   - Run the app against a repo with a PR whose checks fail: the Tab opens with the prompt in the composer, unsent; with no Worktree on the branch, one is created first (setup runs).
Checkpoint:
- Files: `src/Chat.tsx src/App.tsx`
- Commit: `Dashboard: start an Agent session on a failing or reviewed PR`

## Task 12: Settings toggle and final verification
Status: pending
Risk: low
Dependencies: Task 11
Files:
- Modify: src/SettingsPage.tsx
- Modify: .scratch/agent-editor-build/issues/43-dashboard.md
Blast radius:
- none
Acceptance:
- General settings have a "Open the Dashboard when a repository opens" toggle writing `dashboardOpenOnStart`.
- Issue 43's checklist is ticked for what's done, with its status line saying what was and wasn't tried (as issue 42 does).
TDD:
- Behavior: no frontend test framework; verified by type check and a run of the app.
- Test file: none
- Test name: none
- RED command: `pnpm typecheck`
- Expected RED failure: none expected.
- Minimal GREEN change: a `<Toggle>` like the `turnFinished` one (SettingsPage.tsx:145–150).
- Refactor candidates: none
- Mocking: none
Steps:
1. Add the toggle.
2. Update issue 43.
3. VERIFY:
   - `cargo test --workspace`
   - `cargo clippy --workspace --all-targets`
   - `cargo fmt --all --check`
   - `pnpm typecheck`
   - Run the app once through every acceptance line of issue 43.
Checkpoint:
- Files: `src/SettingsPage.tsx .scratch/agent-editor-build/issues/43-dashboard.md`
- Commit: `Dashboard: settings toggle; issue 43 done`

## Environment Context
- **Language:** Rust (edition per crates' Cargo.toml) core + Tauri 2; TypeScript with SolidJS frontend (Vite)
- **Test command:** `cargo test --workspace` (focused: `cargo test -p editor-core --test dashboard <name>` / `cargo test -p editor-core --lib dashboard::tests::<name>`)
- **Linter command:** `cargo clippy --workspace --all-targets`
- **Formatter command:** `cargo fmt --all`
- **Build command:** `pnpm typecheck` (frontend), `pnpm tauri build` (app); run with `pnpm tauri dev`
- **Branch:** dashboard (fast-forward main when done)
- **Conventional commit style:** "Area: sentence" (e.g. "PR checks: only a check's latest attempt counts"), ending with the Co-Authored-By line

**Codebase conventions:** Core owns state and logic (ADR 0003); git and gh through their CLIs only (ADR 0004). Errors from gh are `String`s wrapped as `CoreError::Git`. Serde `rename_all = "camelCase"`, tagged enums `tag = "kind"`. Integration tests in `crates/editor-core/tests/*.rs` with `support/mod.rs` helpers (`RepoWithOrigin`, `git`, `commit`, `eventually`), `#[tokio::test(flavor = "multi_thread")]`, `#![cfg(unix)]` for fake-`gh` tests; parser unit tests on JSON strings in the module. Frontend: views are signals and `<Show>` in App.tsx; module-level signals for shared polling (PrBoard's `myPrs`); comments are prose sentences in the glossary's words.

**Graph Context:**
- Blast radius: ~14 files | Hub nodes: src/App.tsx, crates/editor-core/src/core.rs (graph server unavailable; from reading)
- Bridge nodes: src-tauri/src/main.rs, src/core.ts | Communities crossed: core, tauri glue, frontend

# Plan: Code review (issue 44)

Spec: `.scratch/agent-editor-build/issues/44-code-review.md`. Glossary: `CONTEXT.md` (Worktree, Agent session, Tab, Base, Worktrees overview, Worktree setup, Manual editor, Dashboard; new: Code review, Review Worktree, Pending review).

The core owns the Code review (ADR 0003): the Review Worktree, the diff, the Pending review and its submission, Viewed ticks, the review Agent session and its prompts, and the Plan-mode lock. Everything GitHub goes through `gh` (as `pull_request.rs` does), git through `git.rs` (ADR 0004). A new module `code_review.rs` holds the model; `Core` methods in `core.rs` wrap it. Core tasks are test-first in `crates/editor-core/tests/code_review.rs` (real git via `RepoWithOrigin`, PR heads pushed to `refs/pull/N/head` of the bare origin, `FakeGh` for GitHub, `FakeAgent` for the Agent). The frontend has no test framework; its tasks are verified by `pnpm typecheck` and a run of the app.

Decisions made while planning (beyond the spec):
- Replies to existing threads are posted at once (GitHub's "Add single comment"); the REST create-review call can't carry replies.
- A pending comment that went outdated on Refresh must be moved or removed before Submit; Submit says how many are outdated.
- The fetched PR head is kept under `refs/orchard/review/<N>` so it outlives `git gc` and a force-push of the PR.
- New commits are found by the overlay polling `code_review_new_commits` (one `git ls-remote`) every 60 s while it's open, like the Dashboard's polling.

## Task 1: git helpers for fetching a PR head, detached Worktrees and hard resets
Status: completed
Risk: low
Dependencies: none
Files:
- Modify: crates/editor-core/src/git.rs
- Test: crates/editor-core/src/git.rs
Blast radius:
- crates/editor-core/src/create_worktree.rs (uses `worktree_add`)
Acceptance:
- `git::fetch_ref(repo, remote_ref, local_ref) -> Result<String, String>` runs `git fetch origin +<remote_ref>:<local_ref>` (with the login-shell env, as `fetch_origin` does) and returns the fetched commit id.
- `git::ls_remote_ref(repo, remote_ref) -> Result<Option<String>, String>` returns the remote ref's commit id, `None` if it doesn't exist.
- `git::worktree_add_detached(repo, path, commit)` runs `git worktree add --detach <path> <commit>`.
- `git::reset_hard(worktree, commit)` runs `git reset --hard <commit>`.
- `git::merge_base_of(repo, a, b) -> Result<String, String>` runs `git merge-base <a> <b>` (the existing `merge_base(worktree, base)` at git.rs:1315 always uses `HEAD`, so it doesn't fit).
TDD:
- Behavior: a PR head pushed to `refs/pull/7/head` of an origin can be fetched into a local ref and checked out detached in a new Worktree, then reset to a later head.
- Test file: crates/editor-core/src/git.rs
- Test name: a_pr_head_is_fetched_checked_out_detached_and_reset
- RED command: `cargo test -p editor-core --lib git::tests::a_pr_head_is_fetched_checked_out_detached_and_reset`
- Expected RED failure: no function `fetch_ref` in module `git`.
- Minimal GREEN change: the four wrappers over `run`/`worktree_add` (git.rs:55, :213).
- Refactor candidates: none
- Mocking: none (a temp bare origin and clone built in the test with `git init --bare` and `tempfile`)
Steps:
1. RED: create `#[cfg(test)] mod tests` in git.rs (there is none yet; the helpers are async, so `#[tokio::test]`); build origin + clone, push a commit to `refs/pull/7/head`, assert `ls_remote_ref` gives its id, `fetch_ref` into `refs/orchard/review/7` returns it, `worktree_add_detached` makes a Worktree whose `HEAD` is that id and `symbolic-ref` fails (detached); push a second commit, fetch, `reset_hard`, assert the new `HEAD`.
   Command: `cargo test -p editor-core --lib git::tests::a_pr_head_is_fetched_checked_out_detached_and_reset`
   Expected: FAIL
2. GREEN: add the wrappers (and `merge_base_of`, asserted in the same test as the fork point of the PR branch and `main`).
   Command: `cargo test -p editor-core --lib git::tests::a_pr_head_is_fetched_checked_out_detached_and_reset`
   Expected: PASS
3. REFACTOR: none
4. VERIFY:
   - `cargo test -p editor-core --lib git`
   - `cargo clippy -p editor-core --all-targets`
Checkpoint:
- Files: `crates/editor-core/src/git.rs`
- Commit: `Git: fetch a pull request's head and check it out detached`

## Task 2: PR data a Code review needs
Status: completed
Risk: medium
Dependencies: none
Files:
- Modify: crates/editor-core/src/pull_request.rs
- Test: crates/editor-core/src/pull_request.rs
- Test: crates/editor-core/tests/pull_request.rs
Blast radius:
- src/core.ts (PullRequestDetails, CodeThread types; Task 13)
- src/PrOverlay.tsx (reads threads)
Acceptance:
- `PullRequestDetails` gains `id` (GraphQL node id), `head_oid`, `base_oid`, `viewer_is_reviewer: bool` (requested of the viewer, or the viewer has reviewed), `viewer_last_review_oid: Option<String>` (the commit of the viewer's latest submitted review).
- `CodeThread` gains `side: DiffSide` (`Left`/`Right`, serde `"left"`/`"right"`), `start_line: Option<u32>`, `outdated: bool`; each `ThreadComment` gains `id: u64`. `THREAD_JQ` carries them.
- Existing PR overlay tests still pass.
TDD:
- Behavior: a PR's details carry its head and base commits, whether the user is its reviewer, and the ids and sides of its code threads.
- Test file: crates/editor-core/tests/pull_request.rs
- Test name: details_carry_what_a_code_review_needs
- RED command: `cargo test -p editor-core --test pull_request details_carry_what_a_code_review_needs`
- Expected RED failure: no field `head_oid` on `PullRequestDetails`.
- Minimal GREEN change: add `id,headRefOid,baseRefOid` to `DETAIL_FIELDS` (:658); viewer login from `gh api user --jq .login`, read fresh in `details_for` alongside the threads (no cache in Core, as teams aren't cached either); `viewer_is_reviewer`/`viewer_last_review_oid` from `reviewRequests` and `reviews[].{author.login,commit.oid}`; `THREAD_JQ` emits `id`, `side`, `start_line`, `position == null` as outdated.
- Refactor candidates: none
- Mocking: FakeGh (GitHub is external)
Steps:
1. RED: FakeGh answering `pr view` with the new fields, `api user` with a login, and `api …/comments` with two comments (one a reply, one `side: LEFT`, one with `start_line`); assert the mapped fields.
   Command: `cargo test -p editor-core --test pull_request details_carry_what_a_code_review_needs`
   Expected: FAIL
2. GREEN: as above.
   Command: `cargo test -p editor-core --test pull_request`
   Expected: PASS
3. REFACTOR: none
4. VERIFY:
   - `cargo test -p editor-core --test pull_request`
   - `cargo test -p editor-core --test dashboard`
   - `cargo clippy -p editor-core --all-targets`
Checkpoint:
- Files: `crates/editor-core/src/pull_request.rs crates/editor-core/tests/pull_request.rs`
- Commit: `PR overlay: code threads know their side and ids, PRs their head commit`

## Task 3: Opening a Code review makes its Review Worktree
Status: completed
Risk: high
Dependencies: Task 1, Task 2
Files:
- Create: crates/editor-core/src/code_review.rs
- Modify: crates/editor-core/src/lib.rs
- Modify: crates/editor-core/src/core.rs
- Modify: crates/editor-core/src/create_worktree.rs
- Modify: crates/editor-core/src/app_state.rs
- Modify: crates/editor-core/src/worktrees.rs
- Modify: crates/editor-core/tests/support/mod.rs
- Create: crates/editor-core/tests/code_review.rs
Blast radius:
- crates/editor-core/src/core.rs (create_worktree :1938, start_setup :1971, persist :5165, restore :5239 — hub)
- crates/editor-core/src/worktrees.rs (Worktree list shows the new one)
- src/core.ts (WorktreeInfo :141; Task 13)
Acceptance:
- `Core::open_code_review(number) -> Result<CodeReview, CoreError>`: refuses with `CoreError::NotAReviewer` unless `viewer_is_reviewer`; fetches `refs/pull/<N>/head` into `refs/orchard/review/<N>`; reuses the Workspace's saved Review Worktree for N if its folder exists, else makes `<repo>.worktrees/review-pr-<N>` (via `free_folder`) detached at the head (`NewWorktree::Detached { name, commit }`); emits `WorktreesChanged`.
- Worktree setup starts on open unless this Review Worktree's setup has finished before; the outcome (`Done`) is saved, so after a restart a finished setup isn't rerun and a failed or unfinished one is. Opening doesn't start an Agent session even though setup normally ends in one (`open_first_session` is skipped for Review Worktrees).
- `WorkspaceState.code_reviews: BTreeMap<u64, SavedCodeReview>` (`#[serde(default, skip_serializing_if = "BTreeMap::is_empty")]`) keeps `{ worktree, head, setup_done }`; `persist`/`restore` carry it.
- `CodeReview` returns `{ number, title, worktree, head, base, merge_base, files: Vec<ReviewFile { path, change, additions, deletions }>, setup: Option<SetupInfo> }`.
- `Core::is_review_worktree(path) -> Option<u64>`; `WorktreeInfo` gains `review: Option<u64>` (the PR number of a Review Worktree), filled wherever the Worktree list is built, so the frontend can badge it.
- Test helper `core_with_gh_and_state(fake, gh, state_path)` in `tests/support/mod.rs` (`core_with_state` takes no gh, `core_with_gh` no state path), used by every restart test of this plan.
TDD:
- Behavior: opening a Code review of PR 7 makes a detached Review Worktree at the PR's head, once, and runs its setup only until it has finished.
- Test file: crates/editor-core/tests/code_review.rs
- Test name: opening_a_code_review_makes_its_review_worktree_once
- RED command: `cargo test -p editor-core --test code_review opening_a_code_review_makes_its_review_worktree_once`
- Expected RED failure: no method `open_code_review` on `Core`.
- Minimal GREEN change: as acceptance; changed files from `git diff --numstat --name-status <merge_base> <head>` through `git.rs`.
- Refactor candidates: share the fetch+create with `create_worktree` only if it stays readable.
- Mocking: FakeGh for `pr view` (GitHub); FakeAgent present but unused.
Steps:
1. RED: `RepoWithOrigin`, push a branch to `refs/pull/7/head`; FakeGh with `viewer_is_reviewer` true and repo settings with one setup command that touches a file. Assert: the result's `worktree` ends in `review-pr-7`, `HEAD` is the PR head, detached; the setup ran; a second `open_code_review(7)` returns the same Worktree and doesn't rerun setup; a new Core from the same `state_path` (`core_with_gh_and_state`) doesn't rerun it either; the Worktree list shows it with `review == Some(7)`. Second test `a_pr_the_user_doesnt_review_wont_open` asserting `NotAReviewer`. Third test `a_failed_setup_runs_again_on_the_next_open` with a setup command `exit 1` the first time (it checks for a marker file it then creates).
   Command: `cargo test -p editor-core --test code_review`
   Expected: FAIL
2. GREEN: as acceptance.
   Command: `cargo test -p editor-core --test code_review`
   Expected: PASS
3. REFACTOR: none
4. VERIFY:
   - `cargo test -p editor-core --test code_review --test create_worktree --test worktree_setup --test close_reopen_restore`
   - `cargo clippy -p editor-core --all-targets`
Checkpoint:
- Files: `crates/editor-core/src/code_review.rs crates/editor-core/src/lib.rs crates/editor-core/src/core.rs crates/editor-core/src/create_worktree.rs crates/editor-core/src/app_state.rs crates/editor-core/src/worktrees.rs crates/editor-core/tests/support/mod.rs crates/editor-core/tests/code_review.rs`
- Commit: `Code review: opening one makes a read-only Review Worktree at the PR's head`

## Task 4: The Code review's diffs, full and since the last review
Status: completed
Risk: medium
Dependencies: Task 3
Files:
- Modify: crates/editor-core/src/code_review.rs
- Modify: crates/editor-core/src/core.rs
- Test: crates/editor-core/tests/code_review.rs
Blast radius:
- crates/editor-core/src/diff.rs (read only: `DiffLine` parsing)
Acceptance:
- `Core::code_review_diff(number, path, since_last_review: bool) -> Result<Vec<DiffLine>, CoreError>` diffs `refs/orchard/review/<N>` against the merge base (or against `viewer_last_review_oid` when asked), never the working tree: an edit in the Review Worktree doesn't show.
- `CodeReview` gains `since_last_review: Option<SinceLastReview { commit, files }>`, present when the viewer reviewed before and that commit is reachable (fetched if missing); absent otherwise.
TDD:
- Behavior: the diff is the PR's own, and a re-review can narrow it to what changed since the user's last review.
- Test file: crates/editor-core/tests/code_review.rs
- Test name: the_diff_is_the_prs_and_can_narrow_to_since_the_last_review
- RED command: `cargo test -p editor-core --test code_review the_diff_is_the_prs_and_can_narrow_to_since_the_last_review`
- Expected RED failure: no method `code_review_diff` on `Core`.
- Minimal GREEN change: `git diff <base> refs/orchard/review/<N> -- <path>` through the existing diff parser.
- Refactor candidates: none
- Mocking: FakeGh (GitHub)
Steps:
1. RED: PR with commits A (file a, b) then B (file b); FakeGh says the viewer last reviewed at A. Assert the full diff of `b` has both changes, the since diff only B's, `since_last_review.files == ["b"]`; write junk into the Worktree's `a` and assert the diff of `a` is unchanged.
   Command: `cargo test -p editor-core --test code_review the_diff_is_the_prs_and_can_narrow_to_since_the_last_review`
   Expected: FAIL
2. GREEN: as acceptance.
   Command: `cargo test -p editor-core --test code_review`
   Expected: PASS
3. REFACTOR: none
4. VERIFY:
   - `cargo test -p editor-core --test code_review`
   - `cargo clippy -p editor-core --all-targets`
Checkpoint:
- Files: `crates/editor-core/src/code_review.rs crates/editor-core/src/core.rs crates/editor-core/tests/code_review.rs`
- Commit: `Code review: the PR's diff, or only what changed since my last review`

## Task 5: The Pending review
Status: completed
Risk: medium
Dependencies: Task 3
Files:
- Modify: crates/editor-core/src/code_review.rs
- Modify: crates/editor-core/src/core.rs
- Modify: crates/editor-core/src/app_state.rs
- Test: crates/editor-core/src/code_review.rs
- Test: crates/editor-core/tests/code_review.rs
Blast radius:
- crates/editor-core/src/core.rs (persist/restore)
Acceptance:
- `PendingComment { id, path, side, start_line: Option<u32>, line, body, line_text, outdated: bool, from_question: bool }`; `Question { path, side, start_line, line, attachment }` (the gutter marker of an "Ask Claude").
- `Core::add_review_comment(number, NewReviewComment) -> PendingComment`, `edit_review_comment(number, id, body)`, `remove_review_comment(number, id)`, `add_review_question(number, Question)`; each persists and emits `CoreEvent::PendingReviewChanged { number }`.
- `CodeReview` gains `pending: PendingReview { comments, questions }`; it survives a restart.
- A comment on a line not in the diff is refused (`CoreError::NotInDiff`).
TDD:
- Behavior: comments on lines and ranges are kept as a Pending review across a restart.
- Test file: crates/editor-core/tests/code_review.rs
- Test name: the_pending_review_survives_a_restart
- RED command: `cargo test -p editor-core --test code_review the_pending_review_survives_a_restart`
- Expected RED failure: no method `add_review_comment` on `Core`.
- Minimal GREEN change: `SavedCodeReview.pending`; `line_text` read from the diff at add time.
- Refactor candidates: none
- Mocking: FakeGh (GitHub)
Steps:
1. RED: add a single-line and a range comment and a question, edit one, remove none; new Core from the same `state_path`; `open_code_review(7)` returns them. Unit test `a_comment_off_the_diff_is_refused` in `code_review.rs`.
   Command: `cargo test -p editor-core --test code_review the_pending_review_survives_a_restart`
   Expected: FAIL
2. GREEN: as acceptance.
   Command: `cargo test -p editor-core --test code_review && cargo test -p editor-core --lib code_review`
   Expected: PASS
3. REFACTOR: none
4. VERIFY:
   - `cargo test -p editor-core --test code_review --lib`
   - `cargo clippy -p editor-core --all-targets`
Checkpoint:
- Files: `crates/editor-core/src/code_review.rs crates/editor-core/src/core.rs crates/editor-core/src/app_state.rs crates/editor-core/tests/code_review.rs`
- Commit: `Code review: comments on lines are kept as a Pending review`

## Task 6: Submitting the Pending review
Status: completed
Risk: medium
Dependencies: Task 5
Files:
- Modify: crates/editor-core/src/code_review.rs
- Modify: crates/editor-core/src/core.rs
- Test: crates/editor-core/tests/code_review.rs
Blast radius:
- none beyond the module
Acceptance:
- `Core::submit_review(number, Verdict { Approve | RequestChanges | Comment }, summary) -> Result<(), CoreError>` sends one `gh api -X POST repos/{owner}/{repo}/pulls/<N>/reviews --input -` with `{ commit_id: head, event, body, comments: [{ path, side, line, start_line?, start_side?, body }] }`.
- Refused with `CoreError::OutdatedComments { count }` while any comment is outdated, and `RequestChanges`/`Comment` with an empty summary and no comments is refused as GitHub does.
- On success the Pending review is emptied, `submitted` is saved on the `SavedCodeReview`, `PendingReviewChanged` emitted.
- On failure the Pending review is unchanged and the error is `CoreError::SubmitFailed { message, paths: Vec<String> }`, `paths` being the comments' paths GitHub's error names (422 `errors[].message` mentions the path or the line).
TDD:
- Behavior: Submit sends the verdict, summary and every comment in one call, and a failure keeps them all.
- Test file: crates/editor-core/tests/code_review.rs
- Test name: submit_sends_one_call_and_a_failure_keeps_the_pending_review
- RED command: `cargo test -p editor-core --test code_review submit_sends_one_call_and_a_failure_keeps_the_pending_review`
- Expected RED failure: no method `submit_review` on `Core`.
- Minimal GREEN change: `gh_text` with stdin JSON (add a stdin variant of `run_gh` if missing).
- Refactor candidates: none
- Mocking: FakeGh (GitHub); its `script_tail` saves stdin with `cat > "$dir/stdin"` and fails only while a `$dir/failed-once` marker is absent (creating it), so the first call fails and the second succeeds
Steps:
1. RED: FakeGh that first fails with a 422 body naming `src/b.rs`, then succeeds; assert the first submit errors with that path and the pending comments are still there; the second sends JSON with the head commit, `REQUEST_CHANGES`, the summary and both comments (one with `start_line`), and empties the Pending review.
   Command: `cargo test -p editor-core --test code_review submit_sends_one_call_and_a_failure_keeps_the_pending_review`
   Expected: FAIL
2. GREEN: as acceptance.
   Command: `cargo test -p editor-core --test code_review`
   Expected: PASS
3. REFACTOR: none
4. VERIFY:
   - `cargo test -p editor-core --test code_review`
   - `cargo clippy -p editor-core --all-targets`
Checkpoint:
- Files: `crates/editor-core/src/code_review.rs crates/editor-core/src/core.rs crates/editor-core/tests/code_review.rs`
- Commit: `Code review: Approve, Request changes or Comment, sent in one go`

## Task 7: Replying to an existing thread
Status: completed
Risk: low
Dependencies: Task 2
Files:
- Modify: crates/editor-core/src/code_review.rs
- Modify: crates/editor-core/src/core.rs
- Test: crates/editor-core/tests/code_review.rs
Blast radius:
- none
Acceptance:
- `Core::reply_to_thread(number, comment_id, body) -> Result<ThreadComment, CoreError>` runs `gh api -X POST repos/{owner}/{repo}/pulls/<N>/comments/<id>/replies -f body=…` and returns the new comment.
TDD:
- Behavior: a reply to a thread is posted under that thread's first comment.
- Test file: crates/editor-core/tests/code_review.rs
- Test name: a_reply_is_posted_to_its_thread
- RED command: `cargo test -p editor-core --test code_review a_reply_is_posted_to_its_thread`
- Expected RED failure: no method `reply_to_thread` on `Core`.
- Minimal GREEN change: as acceptance.
- Refactor candidates: none
- Mocking: FakeGh (GitHub)
Steps:
1. RED: assert `gh.args()` is the replies call with the body, and the returned comment is parsed.
   Command: `cargo test -p editor-core --test code_review a_reply_is_posted_to_its_thread`
   Expected: FAIL
2. GREEN: as acceptance.
   Command: `cargo test -p editor-core --test code_review`
   Expected: PASS
3. REFACTOR: none
4. VERIFY:
   - `cargo test -p editor-core --test code_review`
Checkpoint:
- Files: `crates/editor-core/src/code_review.rs crates/editor-core/src/core.rs crates/editor-core/tests/code_review.rs`
- Commit: `Code review: reply to a comment thread`

## Task 8: Viewed ticks are GitHub's
Status: completed
Risk: medium
Dependencies: Task 3
Files:
- Modify: crates/editor-core/src/code_review.rs
- Modify: crates/editor-core/src/core.rs
- Test: crates/editor-core/tests/code_review.rs
Blast radius:
- none
Acceptance:
- `ReviewFile` gains `viewed: bool`, read on open from `gh api graphql` (`pullRequest(number){ files(first:100, after){ nodes{ path viewerViewedState } pageInfo } }`, paginated); `VIEWED` is true.
- `Core::set_review_file_viewed(number, path, viewed)` runs the `markFileAsViewed`/`unmarkFileAsViewed` mutation with the PR's node id; a failure is returned and the tick isn't changed.
TDD:
- Behavior: a file's Viewed tick is read from and written to GitHub.
- Test file: crates/editor-core/tests/code_review.rs
- Test name: viewed_ticks_are_githubs
- RED command: `cargo test -p editor-core --test code_review viewed_ticks_are_githubs`
- Expected RED failure: no field `viewed` on `ReviewFile`.
- Minimal GREEN change: as acceptance.
- Refactor candidates: none
- Mocking: FakeGh (GitHub)
Steps:
1. RED: FakeGh's graphql answer marks `a` VIEWED; assert it on open; `set_review_file_viewed(7, "b", true)` sends `markFileAsViewed` with the node id and path.
   Command: `cargo test -p editor-core --test code_review viewed_ticks_are_githubs`
   Expected: FAIL
2. GREEN: as acceptance.
   Command: `cargo test -p editor-core --test code_review`
   Expected: PASS
3. REFACTOR: none
4. VERIFY:
   - `cargo test -p editor-core --test code_review`
Checkpoint:
- Files: `crates/editor-core/src/code_review.rs crates/editor-core/src/core.rs crates/editor-core/tests/code_review.rs`
- Commit: `Code review: Viewed ticks match GitHub's`

## Task 9: New commits and Refresh
Status: completed
Risk: high
Dependencies: Task 4, Task 5, Task 8
Files:
- Modify: crates/editor-core/src/code_review.rs
- Modify: crates/editor-core/src/core.rs
- Test: crates/editor-core/src/code_review.rs
- Test: crates/editor-core/tests/code_review.rs
Blast radius:
- none beyond the module
Acceptance:
- `Core::code_review_new_commits(number) -> Result<u32, CoreError>`: `ls_remote_ref` of `refs/pull/<N>/head`; 0 if it equals the saved head; else the count `git rev-list --count <saved>..<new>` after a fetch (a force-push counts the new head's commits since the merge base).
- `Core::refresh_code_review(number) -> CodeReview`: fetches, `reset_hard` the Review Worktree to the new head, saves it; each pending comment is re-anchored to the same `line_text` on its side within 20 lines of where it was (nearest wins), else marked `outdated`; files whose blob changed between the old and new head are unmarked Viewed (via Task 8's mutation).
TDD:
- Behavior: Refresh moves the Review Worktree to the new head, keeps pending comments on their lines or marks them outdated, and clears Viewed on changed files.
- Test file: crates/editor-core/tests/code_review.rs
- Test name: refresh_keeps_comments_and_clears_viewed_on_changed_files
- RED command: `cargo test -p editor-core --test code_review refresh_keeps_comments_and_clears_viewed_on_changed_files`
- Expected RED failure: no method `code_review_new_commits` on `Core`.
- Minimal GREEN change: as acceptance; re-anchoring as a pure function `reanchor(old: &PendingComment, new_diff: &[DiffLine]) -> Option<u32>` in `code_review.rs`.
- Refactor candidates: none
- Mocking: FakeGh (GitHub)
Steps:
1. RED: two comments, then push a commit that inserts 3 lines above the first commented line and deletes the second's line; assert `new_commits == 1`, after refresh the first comment's line moved by 3, the second is outdated, the Worktree's `HEAD` is the new head, and `unmarkFileAsViewed` was sent for the changed file only. Unit tests for `reanchor` (moved, gone, ambiguous → nearest).
   Command: `cargo test -p editor-core --test code_review refresh_keeps_comments_and_clears_viewed_on_changed_files`
   Expected: FAIL
2. GREEN: as acceptance.
   Command: `cargo test -p editor-core --test code_review && cargo test -p editor-core --lib code_review`
   Expected: PASS
3. REFACTOR: none
4. VERIFY:
   - `cargo test -p editor-core --test code_review --lib`
Checkpoint:
- Files: `crates/editor-core/src/code_review.rs crates/editor-core/src/core.rs crates/editor-core/tests/code_review.rs`
- Commit: `Code review: new commits pushed mid-review, and Refresh`

## Task 10: A Review Worktree is read-only
Status: completed
Risk: high
Dependencies: Task 3
Files:
- Modify: crates/editor-core/src/core.rs
- Modify: crates/editor-core/src/documents.rs
- Test: crates/editor-core/tests/code_review.rs
Blast radius:
- crates/editor-core/src/core.rs (start_session_in :2473–2501, save_file :2664, set_permission_mode :4430, current_mode_update :5895 — hub)
- crates/editor-core/src/documents.rs (read :130, `save` :157, used by the Manual editor)
- src/Chat.tsx (ModeTabs; Task 17)
Acceptance:
- Files of a Review Worktree open with `FileContent::Text.read_only: true`, and `Core::save_file` (core.rs:2664) into one is refused (`CoreError::ReadOnlyWorktree`) before `documents::save` is reached.
- An Agent session started in a Review Worktree starts in Plan mode (not AskForEdits); `SessionInfo` gains `mode_locked: bool`, true there.
- `set_permission_mode` on a locked session is refused (`CoreError::ModeLocked`); when the Agent leaves Plan mode itself, the core sets Plan again and emits `PermissionModeChanged` with Plan.
TDD:
- Behavior: nothing in a Review Worktree can be edited, by hand or by the Agent.
- Test file: crates/editor-core/tests/code_review.rs
- Test name: a_review_worktree_is_read_only
- RED command: `cargo test -p editor-core --test code_review a_review_worktree_is_read_only`
- Expected RED failure: `read_only` is false for a Review Worktree's file (assertion failure).
- Minimal GREEN change: check `is_review_worktree` in the core's file read and `save_file`, and at the three mode points.
- Refactor candidates: none
- Mocking: FakeAgent (the Agent) scripted with `initialMode` and a mode update leaving plan; FakeGh (GitHub)
Steps:
1. RED: open a Code review; assert a file reads read-only and a write is refused; start a session there, assert it's Plan and `mode_locked`; `set_permission_mode(AcceptEdits)` errors; a scripted Agent mode update to `default` ends with the session back in Plan and `session/set_mode` plan received by the fake agent.
   Command: `cargo test -p editor-core --test code_review a_review_worktree_is_read_only`
   Expected: FAIL
2. GREEN: as acceptance.
   Command: `cargo test -p editor-core --test code_review`
   Expected: PASS
3. REFACTOR: none
4. VERIFY:
   - `cargo test -p editor-core --test code_review --test permissions --test documents --test manual_editor`
   - `cargo clippy -p editor-core --all-targets`
Checkpoint:
- Files: `crates/editor-core/src/core.rs crates/editor-core/src/documents.rs crates/editor-core/tests/code_review.rs`
- Commit: `Code review: the Review Worktree can't be edited, and its Agent stays in Plan mode`

## Task 11: The review Agent session and its prompts
Status: completed
Risk: medium
Dependencies: Task 4, Task 10
Files:
- Modify: crates/editor-core/src/code_review.rs
- Modify: crates/editor-core/src/core.rs
- Test: crates/editor-core/src/code_review.rs
- Test: crates/editor-core/tests/code_review.rs
Blast radius:
- crates/editor-core/src/core.rs (new_session_in, reopen of Recent sessions)
Acceptance:
- `Core::code_review_session(number) -> Result<SessionId, CoreError>`: the saved review session if it's a Tab; reopened from Recent sessions if it was closed; else a new one in the Review Worktree; its acp id saved on `SavedCodeReview.session`. A re-requested review gets the same one.
- `Core::claude_review(number) -> Result<SessionId, CoreError>` sends `code_review::review_prompt(&details, &review)` on that session: the PR's number, title, description, base branch, merge-base and head commits, changed files, and the instruction to read the diff with `git diff <merge_base> <head>` and answer in prose (no edits).
- `Core::ask_attachment(number, path, side, start_line, line) -> Attachment`: `Text { name: "<path>:<start>-<end> (<old|new>)", path, text }` with the selected lines and 5 around them, from the head (new side) or merge base (old side).
TDD:
- Behavior: one review session per PR carries the review prompt, and a question's attachment holds the selected lines with their context.
- Test file: crates/editor-core/tests/code_review.rs
- Test name: one_review_session_per_pr_gets_the_review_prompt
- RED command: `cargo test -p editor-core --test code_review one_review_session_per_pr_gets_the_review_prompt`
- Expected RED failure: no method `code_review_session` on `Core`.
- Minimal GREEN change: as acceptance.
- Refactor candidates: none
- Mocking: FakeAgent (the Agent); FakeGh (GitHub)
Steps:
1. RED: `claude_review(7)` twice returns the same session id; the fake agent's received prompt contains the head sha, the merge base and each changed path; after closing the Tab, `code_review_session(7)` reopens the same acp id. Unit tests `review_prompt_names_the_commits_and_files` and `ask_attachment_has_the_lines_and_their_context`.
   Command: `cargo test -p editor-core --test code_review one_review_session_per_pr_gets_the_review_prompt`
   Expected: FAIL
2. GREEN: as acceptance.
   Command: `cargo test -p editor-core --test code_review && cargo test -p editor-core --lib code_review`
   Expected: PASS
3. REFACTOR: none
4. VERIFY:
   - `cargo test -p editor-core --test code_review --lib`
Checkpoint:
- Files: `crates/editor-core/src/code_review.rs crates/editor-core/src/core.rs crates/editor-core/tests/code_review.rs`
- Commit: `Code review: ask Claude to review the PR, or about selected lines`

## Task 12: When a Review Worktree can be removed
Status: completed
Risk: medium
Dependencies: Task 6, Task 11
Files:
- Modify: crates/editor-core/src/core.rs
- Modify: crates/editor-core/src/merged.rs
- Test: crates/editor-core/tests/code_review.rs
Blast radius:
- crates/editor-core/src/core.rs (merge_overview :2188)
- src/WorktreesOverview.tsx (ready rule :19; Task 18)
Acceptance:
- Merge overview rows gain `review: Option<ReviewRow { number, done: bool }>`; `done` when the PR is merged or closed (`gh pr view <N> --json state`) or the review was submitted. The detached HEAD isn't counted as lost commits for a Review Worktree.
- `Core::removal_check` of a Review Worktree is `ok` after Submit when no other Agent session than the review session uses it.
- Removing it drops its `SavedCodeReview` (the Pending review with it) and deletes `refs/orchard/review/<N>`.
TDD:
- Behavior: a Review Worktree is offered for removal once its review is done, and removing it forgets the review.
- Test file: crates/editor-core/tests/code_review.rs
- Test name: a_review_worktree_is_removable_once_its_review_is_done
- RED command: `cargo test -p editor-core --test code_review a_review_worktree_is_removable_once_its_review_is_done`
- Expected RED failure: no field `review` on the merge overview row.
- Minimal GREEN change: as acceptance.
- Refactor candidates: none
- Mocking: FakeGh (GitHub)
Steps:
1. RED: before Submit the row's `review.done` is false; after a FakeGh-accepted Submit it's true; removing the Worktree leaves no `code_reviews` entry in a restarted Core and no `refs/orchard/review/7`.
   Command: `cargo test -p editor-core --test code_review a_review_worktree_is_removable_once_its_review_is_done`
   Expected: FAIL
2. GREEN: as acceptance.
   Command: `cargo test -p editor-core --test code_review --test merged_worktrees --test remove_worktree`
   Expected: PASS
3. REFACTOR: none
4. VERIFY:
   - `cargo test -p editor-core`
   - `cargo clippy -p editor-core --all-targets`
Checkpoint:
- Files: `crates/editor-core/src/core.rs crates/editor-core/src/merged.rs crates/editor-core/tests/code_review.rs`
- Commit: `Worktrees overview: a Review Worktree is ready to remove once its review is done`

## Task 13: Tauri commands and TypeScript bindings
Status: completed
Risk: medium
Dependencies: Task 6, Task 7, Task 9, Task 11, Task 12
Files:
- Modify: src-tauri/src/main.rs
- Modify: src/core.ts
Blast radius:
- src-tauri/src/main.rs (generate_handler! :1554 — bridge)
- src/core.ts (CoreEvent union :804, PullRequestDetails, SessionInfo, merge overview row — bridge)
Acceptance:
- Commands for `open_code_review`, `code_review_diff`, `add_review_comment`, `edit_review_comment`, `remove_review_comment`, `add_review_question`, `submit_review`, `reply_to_thread`, `set_review_file_viewed`, `code_review_new_commits`, `refresh_code_review`, `code_review_session`, `claude_review`, `ask_attachment`, registered in `generate_handler!`, in the `pull_request_details` pattern (main.rs:1074).
- `core.ts` has the matching types (CodeReview, ReviewFile, PendingReview, PendingComment, Question, Verdict, DiffSide, SinceLastReview, ReviewRow; new fields on PullRequestDetails, CodeThread, ThreadComment, SessionInfo.modeLocked, WorktreeInfo.review, the merge overview row's `review`; `pendingReviewChanged` in CoreEvent) and calls.
TDD:
- Behavior: no frontend test framework; the commands are thin wrappers over tested core methods.
- Test file: none
- Test name: none
- RED command: `pnpm typecheck`
- Expected RED failure: none expected.
- Minimal GREEN change: as acceptance.
- Refactor candidates: none
- Mocking: none
Steps:
1. Add the commands and register them.
2. Add the types and calls to `core.ts`.
3. VERIFY:
   - `cargo build -p orchard`
   - `cargo clippy --workspace --all-targets`
   - `pnpm typecheck`
Checkpoint:
- Files: `src-tauri/src/main.rs src/core.ts`
- Commit: `Code review: commands and bindings for the frontend`

## Task 14: The Review's file list and diff, shared and filterable
Status: completed
Risk: medium
Dependencies: none
Files:
- Create: src/ReviewFiles.tsx
- Modify: src/ReviewOverlay.tsx
Blast radius:
- src/ReviewOverlay.tsx (Review of ticket 42; behaviour must not change)
Acceptance:
- `ReviewFiles` renders the file list with Viewed ticks, progress and a filter box (matches on path, case-insensitive), taking `files`, `current`, `isViewed`, `onSelect`, `onToggleViewed`.
- `useReviewKeys({ next, previous, toggleViewed, close })` holds the J/K/V/Esc capture listener (ReviewOverlay.tsx:362–378).
- ReviewOverlay uses both and behaves as before, now with the filter.
TDD:
- Behavior: no frontend test framework; verified by type check and a run of the app.
- Test file: none
- Test name: none
- RED command: `pnpm typecheck`
- Expected RED failure: none expected.
- Minimal GREEN change: lift the inline parts (:464–:520, :362–:378) out unchanged, then add the filter.
- Refactor candidates: none
- Mocking: none
Steps:
1. Extract `ReviewFiles` and `useReviewKeys`.
2. Add the filter.
3. VERIFY:
   - `pnpm typecheck`
   - Run the app: the Review (Create PR flow) still lists files, ticks, J/K/V work; the filter narrows the list.
Checkpoint:
- Files: `src/ReviewFiles.tsx src/ReviewOverlay.tsx`
- Commit: `Review: filter the file list`

## Task 15: Selecting lines and showing comments in a diff
Status: completed
Risk: high
Dependencies: Task 13
Files:
- Modify: src/DiffView.tsx
Blast radius:
- src/DiffView.tsx (used by GitDrawer, ManualEditor's diff tab, ReviewOverlay — hub)
Acceptance:
- New optional props: `selectable`, `onSelect(sel: { side, startLine, line, rect })` (click a line number, shift-click or drag to extend to a range, on one side), `inline(side, line) => JSX | undefined` rendered as a full-width row after that line (threads, drafts, comment boxes), `gutter(side, line) => JSX | undefined` (the Claude marker).
- Works in unified and side-by-side; with no new props, every existing caller renders exactly as before.
TDD:
- Behavior: no frontend test framework; verified by type check and a run of the app.
- Test file: none
- Test name: none
- RED command: `pnpm typecheck`
- Expected RED failure: none expected.
- Minimal GREEN change: hook the old/new line numbers already on rows (`numbered()` :18, `sideBySide()` :42).
- Refactor candidates: none
- Mocking: none
Steps:
1. Add the props and the selection state.
2. Render `inline` rows and `gutter` content in both layouts.
3. VERIFY:
   - `pnpm typecheck`
   - Run the app: Git drawer diffs and the Review look unchanged.
Checkpoint:
- Files: `src/DiffView.tsx`
- Commit: `Diffs: lines can be selected and comments shown between them`

## Task 16: The Code review overlay
Status: completed
Risk: high
Dependencies: Task 14, Task 15
Files:
- Create: src/CodeReviewOverlay.tsx
- Modify: src/App.tsx
Blast radius:
- src/App.tsx (overlay signals ~:381–:387, rendering ~:1396–:1422, shortcut guards :842–:861 — hub)
Acceptance:
- `codeReviewOf: { number } | null` signal and `<Show>` in App.tsx; the shortcut guards include it.
- Opens on the changes since the last review when `sinceLastReview` is present (a re-requested review), else the full diff.
- Layout: header (PR number and title, "since last review / full diff" toggle when `sinceLastReview` is present, Unified/Side-by-side, "Claude review", Submit), `ReviewFiles`, the current file's `DiffView` with selection; existing threads (with Reply) and pending comments (Edit, Remove; outdated ones listed at the top of their file with Move/Remove) shown inline; the selection menu offers "Comment" and "Ask Claude".
- J/K/V via `useReviewKeys`; V calls `setReviewFileViewed`, failures shown in the header.
- Polls `codeReviewNewCommits` every 60 s; when > 0 shows "N new commits — Refresh".
- Submit dialog: Approve / Request changes / Comment and a summary; a `SubmitFailed` keeps the dialog's text, shows GitHub's message and marks the named files' comments; `OutdatedComments` says how many. After success: offer "Remove Worktree" when `removalCheck` is ok, then close.
- Setup running/failed shown as in a new Worktree (Retry / Start anyway).
TDD:
- Behavior: no frontend test framework; verified by type check and a run of the app.
- Test file: none
- Test name: none
- RED command: `pnpm typecheck`
- Expected RED failure: none expected.
- Minimal GREEN change: as acceptance.
- Refactor candidates: none
- Mocking: none
Steps:
1. Build the overlay on `ReviewFiles` + `DiffView`.
2. Wire it into App.tsx.
3. VERIFY:
   - `pnpm typecheck`
   - Run the app against a repo with a PR requesting the user's review: comment on a line and a range, reply, tick Viewed, push a commit and Refresh, submit.
Checkpoint:
- Files: `src/CodeReviewOverlay.tsx src/App.tsx`
- Commit: `Code review: go through a PR's files, comment and submit a review`

## Task 17: Claude docked in the Code review
Status: pending
Risk: high
Dependencies: Task 16
Files:
- Create: src/ReviewChat.tsx
- Modify: src/CodeReviewOverlay.tsx
- Modify: src/Chat.tsx
- Modify: src/Transcript.tsx
Blast radius:
- src/Chat.tsx (Composer drafts map, ModeTabs :733 — hub)
- src/Transcript.tsx (scroll to an item)
Acceptance:
- `ReviewChat` embeds the review session (Transcript + Composer, own `createTabView` slot `review:<N>`, as Columns.tsx:173 does), collapsible, on the overlay's right.
- "Claude review" calls `core.claudeReview` and opens the pane.
- `Chat.tsx` exports `addAttachment(session, attachment)`; "Ask Claude" gets `core.askAttachment`, adds it, records `addReviewQuestion`, focuses the composer.
- The gutter marker on asked lines scrolls the transcript to the user message carrying that attachment's name (`Transcript` gains a `scrollTo(predicate)` handle).
- "Use as comment" on an Agent reply that answers a question pre-fills an editable draft comment on the question's lines (`fromQuestion`).
- ModeTabs are disabled with "Read-only review" when `modeLocked`.
TDD:
- Behavior: no frontend test framework; verified by type check and a run of the app.
- Test file: none
- Test name: none
- RED command: `pnpm typecheck`
- Expected RED failure: none expected.
- Minimal GREEN change: as acceptance.
- Refactor candidates: share the embed with Columns if it lifts out cleanly.
- Mocking: none
Steps:
1. `addAttachment`, locked ModeTabs, `Transcript` scroll handle.
2. `ReviewChat` and its wiring.
3. VERIFY:
   - `pnpm typecheck`
   - Run with `ORCHARD_ACP_ADAPTER=target/debug/fake-acp-agent`: Claude review sends; Ask Claude adds a chip; the marker scrolls; Use as comment fills a draft; mode can't change. Then once against real Claude Code.
Checkpoint:
- Files: `src/ReviewChat.tsx src/CodeReviewOverlay.tsx src/Chat.tsx src/Transcript.tsx`
- Commit: `Code review: ask Claude to review the PR or about selected lines`

## Task 18: Ways into a Code review
Status: pending
Risk: medium
Dependencies: Task 16
Files:
- Modify: src/App.tsx
- Modify: src/PrOverlay.tsx
- Modify: src/Dashboard.tsx
- Modify: src/WorktreesOverview.tsx
- Modify: src/Worktrees.tsx
- Modify: src/Columns.tsx
Blast radius:
- src/App.tsx (onReviewRequest :1226)
- src/WorktreesOverview.tsx (ready rule :19)
- src/Worktrees.tsx (Sidebar :146)
Acceptance:
- A Dashboard review request opens the Code review (was the PR overlay).
- The PR overlay shows "Code review" when `viewerIsReviewer` and the PR is open.
- A "Review" badge (from `WorktreeInfo.review`) shows on Review Worktrees in the sidebar (`Sidebar` in src/Worktrees.tsx), on their Tabs (the Tabs view's strip in App.tsx/Worktrees.tsx, and Columns.tsx' `tabs-list` :217) and in the Worktrees overview; choosing one in the sidebar opens its Code review.
- The Worktrees overview counts a Review Worktree ready when `review.done`.
TDD:
- Behavior: no frontend test framework; verified by type check and a run of the app.
- Test file: none
- Test name: none
- RED command: `pnpm typecheck`
- Expected RED failure: none expected.
- Minimal GREEN change: as acceptance.
- Refactor candidates: none
- Mocking: none
Steps:
1. Wire each entry point.
2. VERIFY:
   - `pnpm typecheck`
   - Run the app: each way in opens the right Code review; the overview offers removal after Submit.
Checkpoint:
- Files: `src/App.tsx src/PrOverlay.tsx src/Dashboard.tsx src/WorktreesOverview.tsx src/Worktrees.tsx src/Columns.tsx`
- Commit: `Dashboard: a review request opens the Code review`

## Task 19: Glossary, issue 44 and final verification
Status: pending
Risk: low
Dependencies: Task 17, Task 18
Files:
- Modify: CONTEXT.md
- Modify: .scratch/agent-editor-build/issues/44-code-review.md
Blast radius:
- none
Acceptance:
- `CONTEXT.md` has **Code review**, **Review Worktree** and **Pending review** entries in the glossary's style, with _Avoid_ lines (e.g. Code review: _Avoid_ PR review screen; Pending review: _Avoid_ draft review).
- Issue 44's checklist ticked for what's done, the status line saying what was and wasn't tried.
TDD:
- Behavior: documentation only.
- Test file: none
- Test name: none
- RED command: `cargo test -p editor-core`
- Expected RED failure: none expected.
- Minimal GREEN change: as acceptance.
- Refactor candidates: none
- Mocking: none
Steps:
1. Write the entries; update issue 44.
2. VERIFY:
   - `cargo test --workspace`
   - `cargo clippy --workspace --all-targets`
   - `cargo fmt --all --check`
   - `pnpm typecheck`
   - Run the app once through every line of issue 44.
Checkpoint:
- Files: `CONTEXT.md .scratch/agent-editor-build/issues/44-code-review.md`
- Commit: `Code review: glossary entries; issue 44 done`

## Environment Context
- **Language:** Rust core (crates/editor-core) + Tauri 2 shell (src-tauri); TypeScript with SolidJS frontend (Vite)
- **Test command:** `cargo test -p editor-core` (focused: `cargo test -p editor-core --test code_review <name>` / `cargo test -p editor-core --lib code_review::tests::<name>`); `cargo test --workspace` at the end
- **Linter command:** `cargo clippy --workspace --all-targets`
- **Formatter command:** `cargo fmt --all`
- **Build command:** `pnpm typecheck` (frontend); run with `pnpm dev`
- **Branch:** code-review (PR into main; never commit to main)
- **Conventional commit style:** "Area: what the user now sees, in plain words", ending with the Co-Authored-By line

**Codebase conventions:** Core owns state and logic (ADR 0003); git only via `git.rs`, GitHub only via `gh` (`run_gh`/`gh_text`, `gh_env`) (ADR 0004). Serde `rename_all = "camelCase"`, tagged enums `tag = "kind"`; new saved state fields `#[serde(default, skip_serializing_if)]` on `WorkspaceState`, carried by `Core::persist`/`restore`. Events through `CoreEvent` and `core.onEvent`. Integration tests in `crates/editor-core/tests/*.rs`, `#![cfg(unix)] mod support; use support::*;`, `#[tokio::test(flavor = "multi_thread")]`, helpers `RepoWithOrigin`, `FakeGh`, `FakeAgent`, `core_with_gh`, `core_with_state`, `core_with_gh_and_state` (new, Task 3), `eventually`. Frontend: overlays are signals + `<Show>` in App.tsx; comments are sparse prose in the glossary's words; core.ts types mirror serde types by hand.

**Graph Context:**
- Blast radius: ~25 files | Hub nodes: crates/editor-core/src/core.rs, src/App.tsx, src/DiffView.tsx, src/Chat.tsx (graph server unavailable; from reading)
- Bridge nodes: src-tauri/src/main.rs, src/core.ts | Communities crossed: core, tauri glue, frontend

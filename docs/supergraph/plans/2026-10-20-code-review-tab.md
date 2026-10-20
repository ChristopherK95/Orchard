# Plan: the Code review as a Tab (issue 44, follow-up)

Spec: `.scratch/agent-editor-build/issues/44-code-review.md`. Builds on `2026-10-19-code-review.md` (all done).

Why: the Code review is a modal overlay, so the user can't go back to their own Worktrees' Tabs mid-review. It becomes the review Agent session's Tab instead. The core is mostly unchanged. One real bug: `code_review_session` (core.rs:3806) starts a **new** session when the saved one is neither a Tab nor in Recent sessions (it dropped out of the list), so the conversation would be lost.

Decisions settled with the user:
- **Layout:** the Code review shows in the Tabs view's main area when the active Tab is a review session. It holds the file list and diff, with Claude's chat as a collapsible column on the right and a draggable divider. The divider's width and collapsed state are kept in localStorage.
- **The review session:** it starts when the Code review opens and is the only session in its Review Worktree. Switching Tabs, closing the Tab, or restarting Orchard always brings back the same conversation. Closing the Tab moves the session to Recent sessions; it is reopened by acp id even after it has left that list.
- **Tab:** labelled "Review #N" plus the PR title, with the "Review" badge. A dot on the Tab and on the sidebar badge shows new commits.
- **Columns view:** a Review Worktree's column shows the chat plus "Open Code review", which switches to the Tabs view on it.
- **Keys:** J/K/V work only while the review has focus. Esc no longer closes anything.
- **Several reviews:** each keeps its own current file, scroll position and view mode.
- **Worktree setup:** shown as a notice line. The review session starts once the setup is finished.
- **Overlay:** `CodeReviewOverlay` stops being a modal. The ways in (Dashboard, PR overlay, sidebar badge) open or switch to the review Tab.

## Task 1: The review session never loses its conversation
Status: completed
Risk: medium
Dependencies: none
Files:
- Modify: crates/editor-core/src/core.rs
- Modify: crates/editor-core/src/code_review.rs
- Test: crates/editor-core/tests/code_review.rs
Blast radius:
- crates/editor-core/src/core.rs (open_code_review, code_review_session :3806, open_conversation :4802, new_session_in :2550, setup finish path)
- src/core.ts (CodeReview.session; Task 3)
Acceptance:
- `code_review_session(N)` with a saved acp id goes through `open_conversation(worktree, acp_id, …)` (core.rs:4802), which covers a Tab, a Recent session, and one in neither list. `open_conversation` → `reopen` (:4841) returns `Ok` even when `resume_claimed` fails (the session is left Suspended with a "Couldn't reopen" notice). So add a crate-private checked variant that returns `Err` on a failed resume and closes the dead Tab without moving it to Recent. Only then, or when there's no saved id, is a new session started, and its acp id is saved.
- Naming: the core sets the session's name to "Review #N <title>" itself (directly in state, then persist and the usual session-changed event), whichever path gave the session. Names are cut to 40 chars (`TITLE_NAME_LEN`), so long titles end in "…".
- `open_code_review(N)` also makes sure the review session exists once the setup is finished (or when there's no setup). `CodeReview` gains `session: Option<SessionId>`, which is `None` while setup is running or failed. When a Review Worktree's setup reaches `Finished` (the review branch of the setup-finish path, core.rs ~:1314, `review.is_some()` → `Finished`), the core starts or resumes its review session there and emits the usual `SessionCreated`. Update `start_first_session`'s doc comment ("review session is the user's to start") to match.
- The review session is named "Review #N <title>".
- `new_session_in` in a Review Worktree is refused with `CoreError::ReviewWorktreeSession`, because only the review session belongs there.
TDD:
- Behavior: a Code review's Claude conversation survives closing its Tab, a restart, and dropping out of Recent sessions.
- Test file: crates/editor-core/tests/code_review.rs
- Test name: the_review_conversation_survives_closing_restarting_and_leaving_recent
- RED command: `cargo test -p editor-core --test code_review the_review_conversation_survives_closing_restarting_and_leaving_recent`
- Expected RED failure: after the session leaves Recent sessions, `code_review_session` returns a session with a different acp id (assertion), or `CodeReview.session` doesn't exist (compile error).
- Minimal GREEN change: as acceptance.
- Refactor candidates: none
- Mocking: FakeAgent (the Agent; its log shows `session/load`/resume with the saved id); FakeGh (GitHub)
Steps:
1. RED: open a review; `session` is Some with acp id X. Close the Tab: X is in Recent, and `code_review_session` gives X. Restart with `core_with_gh_and_state`: the Tab is restored with X. Close it again, stop the Core, drop X from the state file's `recent` (the only way: Recent holds 20 per Worktree, and no other session may start in a Review Worktree), then restart: `code_review_session` still gives X and the fake agent received a resume of X. Second test: `no_other_session_in_a_review_worktree` (`new_session_in` refused). Third test: `the_review_session_starts_when_setup_finishes`. Fourth test: `a_conversation_the_agent_cant_resume_gets_a_fresh_session` (FakeAgent `failResume`: a new acp id is saved, no dead Suspended Tab is left).
   Command: `cargo test -p editor-core --test code_review`
   Expected: FAIL
2. GREEN: as acceptance.
   Command: `cargo test -p editor-core --test code_review`
   Expected: PASS
3. REFACTOR: none
4. VERIFY:
   - `cargo test -p editor-core`
   - `cargo clippy -p editor-core --all-targets`
Checkpoint:
- Files: `crates/editor-core/src/core.rs crates/editor-core/src/code_review.rs crates/editor-core/tests/code_review.rs`
- Commit: `Code review: Claude's conversation in a review is never lost`

## Task 2: Bindings
Status: pending
Risk: low
Dependencies: Task 1
Files:
- Modify: src/core.ts
Blast radius:
- src/core.ts (CodeReview type; main.rs needs no change, `CodeReview` already serializes through)
Acceptance:
- `CodeReview.session: SessionId | null` in core.ts. `ReviewWorktreeSession` comes through as an error string, as other CoreErrors do.
TDD:
- Behavior: no frontend test framework; type mirror only.
- Test file: none
- Test name: none
- RED command: `pnpm typecheck`
- Expected RED failure: none expected.
- Minimal GREEN change: as acceptance.
- Refactor candidates: none
- Mocking: none
Steps:
1. Mirror the field.
2. VERIFY:
   - `pnpm typecheck`
Checkpoint:
- Files: `src/core.ts`
- Commit: `Code review: bindings know the review's session`

## Task 3: CodeReviewOverlay becomes CodeReviewView
Status: pending
Risk: high
Dependencies: Task 2
Files:
- Create: src/CodeReviewView.tsx (renamed from src/CodeReviewOverlay.tsx with `git mv`)
- Modify: src/ReviewChat.tsx
- Modify: src/ReviewFiles.tsx
- Modify: src/styles.css
- Modify: src/App.tsx
Blast radius:
- src/App.tsx (import at :36 and the overlay `<Show>` at ~:1463, only switched to the renamed component here so the checkpoint builds; the real wiring is Task 4; that interim `<Show>` passes the session from `codeReviewSession(N)` and `active={true}`)
- src/ReviewChat.tsx:6 (imports `ReviewDock` from the old module)
- src/ReviewFiles.tsx (`useReviewKeys` is shared with ReviewOverlay; the Create PR Review's modal behaviour must not change)
Acceptance:
- `CodeReviewView` props: `{ number, session: SessionInfo, active: boolean, … }`. It renders no backdrop, no modal and no close button, and fills its parent. Its parts are the header (PR, toggles, Claude review, Submit), the new-commits bar, setup notice, file list, diff, and the chat column.
- Per-PR UI state (current file, since/full toggle, unified/side-by-side, diff scroll top, dock open) lives in a module-level `Map<number, …>`, so switching Tabs and back keeps it.
- Keys: `useReviewKeys` gains an optional `scope: () => HTMLElement | undefined`, and `close` becomes optional (Esc is handled before anything else today, so the view passes no `close`). In the view, J/K/V act only when `document.activeElement` is inside the review's file list or diff and the view is `active`. Esc does nothing here. The Create PR Review keeps its window-wide keys and Esc.
- The Claude column (ReviewChat) sits behind a vertical drag handle. `localStorage["orchard.codeReview.dock"] = { width, open }` (default 520 px, open; clamped to 380 px–60% of the view). The collapse button and a header button toggle it. A collapsed column leaves a slim strip with a Sparkles button.
- Styles: `.review-overlay` modal rules aren't used by the view; the new `.code-review-view` is a flex row filling the main area, reusing the `.review-body` / `.cr-*` look.
TDD:
- Behavior: no frontend test framework; verified by type check, build and a run of the app.
- Test file: none
- Test name: none
- RED command: `pnpm typecheck`
- Expected RED failure: none expected.
- Minimal GREEN change: as acceptance.
- Refactor candidates: drop `onShowSession` / the no-dock fallback, since the chat is always part of the view.
- Mocking: none
Steps:
1. `git mv` and strip the modal parts; add the per-PR state map.
2. Scoped keys.
3. Resizable, remembered Claude column.
4. VERIFY:
   - `pnpm typecheck`
   - `pnpm build:web`
Checkpoint:
- Files: `src/CodeReviewView.tsx src/CodeReviewOverlay.tsx src/ReviewChat.tsx src/ReviewFiles.tsx src/styles.css src/App.tsx`
- Commit: `Code review: a view of its own instead of an overlay`

## Task 4: The review Tab in the Tabs view, and the ways into it
Status: pending
Risk: high
Dependencies: Task 3
Files:
- Modify: src/App.tsx
- Modify: src/Worktrees.tsx
- Modify: src/PrOverlay.tsx
- Modify: src/WorktreesOverview.tsx
Blast radius:
- src/App.tsx (main area ~:1560–:1630, `codeReviewOf` signal and its `<Show>`, shortcut guards, `onReviewRequest`, "New Agent session here")
- src/Worktrees.tsx (sidebar badge, Tab labels)
Acceptance:
- When the Tabs view's active session is in a Review Worktree (`worktree.review`), the main area renders `CodeReviewView` for that PR, with the session's Transcript and Composer in its Claude column, in place of the plain Transcript and Composer. The Manual editor, Terminal panel and drawers still open beside it.
- `codeReviewOf` and the overlay `<Show>` are gone. Opening a Code review (Dashboard review request, "Code review" in the PR overlay, the sidebar badge, the Worktrees overview badge) runs `core.openCodeReview(N)`. Then it shows the session's Tab if there is one; otherwise it switches to the Review Worktree, which shows the setup notice view, and the Tab appears when the setup's session arrives (`sessionCreated`).
- In a Review Worktree, "New Agent session here", the Recent list's "new" actions and the new-session shortcut are hidden or no-ops. Its empty state shows the Code review with the setup notice.
- The Tab label of a review session is "Review #N <title>" (from the session name) with the Review badge.
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
1. Render the view for review sessions; remove the overlay wiring.
2. Rewire the ways in; hide new-session actions.
3. VERIFY:
   - `pnpm typecheck`
   - `pnpm build:web`
Checkpoint:
- Files: `src/App.tsx src/Worktrees.tsx src/PrOverlay.tsx src/WorktreesOverview.tsx`
- Commit: `Code review: a review is a Tab, so the user can switch to their own work and back`

## Task 5: Columns view and the new-commits dot
Status: pending
Risk: medium
Dependencies: Task 4
Files:
- Modify: src/Columns.tsx
- Modify: src/App.tsx
- Modify: src/Worktrees.tsx
- Create: src/reviewWatch.ts
- Modify: src/CodeReviewView.tsx
- Modify: src/styles.css
Blast radius:
- src/Columns.tsx (column header and body)
Acceptance:
- A Review Worktree's column shows its chat as usual, plus an "Open Code review" button in the column header. The button switches to the Tabs view on that session.
- `reviewWatch.ts`: a module-level store `newCommits: Record<number, number>` that polls `core.codeReviewNewCommits(N)` every 60 s for each Review Worktree in the Worktree list (the polling moves out of the view: remove its 60 s interval, CodeReviewView.tsx, formerly CodeReviewOverlay.tsx:128). The view's "N new commits — Refresh" bar reads it, and Refresh resets it to 0.
- When `newCommits[N] > 0` a dot shows on the review Tab and on the sidebar's Review badge.
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
1. `reviewWatch.ts` and its readers.
2. The Columns button.
3. VERIFY:
   - `pnpm typecheck`
   - `pnpm build:web`
Checkpoint:
- Files: `src/Columns.tsx src/App.tsx src/Worktrees.tsx src/reviewWatch.ts src/CodeReviewView.tsx src/styles.css`
- Commit: `Code review: new commits show on the review's Tab while you work elsewhere`

## Task 6: Glossary, issue 44 and final verification
Status: pending
Risk: low
Dependencies: Task 5
Files:
- Modify: CONTEXT.md
- Modify: .scratch/agent-editor-build/issues/44-code-review.md
Blast radius:
- none
Acceptance:
- CONTEXT.md's Code review entry says it is the review Agent session's Tab, not an overlay. The Review Worktree entry says it holds only that session.
- Issue 44's lines that said "overlay" or "docked in the overlay" now describe the Tab. A line covers the conversation surviving closes and restarts.
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
1. Edit both files.
2. VERIFY:
   - `cargo test --workspace`
   - `cargo clippy --workspace --all-targets` (no new warnings)
   - `cargo fmt --all --check`
   - `pnpm typecheck`
Checkpoint:
- Files: `CONTEXT.md .scratch/agent-editor-build/issues/44-code-review.md`
- Commit: `Code review: glossary and issue 44 describe the review Tab`

## Environment Context
- **Language:** Rust core + Tauri 2; TypeScript/SolidJS frontend (Vite)
- **Test command:** `cargo test -p editor-core` (focused: `cargo test -p editor-core --test code_review <name>`); `cargo test --workspace` at the end
- **Linter command:** `cargo clippy --workspace --all-targets` (3 known older warnings: core.rs complex type, tests/columns.rs ×2)
- **Formatter command:** `cargo fmt --all`
- **Build command:** `pnpm typecheck`, `pnpm build:web`; run with `pnpm dev`
- **Branch:** code-review (PR into main; never commit to main)
- **Conventional commit style:** "Area: what the user now sees, in plain words", ending with `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`

**Codebase conventions:** as in `2026-10-19-code-review.md`. Frontend view state that outlives a component lives in module-level stores (e.g. ReviewOverlay's `viewedBy`, PrBoard's `myPrs`). UI preferences go in localStorage under `orchard.*` (e.g. sidebar folds in Worktrees.tsx:156).

**Graph Context:**
- Blast radius: ~12 files | Hub nodes: src/App.tsx, crates/editor-core/src/core.rs (graph server unavailable; from reading)
- Bridge nodes: src/core.ts, src-tauri/src/main.rs

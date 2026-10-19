# 44: Code review

**What to build:** A Code review of a pull request the user is a reviewer on, inside Orchard: its files gone through one at a time like the Review (42), Viewed ticks shared with GitHub, comments on lines held as a Pending review, and a verdict submitted. Read-only throughout. The Agent can review the PR and answer questions about selected lines, in one review Agent session per PR.

**Blocked by:** 43 (Dashboard's review requests open it).

**Status:** not started

Opening:
- [ ] Opened from a review request on the Dashboard, and from "Code review" in the PR overlay of any open PR the user is a reviewer on (requested, or has reviewed before). Not offered on other PRs.

Review Worktree:
- [ ] Opening makes (or reuses) a dedicated Review Worktree `review-pr-<N>`, detached at the fetched `refs/pull/N/head` (so forks work too). Never a branch, never the user's own Worktree on the PR's branch.
- [ ] Worktree setup runs on first open, and again on a later open only if it never finished or failed.
- [ ] Read-only is enforced: its files open read-only in the Manual editor; its Agent sessions are locked in Plan mode (no mode switch).
- [ ] Shown in the sidebar, Tabs and Worktrees overview with a "Review" badge; choosing it in the sidebar opens the Code review.
- [ ] Removal: offered after Submit (if no other Agent session uses it), and by the Worktrees overview once the PR is merged or closed, or the review submitted.

Code review view:
- [ ] The Review's layout (`CodeReviewOverlay`, sharing the file list and diff parts with `ReviewOverlay`): a file list with a filter, Viewed ticks and progress, one file's diff at a time (`DiffView`, unified or side-by-side), Next / Previous, J / K, V.
- [ ] The diff is the PR's head against its merge base with the PR's base branch, never the Worktree's working tree.
- [ ] Viewed ticks are GitHub's (`markFileAsViewed` / `unmarkFileAsViewed`), so they match the web.
- [ ] Existing threads show at their lines and can be replied to. (Resolving, suggestion blocks: later.)
- [ ] New commits pushed mid-review: a bar "N new commits — Refresh". Refreshing fetches, hard-resets the Review Worktree to the new head, and keeps the pending comments; one whose line is gone stays on the file as outdated. Files that changed lose their Viewed tick.
- [ ] A re-requested review opens on the changes since the user's last review, with a toggle to the full diff.

Pending review:
- [ ] Comments on a line or a range go into a Pending review kept by editor-core, surviving closing the overlay and restarts.
- [ ] Submit asks for Approve, Request changes or Comment and a summary, and sends it in one `gh api` call.
- [ ] A failed Submit (network, stale head, a line no longer in the diff) keeps the Pending review as it was, shows GitHub's error and marks the comments it names.

Agent:
- [ ] One review Agent session per PR, in the Review Worktree, in Plan mode. Its chat is docked in the overlay (collapsible) and is also a normal Tab. A re-requested review continues the same session.
- [ ] "Claude review" sends at once a prompt with the PR's number, title, description, base, merge-base and head commits, and the changed files; the Agent runs `git diff` itself and answers in prose. It always covers the full diff.
- [ ] "Ask Claude" beside "Comment" in a line or range selection adds a chip to the docked composer (file, old/new side, line range, the selected lines with a few around them, as a text attachment); the question is typed there.
- [ ] Lines asked about get a gutter marker that scrolls the docked chat to that question; markers are kept with the Pending review.
- [ ] "Use as comment" on an answer to a question pre-fills an editable draft comment on the asked lines. Nothing the Agent writes is posted without the user.

Tests:
- [ ] Core tests with real `git`, a fake `gh` and the fake ACP agent.

Notes:
- New `CONTEXT.md` terms: Code review, Review Worktree, Pending review.

# 44: Code review

**What to build:** A Code review of a pull request the user is a reviewer on, inside Orchard: its files gone through one at a time like the Review (42), Viewed ticks shared with GitHub, comments on lines held as a Pending review, and a verdict submitted. Read-only throughout. The Agent can review the PR and answer questions about selected lines, in one review Agent session per PR.

**Blocked by:** 43 (Dashboard's review requests open it).

**Status:** implemented; not yet run by hand in the app. Covered by core tests (real `git`, a fake `gh`, the fake ACP agent) and a frontend type check and build. Not tried: the review Tab, Claude's column (Ask Claude's chip, the gutter markers' scrolling, Use as comment, resizing), the new-commits dot and the badges in a running app; real GitHub (Viewed ticks, Submit, Reply, a fork's PR); real Claude Code; Windows.

Opening:
- [x] Opened from a review request on the Dashboard, and from "Code review" in the PR overlay of any open PR the user is a reviewer on (requested, or has reviewed before). Not offered on other PRs.
- [x] The Code review is the review Agent session's Tab, not an overlay: the user can switch to their own Worktrees' Tabs mid-review and back, finding each review's file, scroll position and view mode as they left them. Every way in (Dashboard, PR overlay, a Review badge, the Worktrees overview) opens or switches to that Tab. The Tab is labelled "Review #N" and the PR's title, with the Review badge.

Review Worktree:
- [x] Opening makes (or reuses) a dedicated Review Worktree `review-pr-<N>`, detached at the fetched `refs/pull/N/head` (so forks work too). Never a branch, never the user's own Worktree on the PR's branch.
- [x] Worktree setup runs on first open, and again on a later open only if it never finished or failed.
- [x] Read-only is enforced: its files open read-only in the Manual editor; its Agent sessions are locked in Plan mode (no mode switch).
- [x] Shown in the sidebar, Tabs and Worktrees overview with a "Review" badge; choosing it in the sidebar opens the Code review. In the Columns view its column shows the chat and "Open Code review", which switches to the Tabs view on it.
- [x] It holds only the review Agent session: no other session can be started there.
- [x] Removal: offered after Submit (if no other Agent session uses it), and by the Worktrees overview once the PR is merged or closed, or the review submitted.

Code review view:
- [x] The Review's layout (`CodeReviewView`, sharing the file list and diff parts with `ReviewOverlay`), filling the Tabs view's main area: a file list with a filter, Viewed ticks and progress, one file's diff at a time (`DiffView`, unified or side-by-side), Next / Previous, J / K, V (only while the review has the focus). Esc closes nothing.
- [x] The diff is the PR's head against its merge base with the PR's base branch, never the Worktree's working tree.
- [x] Viewed ticks are GitHub's (`markFileAsViewed` / `unmarkFileAsViewed`), so they match the web.
- [x] Existing threads show at their lines and can be replied to. (Resolving, suggestion blocks: later.)
- [x] New commits pushed mid-review: a bar "N new commits — Refresh", and a dot on the review's Tab and Review badge, kept current every minute while the user works elsewhere. Refreshing fetches, hard-resets the Review Worktree to the new head, and keeps the pending comments; one whose line is gone stays on the file as outdated. Files that changed lose their Viewed tick.
- [x] A re-requested review opens on the changes since the user's last review, with a toggle to the full diff.

Pending review:
- [x] Comments on a line or a range go into a Pending review kept by editor-core, surviving closing its Tab and restarts.
- [x] Submit asks for Approve, Request changes or Comment and a summary, and sends it in one `gh api` call.
- [x] A failed Submit (network, stale head, a line no longer in the diff) keeps the Pending review as it was, shows GitHub's error and marks the comments it names.

Agent:
- [x] One review Agent session per PR, in the Review Worktree, in Plan mode, started when the Code review opens (once the Worktree setup is finished; until then the setup shows as a notice line). Its chat is a column on the Code review's right, collapsible to a slim strip and resizable (its width and state remembered). A re-requested review continues the same session.
- [x] Its conversation is never lost: switching Tabs, closing the Tab (it goes to Recent sessions), restarting Orchard, or the session leaving the Recent sessions all bring back the same conversation, reopened by its id. Only a conversation the Agent can't bring back gets a fresh session.
- [x] "Claude review" sends at once a prompt with the PR's number, title, description, base, merge-base and head commits, and the changed files; the Agent runs `git diff` itself and answers in prose. It always covers the full diff.
- [x] "Ask Claude" beside "Comment" in a line or range selection adds a chip to Claude's composer (file, old/new side, line range, the selected lines with a few around them, as a text attachment); the question is typed there.
- [x] Lines asked about get a gutter marker that scrolls Claude's column to that question; markers are kept with the Pending review.
- [x] "Use as comment" on an answer to a question pre-fills an editable draft comment on the asked lines. Nothing the Agent writes is posted without the user.

Tests:
- [x] Core tests with real `git`, a fake `gh` and the fake ACP agent.

Notes:
- New `CONTEXT.md` terms: Code review, Review Worktree, Pending review.

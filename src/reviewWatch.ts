// New commits pushed to the PRs under Code review (issue 44), asked for every minute for each Review
// Worktree, so the review's Tab and Review badge say so while the user works elsewhere.
import { onCleanup } from "solid-js";
import { createStore } from "solid-js/store";
import { core } from "./core";

/** How often each PR is asked about. */
const EVERY_MS = 60_000;

const [counts, setCounts] = createStore<Record<number, number>>({});
/** Bumped by `seen`, so an answer asked for before a Refresh can't bring the old count back. */
const asked: Record<number, number> = {};

/** The commits pushed to each PR since its Review Worktree's head, as last asked. */
export const newCommits = {
  of: (number: number) => counts[number] ?? 0,
  /** Refreshed: the Review Worktree is at the PR's head again. */
  seen: (number: number) => {
    asked[number] = (asked[number] ?? 0) + 1;
    setCounts(number, 0);
  },
};

function ask(number: number) {
  const n = (asked[number] = (asked[number] ?? 0) + 1);
  // (Quietly: offline is no news.)
  core.codeReviewNewCommits(number).then(
    (count) => n === asked[number] && setCounts(number, count),
    () => {},
  );
}

/** Keeps `newCommits` asked for, every minute, for the PRs `numbers` names (the Review Worktrees'). */
export function watchReviews(numbers: () => number[]) {
  const timer = setInterval(() => numbers().forEach(ask), EVERY_MS);
  onCleanup(() => clearInterval(timer));
}

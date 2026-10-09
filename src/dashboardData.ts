// What the Dashboard shows that the Workspace view doesn't already hold: the reviews requested from
// the user (read from GitHub like the PR board's list, every few minutes), the Worktrees that need
// attention (read again as Worktrees and their git status change), and the Worktrees ready to clean
// up (the Worktrees overview's merged ones). Module-level, like `myPrs`, so they're read once per
// Workspace whether or not the Dashboard is showing.
import { createEffect, createMemo, createSignal, on, onCleanup } from "solid-js";
import { core, type ReviewRequest, type WorktreeAttention, type WorktreeMerge } from "./core";

/** How often the review requests are asked for again while the Workspace is open (as the user's PRs). */
const EVERY_MS = 5 * 60_000;
/** A burst of git status events (a checkout, a rebase) reads the attention list once. */
const COALESCE_MS = 300;

const [requests, setRequests] = createSignal<ReviewRequest[]>([]);
const [requestsError, setRequestsError] = createSignal("");
const [requestsLoading, setRequestsLoading] = createSignal(false);
const [requestsRefreshedAt, setRequestsRefreshedAt] = createSignal<number | null>(null);
let requestsFrom = "";
let requestsAsked = 0;

/** The reviews requested from the user as last read (most recently asked first), and how that went. */
export const reviewRequests = {
  list: requests,
  error: requestsError,
  loading: requestsLoading,
  refreshedAt: requestsRefreshedAt,
  refresh: () => void loadRequests(),
};

async function loadRequests() {
  if (!requestsFrom) return;
  const n = ++requestsAsked;
  setRequestsLoading(true);
  try {
    const next = await core.reviewRequests(requestsFrom);
    if (n !== requestsAsked) return;
    setRequests(next);
    setRequestsError("");
    setRequestsRefreshedAt(Date.now());
  } catch (err) {
    if (n === requestsAsked) setRequestsError(String(err));
  } finally {
    if (n === requestsAsked) setRequestsLoading(false);
  }
}

/** Keeps `reviewRequests` read in the Worktree `worktree` names (the main one), now and every few minutes. */
export function watchReviewRequests(worktree: () => string) {
  const where = createMemo(worktree);
  createEffect(
    on(where, (w) => {
      requestsFrom = w;
      setRequests([]);
      setRequestsError("");
      setRequestsRefreshedAt(null);
      void loadRequests();
    }),
  );
  const timer = setInterval(() => void loadRequests(), EVERY_MS);
  onCleanup(() => {
    clearInterval(timer);
    requestsFrom = "";
    requestsAsked++;
  });
}

const [attentionList, setAttentionList] = createSignal<WorktreeAttention[]>([]);
const [attentionError, setAttentionError] = createSignal("");
let attentionAsked = 0;

/** The Worktrees (the main checkout too) that need attention, each with every reason. */
export const attention = { list: attentionList, error: attentionError };

async function loadAttention() {
  const n = ++attentionAsked;
  try {
    const next = await core.worktreeAttention();
    if (n !== attentionAsked) return;
    setAttentionList(next);
    setAttentionError("");
  } catch (err) {
    if (n === attentionAsked) setAttentionError(String(err));
  }
}

/** Keeps `attention` read: now, and again shortly after Worktrees or their git status change. */
export function watchAttention() {
  setAttentionList([]);
  setAttentionError("");
  void loadAttention();
  let due: ReturnType<typeof setTimeout> | undefined;
  const soon = () => {
    if (due !== undefined) return;
    due = setTimeout(() => {
      due = undefined;
      void loadAttention();
    }, COALESCE_MS);
  };
  let stop: (() => void) | undefined;
  let alive = true;
  onCleanup(() => {
    alive = false;
    stop?.();
    clearTimeout(due);
    attentionAsked++;
  });
  void core
    .onEvent((event) => {
      if (event.kind === "worktreesChanged" || event.kind === "gitStatusChanged") soon();
    })
    .then((unlisten) => (alive ? (stop = unlisten) : unlisten()));
}

const [cleanUpList, setCleanUpList] = createSignal<WorktreeMerge[]>([]);
const [cleanUpError, setCleanUpError] = createSignal("");
let cleanUpAsked = 0;

/** A Worktree branch whose work is in its Base, or whose remote branch was deleted: it can go. */
const canGo = (w: WorktreeMerge) => w.merge?.kind === "merged" || w.merge?.kind === "changesInBase" || w.merge?.kind === "remoteDeleted";

/** The Worktrees ready to clean up, as the Worktrees overview reads them. */
export const cleanUp = { list: cleanUpList, error: cleanUpError, refresh: () => void loadCleanUp() };

async function loadCleanUp() {
  const n = ++cleanUpAsked;
  try {
    const next = await core.mergeOverview();
    if (n !== cleanUpAsked) return;
    setCleanUpList(next.filter(canGo));
    setCleanUpError("");
  } catch (err) {
    if (n === cleanUpAsked) setCleanUpError(String(err));
  }
}

/** Keeps `cleanUp` read: now, and again when the Worktrees change. */
export function watchCleanUp() {
  setCleanUpList([]);
  setCleanUpError("");
  void loadCleanUp();
  let stop: (() => void) | undefined;
  let alive = true;
  onCleanup(() => {
    alive = false;
    stop?.();
    cleanUpAsked++;
  });
  void core
    .onEvent((event) => event.kind === "worktreesChanged" && void loadCleanUp())
    .then((unlisten) => (alive ? (stop = unlisten) : unlisten()));
}

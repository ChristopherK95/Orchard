// What the Dashboard shows that the Workspace view doesn't already hold: the reviews requested from
// the user (read from GitHub like the PR board's list, every few minutes), the Worktrees that need
// attention (read again as Worktrees and their git status change), and the Worktrees ready to clean
// up (the Worktrees overview's merged ones). Module-level, like `myPrs`, so they're read once per
// Workspace whether or not the Dashboard is showing. The statistics of the user's work are the
// exception: GitHub is asked for them only while the Dashboard is shown, and an answer is kept an
// hour (one per period), so coming back to the Dashboard doesn't ask again.
import { createEffect, createMemo, createSignal, on, onCleanup } from "solid-js";
import { core, type ReviewRequest, type Statistics, type StatisticsDays, type WorktreeAttention, type WorktreeMerge } from "./core";

/** How often the review requests are asked for again while the Workspace is open (as the user's PRs). */
const EVERY_MS = 5 * 60_000;
/** A burst of git status events (a checkout, a rebase) reads the attention list once. */
const COALESCE_MS = 300;
/** How long the statistics read for a period are shown before showing the Dashboard asks again. */
const KEEP_MS = 60 * 60_000;

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

const [figures, setFigures] = createSignal<Statistics | null>(null);
const [statisticsError, setStatisticsError] = createSignal("");
const [statisticsLoading, setStatisticsLoading] = createSignal(false);
const [statisticsRefreshedAt, setStatisticsRefreshedAt] = createSignal<number | null>(null);
const [days, setDaysShown] = createSignal<StatisticsDays>(30);
let statisticsFrom = "";
/** The Workspace's period has been read for `statisticsFrom`: until then nothing is asked. */
let daysRead = false;
let statisticsAsked = 0;
/** The answers for the current Worktree, one per period, and when each was read. */
const kept = new Map<StatisticsDays, { figures: Statistics; at: number }>();

/** Shows what's kept for the period chosen (or nothing). */
function showKept() {
  const k = kept.get(days());
  setFigures(k?.figures ?? null);
  setStatisticsRefreshedAt(k?.at ?? null);
}

/** The statistics of the user's work over the Workspace's chosen period, as last read, and how that went. */
export const statistics = {
  figures,
  error: statisticsError,
  loading: statisticsLoading,
  refreshedAt: statisticsRefreshedAt,
  days,
  /** The Dashboard is shown: reads them if there are none for the period, or they're over an hour old. */
  shown: () => {
    if (!statisticsFrom || !daysRead || statisticsLoading()) return;
    const k = kept.get(days());
    if (!k || Date.now() - k.at > KEEP_MS) void loadStatistics();
  },
  refresh: () => void loadStatistics(),
  /** Chooses the period (remembered for the Workspace) and reads its statistics, unless kept from within the hour. */
  setDays: (d: StatisticsDays) => {
    if (!daysRead) return;
    core.setStatisticsPeriod(d).catch((err) => setStatisticsError(String(err)));
    setDaysShown(d);
    showKept();
    const k = kept.get(d);
    if (k && Date.now() - k.at <= KEEP_MS) {
      // Whatever was being asked for the other period is no longer wanted.
      statisticsAsked++;
      setStatisticsLoading(false);
      setStatisticsError("");
    } else void loadStatistics();
  },
};

async function loadStatistics() {
  if (!statisticsFrom || !daysRead) return;
  const n = ++statisticsAsked;
  const d = days();
  setStatisticsLoading(true);
  try {
    const next = await core.statistics(statisticsFrom, d);
    if (n !== statisticsAsked) return;
    const at = Date.now();
    kept.set(d, { figures: next, at });
    setFigures(next);
    setStatisticsError("");
    setStatisticsRefreshedAt(at);
  } catch (err) {
    if (n === statisticsAsked) setStatisticsError(String(err));
  } finally {
    if (n === statisticsAsked) setStatisticsLoading(false);
  }
}

/** Keeps `statistics` for the Worktree `worktree` names (the main one): on a change, forgets what was
 *  read and reads the Workspace's period, then reads the figures if `visible` (the Dashboard is shown). */
export function watchStatistics(worktree: () => string, visible: () => boolean) {
  const where = createMemo(worktree);
  createEffect(
    on(where, (w) => {
      statisticsFrom = w;
      daysRead = false;
      statisticsAsked++;
      kept.clear();
      setFigures(null);
      setStatisticsError("");
      setStatisticsRefreshedAt(null);
      setStatisticsLoading(false);
      if (!w) return;
      const period = (d: StatisticsDays) => {
        if (statisticsFrom !== w || daysRead) return;
        setDaysShown(d);
        daysRead = true;
        if (visible()) statistics.shown();
      };
      // Without a remembered period (it can't be read), the default one.
      core.statisticsPeriod().then(period, () => period(30));
    }),
  );
  onCleanup(() => {
    statisticsFrom = "";
    daysRead = false;
    statisticsAsked++;
  });
}

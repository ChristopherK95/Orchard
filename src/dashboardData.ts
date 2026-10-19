// What the Dashboard shows that the Workspace view doesn't already hold: the reviews requested from
// the user (read from GitHub like the PR board's list, every few minutes), the Worktrees that need
// attention (read again as Worktrees and their git status change), and the Worktrees ready to clean
// up (the Worktrees overview's merged ones). Module-level, like `myPrs`, so they're read once per
// Workspace. The review requests are kept current whether or not the Dashboard is showing (they make
// its badge); the Worktree lists, which run git in every Worktree, only while it shows. The
// statistics of the user's work are asked of GitHub only while the Dashboard is shown, and an answer
// is kept an hour (one per period), so coming back to the Dashboard doesn't ask again.
import { createEffect, createMemo, createSignal, on, onCleanup } from "solid-js";
import { core, type ReviewRequest, type Statistics, type StatisticsDays, type WorktreeAttention, type WorktreeInfo, type WorktreeMerge } from "./core";

/** How often the review requests are asked for again while the Workspace is open (as the user's PRs). */
const EVERY_MS = 5 * 60_000;
/** A burst of git status events (a checkout, a rebase) reads the attention list once. */
const COALESCE_MS = 300;
/** Worktree changes (a rebase, a run of commits) read the clean-up list once. */
const CLEAN_UP_COALESCE_MS = 1000;
/** How old the clean-up list may be when the Dashboard is shown again before it's read again. */
const CLEAN_UP_FRESH_MS = 60_000;
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

/** Keeps `attention` read while `visible`: now, and again shortly after Worktrees or their git
 *  status change. Reading it runs git in every Worktree, and an Agent at work changes git status
 *  all the time, so while the Dashboard is hidden a change only marks it out of date. */
export function watchAttention(visible: () => boolean) {
  setAttentionList([]);
  setAttentionError("");
  let stale = true;
  let due: ReturnType<typeof setTimeout> | undefined;
  const soon = () => {
    stale = true;
    if (due !== undefined || !visible()) return;
    due = setTimeout(() => {
      due = undefined;
      stale = false;
      void loadAttention();
    }, COALESCE_MS);
  };
  createEffect(on(visible, (shown) => shown && stale && soon()));
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
export const cleanUp = { list: cleanUpList, error: cleanUpError };

let overviewRunning: Promise<WorktreeMerge[]> | null = null;

/** `core.mergeOverview()`, shared by callers that ask while it's running: it's the editor's most
 *  expensive question (a run of git commands per Worktree branch), and the Dashboard and the
 *  Worktrees overview often ask at the same moment. */
export function mergeOverview(): Promise<WorktreeMerge[]> {
  overviewRunning ??= core.mergeOverview().finally(() => (overviewRunning = null));
  return overviewRunning;
}

/** What a Worktree's merged state can depend on, as `worktreesChanged` tells it: not its count of
 *  uncommitted changes, which moves with every file an Agent saves. */
export const mergeKey = (worktrees: WorktreeInfo[]) =>
  worktrees.map((w) => `${w.path} ${w.branch ?? ""} ${w.head} ${w.ahead ?? ""} ${w.behind ?? ""}`).join("\n");

async function loadCleanUp() {
  const n = ++cleanUpAsked;
  try {
    const next = await mergeOverview();
    if (n !== cleanUpAsked) return;
    setCleanUpList(next.filter(canGo));
    setCleanUpError("");
  } catch (err) {
    if (n === cleanUpAsked) setCleanUpError(String(err));
  }
}

/** Keeps `cleanUp` read while `visible`: on showing, if the Worktrees' branches have moved since
 *  it was read or it's over a minute old (a merged PR shows only in the remote branches), and
 *  again when they move while it shows. */
export function watchCleanUp(visible: () => boolean) {
  setCleanUpList([]);
  setCleanUpError("");
  let stale = true;
  let readAt = 0;
  let lastKey: string | null = null;
  let due: ReturnType<typeof setTimeout> | undefined;
  const soon = () => {
    stale = true;
    if (due !== undefined || !visible()) return;
    due = setTimeout(() => {
      due = undefined;
      stale = false;
      readAt = Date.now();
      void loadCleanUp();
    }, CLEAN_UP_COALESCE_MS);
  };
  createEffect(on(visible, (shown) => shown && (stale || Date.now() - readAt >= CLEAN_UP_FRESH_MS) && soon()));
  let stop: (() => void) | undefined;
  let alive = true;
  onCleanup(() => {
    alive = false;
    stop?.();
    clearTimeout(due);
    cleanUpAsked++;
  });
  void core
    .onEvent((event) => {
      if (event.kind !== "worktreesChanged") return;
      const key = mergeKey(event.worktrees);
      if (key === lastKey) return;
      lastKey = key;
      soon();
    })
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

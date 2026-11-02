// The Statistics page (ticket 47): six read-only cards on the user's own work in the
// Workspace's repo over the chosen period (7, 30 or 90 days), in this order: their PRs merged, the
// median time to merge, the median time to a first review (and how many of their PRs still wait
// for one), the reviews they gave, their oldest open PR, and the review request waiting longest on
// them. The first four say how they compare with the period before, without judging better or
// worse. A median of only one or two PRs says so; a figure with nothing to count shows "—".
// It's a page of its own, over the Tabs or Columns as the Boards are, so GitHub is asked for them only
// when it's opened (and they're over an hour old), not each time the Dashboard is.
import { createSignal, For, type JSX, onCleanup, Show } from "solid-js";
import { ago } from "./Chat";
import type { Compared, Median, Statistics, StatisticsDays, Waiting } from "./core";
import { statistics } from "./dashboardData";
import { Loader, RefreshCw } from "./icons";

/** The periods the statistics can cover. */
const PERIODS: StatisticsDays[] = [7, 30, 90];

/** The core's `gh` hint speaks of creating a PR; the page says this instead. */
const PR_HINT = "Creating a PR takes the GitHub CLI: install `gh`, run `gh auth login` in a terminal once, then try again.";
const GH_HINT = "This takes the GitHub CLI: install `gh`, run `gh auth login` in a terminal once, then refresh.";

export function StatisticsPage(props: {
  /** Where the page sits: under the title bar. */
  top: number;
  /** The Workspace's error and notice banners, shown here too. */
  banners?: JSX.Element;
}) {
  // (Opening the page reads them, unless they're kept from within the hour.)
  statistics.shown();
  // "Refreshed 2 min ago" and the waiting times move on while the page is showing.
  const [now, setNow] = createSignal(Date.now());
  const tick = setInterval(() => setNow(Date.now()), 30_000);
  onCleanup(() => clearInterval(tick));
  const refreshed = () => {
    now();
    const at = statistics.refreshedAt();
    return at === null ? "" : ago(new Date(at).toISOString());
  };
  return (
    <section class="board statistics-page" style={{ top: `${props.top}px` }}>
      {props.banners}
      <div class="board-head">
        <span class="pr-board-title">Statistics</span>
        <div class="segmented dashboard-period">
          <For each={PERIODS}>
            {(d) => (
              <button classList={{ on: statistics.days() === d }} aria-pressed={statistics.days() === d} onClick={() => statistics.setDays(d)} title={`The last ${d} days`}>
                {d}d
              </button>
            )}
          </For>
        </div>
        <Show when={refreshed()}>
          <span class="dashboard-refreshed">refreshed {refreshed()}</span>
        </Show>
        <button class="ghost icon" onClick={() => statistics.refresh()} disabled={statistics.loading()} title="Ask GitHub again" aria-label="Refresh statistics">
          <RefreshCw classList={{ spin: statistics.loading() }} />
        </button>
        <span class="hint">Esc for the Tabs</span>
      </div>
      <div class="statistics-body">
        <Show when={statistics.error()}>
          <p class="error small dashboard-error">{statistics.error().replace(PR_HINT, GH_HINT)}</p>
        </Show>
        <Show
          when={statistics.figures()}
          fallback={
            <Show when={!statistics.error()}>
              <p class="dashboard-line muted">
                <Loader class="spin" />
                Asking GitHub…
              </p>
            </Show>
          }
        >
          {(figures) => <StatisticsCards figures={figures()} now={now()} />}
        </Show>
      </div>
    </section>
  );
}

/** A length of time as the cards say it: "45 min", "3 h", "2 d". */
export function duration(seconds: number): string {
  const minutes = Math.round(seconds / 60);
  if (minutes < 60) return `${minutes} min`;
  const hours = Math.round(seconds / 3600);
  if (hours < 24) return `${hours} h`;
  return `${Math.round(seconds / 86_400)} d`;
}

/** A median with something to count. */
const counted = (m: Median) => (m.count > 0 && m.seconds !== null ? m.seconds : null);

/** How a figure moved since the period before: "▲ 3", "▼ 2 h", "same as" (`null`: either side has nothing). */
function change(now: number | null, before: number | null, say: (n: number) => string, same: (n: number) => boolean): string | null {
  if (now === null || before === null) return null;
  const by = now - before;
  if (same(by)) return "same as";
  return `${by > 0 ? "▲" : "▼"} ${say(Math.abs(by))} vs`;
}

export function StatisticsCards(props: { figures: Statistics; now: number }) {
  const f = () => props.figures;
  const counts = (c: Compared<number>) => change(c.now, c.before, String, (by) => by === 0);
  const medians = (c: Compared<Median>) => change(counted(c.now), counted(c.before), duration, (by) => Math.round(Math.abs(by) / 60) === 0);
  return (
    <div class="dashboard-stats">
      <Card title="Merged PRs" vs={counts(f().merged)} days={f().days}>
        {f().merged.now}
      </Card>
      <Card title="Median time to merge" vs={medians(f().timeToMerge)} days={f().days}>
        <MedianFigure median={f().timeToMerge.now} />
      </Card>
      <Card
        title="Median time to first review"
        vs={medians(f().timeToFirstReview)}
        days={f().days}
        below={
          <Show when={f().stillWaiting > 0}>
            <span class="dashboard-stat-note">{f().stillWaiting} still waiting</span>
          </Show>
        }
      >
        <MedianFigure median={f().timeToFirstReview.now} />
      </Card>
      <Card title="Reviews given" vs={counts(f().reviewsGiven)} days={f().days}>
        {f().reviewsGiven.now}
      </Card>
      <WaitingCard title="Oldest open PR" waiting={f().oldestOpen} now={props.now} />
      <WaitingCard title="Longest-waiting review request" waiting={f().longestRequest} now={props.now} />
    </div>
  );
}

/** One card: its title, its figure, and (for the first four) how it compares with the period before. */
function Card(props: { title: string; children: JSX.Element; vs?: string | null; days?: number; below?: JSX.Element }) {
  return (
    <div class="dashboard-stat">
      <span class="dashboard-stat-title">{props.title}</span>
      <span class="dashboard-stat-figure">{props.children}</span>
      <Show when={props.days !== undefined}>
        <span class="dashboard-stat-vs">
          {props.vs ?? "—"} previous {props.days}d
        </span>
      </Show>
      {props.below}
    </div>
  );
}

/** A median's figure; over one or two PRs it says how few. */
function MedianFigure(props: { median: Median }) {
  const seconds = () => counted(props.median);
  return (
    <Show when={seconds() !== null} fallback="—">
      {duration(seconds()!)}
      <Show when={props.median.count < 3}>
        <span class="dashboard-stat-n">(n={props.median.count})</span>
      </Show>
    </Show>
  );
}

/** How long a PR has waited, and which one. */
function WaitingCard(props: { title: string; waiting: Waiting | null; now: number }) {
  return (
    <Card
      title={props.title}
      below={
        <Show when={props.waiting}>
          {(w) => (
            <span class="dashboard-stat-note">
              #{w().number} {w().title}
            </span>
          )}
        </Show>
      }
    >
      <Show when={props.waiting} fallback="—">
        {(w) => duration(Math.max(0, (props.now - Date.parse(w().since)) / 1000))}
      </Show>
    </Card>
  );
}

// The Dashboard (ticket 43): the live status wall of the user's own work in the Workspace, and the
// view a Workspace opens on. Fixed sections, in this order: the Agent sessions that need the user,
// reviews requested from them, their open PRs by stage, Worktrees that need attention, Worktrees
// ready to clean up, and how many sessions are Working and Idle. Each item leads to where it lives
// (the Workspace view decides how); a PR with failing checks or review feedback can start an Agent
// session on it. The GitHub sections say when they were last read, and say so inline when `gh`
// can't be used, while the local ones work as ever.
import { createSignal, For, type JSX, onCleanup, Show } from "solid-js";
import { ago } from "./Chat";
import type { AttentionReason, Check, MergeState, MyPullRequest, PullRequestStage, ReviewRequest, SessionInfo, WorktreeAttention, WorktreeMerge } from "./core";
import { attention, cleanUp, reviewRequests } from "./dashboardData";
import {
  Bell,
  ChevronRight,
  CircleCheck,
  CircleX,
  Clock,
  Eraser,
  GitBranch,
  GitPullRequestArrow,
  GitPullRequestDraft,
  Loader,
  MessageSquare,
  MessageSquareCode,
  RefreshCw,
  Sparkles,
  TriangleAlert,
} from "./icons";
import { myPrs } from "./PrBoard";
import { StateDot } from "./StateDot";
import { worktreeColour, worktreeLabel, type WorktreeTab } from "./Worktrees";

/** The core's `gh` hint for the PR list speaks of creating a PR; the Dashboard's sections say this instead. */
const PR_HINT = "Creating a PR takes the GitHub CLI: install `gh`, run `gh auth login` in a terminal once, then try again.";
const GH_HINT = "This takes the GitHub CLI: install `gh`, run `gh auth login` in a terminal once, then refresh.";

/** The user's open PRs, grouped by stage, most in need of them first. */
const STAGES: { title: string; stage: PullRequestStage; icon: () => JSX.Element }[] = [
  { title: "Changes requested", stage: "changesRequested", icon: () => <MessageSquare /> },
  { title: "Checks failing", stage: "checksFailing", icon: () => <CircleX /> },
  { title: "Approved", stage: "approved", icon: () => <CircleCheck /> },
  { title: "Waiting for review", stage: "waitingForReview", icon: () => <Clock /> },
  { title: "Draft", stage: "draft", icon: () => <GitPullRequestDraft /> },
];

const plural = (n: number, thing: string) => `${n} ${thing}${n === 1 ? "" : "s"}`;

const OPERATION: Record<Extract<AttentionReason, { kind: "operation" }>["operation"], string> = {
  merge: "merge",
  rebase: "rebase",
  cherryPick: "cherry-pick",
  revert: "revert",
  am: "patch apply",
};

/** One reason a Worktree needs attention, as its row says it ("3 uncommitted"). */
function reasonText(r: AttentionReason): string {
  switch (r.kind) {
    case "uncommitted":
      return `${r.files} uncommitted`;
    case "unpushed":
      return `${r.commits} unpushed`;
    case "behind":
      return `${r.commits} behind`;
    case "noUpstream":
      return `no upstream, ${plural(r.commits, "commit")}`;
    case "operation":
      return `${OPERATION[r.operation]} in progress`;
  }
}

/** Why a Worktree can go, as its clean-up row says it. */
function mergeText(m: MergeState | null): string {
  if (m?.kind === "merged") return `merged into ${m.into}`;
  if (m?.kind === "changesInBase") return `its changes are in ${m.into}`;
  if (m?.kind === "remoteDeleted") return "remote branch deleted";
  return "";
}

export function Dashboard(props: {
  sessions: SessionInfo[];
  worktrees: WorktreeTab[];
  /** Where the Dashboard sits: under the title bar. */
  top: number;
  /** The Workspace's error and notice banners, shown here too. */
  banners?: JSX.Element;
  onSession: (id: number) => void;
  onPr: (pr: MyPullRequest) => void;
  onReviewRequest: (request: ReviewRequest) => void;
  onWorktree: (path: string) => void;
  /** The Worktrees overview. */
  onCleanUp: () => void;
  /** The PR board. */
  onAllPrs: () => void;
  /** Starts an Agent session on the PR with its prompt in the composer; a failure is shown on its row. */
  onStartSession: (pr: MyPullRequest) => Promise<void>;
}) {
  // "Refreshed 2 min ago" and "asked 3 h ago" move on while the Dashboard is showing.
  const [now, setNow] = createSignal(Date.now());
  const tick = setInterval(() => setNow(Date.now()), 30_000);
  onCleanup(() => clearInterval(tick));
  const since = (at: number | null) => (now(), at === null ? "" : ago(new Date(at).toISOString()));

  const worktreeOn = (branch: string) => props.worktrees.find((w) => !w.removed && w.branch === branch);
  const label = (path: string, branch: string | null) => {
    const w = props.worktrees.find((w) => w.path === path);
    return w ? worktreeLabel(w) : (branch ?? path.split(/[\\/]/).pop() ?? path);
  };
  const needYou = () => props.sessions.filter((s) => s.state === "needsYou");
  const openPrs = () => myPrs.prs().filter((p) => p.state === "open");
  // A suspended session counts as Idle, as in the sidebar: it's only been put to sleep while idle.
  const count = (...states: SessionInfo["state"][]) => props.sessions.filter((s) => states.includes(s.state)).length;

  return (
    <section class="board dashboard" style={{ top: `${props.top}px` }}>
      {props.banners}
      <div class="board-head">
        <span class="pr-board-title">Dashboard</span>
        <span class="hint">Ctrl+D for the view you were in</span>
      </div>
      <div class="dashboard-sections">
        <Section icon={<Bell />} title="Needs you" count={needYou().length} needs empty="No sessions need you">
          <For each={needYou()}>
            {(s) => (
              <Row colour={worktreeColour(s.worktree)} onOpen={() => props.onSession(s.id)} title="Open its Tab">
                <StateDot state={s.state} />
                <span class="dashboard-row-title">{s.name}</span>
                <span class="dashboard-row-wt">{label(s.worktree, null)}</span>
              </Row>
            )}
          </For>
        </Section>

        <Section
          icon={<MessageSquareCode />}
          title="Reviews requested"
          count={reviewRequests.list().length}
          empty="No reviews requested"
          github={{ refreshed: since(reviewRequests.refreshedAt()), loading: reviewRequests.loading(), error: reviewRequests.error(), onRefresh: reviewRequests.refresh }}
        >
          <For each={reviewRequests.list()}>
            {(r) => (
              <Row colour={worktreeOn(r.head) ? worktreeColour(worktreeOn(r.head)!.path) : undefined} onOpen={() => props.onReviewRequest(r)} title={`#${r.number} ${r.title}`}>
                <GitPullRequestArrow />
                <span class="dashboard-row-title">{r.title}</span>
                <span class="pr-number">#{r.number}</span>
                <span class="dashboard-row-meta">
                  {r.author}
                  <Show when={r.requestedAt}>
                    {(at) => (
                      <>
                        {" · asked "}
                        {(now(), ago(at()))}
                      </>
                    )}
                  </Show>
                </span>
                <Show when={r.reRequested}>
                  <span class="dashboard-tag">re-requested</span>
                </Show>
                <ChecksSummary checks={r.checks} />
              </Row>
            )}
          </For>
        </Section>

        <Section
          icon={<GitPullRequestArrow />}
          title="Your PRs"
          count={openPrs().length}
          empty="No open PRs"
          github={{ refreshed: since(myPrs.refreshedAt()), loading: myPrs.loading(), error: myPrs.error().replace(PR_HINT, GH_HINT), onRefresh: myPrs.refresh }}
          action={
            <button class="link" onClick={() => props.onAllPrs()} title="Every PR of yours, merged and closed ones too, on the PR board">
              All PRs →
            </button>
          }
        >
          <For each={STAGES}>
            {(group) => {
              const here = () => openPrs().filter((p) => p.stage === group.stage);
              return (
                <Show when={here().length}>
                  <div class={`dashboard-group pr-stage-${group.stage}`}>
                    {group.icon()}
                    {group.title}
                    <span class="badge">{here().length}</span>
                  </div>
                  <For each={here()}>{(pr) => <PrRow pr={pr} worktree={worktreeOn(pr.head)} onOpen={() => props.onPr(pr)} onStartSession={() => props.onStartSession(pr)} />}</For>
                </Show>
              );
            }}
          </For>
        </Section>

        <Section icon={<GitBranch />} title="Worktrees that need attention" count={attention.list().length} empty="No Worktrees need attention" error={attention.error()}>
          <For each={attention.list()}>
            {(w: WorktreeAttention) => (
              <Row colour={worktreeColour(w.path)} onOpen={() => props.onWorktree(w.path)} title={`${w.path}: switch to it, with its changes`}>
                <GitBranch />
                <span class="dashboard-row-title mono">{label(w.path, w.branch)}</span>
                <span class="dashboard-row-meta">{w.reasons.map(reasonText).join(" · ")}</span>
              </Row>
            )}
          </For>
        </Section>

        <Section icon={<Eraser />} title="Ready to clean up" count={cleanUp.list().length} empty="Nothing to clean up" error={cleanUp.error()} onHead={() => props.onCleanUp()}>
          <For each={cleanUp.list()}>
            {(w: WorktreeMerge) => (
              <Row colour={worktreeColour(w.path)} onOpen={() => props.onCleanUp()} title="Open the Worktrees overview">
                <GitBranch />
                <span class="dashboard-row-title mono">{label(w.path, w.branch)}</span>
                <span class="dashboard-row-meta">{mergeText(w.merge)}</span>
              </Row>
            )}
          </For>
        </Section>

        <Section icon={<Sparkles />} title="Sessions">
          <p class="dashboard-line">
            <span class="dashboard-count">
              <StateDot state="working" />
              {count("working")} Working
            </span>
            <span class="dashboard-count">
              <StateDot state="idle" />
              {count("idle", "suspended")} Idle
            </span>
          </p>
        </Section>
      </div>
    </section>
  );
}

/** A section: its head (and, for a GitHub one, when it was read and a refresh button), then its
 *  items, or a single line when it has none. */
function Section(props: {
  icon: JSX.Element;
  title: string;
  /** Leave it out for a section that's always one line. */
  count?: number;
  needs?: boolean;
  empty?: string;
  error?: string;
  github?: { refreshed: string; loading: boolean; error: string; onRefresh: () => void };
  /** The head opens somewhere too. */
  onHead?: () => void;
  action?: JSX.Element;
  children: JSX.Element;
}) {
  const error = () => props.github?.error || props.error || "";
  const asking = () => !!props.github?.loading && !props.github.refreshed;
  return (
    <div class="dashboard-section">
      <div class="dashboard-section-head">
        {props.icon}
        <Show when={props.onHead} fallback={<span>{props.title}</span>}>
          <button class="link dashboard-head-link" onClick={() => props.onHead!()}>
            {props.title}
            <ChevronRight />
          </button>
        </Show>
        <Show when={props.count !== undefined}>
          <span class="badge" classList={{ needs: props.needs && props.count! > 0 }}>
            {props.count}
          </span>
        </Show>
        <span class="grow" />
        {props.action}
        <Show when={props.github}>
          {(gh) => (
            <>
              <Show when={gh().refreshed}>
                <span class="dashboard-refreshed">refreshed {gh().refreshed}</span>
              </Show>
              <button class="ghost icon" onClick={() => gh().onRefresh()} disabled={gh().loading} title="Ask GitHub again" aria-label={`Refresh ${props.title}`}>
                <RefreshCw classList={{ spin: gh().loading }} />
              </button>
            </>
          )}
        </Show>
      </div>
      <Show when={error()}>
        <p class="error small dashboard-error">{error()}</p>
      </Show>
      <Show
        when={props.count === undefined || props.count > 0}
        fallback={
          <Show when={!error()}>
            <p class="dashboard-line muted">
              <Show when={asking()} fallback={props.empty}>
                <Loader class="spin" />
                Asking GitHub…
              </Show>
            </p>
          </Show>
        }
      >
        {props.children}
      </Show>
    </div>
  );
}

/** One item: the whole row opens where it lives (its title is the button for the keyboard). */
function Row(props: { colour?: string; title: string; onOpen: () => void; children: JSX.Element; below?: JSX.Element }) {
  return (
    <div class="dashboard-row" style={{ "--c": props.colour ?? "var(--line-strong)" }} onClick={() => props.onOpen()}>
      <button
        class="dashboard-row-open"
        onClick={(e) => {
          e.stopPropagation();
          props.onOpen();
        }}
        title={props.title}
      >
        {props.children}
      </button>
      {props.below}
    </div>
  );
}

/** One of the user's PRs; one with failing checks or changes requested can start an Agent session on it. */
function PrRow(props: { pr: MyPullRequest; worktree?: WorktreeTab; onOpen: () => void; onStartSession: () => Promise<void> }) {
  const p = () => props.pr;
  const [starting, setStarting] = createSignal(false);
  const [failed, setFailed] = createSignal("");
  const canStart = () => p().stage === "checksFailing" || p().stage === "changesRequested";
  const start = async () => {
    setStarting(true);
    setFailed("");
    try {
      await props.onStartSession();
    } catch (err) {
      setFailed(String(err));
    } finally {
      setStarting(false);
    }
  };
  return (
    <Row
      colour={props.worktree ? worktreeColour(props.worktree.path) : undefined}
      onOpen={props.onOpen}
      title={`#${p().number} ${p().title}`}
      below={
        <Show when={canStart() || failed()}>
          <div class="dashboard-row-actions" onClick={(e) => e.stopPropagation()}>
            <Show when={canStart()}>
              <button
                onClick={() => void start()}
                disabled={starting()}
                title={
                  p().stage === "checksFailing"
                    ? "A session on its branch, its composer filled with the failed checks and their logs (not sent)"
                    : "A session on its branch, its composer filled with the changes asked for (not sent)"
                }
              >
                <Show when={starting()} fallback={<Sparkles />}>
                  <Loader class="spin" />
                </Show>
                Start an Agent session
              </button>
            </Show>
            <Show when={failed()}>
              <p class="error small">{failed()}</p>
            </Show>
          </div>
        </Show>
      }
    >
      <Show when={p().draft} fallback={<GitPullRequestArrow />}>
        <GitPullRequestDraft />
      </Show>
      <span class="dashboard-row-title">{p().title}</span>
      <span class="pr-number">#{p().number}</span>
      <span class="dashboard-row-wt">{p().head}</span>
      <Show when={p().conflicts}>
        <span class="pr-card-line bad">
          <TriangleAlert />
          Conflicts
        </span>
      </Show>
      <ChecksSummary checks={p().checks} />
    </Row>
  );
}

/** How a PR's checks stand, as the PR board's cards say it. */
function ChecksSummary(props: { checks: Check[] }) {
  const n = (outcome: Check["outcome"]) => props.checks.filter((c) => c.outcome === outcome).length;
  return (
    <Show when={props.checks.length}>
      <span class="pr-card-line dashboard-checks" title={props.checks.map((c) => `${c.name}: ${c.outcome}`).join("\n")}>
        <Show when={n("failure")}>
          <span class="bad">
            <CircleX />
            {n("failure")} failing
          </span>
        </Show>
        <Show when={n("pending")}>
          <span class="wait">
            <Clock />
            {n("pending")} running
          </span>
        </Show>
        <Show when={n("success")}>
          <span class="ok">
            <CircleCheck />
            {n("success")} passed
          </span>
        </Show>
      </span>
    </Show>
  );
}

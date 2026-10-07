// The PR board: the PRs the user opened in the repo (as the GitHub CLI is logged in) in columns by
// where each stands: draft, waiting for review, changes requested, checks failing, approved, and
// done (the latest merged or closed). A card says what's holding it up: who's reviewed it and how,
// which checks failed, whether it conflicts. Clicking one opens it in the PR overlay. Opened from
// the sidebar's Pull requests; the same list feeds both, read again every few minutes.
import { openUrl } from "@tauri-apps/plugin-opener";
import { createEffect, createMemo, createSignal, For, type JSX, Match, on, onCleanup, Show, Switch } from "solid-js";
import { ago } from "./Chat";
import { core, type MyPullRequest, type PullRequestStage, type Reviewer } from "./core";
import {
  Check as CheckIcon,
  CircleCheck,
  CircleDot,
  CircleX,
  Clock,
  ExternalLink,
  GitBranch,
  GitMerge,
  GitPullRequestArrow,
  GitPullRequestClosed,
  GitPullRequestDraft,
  MessageSquare,
  RefreshCw,
  TriangleAlert,
} from "./icons";
import { worktreeColour, type WorktreeTab } from "./Worktrees";

/** How often the user's PRs are asked for again while the Workspace is open. */
const EVERY_MS = 5 * 60_000;

const [prs, setPrs] = createSignal<MyPullRequest[]>([]);
const [error, setError] = createSignal("");
const [loading, setLoading] = createSignal(false);
let from = "";
let asked = 0;

/** The user's PRs as last read (every open one, then the latest merged or closed), and how that went. */
export const myPrs = { prs, error, loading, refresh: () => void load() };

async function load() {
  if (!from) return;
  const n = ++asked;
  setLoading(true);
  try {
    const next = await core.myPullRequests(from);
    if (n !== asked) return;
    setPrs(next);
    setError("");
  } catch (err) {
    if (n === asked) setError(String(err));
  } finally {
    if (n === asked) setLoading(false);
  }
}

/** Keeps `myPrs` read from the Worktree `worktree` names (the main one), now and every few minutes. */
export function watchMyPrs(worktree: () => string) {
  const where = createMemo(worktree);
  createEffect(
    on(where, (w) => {
      from = w;
      setPrs([]);
      setError("");
      void load();
    }),
  );
  const timer = setInterval(() => void load(), EVERY_MS);
  onCleanup(() => {
    clearInterval(timer);
    from = "";
    asked++;
  });
}

const COLUMNS: { title: string; stages: PullRequestStage[]; icon: () => JSX.Element }[] = [
  { title: "Draft", stages: ["draft"], icon: () => <GitPullRequestDraft /> },
  { title: "Waiting for review", stages: ["waitingForReview"], icon: () => <Clock /> },
  { title: "Changes requested", stages: ["changesRequested"], icon: () => <MessageSquare /> },
  { title: "Checks failing", stages: ["checksFailing"], icon: () => <CircleX /> },
  { title: "Approved", stages: ["approved"], icon: () => <CircleCheck /> },
  { title: "Done", stages: ["merged", "closed"], icon: () => <GitMerge /> },
];

export function PrBoard(props: {
  worktrees: WorktreeTab[];
  /** Where the board sits: under the title bar. */
  top: number;
  /** The Workspace's error and notice banners, shown here too. */
  banners?: JSX.Element;
  onOpen: (pr: MyPullRequest) => void;
}) {
  // (Opening the board asks again: it's for seeing where things stand now.)
  myPrs.refresh();
  const open = () => prs().filter((p) => p.state === "open").length;
  return (
    <section class="board pr-board" style={{ top: `${props.top}px` }}>
      {props.banners}
      <div class="board-head">
        <span class="pr-board-title">Your pull requests</span>
        <span class="badge">{open()} open</span>
        <button class="ghost icon" onClick={() => myPrs.refresh()} disabled={loading()} title="Ask GitHub again" aria-label="Refresh pull requests">
          <RefreshCw classList={{ spin: loading() }} />
        </button>
        <span class="hint">Esc for the Tabs</span>
      </div>
      <Show when={error()}>
        <p class="error small pr-board-error">Couldn't list your PRs: {error()}</p>
      </Show>
      <div class="board-columns pr-board-columns">
        <For each={COLUMNS}>
          {(column) => {
            const cards = () => prs().filter((p) => column.stages.includes(p.stage));
            return (
              <div class="board-column">
                <div class={`board-column-head pr-stage-${column.stages[0]}`}>
                  {column.icon()}
                  {column.title}
                  <span class="badge">{cards().length}</span>
                </div>
                <For each={cards()} fallback={<p class="board-empty">{loading() && !prs().length ? "Asking GitHub…" : "None"}</p>}>
                  {(pr) => <Card pr={pr} worktree={props.worktrees.find((w) => !w.removed && w.branch === pr.head)} onOpen={() => props.onOpen(pr)} />}
                </For>
              </div>
            );
          }}
        </For>
      </div>
    </section>
  );
}

function Card(props: { pr: MyPullRequest; worktree?: WorktreeTab; onOpen: () => void }) {
  const p = () => props.pr;
  const count = (outcome: string) => p().checks.filter((c) => c.outcome === outcome).length;
  const failing = () => p().checks.filter((c) => c.outcome === "failure");
  const done = () => p().mergedAt ?? p().closedAt;
  return (
    <div
      class={`board-card pr-card ${p().stage}`}
      style={{ "--c": props.worktree ? worktreeColour(props.worktree.path) : "var(--line-strong)" }}
      onClick={() => props.onOpen()}
    >
      <div class="board-card-top">
        <GitBranch />
        <span class="wt" title={`${p().head} → ${p().base}${props.worktree ? "\nChecked out in a Worktree" : ""}`}>
          {p().head} → {p().base}
        </span>
        <button
          class="ghost icon pr-card-link"
          onClick={(e) => {
            e.stopPropagation();
            void openUrl(p().url).catch((err) => setError(String(err)));
          }}
          title={`${p().url}: open it in the browser`}
          aria-label="Open on GitHub"
        >
          <ExternalLink />
        </button>
      </div>
      <div class="board-card-head">
        <Show when={p().state === "open"} fallback={p().state === "merged" ? <GitMerge class="merged" /> : <GitPullRequestClosed class="closed" />}>
          <Show when={p().draft} fallback={<GitPullRequestArrow />}>
            <GitPullRequestDraft />
          </Show>
        </Show>
        <button
          class="board-card-open"
          onClick={(e) => {
            e.stopPropagation();
            props.onOpen();
          }}
          title={`#${p().number} ${p().title}`}
        >
          {p().title}
        </button>
        <span class="end pr-number">#{p().number}</span>
      </div>
      <Show when={p().conflicts && p().state === "open"}>
        <span class="pr-card-line bad">
          <TriangleAlert />
          Conflicts with {p().base}
        </span>
      </Show>
      <Show when={p().checks.length && p().state === "open"}>
        <span class="pr-card-line pr-card-checks" title={p().checks.map((c) => `${c.name}: ${c.outcome}`).join("\n")}>
          <Show when={count("failure")}>
            <span class="bad">
              <CircleX />
              {count("failure")} failing
            </span>
          </Show>
          <Show when={count("pending")}>
            <span class="wait">
              <Clock />
              {count("pending")} running
            </span>
          </Show>
          <Show when={count("success")}>
            <span class="ok">
              <CircleCheck />
              {count("success")} passed
            </span>
          </Show>
        </span>
        <Show when={failing().length}>
          <span class="pr-card-failing">
            {failing()
              .slice(0, 3)
              .map((c) => c.name)
              .join(", ")}
            {failing().length > 3 ? ` and ${failing().length - 3} more` : ""}
          </span>
        </Show>
      </Show>
      <Show when={p().reviewers.length && p().state === "open"}>
        <span class="pr-card-reviewers">
          <For each={p().reviewers}>{(r) => <ReviewerChip reviewer={r} />}</For>
        </span>
      </Show>
      <span class="summary">
        <Show when={done()} fallback={`Updated ${ago(p().updatedAt)}`}>
          {(at) => `${p().state === "merged" ? "Merged" : "Closed"} ${ago(at())}`}
        </Show>
      </span>
    </div>
  );
}

const REVIEWED: Record<Reviewer["state"], string> = {
  requested: "asked to review",
  approved: "approved",
  changesRequested: "asked for changes",
  commented: "commented",
  dismissed: "review dismissed",
};

function ReviewerChip(props: { reviewer: Reviewer }) {
  const r = () => props.reviewer;
  /** Asked to review again since their last review: waited on, whatever that review said. */
  const again = () => r().requested && r().state !== "requested";
  return (
    <span
      class={`pr-card-reviewer ${again() ? "requested" : r().state}`}
      title={`${r().name}${r().team ? " (team)" : ""}: ${REVIEWED[r().state]}${again() ? ", asked to review again" : ""}`}
    >
      <Switch fallback={<MessageSquare />}>
        <Match when={again()}>
          <Clock />
        </Match>
        <Match when={r().state === "approved"}>
          <CheckIcon />
        </Match>
        <Match when={r().state === "changesRequested"}>
          <CircleX />
        </Match>
        <Match when={r().state === "requested"}>
          <CircleDot />
        </Match>
      </Switch>
      {r().name}
    </span>
  );
}

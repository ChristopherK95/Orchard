// The PR overlay: the PR of a Worktree's branch (its open one, else its latest) as GitHub has it,
// read through the GitHub CLI. Where it stands (open, draft, merged, closed; whether it can be
// merged; its checks), who's on it (author, assignees, reviewers and where each stands), and what's
// been said: the description and conversation, the comments on its code by thread, its commits.
// Opened from the title bar in the Tabs view, a column's header, a Board card, or the sidebar's list
// of the user's PRs (any of the repo's, by number). Comments can be narrowed to the people in some
// of the organisation's teams (a sub-team counts as its top team).
import { openUrl } from "@tauri-apps/plugin-opener";
import { createContext, createEffect, createMemo, createSignal, For, type JSX, Match, on, onCleanup, onMount, Show, Switch, useContext } from "solid-js";
import { createStore } from "solid-js/store";
import { ago } from "./Chat";
import { core, type Check, type CodeThread, type PullRequestComment, type PullRequestDetails, type Reviewer, type ReviewerState } from "./core";
import {
  Check as CheckIcon,
  CircleCheck,
  CircleDot,
  CircleMinus,
  CircleX,
  Clock,
  ExternalLink,
  GitBranch,
  GitCommit,
  GitMerge,
  GitPullRequestArrow,
  GitPullRequestClosed,
  GitPullRequestDraft,
  Loader,
  MessageSquare,
  MessageSquareCode,
  RefreshCw,
  RotateCcw,
  TriangleAlert,
  X,
} from "./icons";
import { handleTranscriptClick, renderMarkdown } from "./markdown";

/** What the overlay last found for each Worktree (null: no PR), so the buttons can say "#12". */
const [known, setKnown] = createStore<Record<string, { number: number; state: PullRequestDetails["state"] } | null>>({});

/** The button that opens the overlay: "PR", or the PR's number once it's known. */
export function PrButton(props: { worktree: string; class?: string; compact?: boolean; disabled?: boolean; onOpen: () => void }) {
  const pr = () => known[props.worktree];
  return (
    <button
      class={`pr-button ${props.class ?? ""}`}
      classList={{ [pr()?.state ?? "unknown"]: true }}
      disabled={props.disabled}
      onClick={(e) => {
        e.stopPropagation();
        props.onOpen();
      }}
      title={pr() ? `PR #${pr()!.number}: its state, reviewers and comments` : "This branch's pull request: its state, reviewers and comments"}
      aria-label="Pull request"
    >
      <GitPullRequestArrow />
      <Show when={!props.compact || pr()}>
        <span class="label">{pr() ? `#${pr()!.number}` : "PR"}</span>
      </Show>
    </button>
  );
}

type Tab = "conversation" | "code" | "commits" | "checks";

/** The group for people in none of the teams (bots, outside contributors). */
const OTHER = "Other";
/** Bots that get a group of their own, by login (GitHub's API says `name` or `name[bot]`). */
const BOTS: Record<string, string> = { coderabbitai: "Coderabbit", "github-actions": "Actions" };
const botOf = (login: string) => BOTS[login.toLowerCase().replace(/^app\//, "").replace(/\[bot\]$/, "")];
/** A team's name as its chip and tags say it. */
const short = (name: string) => (name === "Developers" ? "Dev" : name);
/** The teams whose people's comments are shown (none: everyone's). Kept between openings. */
const [only, setOnly] = createSignal<string[]>([]);

/** Who's in which team, and the filter, for everything in the overlay. */
interface Teams {
  /** The (short) names of the teams `login` is in; none when they're in no team. */
  of: (login: string) => string[];
  /** The class giving the team its colour, on its chip and tags. */
  colour: (team: string) => string;
  /** Whether `login`'s comments pass the filter. */
  shown: (login: string) => boolean;
}
const TeamsContext = createContext<Teams>({ of: () => [], colour: () => "team-other", shown: () => true });

function createTeams(pr: () => PullRequestDetails): Teams {
  const byLogin = createMemo(() => {
    const map = new Map<string, string[]>();
    for (const team of pr().teams) for (const login of team.members) map.set(login.toLowerCase(), [...(map.get(login.toLowerCase()) ?? []), short(team.name)]);
    return map;
  });
  const order = createMemo(() => pr().teams.map((t) => short(t.name)));
  const of = (login: string) => {
    const bot = botOf(login);
    return bot ? [bot] : (byLogin().get(login.toLowerCase()) ?? []);
  };
  return {
    of,
    colour: (team) =>
      team === OTHER ? "team-other" : Object.values(BOTS).includes(team) ? `team-${team.toLowerCase()}` : `team-${order().indexOf(team) % 6}`,
    shown: (login) => {
      const picked = only();
      if (!picked.length) return true;
      const teams = of(login);
      return teams.length ? teams.some((t) => picked.includes(t)) : picked.includes(OTHER);
    },
  };
}

/** Everyone who said something on the PR, once each time they did. */
const speakers = (pr: PullRequestDetails) => [
  ...pr.comments.map((c) => c.author),
  ...reviewEntries(pr).map((r) => r.author),
  ...pr.threads.flatMap((t) => t.comments.map((c) => c.author)),
];

/** The chips that narrow the comments to some teams' people, with how many comments each has. */
function TeamFilter(props: { pr: PullRequestDetails }) {
  const teams = useContext(TeamsContext);
  const counts = createMemo(() => {
    const n = new Map<string, number>();
    for (const login of speakers(props.pr)) {
      const of = teams.of(login);
      for (const t of of.length ? of : [OTHER]) n.set(t, (n.get(t) ?? 0) + 1);
    }
    return n;
  });
  /** The teams with someone who commented, in the organisation's order, then Other; and any
   *  picked earlier that has none here (so it can be unpicked). */
  const chips = createMemo(() => {
    const names = [...props.pr.teams.map((t) => short(t.name)), ...Object.values(BOTS), OTHER];
    return names.filter((t, i) => names.indexOf(t) === i && (counts().has(t) || only().includes(t)));
  });
  const toggle = (t: string) => setOnly((now) => (now.includes(t) ? now.filter((x) => x !== t) : [...now, t]));
  return (
    <Show when={props.pr.teams.length || props.pr.teamsError}>
      <div class="pr-filter">
        <span class="muted">Comments from</span>
        <button class="pr-chip" classList={{ on: only().length === 0 }} onClick={() => setOnly([])}>
          Everyone
        </button>
        <For each={chips()}>
          {(t) => (
            <button class={`pr-chip ${teams.colour(t)}`} classList={{ on: only().includes(t) }} onClick={() => toggle(t)} title={t === OTHER ? "Anyone in none of these" : Object.values(BOTS).includes(t) ? `The ${t} bot` : `People in ${t}${t === "Dev" ? " or any of its sub-teams" : ""}`}>
              {t}
              <span class="badge">{counts().get(t) ?? 0}</span>
            </button>
          )}
        </For>
        <Show when={props.pr.teamsError}>
          <span class="pr-filter-error" title={props.pr.teamsError!}>
            <TriangleAlert />
            Teams unavailable
          </span>
        </Show>
      </div>
    </Show>
  );
}

/** The teams someone is in, as small tags after their name. */
function TeamTags(props: { login: string }) {
  const teams = useContext(TeamsContext);
  return <For each={teams.of(props.login)}>{(t) => <span class={`pr-team ${teams.colour(t)}`}>{t}</span>}</For>;
}

export function PrOverlay(props: {
  worktree: string;
  /** A PR of the Worktree's repo to show instead of its branch's. */
  number?: number;
  label: string;
  colour: string;
  /** "Open in editor" on a code block. */
  onOpenSnippet: (code: string, label: string) => void;
  onClose: () => void;
}) {
  const [pr, setPr] = createSignal<PullRequestDetails | null | undefined>(undefined);
  const [error, setError] = createSignal("");
  const [loading, setLoading] = createSignal(false);
  const [tab, setTab] = createSignal<Tab>("conversation");

  const load = async () => {
    setLoading(true);
    setError("");
    try {
      const next = await core.pullRequestDetails(props.worktree, props.number);
      setPr(next);
      if (props.number === undefined) setKnown(props.worktree, next ? { number: next.number, state: next.state } : null);
    } catch (err) {
      setError(String(err));
    } finally {
      setLoading(false);
    }
  };
  onMount(() => void load());
  /** A re-run takes GitHub a moment to start: its checks are asked for again after it. */
  let reloadTimer: number | undefined;
  const rerunStarted = () => {
    clearTimeout(reloadTimer);
    reloadTimer = window.setTimeout(() => void load(), 4000);
  };
  onCleanup(() => clearTimeout(reloadTimer));

  const onKey = (e: KeyboardEvent) => {
    if (e.key !== "Escape") return;
    e.preventDefault();
    e.stopPropagation();
    props.onClose();
  };
  onMount(() => window.addEventListener("keydown", onKey, true));
  onCleanup(() => window.removeEventListener("keydown", onKey, true));

  const open = (url: string) => void openUrl(url).catch((err) => setError(String(err)));
  /** Links open in the browser; a code block's Open in editor closes the overlay first. */
  const onBodyClick = (e: MouseEvent) =>
    void handleTranscriptClick(
      e,
      openUrl,
      (code, label) => {
        props.onClose();
        props.onOpenSnippet(code, label);
      },
      setError,
    );

  return (
    <div class="modal-backdrop pr-backdrop" onClick={(e) => e.target === e.currentTarget && props.onClose()}>
      <div class="modal pr-overlay" role="dialog" aria-label="Pull request" style={{ "--c": props.colour }}>
        <div class="modal-head pr-head">
          <Show when={pr()} fallback={<GitPullRequestArrow class="pr-mark" />}>
            {(p) => <StateIcon pr={p()} class="pr-mark" />}
          </Show>
          <b>
            <span class="pr-title">
              <Show when={pr()} fallback="Pull request">
                {(p) => (
                  <>
                    {p().title} <span class="pr-number">#{p().number}</span>
                  </>
                )}
              </Show>
            </span>
            <span class="pr-branches muted">
              <GitBranch />
              <span class="mono">{pr()?.head ?? props.label}</span>
              <Show when={pr()}>
                {(p) => (
                  <>
                    <span class="arrow">→</span>
                    <span class="mono">{p().base}</span>
                  </>
                )}
              </Show>
            </span>
          </b>
          <Show when={pr()}>
            {(p) => (
              <>
                <StateChip pr={p()} />
                <button class="outline" onClick={() => open(p().url)} title={`${p().url}: open it in the browser`}>
                  <ExternalLink />
                  GitHub
                </button>
              </>
            )}
          </Show>
          <button class="ghost icon" onClick={() => void load()} disabled={loading()} title="Ask GitHub again" aria-label="Refresh">
            <RefreshCw classList={{ spin: loading() }} />
          </button>
          <button class="ghost icon" onClick={() => props.onClose()} title="Close (Esc)" aria-label="Close">
            <X />
          </button>
        </div>
        <Show when={error()}>
          <p class="error modal-error pr-error">{error()}</p>
        </Show>
        <Switch>
          <Match when={pr() === undefined && !error()}>
            <div class="review-loading muted">
              <Loader class="spin" />
              Asking GitHub…
            </div>
          </Match>
          <Match when={pr() === null}>
            <div class="pr-empty">
              <GitPullRequestArrow />
              <p>
                <span class="mono">{props.label}</span> has no pull request yet.
              </p>
              <p class="muted">Review opens one once its changes look done.</p>
            </div>
          </Match>
          <Match when={pr()}>
            {(p) => {
              const teams = createTeams(p);
              /** How many conversation entries and code threads the team filter lets through. */
              const conversationCount = () => [...p().comments.map((c) => c.author), ...reviewEntries(p()).map((r) => r.author)].filter(teams.shown).length;
              const threadCount = () => p().threads.filter((t) => t.comments.some((c) => teams.shown(c.author))).length;
              return (
                <TeamsContext.Provider value={teams}>
                  <Summary pr={p()} />
                  <div class="pr-body">
                    <div class="pr-main">
                      <nav class="segmented pr-tabs">
                        <button classList={{ on: tab() === "conversation" }} onClick={() => setTab("conversation")}>
                          <MessageSquare />
                          Conversation
                          <span class="badge">{conversationCount()}</span>
                        </button>
                        <button classList={{ on: tab() === "code" }} onClick={() => setTab("code")}>
                          <MessageSquareCode />
                          Code comments
                          <span class="badge">{threadCount()}</span>
                        </button>
                        <button classList={{ on: tab() === "commits" }} onClick={() => setTab("commits")}>
                          <GitCommit />
                          Commits
                          <span class="badge">{p().commits.length}</span>
                        </button>
                        <button classList={{ on: tab() === "checks" }} onClick={() => setTab("checks")}>
                          <CircleCheck />
                          Checks
                          <span class="badge">{p().checks.length}</span>
                        </button>
                      </nav>
                      <Show when={tab() === "conversation" || tab() === "code"}>
                        <TeamFilter pr={p()} />
                      </Show>
                      <div class="pr-scroll" onClick={onBodyClick}>
                        <Switch>
                          <Match when={tab() === "conversation"}>
                            <Conversation pr={p()} onOpen={open} />
                          </Match>
                          <Match when={tab() === "code"}>
                            <CodeComments pr={p()} onOpen={open} />
                          </Match>
                          <Match when={tab() === "commits"}>
                            <ul class="pr-list">
                              <For each={p().commits} fallback={<li class="muted">No commits.</li>}>
                                {(c) => (
                                  <li class="pr-commit">
                                    <span class="mono muted">{c.shortId}</span>
                                    <span class="grow ellipsis">{c.subject}</span>
                                    <span class="muted">{c.author}</span>
                                    <span class="muted when">{ago(c.date)}</span>
                                  </li>
                                )}
                              </For>
                            </ul>
                          </Match>
                          <Match when={tab() === "checks"}>
                            <Checks pr={p()} worktree={props.worktree} onOpen={open} onError={setError} onRerun={rerunStarted} />
                          </Match>
                        </Switch>
                      </div>
                    </div>
                    <Sidebar pr={p()} onChecks={() => setTab("checks")} />
                  </div>
                </TeamsContext.Provider>
              );
            }}
          </Match>
        </Switch>
      </div>
    </div>
  );
}

function StateIcon(props: { pr: PullRequestDetails; class?: string }) {
  return (
    <Switch fallback={<GitPullRequestArrow class={`${props.class ?? ""} open`} />}>
      <Match when={props.pr.state === "merged"}>
        <GitMerge class={`${props.class ?? ""} merged`} />
      </Match>
      <Match when={props.pr.state === "closed"}>
        <GitPullRequestClosed class={`${props.class ?? ""} closed`} />
      </Match>
      <Match when={props.pr.draft}>
        <GitPullRequestDraft class={`${props.class ?? ""} draft`} />
      </Match>
    </Switch>
  );
}

function StateChip(props: { pr: PullRequestDetails }) {
  const kind = () => (props.pr.state === "open" && props.pr.draft ? "draft" : props.pr.state);
  const text = { open: "Open", draft: "Draft", merged: "Merged", closed: "Closed" };
  return (
    <span class={`pr-state ${kind()}`}>
      <StateIcon pr={props.pr} />
      {text[kind()]}
    </span>
  );
}

/** The line under the title: who opened it when, and how big it is. */
function Summary(props: { pr: PullRequestDetails }) {
  const p = () => props.pr;
  return (
    <div class="pr-summary">
      <span>
        <b>{p().author}</b> opened it {ago(p().createdAt)}
      </span>
      <Show when={p().state === "merged"}>
        <span>
          · merged{p().mergedBy ? <> by <b>{p().mergedBy}</b></> : ""} {ago(p().mergedAt)}
        </span>
      </Show>
      <Show when={p().state === "closed"}>
        <span>· closed {ago(p().closedAt)}</span>
      </Show>
      <span>· updated {ago(p().updatedAt)}</span>
      <span class="grow" />
      <span class="pr-size">
        <span class="add">+{p().additions}</span> <span class="del">−{p().deletions}</span>
        <span class="muted">
          {" "}
          in {p().changedFiles} file{p().changedFiles === 1 ? "" : "s"} · {p().commits.length} commit{p().commits.length === 1 ? "" : "s"}
        </span>
      </span>
    </div>
  );
}

/** GitHub's HTML comments (PR templates' hints) would show as text: they're dropped. */
const markdown = (text: string) => renderMarkdown(text.replace(/<!--[\s\S]*?-->/g, "").trim());

const REVIEW_VERB: Record<string, string> = {
  APPROVED: "approved these changes",
  CHANGES_REQUESTED: "requested changes",
  COMMENTED: "reviewed",
  DISMISSED: "reviewed (dismissed)",
};

/** Reviews worth a place in the conversation: a verdict, or something said. (A bare "reviewed"
 *  only holds comments on the code, which have their own tab.) */
const reviewEntries = (pr: PullRequestDetails) => pr.reviews.filter((r) => r.state !== "COMMENTED" || r.body.trim());

function Conversation(props: { pr: PullRequestDetails; onOpen: (url: string) => void }) {
  type Entry =
    | { kind: "comment"; at: string; comment: PullRequestComment }
    | { kind: "review"; at: string; author: string; state: string; body: string };
  const teams = useContext(TeamsContext);
  const entries = createMemo(() => {
    const all: Entry[] = [
      ...props.pr.comments.map((comment): Entry => ({ kind: "comment", at: comment.createdAt, comment })),
      ...reviewEntries(props.pr).map((r): Entry => ({ kind: "review", at: r.submittedAt, author: r.author, state: r.state, body: r.body })),
    ];
    return all.filter((e) => teams.shown(e.kind === "comment" ? e.comment.author : e.author)).sort((a, b) => a.at.localeCompare(b.at));
  });
  return (
    <div class="pr-timeline">
      <Show when={only().length === 0}>
        <Post author={props.pr.author} what="opened it" at={props.pr.createdAt} kind="description">
          <Show when={props.pr.body.trim()} fallback={<p class="muted">No description.</p>}>
            <div class="markdown" innerHTML={markdown(props.pr.body)} />
          </Show>
        </Post>
      </Show>
      <For each={entries()}>
        {(e) => (
          <Switch>
            <Match when={e.kind === "comment" && e.comment}>
              {(c) => (
                <Post
                  author={c().author}
                  what={c().edited ? "commented (edited)" : "commented"}
                  at={c().createdAt}
                  url={c().url}
                  onOpen={props.onOpen}
                >
                  <Show when={!c().minimized} fallback={<p class="muted">Hidden on GitHub.</p>}>
                    <div class="markdown" innerHTML={markdown(c().body)} />
                  </Show>
                </Post>
              )}
            </Match>
            <Match when={e.kind === "review" && e}>
              {(r) => {
                const review = r() as Extract<Entry, { kind: "review" }>;
                return (
                  <Post author={review.author} what={REVIEW_VERB[review.state] ?? "reviewed"} at={review.at} kind={review.state.toLowerCase()}>
                    <Show when={review.body.trim()}>
                      <div class="markdown" innerHTML={markdown(review.body)} />
                    </Show>
                  </Post>
                );
              }}
            </Match>
          </Switch>
        )}
      </For>
      <Show when={entries().length === 0}>
        <p class="muted pr-quiet">{only().length ? `No comments from ${only().join(" or ")}.` : "Nobody has commented yet."}</p>
      </Show>
    </div>
  );
}

/** One entry in the conversation: who, what they did and when, then what they said. */
function Post(props: { author: string; what: string; at: string; url?: string | null; kind?: string; onOpen?: (url: string) => void; children?: JSX.Element }) {
  return (
    <article class={`pr-post ${props.kind ?? ""}`}>
      <header>
        <ReviewMark kind={props.kind} />
        <b>{props.author}</b>
        <TeamTags login={props.author} />
        <span class="muted">
          {props.what} {ago(props.at)}
        </span>
        <span class="grow" />
        <Show when={props.url && props.onOpen}>
          <button class="ghost icon" onClick={() => props.onOpen!(props.url!)} title="Open it on GitHub" aria-label="Open on GitHub">
            <ExternalLink />
          </button>
        </Show>
      </header>
      <Show when={props.children}>
        <div class="pr-post-body">{props.children}</div>
      </Show>
    </article>
  );
}

function ReviewMark(props: { kind?: string }) {
  return (
    <Switch fallback={<MessageSquare class="pr-post-mark" />}>
      <Match when={props.kind === "approved"}>
        <CircleCheck class="pr-post-mark approved" />
      </Match>
      <Match when={props.kind === "changes_requested"}>
        <CircleX class="pr-post-mark changes" />
      </Match>
      <Match when={props.kind === "description"}>
        <GitPullRequestArrow class="pr-post-mark" />
      </Match>
    </Switch>
  );
}

/** The comments on its code: by file, each thread under the lines it's about. */
function CodeComments(props: { pr: PullRequestDetails; onOpen: (url: string) => void }) {
  const teams = useContext(TeamsContext);
  const byFile = createMemo(() => {
    const files = new Map<string, CodeThread[]>();
    for (const t of props.pr.threads.filter((t) => t.comments.some((c) => teams.shown(c.author)))) files.set(t.path, [...(files.get(t.path) ?? []), t]);
    return [...files.entries()].sort(([a], [b]) => a.localeCompare(b));
  });
  return (
    <div class="pr-threads">
      <Show when={props.pr.threadsError}>
        <p class="error small">Couldn't read the comments on its code: {props.pr.threadsError}</p>
      </Show>
      <Show when={!props.pr.threadsError && byFile().length === 0}>
        <p class="muted pr-quiet">{only().length && props.pr.threads.length ? `No comments on its code from ${only().join(" or ")}.` : "No comments on its code."}</p>
      </Show>
      <For each={byFile()}>
        {([path, threads]) => (
          <section class="pr-file">
            <h4 class="mono">{path}</h4>
            <For each={threads}>
              {(t) => (
                <div class="pr-thread" classList={{ outdated: t.line === null }}>
                  <div class="pr-thread-head">
                    <span class="mono muted">line {t.line ?? t.originalLine ?? "?"}</span>
                    <Show when={t.line === null}>
                      <span class="pr-outdated">Outdated</span>
                    </Show>
                    <span class="grow" />
                    <button class="ghost icon" onClick={() => props.onOpen(t.url)} title="Open the thread on GitHub" aria-label="Open on GitHub">
                      <ExternalLink />
                    </button>
                  </div>
                  <Hunk text={t.diffHunk} />
                  <For each={t.comments}>
                    {(c) => (
                      // (A reply from outside the filter stays, faded, so the thread still reads.)
                      <div class="pr-thread-comment" classList={{ faded: !teams.shown(c.author) }}>
                        <div class="who">
                          <b>{c.author}</b>
                          <TeamTags login={c.author} />
                          <span class="muted">{ago(c.createdAt)}</span>
                        </div>
                        <div class="markdown" innerHTML={markdown(c.body)} />
                      </div>
                    )}
                  </For>
                </div>
              )}
            </For>
          </section>
        )}
      </For>
    </div>
  );
}

/** The last lines of a thread's diff hunk: the ones it's about. */
function Hunk(props: { text: string }) {
  const lines = () => props.text.split("\n").filter((l) => !l.startsWith("@@")).slice(-6);
  return (
    <Show when={lines().length}>
      <pre class="pr-hunk">
        <For each={lines()}>{(l) => <span class={l.startsWith("+") ? "add" : l.startsWith("-") ? "del" : ""}>{l || " "}</span>}</For>
      </pre>
    </Show>
  );
}

/** The Checks tab: every check, a GitHub Actions one with Re-run once it's finished, and Re-run
 *  failed for every failed job of the runs that have them. */
function Checks(props: { pr: PullRequestDetails; worktree: string; onOpen: (url: string) => void; onError: (e: string) => void; onRerun: () => void }) {
  /** What's being re-run (a job id, or "failed"), and what has been since the checks were read. */
  const [busy, setBusy] = createSignal<number | "failed" | null>(null);
  const [started, setStarted] = createSignal<(number | "failed")[]>([]);
  // (Read again, the checks say for themselves what's running: a re-run job gets a new id.)
  createEffect(on(() => props.pr, () => setStarted([]), { defer: true }));
  const finished = (c: Check) => c.outcome !== "pending";
  /** The runs with a failed job. */
  const failedRuns = () => [...new Set(props.pr.checks.filter((c) => c.outcome === "failure" && c.job).map((c) => c.job!.run))];
  const rerun = async (what: number | "failed", go: () => Promise<unknown>) => {
    setBusy(what);
    props.onError("");
    try {
      await go();
      setStarted((s) => [...s, what]);
      props.onRerun();
    } catch (err) {
      props.onError(`Couldn't re-run: ${String(err)}`);
    } finally {
      setBusy(null);
    }
  };
  return (
    <>
      <Show when={failedRuns().length}>
        <div class="pr-checks-actions">
          <button
            class="outline"
            disabled={busy() !== null || started().includes("failed")}
            onClick={() => void rerun("failed", () => Promise.all(failedRuns().map((run) => core.rerunChecks(props.worktree, run))))}
            title="Re-run every failed job (and what depends on it) of the workflow runs that have one"
          >
            <RotateCcw classList={{ spin: busy() === "failed" }} />
            {started().includes("failed") ? "Re-run started" : "Re-run failed jobs"}
          </button>
        </div>
      </Show>
      <ul class="pr-list">
        <For each={props.pr.checks} fallback={<li class="muted">No checks ran on it.</li>}>
          {(c) => (
            <li class="pr-check">
              <CheckMark check={c} />
              <span class="grow ellipsis">
                <Show when={c.workflow}>
                  <span class="muted">{c.workflow} / </span>
                </Show>
                {c.name}
              </span>
              <Show when={c.job && finished(c) ? c.job : null}>
                {(job) => (
                  <Show when={!started().includes(job().job)} fallback={<span class="muted pr-rerun-started">Re-run started</span>}>
                    <button
                      class="link"
                      disabled={busy() !== null}
                      onClick={() => void rerun(job().job, () => core.rerunChecks(props.worktree, job().run, { id: job().job, name: c.name }))}
                      title="Re-run this job (and the jobs that depend on it)"
                    >
                      <RotateCcw classList={{ spin: busy() === job().job }} />
                      Re-run
                    </button>
                  </Show>
                )}
              </Show>
              <Show when={c.url}>
                {(url) => (
                  <button class="link" onClick={() => props.onOpen(url())} title={url()}>
                    Details
                    <ExternalLink />
                  </button>
                )}
              </Show>
            </li>
          )}
        </For>
      </ul>
    </>
  );
}

const REVIEWER: Record<ReviewerState, string> = {
  requested: "Awaiting review",
  approved: "Approved",
  changesRequested: "Changes requested",
  commented: "Commented",
  dismissed: "Dismissed",
};

function ReviewerRow(props: { reviewer: Reviewer }) {
  const r = () => props.reviewer;
  /** Asked again after reviewing: waited on, though their last word still shows. */
  const again = () => r().requested && r().state !== "requested";
  return (
    <li class="pr-reviewer">
      <Switch fallback={<MessageSquare class="commented" />}>
        <Match when={r().state === "approved"}>
          <CheckIcon class="approved" />
        </Match>
        <Match when={r().state === "changesRequested"}>
          <CircleX class="changes" />
        </Match>
        <Match when={r().state === "requested"}>
          <CircleDot class="requested" />
        </Match>
        <Match when={r().state === "dismissed"}>
          <CircleMinus class="commented" />
        </Match>
      </Switch>
      <span class="grow pr-reviewer-name">
        <span class="ellipsis">{r().name}</span>
        <Show when={r().team} fallback={<TeamTags login={r().name} />}>
          <span class="muted">(team)</span>
        </Show>
      </span>
      <span class={`pr-reviewer-state ${r().state}`} title={again() ? "Asked to review again" : undefined}>
        {REVIEWER[r().state]}
        <Show when={again()}>
          <Clock />
        </Show>
      </span>
    </li>
  );
}

const DECISION: Record<string, { text: string; kind: string }> = {
  APPROVED: { text: "Approved", kind: "ok" },
  CHANGES_REQUESTED: { text: "Changes requested", kind: "bad" },
  REVIEW_REQUIRED: { text: "Review required", kind: "wait" },
};

/** Whether it can be merged, in words. */
function mergeStatus(pr: PullRequestDetails): { text: string; kind: string } {
  if (pr.state === "merged") return { text: `Merged into ${pr.base}`, kind: "merged" };
  if (pr.state === "closed") return { text: "Closed without merging", kind: "closed" };
  if (pr.draft) return { text: "Draft: not ready for review", kind: "wait" };
  if (pr.mergeable === "CONFLICTING" || pr.mergeState === "DIRTY") return { text: `Conflicts with ${pr.base}`, kind: "bad" };
  switch (pr.mergeState) {
    case "CLEAN":
    case "HAS_HOOKS":
      return { text: "Ready to merge", kind: "ok" };
    case "BEHIND":
      return { text: `Behind ${pr.base}`, kind: "wait" };
    case "BLOCKED":
      return { text: "Blocked (reviews or required checks)", kind: "wait" };
    case "UNSTABLE":
      return { text: "Mergeable, but checks are failing", kind: "bad" };
    default:
      return { text: "GitHub is still working it out", kind: "wait" };
  }
}

function Sidebar(props: { pr: PullRequestDetails; onChecks: () => void }) {
  const p = () => props.pr;
  const counts = createMemo(() => {
    const n = { success: 0, failure: 0, pending: 0, skipped: 0 };
    for (const c of p().checks) n[c.outcome]++;
    return n;
  });
  return (
    <aside class="pr-side">
      <section>
        <h4>Status</h4>
        <p class={`pr-status ${mergeStatus(p()).kind}`}>{mergeStatus(p()).text}</p>
        <Show when={p().reviewDecision && DECISION[p().reviewDecision!]}>
          {(d) => <p class={`pr-status ${d().kind}`}>{d().text}</p>}
        </Show>
      </section>
      <section>
        <h4>Reviewers</h4>
        <ul class="pr-people">
          <For each={p().reviewers} fallback={<li class="muted">No reviewers.</li>}>
            {(r) => <ReviewerRow reviewer={r} />}
          </For>
        </ul>
      </section>
      <section>
        <h4>Assignees</h4>
        <ul class="pr-people">
          <For each={p().assignees} fallback={<li class="muted">Nobody.</li>}>
            {(a) => (
              <li class="pr-assignee">
                {a}
                <TeamTags login={a} />
              </li>
            )}
          </For>
        </ul>
      </section>
      <section>
        <h4>Checks</h4>
        <Show when={p().checks.length} fallback={<p class="muted">None ran.</p>}>
          <button class="pr-checks-summary" onClick={() => props.onChecks()} title="See every check">
            <Show when={counts().failure}>
              <span class="bad">
                <CircleX />
                {counts().failure} failing
              </span>
            </Show>
            <Show when={counts().pending}>
              <span class="wait">
                <Loader />
                {counts().pending} pending
              </span>
            </Show>
            <Show when={counts().success}>
              <span class="ok">
                <CircleCheck />
                {counts().success} passed
              </span>
            </Show>
            <Show when={counts().skipped}>
              <span class="muted">
                <CircleMinus />
                {counts().skipped} skipped
              </span>
            </Show>
          </button>
        </Show>
      </section>
      <Show when={p().labels.length}>
        <section>
          <h4>Labels</h4>
          <div class="pr-labels">
            <For each={p().labels}>{(l) => <span class="pr-label" style={{ "--l": `#${l.color || "71717a"}` }}>{l.name}</span>}</For>
          </div>
        </section>
      </Show>
      <Show when={p().milestone}>
        <section>
          <h4>Milestone</h4>
          <p>{p().milestone}</p>
        </section>
      </Show>
    </aside>
  );
}

function CheckMark(props: { check: Check }) {
  return (
    <Switch>
      <Match when={props.check.outcome === "success"}>
        <CircleCheck class="pr-check-mark ok" />
      </Match>
      <Match when={props.check.outcome === "failure"}>
        <TriangleAlert class="pr-check-mark bad" />
      </Match>
      <Match when={props.check.outcome === "pending"}>
        <Loader class="pr-check-mark wait spin" />
      </Match>
      <Match when={props.check.outcome === "skipped"}>
        <CircleMinus class="pr-check-mark muted" />
      </Match>
    </Switch>
  );
}

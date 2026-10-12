// The sidebar and context strip (ticket 06, redone in the shadcn style): the Workspace's Worktrees
// down the left, each folding open to its Agent sessions as cards (a face, a name, where it stands),
// the whole folding to an icon rail; and a strip saying exactly where your next prompt goes.
import { createSignal, For, type JSX, Match, Show, Switch } from "solid-js";
import { Avatar } from "./Avatar";
import { ago } from "./Chat";
import type { MyPullRequest, SessionId, SessionInfo, WorktreeInfo } from "./core";
import { myPrs } from "./PrBoard";
import { shellsIn } from "./shells";
import {
  ArrowDownUp,
  ChevronRight,
  ChevronsUpDown,
  CirclePause,
  Columns2,
  CirclePlay,
  FileDiff,
  Folder,
  GitBranch,
  GitPullRequestArrow,
  GitPullRequestDraft,
  History,
  LayoutGrid,
  ListTree,
  Loader,
  Plus,
  RefreshCw,
  Search,
  Settings,
  SquareTerminal,
  Trash2,
  X,
} from "./icons";
import { StateDot } from "./StateDot";
import { ContextBar, useSessionFacts } from "./sessionFacts";
import { STATE_LABEL } from "./stateLabels";

/** The design's eight Worktree colours (wt-1 … wt-8): muted, so state colours stay louder. */
const PALETTE = ["#6f87c4", "#a287c2", "#5d9cab", "#b89a66", "#6aa87a", "#c07d86", "#bd845c", "#848a96"];

/** A Worktree as the sidebar shows it: one git lists, or one that's gone but still has sessions. */
export type WorktreeTab = WorktreeInfo & { removed?: boolean };

/** A Worktree that disappeared (e.g. removed in a terminal) while sessions still run in it. */
export const removedWorktree = (path: string): WorktreeTab => ({
  path, branch: null, head: "", isMain: false, ahead: null, behind: null, changed: 0, removed: true,
});

/** A stable colour per Worktree path. */
export function worktreeColour(path: string): string {
  let hash = 0;
  for (const c of path) hash = (hash * 31 + c.charCodeAt(0)) | 0;
  return PALETTE[Math.abs(hash) % PALETTE.length];
}

export function worktreeLabel(w: WorktreeTab): string {
  if (w.removed) return `${w.path.split(/[\\/]/).pop()} (removed)`;
  return w.branch ?? `detached ${w.head}`;
}

/** Where a Worktree's sessions stand, most urgent first, at the end of its sidebar row. */
function worktreeSignal(sessions: SessionInfo[], settingUp: boolean): JSX.Element {
  if (sessions.some((s) => s.state === "needsYou")) return <span class="state-badge needsYou" title="A session here needs you">Needs you</span>;
  if (settingUp) return <Loader class="spin setting-up" aria-label="Setting up" />;
  if (sessions.some((s) => s.state === "working")) return <span class="state-badge working"><span class="dot" />Working</span>;
  if (sessions.some((s) => s.state === "idle" || s.state === "suspended")) return <span class="state-badge idle">Idle</span>;
  if (sessions.some((s) => s.state === "exited")) return <StateDot state="exited" title="An Agent exited" />;
  return null;
}

/** The Worktree's running shells, in its sidebar row: how many, and whether one runs something.
 *  Clicking it (the row handles that) shows its Terminal panel. */
function ShellBadge(props: { path: string }) {
  const shells = () => shellsIn(props.path);
  const title = () =>
    `Shells running here (click to show the terminal):\n${shells()
      .map((t) => `• ${t.name}${t.busy ? " (running)" : ""}`)
      .join("\n")}`;
  return (
    <Show when={shells().length > 0}>
      <span class="shell-badge" classList={{ busy: shells().some((t) => t.busy) }} title={title()}>
        <SquareTerminal />
        <Show when={shells().length > 1}>{shells().length}</Show>
      </span>
    </Show>
  );
}

export interface SidebarProps {
  workspaceName: string;
  workspaceRoot: string;
  /** Folded to the icon rail. */
  collapsed: boolean;
  /** The app mark (an SVG), for the Workspace switcher. */
  mark: string;
  worktrees: WorktreeTab[];
  active: string;
  /** The Columns view's pinned Worktrees (the others are dimmed; choosing one gives it a column);
   *  null in the Tabs view, where the active Worktree lists its sessions. */
  pinned: string[] | null;
  sessionsIn: (path: string) => SessionInfo[];
  /** The session the Tabs view shows. */
  activeSession: SessionId | null;
  /** A Worktree setup is running there (its first session hasn't started yet). */
  settingUp: (path: string) => boolean;
  onSelect: (path: string) => void;
  /** Selects the Worktree and shows its Terminal panel (its row's shell badge). */
  onShowTerminal: (path: string) => void;
  onShowSession: (id: SessionId) => void;
  onCloseSession: (id: SessionId) => void;
  onNewSession: (path: string) => void;
  /** Opens the Worktree's Recent sessions menu under `anchor`. */
  onRecent: (path: string, anchor: DOMRect) => void;
  onNewWorktree: () => void;
  onOverview: () => void;
  onSwitchWorkspace: () => void;
  onSearch: () => void;
  onSettings: () => void;
  onOpenMyPr: (pr: MyPullRequest) => void;
  onPrBoard: () => void;
}

/** Which Worktrees were folded open (true) or shut by hand, by Workspace root: kept across restarts. */
const [folds, setFolds] = createSignal<Record<string, Record<string, boolean>>>({});
const foldsKey = (root: string) => `orchard.sidebar.folds:${root}`;
function foldsIn(root: string): Record<string, boolean> {
  if (!(root in folds())) {
    let saved = {};
    try {
      saved = JSON.parse(localStorage.getItem(foldsKey(root)) ?? "{}");
    } catch {}
    setFolds((all) => ({ ...all, [root]: saved }));
  }
  return folds()[root];
}
function setFold(root: string, path: string, open: boolean) {
  const next = { ...foldsIn(root), [path]: open };
  setFolds((all) => ({ ...all, [root]: next }));
  localStorage.setItem(foldsKey(root), JSON.stringify(next));
}

export function Sidebar(props: SidebarProps) {
  const dimmed = (w: WorktreeTab) => !!props.pinned && !props.pinned.includes(w.path);
  /** Open as last folded by hand; until then, the active Worktree and any with a session that
   *  needs you are. */
  const isOpen = (path: string) =>
    foldsIn(props.workspaceRoot)[path] ?? (path === props.active || props.sessionsIn(path).some((s) => s.state === "needsYou"));
  const fold = (path: string, open: boolean) => setFold(props.workspaceRoot, path, open);
  /** A click on a header's shell badge shows the terminal; anywhere else folds it open or shut
   *  (only a session card switches to a session). */
  const choose = (e: MouseEvent, path: string) =>
    (e.target as Element).closest(".shell-badge") ? props.onShowTerminal(path) : fold(path, !isOpen(path));
  const title = (w: WorktreeTab) =>
    w.removed ? `${w.path}: no longer a Worktree; its sessions still run` : dimmed(w) ? `${worktreeLabel(w)}: give it a column` : w.path;
  return (
    <Show
      when={!props.collapsed}
      fallback={
        <aside class="sidebar rail">
          <button class="logo-box" onClick={() => props.onSwitchWorkspace()} title={`${props.workspaceName}: switch repository (Ctrl+Shift+O)`}>
            <span class="logo" innerHTML={props.mark} />
          </button>
          <span class="rail-sep" />
          <For each={props.worktrees}>
            {(w) => (
              <button
                class="rail-item"
                classList={{ on: props.pinned ? !dimmed(w) : w.path === props.active, dimmed: dimmed(w), focused: !!props.pinned && w.path === props.active }}
                style={{ "--c": worktreeColour(w.path) }}
                title={title(w)}
                aria-label={worktreeLabel(w)}
                onClick={() => props.onSelect(w.path)}
              >
                <GitBranch />
                <Show when={props.sessionsIn(w.path).some((s) => s.state === "needsYou")}>
                  <span class="needs-dot" />
                </Show>
                <Show when={shellsIn(w.path).length > 0}>
                  <span class="shell-dot" title="A shell is running" />
                </Show>
              </button>
            )}
          </For>
          <button class="rail-item add" onClick={() => props.onNewWorktree()} title="New Worktree with an Agent session" aria-label="New Worktree">
            <Plus />
          </button>
          <span class="grow" />
          <button class="rail-item" onClick={() => props.onSettings()} title="Settings (Ctrl+,)" aria-label="Settings">
            <Settings />
          </button>
        </aside>
      }
    >
      <aside class="sidebar">
        <button class="workspace-switcher" onClick={() => props.onSwitchWorkspace()} title="Switch repository (Ctrl+Shift+O)">
          <span class="logo-box">
            <span class="logo" innerHTML={props.mark} />
          </span>
          <span class="names">
            <span class="name">{props.workspaceName}</span>
            <span class="path">{props.workspaceRoot}</span>
          </span>
          <ChevronsUpDown />
        </button>
        <button class="sidebar-search" onClick={() => props.onSearch()} title="Go to a file in this Worktree, or start a session">
          <Search />
          <span class="grow">Search files…</span>
          <kbd>Ctrl P</kbd>
        </button>
        <div class="group-label">
          <span class="grow">Worktrees</span>
          <button class="ghost icon" onClick={() => props.onOverview()} title="Every Worktree, and which are merged and can be removed" aria-label="Worktrees overview">
            <ListTree />
          </button>
          <button class="ghost icon" onClick={() => props.onNewWorktree()} title="New Worktree with an Agent session" aria-label="New Worktree">
            <Plus />
          </button>
        </div>
        <nav class="sidebar-list">
          <For each={props.worktrees}>
            {(w) => {
              const sessions = () => props.sessionsIn(w.path);
              const open = () => isOpen(w.path);
              return (
                <section class="wt-group" classList={{ open: open(), dimmed: dimmed(w) }} style={{ "--c": worktreeColour(w.path) }}>
                  <div class="wt-head" classList={{ on: w.path === props.active, removed: !!w.removed }}>
                    <button
                      class="wt-head-main"
                      title={w.removed ? title(w) : `${w.path}\n${open() ? "Fold" : "Unfold"}`}
                      aria-expanded={open()}
                      onClick={(e) => choose(e, w.path)}
                    >
                      <span class="fold">
                        <ChevronRight />
                      </span>
                      <GitBranch class="wt-glyph" />
                      <span class="label">
                        {w.removed ? w.path.split(/[\\/]/).pop() : worktreeLabel(w)}
                        <Show when={sessions().length}>
                          <span class="wt-count"> ({sessions().length})</span>
                        </Show>
                      </span>
                      <ShellBadge path={w.path} />
                      <Show when={!open()} fallback={props.settingUp(w.path) && <Loader class="spin setting-up" aria-label="Setting up" />}>
                        {worktreeSignal(sessions(), props.settingUp(w.path))}
                      </Show>
                    </button>
                    <Show when={!w.removed}>
                      <span class="wt-actions">
                        <Show when={dimmed(w)}>
                          <button class="ghost icon" onClick={() => props.onSelect(w.path)} title="Give it a column in the Columns view" aria-label="Give it a column">
                            <Columns2 />
                          </button>
                        </Show>
                        <Show when={!props.pinned}>
                          <button
                            class="ghost icon recent-menu"
                            onClick={(e) => props.onRecent(w.path, e.currentTarget.getBoundingClientRect())}
                            title="Closed sessions in this Worktree, and its conversations started elsewhere"
                            aria-label="Recent sessions"
                          >
                            <History />
                          </button>
                        </Show>
                        <Show when={!props.settingUp(w.path)}>
                          <button class="ghost icon" onClick={() => props.onNewSession(w.path)} title="New Agent session in this Worktree" aria-label="New session">
                            <Plus />
                          </button>
                        </Show>
                      </span>
                    </Show>
                  </div>
                  <Show when={open()}>
                    <SessionCards {...props} worktree={w} sessions={sessions()} />
                  </Show>
                </section>
              );
            }}
          </For>
        </nav>
        <MyPullRequests worktrees={props.worktrees} onOpen={props.onOpenMyPr} onBoard={props.onPrBoard} />
        <button class="sidebar-footer" onClick={() => props.onSettings()} title="Settings (Ctrl+,)">
          <Settings />
          <span class="grow">Settings</span>
          <kbd>Ctrl ,</kbd>
        </button>
      </aside>
    </Show>
  );
}

/** Under the Worktrees: the open PRs the user opened in the repo (as the GitHub CLI is logged in). */
function MyPullRequests(props: { worktrees: WorktreeTab[]; onOpen: (pr: MyPullRequest) => void; onBoard: () => void }) {
  const prs = () => myPrs.prs().filter((p) => p.state === "open");
  const { error, loading } = myPrs;
  /** The Worktree on the PR's branch, if there is one. */
  const worktreeOf = (pr: MyPullRequest) => props.worktrees.find((w) => !w.removed && w.branch === pr.head);
  const review = { APPROVED: ["approved", "Approved"], CHANGES_REQUESTED: ["changes", "Changes"], REVIEW_REQUIRED: null } as const;
  return (
    <>
      <div class="group-label">
        <span class="grow">Pull requests</span>
        <button class="ghost icon" onClick={() => props.onBoard()} title="Your PRs on a board, by where each stands" aria-label="Pull request board">
          <LayoutGrid />
        </button>
        <button class="ghost icon" onClick={() => myPrs.refresh()} disabled={loading()} title="Ask GitHub again" aria-label="Refresh pull requests">
          <RefreshCw classList={{ spin: loading() }} />
        </button>
      </div>
      <nav class="sidebar-list my-prs">
        <Show when={error()}>
          <span class="my-prs-note error" title={error()}>
            Couldn't list your PRs: {error()}
          </span>
        </Show>
        <Show when={!error() && !loading() && !prs().length}>
          <span class="my-prs-note">You have no open PRs here.</span>
        </Show>
        <For each={prs()}>
          {(pr) => {
            const w = () => worktreeOf(pr);
            const decision = () => (pr.reviewDecision ? review[pr.reviewDecision] : null);
            return (
              <button
                class="sidebar-item my-pr"
                style={{ "--c": w() ? worktreeColour(w()!.path) : "var(--fg-3)" }}
                title={`#${pr.number} ${pr.title}\n${pr.head} → ${pr.base}${pr.draft ? " (draft)" : ""}\nUpdated ${ago(pr.updatedAt)}${w() ? "\nChecked out in a Worktree" : ""}`}
                onClick={() => props.onOpen(pr)}
              >
                <Show when={pr.draft} fallback={<GitPullRequestArrow class="wt-glyph" />}>
                  <GitPullRequestDraft class="wt-glyph" />
                </Show>
                <span class="label">{pr.title}</span>
                <Show when={decision()}>{(d) => <span class={`state-badge ${d()[0]}`}>{d()[1]}</span>}</Show>
                <span class="my-pr-number">#{pr.number}</span>
              </button>
            );
          }}
        </For>
      </nav>
    </>
  );
}

/** A Worktree's sessions, under it, a card each; or, with none, a way to start one. */
function SessionCards(props: SidebarProps & { worktree: WorktreeTab; sessions: SessionInfo[] }) {
  return (
    <div class="session-cards">
      <For
        each={props.sessions}
        fallback={
          <Show when={!props.worktree.removed && !props.settingUp(props.worktree.path)}>
            <button class="session-empty" onClick={() => props.onNewSession(props.worktree.path)} title="New Agent session in this Worktree">
              <Plus />
              New session
            </button>
          </Show>
        }
      >
        {(s) => <SessionCard session={s} colour={worktreeColour(props.worktree.path)} on={s.id === props.activeSession} onShow={props.onShowSession} onClose={props.onCloseSession} />}
      </For>
    </div>
  );
}

/** One session: its face and name, then where it stands, how long since it did anything, the
 *  model, how full its context is and what it has cost. */
function SessionCard(props: { session: SessionInfo; colour: string; on: boolean; onShow: (id: SessionId) => void; onClose: (id: SessionId) => void }) {
  const s = () => props.session;
  const { context, cost, active, model, title } = useSessionFacts(s);
  return (
    <div class={`session-card ${s().state}`} classList={{ on: props.on }}>
      <button
        class="session-card-main"
        title={title()}
        onClick={() => !props.on && props.onShow(s().id)}
        onAuxClick={(e) => e.button === 1 && props.onClose(s().id)}
      >
        <Avatar name={s().name} colour={props.colour} />
        <span class="session-card-body">
          <span class="session-card-line">
            <span class="name">{s().name}</span>
            <Show when={s().unread > 0 && !props.on}>
              <span class="count-badge">{s().unread} new</span>
            </Show>
            <span class="grow" />
            <ContextBar context={context()} />
          </span>
          <span class="session-card-line meta">
            <span class="state">
              <Show when={s().state === "working"} fallback={<StateDot state={s().state} />}>
                <Loader class="spin" />
              </Show>
              {STATE_LABEL[s().state]}
            </span>
            <Show when={active()}>
              <span class="ago">{active()}</span>
            </Show>
            <Show when={model()}>
              <span class="model">{model()}</span>
            </Show>
            <span class="grow" />
            <Show when={cost()}>
              <span class="cost">{cost()}</span>
            </Show>
          </span>
        </span>
      </button>
      <button class="close-tab" aria-label={`Close ${s().name}`} title="Close (it stays in Recent sessions; Ctrl+Shift+T reopens)" onClick={() => props.onClose(s().id)}>
        <X />
      </button>
    </div>
  );
}

/** Under the header in the Tabs view: the Worktree's folder, how far it is from its upstream and
 *  its uncommitted changes; and what can be done to the session and the Worktree. */
export function ContextStrip(props: {
  worktree: WorktreeTab | undefined;
  session: SessionInfo | undefined;
  onRemove: (path: string) => void;
  onSuspend: (session: SessionInfo) => void;
  onResume: (session: SessionInfo) => void;
}) {
  return (
    <Show when={props.worktree}>
      {(w) => (
        <div class="context-strip">
          <span class="meta path-meta" title={w().path}>
            <Folder />
            <span class="mono ellipsis">{w().path}</span>
          </span>
          <Show when={!w().removed} fallback={<span class="meta gone">folder removed — sessions still run</span>}>
            <Show when={w().ahead !== null}>
              <span class="meta" title="Commits ahead of / behind its upstream">
                <ArrowDownUp />
                <span class="mono">
                  ↑{w().ahead} ↓{w().behind}
                </span>
              </span>
            </Show>
            <span class="meta">
              <FileDiff />
              {w().changed === 0 ? "no changes" : `${w().changed} changed`}
            </span>
          </Show>
          <span class="grow" />
          <Show when={props.session}>
            {(s) => (
              <Switch>
                <Match when={s().state === "idle"}>
                  <button class="ghost small" onClick={() => props.onSuspend(s())} title="Stop this session's Agent to free memory; sending a message resumes it">
                    <CirclePause />
                    Suspend
                  </button>
                </Match>
                <Match when={s().state === "exited"}>
                  <button class="ghost small" onClick={() => props.onResume(s())} title="Bring the Agent back with this conversation">
                    <CirclePlay />
                    Resume
                  </button>
                </Match>
              </Switch>
            )}
          </Show>
          <Show when={!w().isMain && !w().removed}>
            <button class="ghost small" onClick={() => props.onRemove(w().path)} title="Remove this Worktree (asks first)">
              <Trash2 />
              Remove Worktree…
            </button>
          </Show>
        </div>
      )}
    </Show>
  );
}

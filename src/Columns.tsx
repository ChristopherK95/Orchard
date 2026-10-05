// The Columns view (ticket 28): each pinned Worktree gets a panel with its own session tabs and
// transcript, streamed in its own view slot. One column has focus: it's the active Worktree, so the
// next prompt, Y / N, the drawer and the palette all follow it. Only the focused column has the
// full composer; the others show a one-line stub that focuses them. Each column's tab list ends
// with Terminal: its Worktree's shell, in place of the transcript.
import { createEffect, createSignal, For, lazy, Match, onCleanup, Show, Switch } from "solid-js";
import { Banner, Composer, FindOtherConversations, RecentList, reportHeight } from "./Chat";
import type { OtherConversation, RecentSession, SessionId, SessionInfo } from "./core";
import { ArrowUp, CirclePause, CirclePlay, Ellipsis, GitBranch, Pin, Plus, Sparkles, SquareTerminal, Trash2, X } from "./icons";
import { Setup, type SetupView } from "./Setup";
import { shellRunning } from "./shells";
import { StateBadge } from "./StateDot";
import { createTabView, type TabView } from "./TabView";
import { Transcript } from "./Transcript";
import { worktreeColour, worktreeLabel, type WorktreeTab } from "./Worktrees";

// (xterm.js loads with the first Terminal tab shown.)
const TerminalPanel = lazy(() => import("./TerminalPanel").then((m) => ({ default: m.TerminalPanel })));

/** The narrowest window the Columns view is offered at. */
export const COLUMNS_MIN_WIDTH = 1600;

export interface ColumnsProps {
  /** The pinned Worktrees, in sidebar order. */
  worktrees: WorktreeTab[];
  /** The Worktrees without a column (offered when none has one). */
  unpinned: WorktreeTab[];
  onPin: (path: string) => void;
  onUnpin: (path: string) => void;
  /** The focused column's Worktree. */
  focused: string;
  sessionsIn: (path: string) => SessionInfo[];
  /** The session a column shows: its last-used one, else its first. */
  shownIn: (path: string) => SessionId | undefined;
  setupOf: (path: string) => SetupView | undefined;
  recentIn: (path: string) => RecentSession[];
  /** A column's view slot, as it comes (and null as it goes), for Y / N on the focused one. */
  register: (path: string, view: TabView | null) => void;
  onFocus: (path: string) => void;
  onShow: (path: string, id: SessionId) => void;
  onNewSession: (path: string) => void;
  onCloseTab: (id: SessionId) => void;
  onReopen: (session: RecentSession) => void;
  /** Opens a conversation started outside the editor (in a terminal, say). */
  onOpenOther: (conversation: OtherConversation) => void;
  onSuspend: (session: SessionInfo) => void;
  onResume: (session: SessionInfo) => void;
  onRemove: (worktree: WorktreeTab) => void;
  onOpenSettings: () => void;
  onOpenSnippet: (code: string, label: string) => void;
  /** Whether a column shows its Terminal tab (its Worktree's shell) rather than a session. */
  terminalOpen: (path: string) => boolean;
  onToggleTerminal: (path: string) => void;
  onError: (message: string) => void;
}

export function Columns(props: ColumnsProps) {
  // Columns are keyed by path: each `worktreesChanged` (an Agent editing a file changes its count)
  // brings new Worktree objects, and keying on those rebuilt every column and its transcript.
  const byPath = (path: string) => props.worktrees.find((w) => w.path === path);
  return (
    <Show
      when={props.worktrees.length > 0}
      fallback={
        <div class="center empty-worktree">
          <Pin />
          <p>No Worktrees have a column yet.</p>
          <div class="list-box">
            <div class="section-label">
              Worktrees
              <span class="note">more later from the sidebar</span>
            </div>
            <For each={props.unpinned}>
              {(w) => (
                <div class="recent-row" style={{ "--c": worktreeColour(w.path) }}>
                  <GitBranch class="wt-glyph" />
                  <span class="grow mono">{worktreeLabel(w)}</span>
                  <button class="link" onClick={() => props.onPin(w.path)}>
                    Open as a column
                  </button>
                </div>
              )}
            </For>
          </div>
        </div>
      }
    >
      <div class="columns">
        <For each={props.worktrees.map((w) => w.path)}>
          {(path) => <Show when={byPath(path)}>{(w) => <Column {...props} worktree={w()} isFocused={path === props.focused} />}</Show>}
        </For>
      </div>
    </Show>
  );
}

function Column(props: ColumnsProps & { worktree: WorktreeTab; isFocused: boolean }) {
  const path = props.worktree.path;
  const view = createTabView(`column:${path}`);
  props.register(path, view);
  onCleanup(() => {
    props.register(path, null);
    view.hide();
  });
  // Streams whichever session the column shows now.
  createEffect(() => {
    const id = props.shownIn(path);
    if (id === undefined) {
      if (view.shown() !== null) view.hide();
    } else if (id !== view.shown()) view.show(id).catch((err) => props.onError(String(err)));
  });
  const sessions = () => props.sessionsIn(path);
  const session = () => sessions().find((s) => s.id === view.shown());
  const terminal = () => props.terminalOpen(path) && !props.worktree.removed;
  /** The floating composer's (or stub's) height: the transcript's last item stays above it. */
  const [composerHeight, setComposerHeight] = createSignal(0);
  const setup = () => {
    const s = props.setupOf(path);
    return s && s.status.kind !== "done" ? s : undefined;
  };
  /** A session tab: shows it (in place of the terminal, if that's up). */
  const showSession = (id: SessionId) => {
    if (props.terminalOpen(path)) props.onToggleTerminal(path);
    props.onShow(path, id);
  };

  return (
    <section
      ref={(el) => createEffect(() => props.isFocused && el.scrollIntoView({ block: "nearest", inline: "nearest" }))}
      class="column"
      classList={{ focused: props.isFocused }}
      style={{ "--c": worktreeColour(path) }}
      onMouseDown={() => !props.isFocused && props.onFocus(path)}
    >
      <ColumnHeader {...props} session={session()} />
      <nav class="column-tabs">
        <div class="tabs-list">
          <For each={sessions()}>
            {(s) => (
              <span class="trigger-wrap" classList={{ on: s.id === view.shown() && !terminal() }}>
                <button class="trigger" onClick={() => showSession(s.id)} onAuxClick={(e) => e.button === 1 && props.onCloseTab(s.id)}>
                  {s.name}
                  <Show when={s.state === "needsYou"}>
                    <span class="needs-dot" title="Needs you" />
                  </Show>
                  <Show when={s.unread > 0 && (s.id !== view.shown() || terminal())}>
                    <span class="unread">{s.unread}</span>
                  </Show>
                </button>
                <button class="close-tab" aria-label={`Close ${s.name}`} title="Close (it stays in Recent sessions; Ctrl+Shift+T reopens)" onClick={() => props.onCloseTab(s.id)}>
                  <X />
                </button>
              </span>
            )}
          </For>
          <Show when={!props.worktree.removed}>
            <Show when={sessions().length > 0}>
              <span class="divider" />
            </Show>
            <span class="trigger-wrap" classList={{ on: terminal() }}>
              <button
                class="trigger"
                onClick={() => !terminal() && props.onToggleTerminal(path)}
                title="This Worktree's shell (Ctrl+` in the focused column)"
              >
                <SquareTerminal />
                Terminal
                <Show when={shellRunning(path)}>
                  <span class="shell-dot" title="A shell is running" />
                </Show>
              </button>
            </span>
          </Show>
        </div>
        <button
          class="ghost icon"
          onClick={() => props.onNewSession(path)}
          title="New Agent session in this Worktree"
          aria-label="New Agent session"
          disabled={props.worktree.removed || !!setup()}
        >
          <Plus />
        </button>
      </nav>
      <Show when={sessions().length > 1 && !terminal()}>
        <Banner tone="warn">{sessions().length} Agent sessions share this Worktree, so they can edit the same files.</Banner>
      </Show>
      <div class="column-body">
        <Show
          when={!terminal()}
          fallback={
            <TerminalPanel
              slot={`column:${path}`}
              worktree={path}
              label={worktreeLabel(props.worktree)}
              colour={worktreeColour(path)}
              placement="fill"
              takeFocus={props.isFocused}
            />
          }
        >
          <Show
            when={session()}
            keyed
            fallback={
              <Show
                when={setup()}
                fallback={
                  <div class="center empty-worktree">
                    <GitBranch />
                    <p>No Agent sessions in this Worktree yet.</p>
                    <Show when={!props.worktree.removed}>
                      <button class="primary" onClick={() => props.onNewSession(path)}>
                        <Sparkles />
                        New Agent session here
                      </button>
                    </Show>
                    <Show when={props.recentIn(path).length}>
                      <RecentList sessions={props.recentIn(path)} onReopen={props.onReopen} />
                    </Show>
                    <Show when={!props.worktree.removed}>
                      <FindOtherConversations worktree={path} onOpen={props.onOpenOther} />
                    </Show>
                  </div>
                }
              >
                {(s) => <Setup worktree={path} setup={s()} onError={props.onError} onOpenSettings={props.onOpenSettings} />}
              </Show>
            }
          >
            {(s) => (
              <>
                <Transcript
                  sessionId={s.id}
                  items={view.items}
                  start={view.start()}
                  onLoadEarlier={view.loadEarlier}
                  onError={props.onError}
                  onOpenSnippet={props.onOpenSnippet}
                  answerKeys={() => props.isFocused}
                  bottomInset={composerHeight}
                />
                <Show
                  when={props.isFocused}
                  fallback={<ComposerStub worktree={props.worktree} session={s} onFocus={() => props.onFocus(path)} onHeight={setComposerHeight} />}
                >
                  <Composer session={s} onHeight={setComposerHeight} />
                </Show>
              </>
            )}
          </Show>
        </Show>
      </div>
    </section>
  );
}

/** A column's header: its Worktree, how it stands, the most urgent state, and a menu. */
function ColumnHeader(props: ColumnsProps & { worktree: WorktreeTab; isFocused: boolean; session: SessionInfo | undefined }) {
  const path = props.worktree.path;
  const needYou = () => props.sessionsIn(path).filter((s) => s.state === "needsYou").length;
  /** The badge: Needs you if any session here does (so it can't hide behind another tab or the
   *  shell), else the shown session's state. */
  const state = () => (needYou() > 0 ? "needsYou" : props.session?.state);
  const [menuAt, setMenuAt] = createSignal<{ left: number; top: number } | null>(null);
  const close = (e: MouseEvent) => !(e.target as Element | null)?.closest?.(".column-menu, .column-menu-button") && setMenuAt(null);
  window.addEventListener("click", close);
  onCleanup(() => window.removeEventListener("click", close));
  const run = (action: () => void) => () => {
    setMenuAt(null);
    action();
  };
  return (
    <div class="column-head">
      <span class="swatch" />
      <span class="branch" title={path}>
        {props.worktree.removed ? path.split(/[\\/]/).pop() : worktreeLabel(props.worktree)}
      </span>
      <Show when={props.worktree.removed}>
        <span class="gone">folder removed</span>
      </Show>
      <Show when={props.worktree.ahead !== null}>
        <span class="counts">
          ↑{props.worktree.ahead} ↓{props.worktree.behind}
        </span>
      </Show>
      <Show when={props.worktree.changed > 0}>
        <span class="changed">{props.worktree.changed} changed</span>
      </Show>
      <span class="grow" />
      <Show when={state()}>{(s) => <StateBadge state={s()} label={needYou() > 1 ? `${needYou()} need you` : undefined} />}</Show>
      <button
        class="ghost icon column-menu-button"
        onClick={(e) => {
          const r = e.currentTarget.getBoundingClientRect();
          setMenuAt((now) => (now ? null : { left: r.right - 220, top: r.bottom + 4 }));
        }}
        title="More for this column"
        aria-label="Column menu"
      >
        <Ellipsis />
      </button>
      <Show when={menuAt()}>
        {(at) => (
          <div class="menu column-menu" style={{ left: `${at().left}px`, top: `${at().top}px` }}>
            <Switch>
              <Match when={props.session?.state === "idle" && props.session}>
                {(s) => (
                  <button class="menu-item" onClick={run(() => props.onSuspend(s()))} title="Stop this session's Agent to free memory; sending a message resumes it">
                    <CirclePause />
                    Suspend {s().name}
                  </button>
                )}
              </Match>
              <Match when={props.session?.state === "exited" && props.session}>
                {(s) => (
                  <button class="menu-item" onClick={run(() => props.onResume(s()))} title="Bring the Agent back with this conversation">
                    <CirclePlay />
                    Resume {s().name}
                  </button>
                )}
              </Match>
            </Switch>
            <Show when={!props.worktree.isMain && !props.worktree.removed}>
              <button class="menu-item" onClick={run(() => props.onRemove(props.worktree))} title="Remove this Worktree (asks first)">
                <Trash2 />
                Remove Worktree…
              </button>
            </Show>
            <button class="menu-item" onClick={run(() => props.onUnpin(path))} title="Its sessions keep running">
              <X />
              Close column
            </button>
          </div>
        )}
      </Show>
    </div>
  );
}

/** An unfocused column's composer: one line that focuses the column. */
function ComposerStub(props: { worktree: WorktreeTab; session: SessionInfo; onFocus: () => void; onHeight: (px: number) => void }) {
  const branch = () => worktreeLabel(props.worktree);
  return (
    <div class="composer stub" ref={(el) => reportHeight(el, props.onHeight)}>
      <button class="composer-stub" onClick={() => props.onFocus()}>
        <span class="grow ellipsis">
          <Show when={props.session.state !== "needsYou"} fallback="Answer the permission above, or click to focus">
            Message {branch()}
            {props.session.state === "suspended" ? " — sending resumes it" : ""}
          </Show>
        </span>
        <Show when={props.session.state === "needsYou"} fallback={<ArrowUp />}>
          <kbd>Alt ←→</kbd>
        </Show>
      </button>
    </div>
  );
}

// The Columns view (ticket 28): each pinned Worktree gets a panel with its own session tabs and
// transcript, streamed in its own view slot. One column has focus: it's the active Worktree, so the
// next prompt, Y / N, the drawer and the palette all follow it. Every column has the full
// composer (so switching focus doesn't resize it), but only the focused one takes the keyboard;
// clicking into another focuses that column. Each column's tab list ends with Terminal: its
// Worktree's shell, in place of the transcript. Dragging the gap between two columns moves width
// from one to the other (ticket 36).
import { createEffect, createSignal, For, type JSX, lazy, Match, onCleanup, onMount, Show, Switch } from "solid-js";
import { Avatar } from "./Avatar";
import { Banner, Composer, FindOtherConversations, RecentList } from "./Chat";
import type { OtherConversation, PermissionRequest, RecentSession, SessionId, SessionInfo } from "./core";
import { setupOver } from "./core";
import { Check, ChevronDown, CirclePause, Columns2, CirclePlay, Ellipsis, GitBranch, MessageSquareCode, Pin, Plus, Sparkles, SquareTerminal, Trash2, X } from "./icons";
import { PrButton } from "./PrOverlay";
import { useSessionFacts } from "./sessionFacts";
import { Setup, type SetupView } from "./Setup";
import { shellRunning } from "./shells";
import { StateBadge, StateDot } from "./StateDot";
import { STATE_LABEL } from "./stateLabels";
import { createTabView, type TabView } from "./TabView";
import { Transcript } from "./Transcript";
import { ReviewBadge, worktreeColour, worktreeLabel, type WorktreeTab } from "./Worktrees";

// (xterm.js loads with the first Terminal tab shown.)
const TerminalPanel = lazy(() => import("./TerminalPanel").then((m) => ({ default: m.TerminalPanel })));

/** The narrowest window the Columns view is offered at. */
export const COLUMNS_MIN_WIDTH = 1600;

/** The narrowest a column gets (as `.column`'s min-width), and a column's share when even. */
const COLUMN_MIN = 640;
const EVEN = 1000;

export interface ColumnsProps {
  /** The pinned Worktrees, in the Worktree switcher's order. */
  worktrees: WorktreeTab[];
  /** Every Worktree that can have a column, in the Worktree switcher's order (the column picker's list). */
  available: WorktreeTab[];
  /** The Worktrees without a column (offered when none has one). */
  unpinned: WorktreeTab[];
  /** The column picker's confirmation: exactly these Worktrees have a column now. */
  onSetColumns: (paths: string[]) => void;
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
  /** The branch name was clicked: the branch picker, under `anchor`. */
  onSwitchBranch: (worktree: string, anchor: Element) => void;
  /** The PR of the Worktree's branch, in the PR overlay. */
  onOpenPr: (worktree: string) => void;
  /** A Review Worktree's badge: its Code review, of PR `number`. */
  onCodeReview: (number: number) => void;
  onOpenSettings: () => void;
  onOpenSnippet: (code: string, label: string) => void;
  /** "Open in editor" on a permission card: the column takes focus too. */
  onOpenProposal: (sessionId: SessionId, request: PermissionRequest) => void;
  /** Whether a column shows its Terminal tab (its Worktree's shell) rather than a session. */
  terminalOpen: (path: string) => boolean;
  onToggleTerminal: (path: string) => void;
  /** The columns' widths, as shares of the row, by Worktree (one without has `EVEN`). */
  shares: Record<string, number>;
  /** New shares: while dragging, then saved (`save`) when it's let go. */
  onShares: (shares: Record<string, number>, save: boolean) => void;
  onError: (message: string) => void;
}

export function Columns(props: ColumnsProps) {
  const [picking, setPicking] = createSignal(false);
  return (
    <div class="columns-area">
      <div class="columns-bar">
        <button class="ghost small" onClick={() => setPicking(true)} title="Choose which Worktrees have a column">
          <Columns2 />
          Manage columns
        </button>
      </div>
      <ColumnRow {...props} />
      <Show when={picking()}>
        <ColumnPicker
          available={props.available}
          pinned={props.worktrees.map((w) => w.path)}
          onConfirm={(paths) => {
            setPicking(false);
            props.onSetColumns(paths);
          }}
          onClose={() => setPicking(false)}
        />
      </Show>
    </div>
  );
}

/** The modal behind "Manage columns": a checkbox per Worktree, applied together on confirm. */
function ColumnPicker(props: { available: WorktreeTab[]; pinned: string[]; onConfirm: (paths: string[]) => void; onClose: () => void }) {
  const [chosen, setChosen] = createSignal(new Set(props.pinned));
  const toggle = (path: string) =>
    setChosen((now) => {
      const next = new Set(now);
      if (!next.delete(path)) next.add(path);
      return next;
    });
  const confirm = () =>
    props.onConfirm(
      props.available
        .filter((w) => chosen().has(w.path))
        .map((w) => w.path),
    );
  const onKey = (e: KeyboardEvent) => e.key === "Escape" && props.onClose();
  let dialog!: HTMLDivElement;
  onMount(() => dialog.focus());
  window.addEventListener("keydown", onKey);
  onCleanup(() => window.removeEventListener("keydown", onKey));
  return (
    <div class="modal-backdrop" onClick={(e) => e.target === e.currentTarget && props.onClose()}>
      <div class="modal" role="dialog" aria-label="Manage columns" tabIndex={-1} ref={dialog}>
        <div class="modal-head">
          <b>Manage columns</b>
          <button class="ghost icon" onClick={() => props.onClose()} aria-label="Close" title="Close (Esc)">
            <X />
          </button>
        </div>
        <div class="modal-body">
          <p class="hint">Tick the Worktrees to show as columns. Closing a column leaves its sessions running.</p>
          <For each={props.available}>
            {(w) => (
              <label class="check-row" style={{ "--c": worktreeColour(w.path) }}>
                <input type="checkbox" checked={chosen().has(w.path)} onChange={() => toggle(w.path)} disabled={w.removed && !chosen().has(w.path)} />
                <GitBranch class="wt-glyph" />
                <span class="mono grow">{worktreeLabel(w)}</span>
                <Show when={w.removed}>
                  <span class="gone">folder removed</span>
                </Show>
              </label>
            )}
          </For>
        </div>
        <div class="modal-foot">
          <button onClick={() => props.onClose()}>Cancel</button>
          <button class="primary" onClick={confirm}>
            <Check />
            Show {chosen().size} {chosen().size === 1 ? "column" : "columns"}
          </button>
        </div>
      </div>
    </div>
  );
}

function ColumnRow(props: ColumnsProps) {
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
              <span class="note">more later from the Worktree switcher (Ctrl+Space)</span>
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
          {(path, i) => (
            <>
              <Show when={i() > 0}>
                <ResizeHandle {...props} left={props.worktrees[i() - 1]?.path} right={path} />
              </Show>
              <Show when={byPath(path)}>{(w) => <Column {...props} worktree={w()} isFocused={path === props.focused} />}</Show>
            </>
          )}
        </For>
      </div>
    </Show>
  );
}

/** Between two columns: dragging moves width from one to the other (neither below `COLUMN_MIN`);
 *  a double-click evens the two out. */
function ResizeHandle(props: ColumnsProps & { left: string | undefined; right: string }) {
  const [dragging, setDragging] = createSignal(false);
  const share = (path: string) => props.shares[path] ?? EVEN;
  const drag = (e: PointerEvent) => {
    const handle = e.currentTarget as HTMLElement;
    const left = props.left;
    const [a, b] = [handle.previousElementSibling, handle.nextElementSibling];
    if (!left || e.button !== 0 || !a || !b) return;
    e.preventDefault();
    handle.setPointerCapture(e.pointerId);
    const width = a.getBoundingClientRect().width + b.getBoundingClientRect().width;
    const startA = a.getBoundingClientRect().width;
    const total = share(left) + share(props.right);
    const start = e.clientX;
    if (width < 2 * COLUMN_MIN) return;
    setDragging(true);
    let latest = props.shares;
    const move = (m: PointerEvent) => {
      const wa = Math.min(width - COLUMN_MIN, Math.max(COLUMN_MIN, startA + m.clientX - start));
      const sa = Math.round((total * wa) / width);
      latest = { ...props.shares, [left]: sa, [props.right]: total - sa };
      props.onShares(latest, false);
    };
    const up = () => {
      setDragging(false);
      handle.removeEventListener("pointermove", move);
      handle.removeEventListener("pointerup", up);
      props.onShares(latest, true);
    };
    handle.addEventListener("pointermove", move);
    handle.addEventListener("pointerup", up);
  };
  const even = () => {
    const left = props.left;
    if (!left) return;
    const total = share(left) + share(props.right);
    const half = Math.round(total / 2);
    props.onShares({ ...props.shares, [left]: half, [props.right]: total - half }, true);
  };
  return (
    <div class="column-resize" classList={{ dragging: dragging() }} onPointerDown={drag} onDblClick={even} title="Drag to resize; double-click to even out">
      <span class="grip" />
    </div>
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
  /** The floating composer's height: the transcript's last item stays above it. */
  const [composerHeight, setComposerHeight] = createSignal(0);
  const setup = () => {
    const s = props.setupOf(path);
    return s && !setupOver(s.status) ? s : undefined;
  };
  /** A session tab: shows it (in place of the terminal, if that's up). */
  const showSession = (id: SessionId) => {
    if (props.terminalOpen(path)) props.onToggleTerminal(path);
    props.onShow(path, id);
  };

  return (
    <section
      ref={(el) => {
        createEffect(() => props.isFocused && el.scrollIntoView({ block: "nearest", inline: "nearest" }));
        // A press on a control focuses on its click instead: focusing grows the composer and moves
        // the transcript, so the control (say a permission's Yes) would slide out from under the
        // pointer before mouseup. Capture, so it still runs if the control removes itself.
        el.addEventListener("click", (e) => !props.isFocused && isControl(e.target) && props.onFocus(path), true);
      }}
      class="column"
      classList={{ focused: props.isFocused }}
      style={{ "--c": worktreeColour(path), "flex-grow": props.shares[path] ?? EVEN }}
      onMouseDown={(e) => !props.isFocused && !isControl(e.target) && props.onFocus(path)}
    >
      <ColumnHeader {...props} session={session()} />
      <SessionTabs
        path={path}
        sessions={sessions()}
        shown={terminal() ? null : view.shown()}
        facts={terminal() ? undefined : session()}
        onShow={showSession}
        onClose={props.onCloseTab}
        actions={
          // (A Review Worktree has only its Code review's session.)
          <Show when={props.worktree.review === null}>
            <button
              class="ghost icon"
              onClick={() => props.onNewSession(path)}
              title="New Agent session in this Worktree"
              aria-label="New Agent session"
              disabled={props.worktree.removed || !!setup()}
            >
              <Plus />
            </button>
          </Show>
        }
      >
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
      </SessionTabs>
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
                  <Show
                    when={props.worktree.review}
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
                    {(n) => (
                      <div class="center empty-worktree">
                        <MessageSquareCode />
                        <p>Its review session starts with the Code review.</p>
                        <button class="primary" onClick={() => props.onCodeReview(n())}>
                          <MessageSquareCode />
                          Open Code review
                        </button>
                      </div>
                    )}
                  </Show>
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
                  onOpenProposal={(id, request) => {
                    props.onFocus(path);
                    props.onOpenProposal(id, request);
                  }}
                  answerKeys={() => props.isFocused}
                  bottomInset={composerHeight}
                />
                <Composer session={s} onHeight={setComposerHeight} takeFocus={props.isFocused} />
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
      <Switch
        fallback={
          <button class="branch opens-branches" onClick={(e) => props.onSwitchBranch(path, e.currentTarget)} title={`${path}: switch branch`}>
            {worktreeLabel(props.worktree)}
            <ChevronDown class="chev" />
          </button>
        }
      >
        <Match when={props.worktree.removed}>
          <span class="branch" title={path}>
            {path.split(/[\\/]/).pop()}
          </span>
        </Match>
        {/* (A Review Worktree stays on its PR's head: no branch to switch.) */}
        <Match when={props.worktree.review}>
          {(n) => (
            <>
              <span class="branch" title={path}>
                {worktreeLabel(props.worktree)}
              </span>
              <ReviewBadge number={n()} onOpen={props.onCodeReview} />
            </>
          )}
        </Match>
      </Switch>
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
      <Show when={props.worktree.review}>
        {(n) => (
          <button class="ghost small" onClick={() => props.onCodeReview(n())} title="The Code review, in the Tabs view">
            <MessageSquareCode />
            Open Code review
          </button>
        )}
      </Show>
      <Show when={!props.worktree.removed}>
        <PrButton worktree={path} class="ghost" onOpen={() => props.onOpenPr(path)} />
      </Show>
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

/** A Worktree's session tabs: a column's, and the Tabs view's above the chat. A tab per session,
 *  with its state, unread count and close ×; then `children` (a column's Terminal tab), the shown
 *  session's facts, and `actions`. */
export function SessionTabs(props: {
  path: string;
  sessions: SessionInfo[];
  /** The session showing; null while something else (a column's Terminal) does. */
  shown: SessionId | null | undefined;
  facts: SessionInfo | undefined;
  onShow: (id: SessionId) => void;
  onClose: (id: SessionId) => void;
  children?: JSX.Element;
  actions?: JSX.Element;
}) {
  return (
    <nav class="column-tabs" style={{ "--c": worktreeColour(props.path) }}>
      <div class="tabs-list">
        <For each={props.sessions}>
          {(s) => (
            <span class="trigger-wrap" classList={{ on: s.id === props.shown }}>
              <button class="trigger" onClick={() => props.onShow(s.id)} onAuxClick={(e) => e.button === 1 && props.onClose(s.id)} title={`${s.name}: ${STATE_LABEL[s.state]}`}>
                <Avatar name={s.name} colour={worktreeColour(props.path)} size={16} />
                {s.name}
                <StateDot state={s.state} />
                <Show when={s.unread > 0 && s.id !== props.shown}>
                  <span class="unread">{s.unread}</span>
                </Show>
              </button>
              <button class="close-tab" aria-label={`Close ${s.name}`} title="Close (it stays in Recent sessions; Ctrl+Shift+T reopens)" onClick={() => props.onClose(s.id)}>
                <X />
              </button>
            </span>
          )}
        </For>
        {props.children}
      </div>
      <span class="grow" />
      <Show when={props.facts} keyed>
        {(s) => <SessionFacts session={s} />}
      </Show>
      {props.actions}
    </nav>
  );
}

/** The shown session's model, how long since it was active, and its cost, beside the tabs. */
function SessionFacts(props: { session: SessionInfo }) {
  const facts = useSessionFacts(() => props.session);
  return (
    <span class="column-facts" title={facts.title()}>
      <Show when={facts.model()}>
        <span class="model">{facts.model()}</span>
      </Show>
      <Show when={facts.active()}>
        <span>{facts.active()}</span>
      </Show>
      <Show when={facts.cost()}>
        <span>{facts.cost()}</span>
      </Show>
    </span>
  );
}

/** Whether a press lands on something clickable, which focuses its column on click, not mousedown. */
function isControl(target: EventTarget | null) {
  return target instanceof Element && !!target.closest("button, a, input, textarea, select, [role='button']");
}

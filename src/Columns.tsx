// The Columns view (ticket 28): each pinned Worktree gets a column with its own session Tabs and
// transcript, streamed in its own view slot. One column has focus: it's the active Worktree, so the
// next prompt, Y / N, the drawer and the palette all follow it. Only the focused column has the
// full composer; the others show a one-line stub that focuses them.
import { createEffect, createSignal, For, lazy, onCleanup, Show } from "solid-js";
import { Banner, Composer, FindOtherConversations, RecentList, reportHeight, STATE_LABEL } from "./Chat";
import type { OtherConversation, RecentSession, SessionId, SessionInfo } from "./core";
import { CirclePause, GitBranch, Pin, Plus, Sparkles, SquareTerminal, X } from "./icons";
import { Setup, type SetupView } from "./Setup";
import { StateDot } from "./StateDot";
import { createTabView, type TabView } from "./TabView";
import { Transcript } from "./Transcript";
import { ContextBar, worktreeColour, worktreeLabel, type WorktreeTab } from "./Worktrees";

// (xterm.js loads with the first Terminal panel.)
const TerminalPanel = lazy(() => import("./TerminalPanel").then((m) => ({ default: m.TerminalPanel })));

/** The narrowest window the Columns view is offered at. */
export const COLUMNS_MIN_WIDTH = 1600;

export interface ColumnsProps {
  /** The pinned Worktrees, in Worktree row order. */
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
  /** Whether a column's Terminal panel (its Worktree's shell, under the transcript) is open. */
  terminalOpen: (path: string) => boolean;
  onToggleTerminal: (path: string) => void;
  /** The columns' Terminal panels' height (one for all, so they line up). */
  terminalHeight: number;
  onTerminalHeight: (px: number) => void;
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
              <span class="note">more later with “+ column” in the title bar</span>
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
  /** The floating composer's (or stub's) height: the transcript's last item stays above it. */
  const [composerHeight, setComposerHeight] = createSignal(0);
  const setup = () => {
    const s = props.setupOf(path);
    return s && s.status.kind !== "done" ? s : undefined;
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
      <nav class="tabs">
        <For each={sessions()}>
          {(s) => (
            <span class={`tab-wrap ${s.id === view.shown() ? "active" : ""}`}>
              <button class="tab" onClick={() => props.onShow(path, s.id)} onAuxClick={(e) => e.button === 1 && props.onCloseTab(s.id)}>
                <Sparkles />
                <StateDot state={s.state} title={STATE_LABEL[s.state]} />
                <span class="name">{s.name}</span>
                <Show when={s.id === view.shown()} fallback={<Show when={s.unread}>{(n) => <span class="badge">{n()}</span>}</Show>}>
                  <span class="sep">·</span>
                  <span class={`state ${s.state}`}>{STATE_LABEL[s.state]}</span>
                </Show>
              </button>
              <button class="close-tab" aria-label={`Close ${s.name}`} title="Close (it stays in Recent sessions; Ctrl+Shift+T reopens)" onClick={() => props.onCloseTab(s.id)}>
                <X />
              </button>
            </span>
          )}
        </For>
        <button class="ghost add-tab" onClick={() => props.onNewSession(path)} title="New Agent session in this Worktree" disabled={props.worktree.removed || !!setup()}>
          <Plus />
          session
        </button>
        <button
          class="ghost icon terminal-toggle"
          classList={{ on: props.terminalOpen(path) }}
          onClick={() => props.onToggleTerminal(path)}
          title="A terminal in this Worktree, under its transcript (Ctrl+` in the focused column)"
          aria-label="Terminal"
          disabled={props.worktree.removed}
        >
          <SquareTerminal />
        </button>
      </nav>
      <Show when={sessions().length > 1}>
        <Banner tone="warn">{sessions().length} Agent sessions share this Worktree, so they can edit the same files.</Banner>
      </Show>
      <div class="column-body">
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
      </div>
      <Show when={props.terminalOpen(path) && !props.worktree.removed}>
        <TerminalPanel
          slot={`column:${path}`}
          worktree={path}
          label={worktreeLabel(props.worktree)}
          colour={worktreeColour(path)}
          placement="bottom"
          size={props.terminalHeight}
          onSize={props.onTerminalHeight}
          takeFocus={props.isFocused}
          onClose={() => props.onToggleTerminal(path)}
        />
      </Show>
    </section>
  );
}

/** The focused column's header is the full context bar; the others say only which Worktree. */
function ColumnHeader(props: ColumnsProps & { worktree: WorktreeTab; isFocused: boolean; session: SessionInfo | undefined }) {
  const needYou = () => props.sessionsIn(props.worktree.path).filter((s) => s.state === "needsYou").length;
  return (
    <Show
      when={!props.isFocused}
      fallback={
        <ContextBar
          worktree={props.worktree}
          session={props.session}
          stateLabel={STATE_LABEL}
          onUnpin={() => props.onUnpin(props.worktree.path)}
          onRemove={() => props.onRemove(props.worktree)}
          onSuspend={props.onSuspend}
          onResume={props.onResume}
        />
      }
    >
      <div class="context-bar column-head">
        <GitBranch />
        <span class="ctx-branch">{worktreeLabel(props.worktree)}</span>
        <Show when={props.worktree.ahead !== null}>
          <span class="counts">
            ↑{props.worktree.ahead} ↓{props.worktree.behind}
          </span>
        </Show>
        <span class="bar-actions">
          <Show
            when={needYou() > 0}
            fallback={
              <Show when={props.session?.state === "idle" ? props.session : undefined}>
                {(s) => (
                  <button onClick={() => props.onSuspend(s())} title="Stop this session's Agent to free memory; sending a message resumes it">
                    <CirclePause />
                    Suspend
                  </button>
                )}
              </Show>
            }
          >
            <span class="badge needs">{needYou()} need{needYou() === 1 ? "s" : ""} you</span>
          </Show>
          <button class="icon" onClick={() => props.onUnpin(props.worktree.path)} title="Close this column (its sessions keep running)" aria-label="Close this column">
            <X />
          </button>
        </span>
      </div>
    </Show>
  );
}

/** An unfocused column's composer: one dimmed line that focuses the column. */
function ComposerStub(props: { worktree: WorktreeTab; session: SessionInfo; onFocus: () => void; onHeight: (px: number) => void }) {
  const branch = () => worktreeLabel(props.worktree);
  return (
    <div class="composer stub" ref={(el) => reportHeight(el, props.onHeight)}>
      <button class="composer-stub" onClick={() => props.onFocus()}>
        <Show when={props.session.state !== "needsYou"} fallback="Answer the permission above, or click to focus">
          Message <GitBranch /> {branch()}
          {props.session.state === "suspended" ? " — sending resumes it" : ""}
        </Show>
      </button>
    </div>
  );
}

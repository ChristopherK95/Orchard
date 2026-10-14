// The Board (ticket 22): every Agent session in the Workspace in columns by state (Needs you,
// Working, Idle, Suspended/Exited), filterable by Worktree. A card waiting on a permission can be
// allowed or denied from here; clicking a card expands it in place into its conversation and a composer (any
// number at once), and a button on the card opens its Tab. Cards move as states change (and are lit for a few seconds when they
// do): they're drawn from the same session store as the Tabs.
import { createEffect, createMemo, createResource, createSignal, For, type JSX, onCleanup, Show } from "solid-js";
import { core, type PermissionRequest, type SessionId, type SessionInfo, type SessionState } from "./core";
import { Avatar } from "./Avatar";
import { Composer } from "./Chat";
import { CirclePause, CirclePlay, GitBranch, Search } from "./icons";
import { noOption, yesOption } from "./PermissionCard";
import { PrButton } from "./PrOverlay";
import { settleProposal } from "./proposals";
import { ContextBar, useSessionFacts } from "./sessionFacts";
import { StateDot } from "./StateDot";
import { createTabView } from "./TabView";
import { Transcript } from "./Transcript";
import { worktreeColour, worktreeLabel, type WorktreeTab } from "./Worktrees";

const COLUMNS: { title: string; states: SessionState[] }[] = [
  { title: "Needs you", states: ["needsYou"] },
  { title: "Working", states: ["working"] },
  { title: "Idle", states: ["idle"] },
  { title: "Suspended / Exited", states: ["suspended", "exited"] },
];

/** What a card says under its name, by state (the Board has no transcript to quote). */
const SUMMARY: Partial<Record<SessionState, string>> = {
  working: "Working…",
  suspended: "Sending a message wakes this session up.",
  exited: "The Agent process ended.",
};

/** How long a card that changed column stays lit. */
const MOVED_FOR_MS = 6000;
const BOARD_SLOT = "board"; // (each expanded card has its own view slot, "board:<id>")

export function Board(props: {
  sessions: SessionInfo[];
  worktrees: WorktreeTab[];
  /** The Worktree shown (null: all of them); kept by the Workspace, so it lasts between visits. */
  only: string | null;
  onOnly: (worktree: string | null) => void;
  /** The PR of a Worktree's branch, in the PR overlay. */
  onOpenPr: (worktree: string) => void;
  /** Where the Board sits: under the title bar. */
  top: number;
  /** The Workspace's error and notice banners, shown here too. */
  banners?: JSX.Element;
  /** Opens the session's Tab. */
  onOpen: (id: number) => void;
  /** "Open in editor" on a code block or a file edit's permission card in an expanded card. */
  onOpenSnippet: (code: string, label: string) => void;
  onOpenProposal: (sessionId: SessionId, request: PermissionRequest) => void;
  onError: (message: string) => void;
}) {
  // Cards expand in place, so several conversations can be open and used side by side.
  const [expanded, setExpanded] = createSignal<ReadonlySet<SessionId>>(new Set());
  const toggle = (id: SessionId) =>
    setExpanded((all) => {
      const next = new Set(all);
      if (!next.delete(id)) next.add(id);
      return next;
    });
  // A card that moves to another column is lit for a few seconds, so it's clear what changed.
  const [moved, setMoved] = createSignal<ReadonlySet<SessionId>>(new Set());
  const lastState = new Map<SessionId, SessionState>();
  const timers = new Map<SessionId, ReturnType<typeof setTimeout>>();
  const column = (state: SessionState) => COLUMNS.findIndex((c) => c.states.includes(state));
  createEffect(() => {
    for (const s of props.sessions) {
      const before = lastState.get(s.id);
      lastState.set(s.id, s.state);
      if (before === undefined || column(before) === column(s.state)) continue;
      setMoved((all) => new Set(all).add(s.id));
      clearTimeout(timers.get(s.id));
      timers.set(
        s.id,
        setTimeout(() => {
          timers.delete(s.id);
          setMoved((all) => {
            const rest = new Set(all);
            rest.delete(s.id);
            return rest;
          });
        }, MOVED_FOR_MS),
      );
    }
  });
  onCleanup(() => timers.forEach(clearTimeout));
  const shown = createMemo(() => props.sessions.filter((s) => props.only === null || s.worktree === props.only));
  const label = (path: string) => {
    const w = props.worktrees.find((w) => w.path === path);
    return w ? worktreeLabel(w) : path.split(/[\\/]/).pop();
  };

  return (
    <section class="board" style={{ top: `${props.top}px` }}>
      {props.banners}
      <div class="board-head">
        <span class="label">Filter</span>
        <button class="chip all" classList={{ on: props.only === null }} onClick={() => props.onOnly(null)}>
          All
          <span class="badge">{props.sessions.length}</span>
        </button>
        <For each={props.worktrees}>
          {(w) => {
            const here = () => props.sessions.filter((s) => s.worktree === w.path);
            return (
              <button
                class="chip"
                classList={{ on: props.only === w.path }}
                style={{ "--c": worktreeColour(w.path) }}
                onClick={() => props.onOnly(props.only === w.path ? null : w.path)}
                title={w.path}
              >
                <GitBranch />
                <span class="label-text">{worktreeLabel(w)}</span>
                <span class="badge" classList={{ needs: here().some((s) => s.state === "needsYou") }}>
                  {here().length}
                </span>
              </button>
            );
          }}
        </For>
        <span class="hint">Esc or Ctrl+B for the Tabs</span>
      </div>
      <div class="board-columns">
        <For each={COLUMNS}>
          {(column) => {
            const cards = () => shown().filter((s) => column.states.includes(s.state));
            return (
              <div class="board-column">
                <div class={`board-column-head ${column.states[0]}`}>
                  <StateDot state={column.states[0]} />
                  {column.title}
                  <span class="badge" classList={{ needs: column.states[0] === "needsYou" && cards().length > 0 }}>
                    {cards().length}
                  </span>
                </div>
                <For each={cards()} fallback={<p class="board-empty">None</p>}>
                  {(session) => (
                    <Card
                      session={session}
                      worktree={label(session.worktree) ?? ""}
                      moved={moved().has(session.id)}
                      expanded={expanded().has(session.id)}
                      onToggle={() => toggle(session.id)}
                      onOpenSnippet={props.onOpenSnippet}
                      onOpenProposal={props.onOpenProposal}
                      onError={props.onError}
                      onOpen={() => props.onOpen(session.id)}
                      onOpenPr={() => props.onOpenPr(session.worktree)}
                    />
                  )}
                </For>
              </div>
            );
          }}
        </For>
      </div>
    </section>
  );
}

/** An expanded card's conversation and a composer to prompt the session from here. Each has its own view slot, so any number of cards can be open at once. */
function Conversation(props: {
  session: SessionInfo;
  onOpenSnippet: (code: string, label: string) => void;
  onOpenProposal: (sessionId: SessionId, request: PermissionRequest) => void;
  onError: (message: string) => void;
}) {
  const id = props.session.id;
  const view = createTabView(`${BOARD_SLOT}:${id}`);
  view.show(id).catch((err) => props.onError(String(err)));
  onCleanup(() => view.hide());
  const [composerHeight, setComposerHeight] = createSignal(0);
  return (
    <div class="board-convo" onClick={(e) => e.stopPropagation()}>
      <Transcript
        sessionId={id}
        items={view.items}
        start={view.start()}
        onLoadEarlier={view.loadEarlier}
        onError={props.onError}
        onOpenSnippet={props.onOpenSnippet}
        onOpenProposal={props.onOpenProposal}
        bottomInset={composerHeight}
      />
      <Composer session={props.session} onHeight={setComposerHeight} />
    </div>
  );
}

function Card(props: {
  session: SessionInfo;
  worktree: string;
  moved: boolean;
  expanded: boolean;
  onToggle: () => void;
  onOpen: () => void;
  onOpenPr: () => void;
  onOpenSnippet: (code: string, label: string) => void;
  onOpenProposal: (sessionId: SessionId, request: PermissionRequest) => void;
  onError: (message: string) => void;
}) {
  // The question it's waiting on, read again whenever it comes to need you (or is answered).
  const [pending, { refetch }] = createResource(
    () => (props.session.state === "needsYou" ? props.session.id : null),
    (id) => core.pendingPermission(id),
  );
  const facts = useSessionFacts(() => props.session);
  const [answering, setAnswering] = createSignal(false);
  const [error, setError] = createSignal("");
  const act = (action: Promise<void>) => action.catch((err) => setError(String(err)));
  const answer = async (optionId: string, toolCallId: string) => {
    setAnswering(true);
    setError("");
    try {
      await core.answerPermission(props.session.id, toolCallId, optionId);
      settleProposal(props.session.id, toolCallId);
      void refetch(); // (another question may be waiting behind it)
    } catch (err) {
      setError(String(err));
    } finally {
      setAnswering(false);
    }
  };

  return (
    // (The whole card expands with the mouse; its name is the button for the keyboard.)
    <div
      class={`board-card ${props.session.state}`}
      classList={{ moved: props.moved, expanded: props.expanded }}
      style={{ "--c": worktreeColour(props.session.worktree) }}
      onClick={() => props.onToggle()}
    >
      <div class="board-card-top">
        <GitBranch />
        <span class="wt">{props.worktree}</span>
        <span class="grow" />
        <PrButton worktree={props.session.worktree} class="ghost" compact onOpen={props.onOpenPr} />
        <button
          class="ghost icon board-card-tabs"
          onClick={(e) => {
            e.stopPropagation();
            props.onOpen();
          }}
          title="Open in Tabs"
          aria-label="Open in Tabs"
        >
          <Search />
        </button>
      </div>
      <div class="board-card-head" title={facts.title()}>
        <Avatar name={props.session.name} colour={worktreeColour(props.session.worktree)} size={36} />
        <span class="who">
          <span class="name-line">
            <button
              class="board-card-open"
              onClick={(e) => {
                e.stopPropagation();
                props.onToggle();
              }}
              aria-expanded={props.expanded}
              title={props.expanded ? "Collapse the conversation" : "Expand to the conversation"}
            >
              {props.session.name}
            </button>
            <Show when={props.session.unread > 0}>
              <span class="badge">{props.session.unread}</span>
            </Show>
            <ContextBar context={facts.context()} />
          </span>
          <span class="facts">
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
        </span>
      </div>
      <Show when={SUMMARY[props.session.state]}>{(text) => <span class="summary">{text()}</span>}</Show>
      <Show when={props.session.state === "idle" || props.session.state === "exited"}>
        <div class="card-actions" onClick={(e) => e.stopPropagation()}>
          <Show
            when={props.session.state === "idle"}
            fallback={
              <button onClick={() => void act(core.resumeSession(props.session.id))} title="Bring the Agent back with this conversation">
                <CirclePlay />
                Resume
              </button>
            }
          >
            <button onClick={() => void act(core.suspendSession(props.session.id))} title="Stop this session's Agent to free memory; sending a message resumes it">
              <CirclePause />
              Suspend
            </button>
          </Show>
        </div>
      </Show>
      <Show when={props.session.state !== "needsYou" && error()}>
        <p class="error small" onClick={(e) => e.stopPropagation()}>
          {error()}
        </p>
      </Show>
      <Show when={props.session.state === "needsYou" && pending.error}>
        <p class="error small" onClick={(e) => e.stopPropagation()}>
          Couldn't read its question: {String(pending.error)}
        </p>
      </Show>
      <Show when={props.session.state === "needsYou" && !pending.error ? pending.latest : undefined}>
        {(request) => (
          <div class="board-question" onClick={(e) => e.stopPropagation()}>
            <span class="what">
              <b>{request().title}</b>
              <Show when={request().target && !request().title.includes(request().target!)}>
                {" "}
                <span class="mono">{request().target}</span>
              </Show>
            </span>
            <div class="actions">
              <Show when={yesOption(request())}>
                {(yes) => (
                  <button class="primary" disabled={answering() || pending.loading} onClick={() => void answer(yes().id, request().toolCallId)}>
                    {yes().name}
                  </button>
                )}
              </Show>
              <Show when={noOption(request())}>
                {(no) => (
                  <button disabled={answering() || pending.loading} onClick={() => void answer(no().id, request().toolCallId)}>
                    {no().name}
                  </button>
                )}
              </Show>
            </div>
            <Show when={error()}>
              <p class="error small">{error()}</p>
            </Show>
          </div>
        )}
      </Show>
      <Show when={props.expanded}>
        <Conversation
          session={props.session}
          onOpenSnippet={props.onOpenSnippet}
          onOpenProposal={props.onOpenProposal}
          onError={props.onError}
        />
      </Show>
    </div>
  );
}

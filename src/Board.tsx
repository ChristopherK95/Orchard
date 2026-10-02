// The Board (ticket 22): every Agent session in the Workspace in columns by state (Needs you,
// Working, Idle, Suspended/Exited), filterable by Worktree. A card waiting on a permission can be
// allowed or denied from here; clicking a card opens its Tab. Cards move as states change: they're
// drawn from the same session store as the Tabs.
import { createMemo, createResource, createSignal, For, type JSX, Show } from "solid-js";
import { core, type SessionInfo, type SessionState } from "./core";
import { noOption, yesOption } from "./PermissionCard";
import { worktreeColour, worktreeLabel, type WorktreeTab } from "./Worktrees";

const COLUMNS: { title: string; states: SessionState[] }[] = [
  { title: "Needs you", states: ["needsYou"] },
  { title: "Working", states: ["working"] },
  { title: "Idle", states: ["idle"] },
  { title: "Suspended / Exited", states: ["suspended", "exited"] },
];

export function Board(props: {
  sessions: SessionInfo[];
  worktrees: WorktreeTab[];
  /** The Worktree shown (null: all of them); kept by the Workspace, so it lasts between visits. */
  only: string | null;
  onOnly: (worktree: string | null) => void;
  /** Where the Board sits: under the title bar. */
  top: number;
  /** The Workspace's error and notice banners, shown here too. */
  banners?: JSX.Element;
  onOpen: (id: number) => void;
}) {
  const shown = createMemo(() => props.sessions.filter((s) => props.only === null || s.worktree === props.only));
  const label = (path: string) => {
    const w = props.worktrees.find((w) => w.path === path);
    return w ? worktreeLabel(w) : path.split(/[\\/]/).pop();
  };

  return (
    <section class="board" style={{ top: `${props.top}px` }}>
      {props.banners}
      <div class="board-head">
        <label>
          Worktree{" "}
          <select value={props.only ?? ""} onChange={(e) => props.onOnly(e.currentTarget.value || null)}>
            <option value="">All Worktrees</option>
            <For each={props.worktrees}>{(w) => <option value={w.path}>{worktreeLabel(w)}</option>}</For>
          </select>
        </label>
        <span class="grow" />
        <span class="muted small">Esc or Ctrl+B for the Tabs</span>
      </div>
      <div class="board-columns">
        <For each={COLUMNS}>
          {(column) => {
            const cards = () => shown().filter((s) => column.states.includes(s.state));
            return (
              <div class="board-column">
                <div class="board-column-head">
                  <b>{column.title}</b> <span class="muted">{cards().length}</span>
                </div>
                <For each={cards()} fallback={<p class="muted small center">None</p>}>
                  {(session) => <Card session={session} worktree={label(session.worktree) ?? ""} onOpen={() => props.onOpen(session.id)} />}
                </For>
              </div>
            );
          }}
        </For>
      </div>
    </section>
  );
}

function Card(props: { session: SessionInfo; worktree: string; onOpen: () => void }) {
  // The question it's waiting on, read again whenever it comes to need you (or is answered).
  const [pending, { refetch }] = createResource(
    () => (props.session.state === "needsYou" ? props.session.id : null),
    (id) => core.pendingPermission(id),
  );
  const [answering, setAnswering] = createSignal(false);
  const [error, setError] = createSignal("");
  const answer = async (optionId: string, toolCallId: string) => {
    setAnswering(true);
    setError("");
    try {
      await core.answerPermission(props.session.id, toolCallId, optionId);
      void refetch(); // (another question may be waiting behind it)
    } catch (err) {
      setError(String(err));
    } finally {
      setAnswering(false);
    }
  };

  return (
    // (The whole card opens the Tab with the mouse; its name is the button for the keyboard.)
    <div class={`board-card ${props.session.state}`} style={{ "--c": worktreeColour(props.session.worktree) }} onClick={() => props.onOpen()}>
      <div class="board-card-head">
        <span class={`dot ${props.session.state}`} />
        <button
          class="board-card-open"
          onClick={(e) => {
            e.stopPropagation();
            props.onOpen();
          }}
          title="Open its Tab"
        >
          {props.session.name}
        </button>
        <Show when={props.session.unread > 0}>
          <span class="badge">{props.session.unread}</span>
        </Show>
        <span class="grow" />
        <span class="worktree-chip mono">{props.worktree}</span>
      </div>
      <Show when={props.session.state === "needsYou" && pending.error}>
        <p class="error small" onClick={(e) => e.stopPropagation()}>
          Couldn't read its question: {String(pending.error)}
        </p>
      </Show>
      <Show when={props.session.state === "needsYou" && !pending.error ? pending.latest : undefined}>
        {(request) => (
          <div class="board-question" onClick={(e) => e.stopPropagation()}>
            <span>
              {request().title}
              <Show when={request().target && !request().title.includes(request().target!)}>
                {" "}
                <span class="mono muted">{request().target}</span>
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
    </div>
  );
}

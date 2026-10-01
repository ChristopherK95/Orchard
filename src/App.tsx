// Walking skeleton view (ticket 01): prerequisite gate → open a Workspace → one Tab with a
// streaming transcript and a composer. It only renders core state and sends commands (ADR 0003).
import { createEffect, createSignal, For, Match, onCleanup, onMount, Show, Switch } from "solid-js";
import { createStore, produce, reconcile } from "solid-js/store";
import {
  core,
  type MissingPrerequisite,
  type PermissionMode,
  type SessionId,
  type SessionInfo,
  type SessionState,
  type TranscriptDelta,
  type TranscriptItem,
  type WorkspaceInfo,
} from "./core";
import { benchMode, type BenchDriver, runBenchmark } from "./benchmark";
import { answerByKey, PermissionCard } from "./PermissionCard";
import { notify, NOTIFY_WHEN_BACKGROUND_TURN_FINISHES, onNotificationClicked } from "./notify";

const STATE_LABEL: Record<SessionState, string> = {
  working: "Working",
  needsYou: "Needs you",
  idle: "Idle",
  exited: "Exited",
};

export function App() {
  const [problems, setProblems] = createSignal<MissingPrerequisite[] | null>(null);
  const [workspace, setWorkspace] = createSignal<WorkspaceInfo | null>(null);

  onMount(async () => setProblems(await core.prerequisites()));

  return (
    <Switch fallback={<div class="center muted">Checking prerequisites…</div>}>
      <Match when={problems()?.length}>
        <Prerequisites problems={problems()!} />
      </Match>
      <Match when={problems() && !workspace()}>
        <OpenWorkspace onOpened={setWorkspace} />
      </Match>
      <Match when={workspace()}>
        <WorkspaceView workspace={workspace()!} />
      </Match>
    </Switch>
  );
}

function Prerequisites(props: { problems: MissingPrerequisite[] }) {
  return (
    <div class="center">
      <div class="card">
        <h1>Missing prerequisites</h1>
        <p class="muted">The editor can't start until these are installed:</p>
        <ul>
          <For each={props.problems}>{(p) => <li>{p.message}</li>}</For>
        </ul>
        <p class="muted">Restart the editor afterwards.</p>
      </div>
    </div>
  );
}

function OpenWorkspace(props: { onOpened: (w: WorkspaceInfo) => void }) {
  const [path, setPath] = createSignal("");
  const [error, setError] = createSignal("");
  onMount(async () => {
    setPath((await core.defaultWorkspacePath()) ?? "");
    // The benchmark (ticket 05) opens its repository without anyone clicking.
    if (path() && (await benchMode())) props.onOpened(await core.openWorkspace(path()));
  });

  const open = async (e: Event) => {
    e.preventDefault();
    setError("");
    try {
      props.onOpened(await core.openWorkspace(path().trim()));
    } catch (err) {
      setError(String(err));
    }
  };

  return (
    <div class="center">
      <form class="card" onSubmit={open}>
        <h1>Open a repository</h1>
        <input value={path()} onInput={(e) => setPath(e.currentTarget.value)} placeholder="Path to a git repository" autofocus />
        <button type="submit" class="primary" disabled={!path().trim()}>
          Open
        </button>
        <Show when={error()}>
          <p class="error">{error()}</p>
        </Show>
      </form>
    </div>
  );
}

function WorkspaceView(props: { workspace: WorkspaceInfo }) {
  // Sessions as the core reports them; one of them is the visible Tab.
  const [sessions, setSessions] = createStore<Record<SessionId, SessionInfo>>({});
  const [order, setOrder] = createSignal<SessionId[]>([]);
  const [activeId, setActiveId] = createSignal<SessionId | null>(null);
  const session = () => {
    const id = activeId();
    return id === null ? undefined : sessions[id];
  };
  // The visible Tab's transcript from `start` on (the core sends the latest page first).
  const [items, setItems] = createStore<TranscriptItem[]>([]);
  const [start, setStart] = createSignal(0);
  const [error, setError] = createSignal("");

  // Each show() gets a token; batches from an earlier stream (even of the same session) are dropped.
  let currentShow = 0;
  const applyFor = (token: number) => (batch: TranscriptDelta[]) => {
    if (token !== currentShow) return;
    for (const delta of batch) {
      if (delta.kind === "reset") {
        setStart(delta.start);
        setItems(reconcile(delta.items));
        continue;
      }
      const at = delta.index - start();
      if (at < 0) continue; // a change to an item before the loaded page
      if (delta.kind === "itemAdded" || delta.kind === "itemUpdated") setItems(at, delta.item);
      else
        setItems(
          produce((all) => {
            const item = all[at];
            if (item?.kind === "agent") item.text += delta.text;
          }),
        );
    }
  };

  const show = async (id: SessionId) => {
    const token = ++currentShow;
    setActiveId(id);
    setItems([]);
    setStart(0);
    try {
      await core.showSession(id, applyFor(token));
    } catch (err) {
      setError(String(err));
    }
  };

  const newSession = async (): Promise<SessionId | undefined> => {
    setError("");
    try {
      const id = await core.newSession();
      await show(id);
      return id;
    } catch (err) {
      setError(String(err));
    }
  };

  // Turns the benchmark is waiting on: resolved when that session next becomes Idle.
  const turnWaiters = new Map<SessionId, () => void>();
  const benchDriver: BenchDriver = {
    newSession: async () => {
      const id = await newSession();
      if (id === undefined) throw new Error(error());
      return id;
    },
    show,
    send: (id, text) =>
      new Promise((resolve, reject) => {
        turnWaiters.set(id, resolve);
        core.sendPrompt(id, text).catch(reject);
      }),
    state: (id) => sessions[id]?.state,
  };

  let loadingEarlier = false;
  const loadEarlier = async () => {
    const id = activeId();
    const before = start();
    if (id === null || before === 0 || loadingEarlier) return;
    loadingEarlier = true;
    const token = currentShow;
    try {
      const page = await core.transcriptPageBefore(id, before);
      // Only prepend if nothing moved underneath us (another Tab shown, or a Reset).
      if (token !== currentShow || start() !== before) return;
      setItems((current) => [...page.items, ...current]);
      setStart(page.start);
    } catch (err) {
      setError(String(err));
    } finally {
      loadingEarlier = false;
    }
  };

  // A session you're not looking at (another Tab is visible, or the window is in the background)
  // raises an OS notification when it needs you (spec story 25).
  const onStateChanged = (id: SessionId, state: SessionState) => {
    const { name, state: previous } = sessions[id]; // read before the store updates
    setSessions(id, "state", state);
    if (state === "idle") {
      turnWaiters.get(id)?.();
      turnWaiters.delete(id);
    }
    const unseen = id !== activeId() || !document.hasFocus();
    if (!unseen) return;
    if (state === "needsYou") void notify(id, `${name} needs you`, "The Agent is waiting for your answer.");
    else if (NOTIFY_WHEN_BACKGROUND_TURN_FINISHES && previous === "working" && state === "idle")
      void notify(id, `${name} finished`, "The Agent's turn is done.");
  };

  onMount(async () => {
    const unlisten = await core.onEvent((event) => {
      if (event.kind === "sessionCreated") {
        setSessions(event.session.id, event.session);
        setOrder((ids) => [...ids, event.session.id]);
      } else if (!sessions[event.sessionId]) return;
      else if (event.kind === "sessionStateChanged") onStateChanged(event.sessionId, event.state);
      else if (event.kind === "permissionModeChanged") setSessions(event.sessionId, "permissionMode", event.mode);
      else setSessions(event.sessionId, "unread", event.unread);
    });
    onCleanup(unlisten);
    // Clicking a notification opens the session it was about.
    const unlistenClicks = await onNotificationClicked((id) => sessions[id] && id !== activeId() && void show(id));
    onCleanup(unlistenClicks);
    // Y/N answer the oldest open permission card anywhere in the Tab (outside text fields).
    const onKey = (e: KeyboardEvent) => {
      const s = session();
      if (s?.state === "needsYou" && answerByKey(e, s.id, items)) e.preventDefault();
    };
    window.addEventListener("keydown", onKey);
    onCleanup(() => window.removeEventListener("keydown", onKey));
    const first = await newSession();
    if (first !== undefined && (await benchMode())) void runBenchmark(benchDriver, first);
  });

  const sharing = () => {
    const s = session();
    return s ? order().filter((id) => sessions[id].worktree === s.worktree).length : 0;
  };

  return (
    <div class="workspace">
      <header class="titlebar">
        <b>{props.workspace.name}</b>
        <span class="muted mono">{props.workspace.root}</span>
      </header>
      <nav class="tabs">
        <For each={order()}>
          {(id) => (
            <button class={`tab ${id === activeId() ? "active" : ""}`} onClick={() => id !== activeId() && void show(id)}>
              <span class={`dot ${sessions[id].state}`} title={STATE_LABEL[sessions[id].state]} />
              {sessions[id].name}
              <Show when={id === activeId()} fallback={<Show when={sessions[id].unread}>{(n) => <span class="badge">{n()}</span>}</Show>}>
                <span class="muted">{STATE_LABEL[sessions[id].state]}</span>
              </Show>
            </button>
          )}
        </For>
        <button class="ghost add-tab" onClick={() => void newSession()} title="New Agent session in this Worktree">
          ＋ session
        </button>
      </nav>
      <Show when={sharing() > 1}>
        <p class="warning banner">
          {sharing()} Agent sessions share this Worktree, so they can edit the same files.
        </p>
      </Show>
      <Show when={error()}>
        <p class="error banner">{error()}</p>
      </Show>
      <Show when={session()} keyed>
        {(s) => (
          <>
            <Transcript sessionId={s.id} items={items} hasEarlier={start() > 0} onLoadEarlier={loadEarlier} />
            <Composer session={s} />
          </>
        )}
      </Show>
    </div>
  );
}

function Transcript(props: {
  sessionId: SessionId;
  items: TranscriptItem[];
  hasEarlier: boolean;
  onLoadEarlier: () => void;
}) {
  let log!: HTMLDivElement;
  // Keep the newest output in view while it streams (virtualisation arrives in ticket 02).
  createEffect(() => {
    const last = props.items[props.items.length - 1];
    void (last && (last.kind === "permission" ? last.outcome : last.text));
    void props.items.length;
    requestAnimationFrame(() => (log.scrollTop = log.scrollHeight));
  });
  return (
    <div class="transcript" ref={log}>
      <Show when={props.hasEarlier}>
        <button class="ghost load-earlier" onClick={() => props.onLoadEarlier()}>
          Load earlier messages
        </button>
      </Show>
      <For each={props.items}>
        {(item) =>
          item.kind === "permission" ? (
            <PermissionCard sessionId={props.sessionId} request={item.request} outcome={item.outcome} />
          ) : (
            <div class={`msg ${item.kind}`}>{item.text}</div>
          )
        }
      </For>
    </div>
  );
}

const MODE_LABEL: Record<PermissionMode, string> = {
  askForEdits: "Ask for edits",
  acceptEdits: "Accept edits",
  plan: "Plan",
};

function Composer(props: { session: SessionInfo }) {
  const [text, setText] = createSignal("");
  const [error, setError] = createSignal("");
  const disabled = () => props.session.state !== "idle";
  let input!: HTMLTextAreaElement;
  onMount(() => input.focus());
  // Back to the composer once a turn (or a permission question) is over.
  createEffect(() => props.session.state === "idle" && input.focus());

  const setMode = async (mode: PermissionMode) => {
    setError("");
    try {
      await core.setPermissionMode(props.session.id, mode);
    } catch (err) {
      setError(String(err));
    }
  };

  const send = async () => {
    const prompt = text().trim();
    if (!prompt || disabled()) return;
    setError("");
    try {
      await core.sendPrompt(props.session.id, prompt);
      setText("");
      input.focus();
    } catch (err) {
      setError(String(err));
    }
  };

  return (
    <div class="composer">
      <Show when={error()}>
        <p class="error">{error()}</p>
      </Show>
      <textarea
        ref={input}
        value={text()}
        onInput={(e) => setText(e.currentTarget.value)}
        onKeyDown={(e) => {
          if (e.key === "Enter" && !e.shiftKey) {
            e.preventDefault();
            void send();
          }
        }}
        placeholder={
          props.session.state === "exited"
            ? "The Agent exited."
            : props.session.state === "needsYou"
              ? "Answer the permission card above first (Y / N)."
              : "Message the Agent (Enter to send, Shift+Enter for a newline)"
        }
        disabled={props.session.state === "exited" || props.session.state === "needsYou"}
      />
      <div class="composer-side">
        <select
          title="Permission mode"
          value={props.session.permissionMode}
          onChange={(e) => void setMode(e.currentTarget.value as PermissionMode)}
          disabled={props.session.state === "exited"}
        >
          <For each={Object.keys(MODE_LABEL) as PermissionMode[]}>
            {(mode) => <option value={mode}>{MODE_LABEL[mode]}</option>}
          </For>
        </select>
        <button class="primary" onClick={send} disabled={disabled() || !text().trim()}>
          Send
        </button>
      </div>
    </div>
  );
}

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
import { answerByKey, PermissionCard } from "./PermissionCard";

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
  onMount(async () => setPath((await core.defaultWorkspacePath()) ?? ""));

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
  // Sessions as the core reports them; the Tab shows the one this view created.
  const [sessions, setSessions] = createStore<Record<SessionId, SessionInfo>>({});
  const [activeId, setActiveId] = createSignal<SessionId | null>(null);
  const session = () => {
    const id = activeId();
    return id === null ? undefined : sessions[id];
  };
  const [items, setItems] = createStore<TranscriptItem[]>([]);
  const [error, setError] = createSignal("");

  const apply = (batch: TranscriptDelta[]) => {
    for (const delta of batch) {
      if (delta.kind === "reset") setItems(reconcile(delta.items));
      else if (delta.kind === "itemAdded" || delta.kind === "itemUpdated") setItems(delta.index, delta.item);
      else
        setItems(
          produce((all) => {
            const item = all[delta.index];
            if (item.kind === "agent") item.text += delta.text;
          }),
        );
    }
  };

  onMount(async () => {
    const unlisten = await core.onEvent((event) => {
      if (event.kind === "sessionCreated") setSessions(event.session.id, event.session);
      else if (!sessions[event.sessionId]) return;
      else if (event.kind === "sessionStateChanged") setSessions(event.sessionId, "state", event.state);
      else setSessions(event.sessionId, "permissionMode", event.mode);
    });
    onCleanup(unlisten);
    // Y/N answer the oldest open permission card anywhere in the Tab (outside text fields).
    const onKey = (e: KeyboardEvent) => {
      const s = session();
      if (s?.state === "needsYou" && answerByKey(e, s.id, items)) e.preventDefault();
    };
    window.addEventListener("keydown", onKey);
    onCleanup(() => window.removeEventListener("keydown", onKey));
    try {
      const id = await core.newSession();
      setActiveId(id);
      await core.watchSession(id, apply);
    } catch (err) {
      setError(String(err));
    }
  });

  return (
    <div class="workspace">
      <header class="titlebar">
        <b>{props.workspace.name}</b>
        <span class="muted mono">{props.workspace.root}</span>
      </header>
      <nav class="tabs">
        <Show when={session()}>
          {(s) => (
            <div class="tab active">
              <span class={`dot ${s().state}`} title={STATE_LABEL[s().state]} />
              {s().name}
              <span class="muted">{STATE_LABEL[s().state]}</span>
            </div>
          )}
        </Show>
      </nav>
      <Show when={error()}>
        <p class="error banner">{error()}</p>
      </Show>
      <Show when={session()}>
        {(s) => (
          <>
            <Transcript sessionId={s().id} items={items} />
            <Composer session={s()} />
          </>
        )}
      </Show>
    </div>
  );
}

function Transcript(props: { sessionId: SessionId; items: TranscriptItem[] }) {
  let log!: HTMLDivElement;
  // Keep the newest output in view while it streams (virtualisation arrives in ticket 02).
  createEffect(() => {
    for (const item of props.items) void (item.kind === "permission" ? item.outcome : item.text);
    requestAnimationFrame(() => (log.scrollTop = log.scrollHeight));
  });
  return (
    <div class="transcript" ref={log}>
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

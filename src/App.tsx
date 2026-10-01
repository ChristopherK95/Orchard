// Walking skeleton view (ticket 01): prerequisite gate → open a Workspace → one Tab with a
// streaming transcript and a composer. It only renders core state and sends commands (ADR 0003).
import { batch, createEffect, createSignal, For, Match, onCleanup, onMount, Show, Switch } from "solid-js";
import { createStore, produce, reconcile } from "solid-js/store";
import {
  core,
  type LoadedSettings,
  type MissingPrerequisite,
  type PermissionMode,
  type SessionId,
  type SessionInfo,
  type SessionState,
  type TranscriptDelta,
  type TranscriptItem,
  type WorkspaceInfo,
  type WorktreeInfo,
} from "./core";
import { type BenchDriver, runBenchmark } from "./benchmark";
import { answerByKey } from "./PermissionCard";
import { NewWorktreeDialog } from "./NewWorktreeDialog";
import { Transcript } from "./Transcript";
import { ContextBar, removedWorktree, worktreeColour, WorktreeRow, type WorktreeTab } from "./Worktrees";
import { notify, onNotificationClicked } from "./notify";
import { keepOutput, Setup, type SetupView } from "./Setup";

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
    if (path() && (await core.benchMode())) {
      try {
        props.onOpened(await core.openWorkspace(path()));
      } catch (err) {
        setError(String(err));
      }
    }
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

  // Worktrees as the core reports them, the one whose sessions are shown, and the session last
  // used in each (so switching back returns to it).
  const [worktrees, setWorktrees] = createSignal<WorktreeInfo[]>([]);
  const [activeWorktree, setActiveWorktree] = createSignal(props.workspace.root);
  const lastSessionIn = new Map<string, SessionId>();
  const [creatingWorktree, setCreatingWorktree] = createSignal(false);
  /** Worktree setups this editor started, by Worktree path; shown until the first session opens. */
  const [setups, setSetups] = createStore<Record<string, SetupView>>({});
  const updateSetup = (worktree: string, change: (setup: SetupView) => void) =>
    setSetups(
      produce((all) => {
        all[worktree] ??= { commands: [], status: { kind: "running", step: 0 }, output: "" };
        change(all[worktree]);
      }),
    );
  /** The active Worktree's first session comes from its setup (or Start anyway), not "＋ session". */
  const settingUp = () => {
    const setup = setups[activeWorktree()];
    return !!setup && setup.status.kind !== "done";
  };
  const [settings, setSettings] = createSignal<LoadedSettings | null>(null);
  /** Something to know that isn't an error (e.g. a fetch failed, so a Worktree started from stale refs). */
  const [notice, setNotice] = createSignal("");
  const sessionsIn = (path: string) => order().map((id) => sessions[id]).filter((s) => s.worktree === path);
  const rowWorktrees = (): WorktreeTab[] => {
    const listed = worktrees();
    const vanished = [...new Set(order().map((id) => sessions[id].worktree))].filter((p) => !listed.some((w) => w.path === p));
    return [...listed, ...vanished.map(removedWorktree)];
  };
  const worktree = () => rowWorktrees().find((w) => w.path === activeWorktree());

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
    const path = sessions[id]?.worktree;
    if (path) {
      setActiveWorktree(path);
      lastSessionIn.set(path, id);
    }
    setActiveId(id);
    setItems([]);
    setStart(0);
    try {
      await core.showSession(id, applyFor(token));
    } catch (err) {
      setError(String(err));
    }
  };

  /** Shows a Worktree: its last-used session, else its first, else an empty Tab row. */
  const selectWorktree = (path: string) => {
    const here = sessionsIn(path);
    const last = lastSessionIn.get(path);
    const id = here.some((s) => s.id === last) ? last : here[0]?.id;
    if (id !== undefined) return void show(id);
    ++currentShow; // nothing should stream into an empty Worktree's view
    setActiveWorktree(path);
    setActiveId(null);
    setItems([]);
    void core.hideTabs();
  };

  /** A new Agent session in the active Worktree. */
  const newSession = async (path = activeWorktree()): Promise<SessionId | undefined> => {
    setError("");
    try {
      const id = await core.newSessionIn(path);
      await show(id);
      return id;
    } catch (err) {
      setError(String(err));
    }
  };

  // Benchmark turns in flight: finished once that session has been Working and is Idle again.
  const turnWaiters = new Map<SessionId, { sawWorking: boolean; resolve: () => void; reject: (e: Error) => void }>();
  const benchDriver: BenchDriver = {
    newSession: async () => {
      const id = await newSession();
      if (id === undefined) throw new Error(error());
      return id;
    },
    show,
    turn: (id, text) =>
      new Promise((resolve, reject) => {
        turnWaiters.set(id, { sawWorking: false, resolve, reject });
        core.sendPrompt(id, text).catch((err) => {
          turnWaiters.delete(id);
          reject(err);
        });
      }),
  };
  const onBenchState = (id: SessionId, state: SessionState) => {
    const waiter = turnWaiters.get(id);
    if (!waiter) return;
    if (state === "working") waiter.sawWorking = true;
    else if (state === "exited") waiter.reject(new Error("the Agent exited mid-turn"));
    else if (state === "idle" && waiter.sawWorking) waiter.resolve();
    else return;
    if (state !== "working") turnWaiters.delete(id);
  };

  /** Prepends the page before the loaded items; returns how many items were added. */
  let loadingEarlier = false;
  const loadEarlier = async (): Promise<number> => {
    const id = activeId();
    const before = start();
    if (id === null || before === 0 || loadingEarlier) return 0;
    loadingEarlier = true;
    const token = currentShow;
    try {
      const page = await core.transcriptPageBefore(id, before);
      // Only prepend if nothing moved underneath us (another Tab shown, or a Reset).
      if (token !== currentShow || start() !== before) return 0;
      // Items and start change together, so row keys (start + index) never point at the wrong item.
      batch(() => {
        setItems((current) => [...page.items, ...current]);
        setStart(page.start);
      });
      return page.items.length;
    } finally {
      loadingEarlier = false;
    }
  };

  // A session you're not looking at (another Tab is visible, or the window is in the background)
  // raises an OS notification when it needs you (spec story 25).
  const onStateChanged = (id: SessionId, state: SessionState) => {
    const { name, state: previous } = sessions[id]; // read before the store updates
    setSessions(id, "state", state);
    onBenchState(id, state);
    const unseen = id !== activeId() || !document.hasFocus();
    if (!unseen) return;
    if (state === "needsYou") void notify(id, `${name} needs you`, "The Agent is waiting for your answer.");
    else if (settings()?.settings.notifications.turnFinished && previous === "working" && state === "idle")
      void notify(id, `${name} finished`, "The Agent's turn is done.");
  };

  let sawWorktreesEvent = false;
  let sawSettingsEvent = false;
  onMount(async () => {
    const unlisten = await core.onEvent((event) => {
      if (event.kind === "worktreesChanged") {
        sawWorktreesEvent = true;
        setWorktrees(event.worktrees);
        // The active Worktree was removed: fall back to the main checkout.
        const active = activeWorktree();
        if (!event.worktrees.some((w) => w.path === active) && sessionsIn(active).length === 0) selectWorktree(props.workspace.root);
      } else if (event.kind === "settingsChanged") {
        sawSettingsEvent = true;
        setSettings(event.settings);
      } else if (event.kind === "setupChanged") {
        const { worktree, commands, status } = event;
        updateSetup(worktree, (s) => Object.assign(s, { commands, status }));
        // Setup finished while you watched it: on to its session.
        if (status.kind === "done" && activeWorktree() === worktree && activeId() === null) void show(status.sessionId);
      } else if (event.kind === "setupOutput") {
        const { text } = event;
        updateSetup(event.worktree, (s) => (s.output = keepOutput(s.output, text)));
      } else if (event.kind === "sessionCreated") {
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
    // Worktrees may have changed while the editor was in the background (the watcher covers the rest).
    const onFocus = () => void core.refreshWorktrees();
    window.addEventListener("focus", onFocus);
    onCleanup(() => window.removeEventListener("focus", onFocus));
    const loaded = await core.settings();
    if (!sawSettingsEvent) setSettings(loaded);
    // Setups started before this view (e.g. the webview reloaded); events since then win.
    for (const setup of await core.setups()) if (!setups[setup.worktree]) setSetups(setup.worktree, setup);
    const snapshot = await core.worktrees();
    if (!sawWorktreesEvent) setWorktrees(snapshot);
    const first = await newSession();
    if (first !== undefined && (await core.benchMode())) void runBenchmark(benchDriver, first).catch((err) => setError(`Benchmark failed: ${err}`));
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
        <span class="grow" />
        <button class="ghost" onClick={() => core.openRepoSettings().catch((err) => setError(String(err)))} title="Settings for this repo, e.g. its Worktree setup commands">
          Repo settings
        </button>
      </header>
      <WorktreeRow worktrees={rowWorktrees()} active={activeWorktree()} sessionsIn={sessionsIn} onSelect={selectWorktree} onNewSession={(path) => void newSession(path)} onNewWorktree={() => setCreatingWorktree(true)} />
      <Show when={creatingWorktree()}>
        <NewWorktreeDialog
          activeBranch={worktree()?.branch ?? null}
          onCreated={(created) => {
            // The Worktree exists now: close, and start its session in the main view, where a
            // failure leaves the (empty) Worktree selected with "＋ session" to retry. With a
            // setup, show it running; the core starts the session once it's done.
            setCreatingWorktree(false);
            setNotice(created.warning ?? "");
            const setup = created.setup;
            if (!setup) return void newSession(created.worktree.path);
            // Events may have got here first: keep their status and output.
            updateSetup(setup.worktree, (s) => (s.commands = setup.commands));
            selectWorktree(setup.worktree);
          }}
          onGoToWorktree={(path) => {
            setCreatingWorktree(false);
            selectWorktree(path);
          }}
          onClose={() => setCreatingWorktree(false)}
        />
      </Show>
      <nav class="tabs" style={{ "--c": worktreeColour(activeWorktree()) }}>
        <For each={sessionsIn(activeWorktree()).map((s) => s.id)}>
          {(id) => (
            <button class={`tab ${id === activeId() ? "active" : ""}`} onClick={() => id !== activeId() && void show(id)}>
              <span class="agent-glyph">✦</span>
              <span class={`dot ${sessions[id].state}`} title={STATE_LABEL[sessions[id].state]} />
              {sessions[id].name}
              <Show when={id === activeId()} fallback={<Show when={sessions[id].unread}>{(n) => <span class="badge">{n()}</span>}</Show>}>
                <span class="muted">{STATE_LABEL[sessions[id].state]}</span>
              </Show>
            </button>
          )}
        </For>
        <button class="ghost add-tab" onClick={() => void newSession()} title="New Agent session in this Worktree" disabled={worktree()?.removed || settingUp()}>
          ＋ session
        </button>
      </nav>
      <ContextBar worktree={worktree()} session={session()} stateLabel={STATE_LABEL} />
      <Show when={sharing() > 1}>
        <p class="warning banner">
          {sharing()} Agent sessions share this Worktree, so they can edit the same files.
        </p>
      </Show>
      <Show when={notice()}>
        <p class="warning banner" onClick={() => setNotice("")} title="Click to dismiss">
          {notice()}
        </p>
      </Show>
      <Show when={settings()?.error}>
        {(e) => <p class="error banner">Settings not applied: {e()}</p>}
      </Show>
      <Show when={error()}>
        <p class="error banner">{error()}</p>
      </Show>
      <Show
        when={session()}
        keyed
        fallback={
          <Show
            when={setups[activeWorktree()]?.status.kind !== "done" && setups[activeWorktree()]}
            fallback={
              <div class="center muted empty-worktree">
                <p>No Agent sessions in this Worktree yet.</p>
                <button class="primary" onClick={() => void newSession()}>
                  ＋ session
                </button>
              </div>
            }
          >
            {(setup) => <Setup worktree={activeWorktree()} setup={setup()} onError={setError} />}
          </Show>
        }
      >
        {(s) => (
          <>
            <Transcript sessionId={s.id} items={items} start={start()} onLoadEarlier={loadEarlier} onError={setError} />
            <Composer session={s} />
          </>
        )}
      </Show>
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

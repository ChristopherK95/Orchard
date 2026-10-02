// Walking skeleton view (ticket 01): prerequisite gate → open a Workspace → one Tab with a
// streaming transcript and a composer. It only renders core state and sends commands (ADR 0003).
import { batch, createEffect, createSignal, For, lazy, Match, onCleanup, onMount, Show, Switch } from "solid-js";
import { createStore, produce, reconcile } from "solid-js/store";
import {
  core,
  type LoadedSettings,
  type MissingPrerequisite,
  type AutoSuspendReason,
  type PermissionMode,
  type RecentSession,
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
import { RemoveWorktreeDialog } from "./RemoveWorktreeDialog";
import { CommandPalette, type PaletteCommand } from "./CommandPalette";
import { EditNotes } from "./EditNotes";
import { FilesDrawer } from "./FilesDrawer";
import { GitDrawer } from "./GitDrawer";
import type { OpenRequest } from "./ManualEditor";
// CodeMirror loads with the first file opened, not at startup.
const ManualEditor = lazy(() => import("./ManualEditor").then((m) => ({ default: m.ManualEditor })));
import { Transcript } from "./Transcript";
import { ContextBar, removedWorktree, worktreeColour, worktreeLabel, WorktreeRow, type WorktreeTab } from "./Worktrees";
import { notify, onNotificationClicked } from "./notify";
import { keepOutput, Setup, type SetupView } from "./Setup";

const STATE_LABEL: Record<SessionState, string> = {
  working: "Working",
  needsYou: "Needs you",
  idle: "Idle",
  suspended: "Suspended",
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
  /** Each Worktree's Recent sessions (closed Tabs), as the core last reported them. */
  const [recent, setRecent] = createStore<Record<string, RecentSession[]>>({});
  const [recentOpen, setRecentOpen] = createSignal(false);
  createEffect(() => {
    const path = activeWorktree();
    if (!recent[path]) void core.recentSessions(path).then((list) => !recent[path] && setRecent(path, list));
  });
  /** The Files drawer and Ctrl+P; `revision` per Worktree, bumped when its files change. */
  /** The drawer on the right edge: the Worktree's files, or its git status (one at a time). */
  const [drawer, setDrawer] = createSignal<"files" | "git" | null>(null);
  const filesOpen = () => drawer() === "files";
  const gitOpen = () => drawer() === "git";
  const setFilesOpen = (open: boolean) => setDrawer(open ? "files" : null);
  const toggleDrawer = (which: "files" | "git") => setDrawer((now) => (now === which ? null : which));
  const [paletteOpen, setPaletteOpen] = createSignal(false);
  const [filesRevision, setFilesRevision] = createStore<Record<string, number>>({});
  const [revealed, setRevealed] = createSignal<{ path: string; n: number } | null>(null);
  /** The Worktree being looked at gets its files indexed and watched. */
  createEffect(() => void core.showWorktree(activeWorktree()).catch(() => {}));
  /** The Manual editor pane, open while it has file tabs. */
  const [editorOpen, setEditorOpen] = createSignal(false);
  /** Every open request, in order (none lost while the pane's code is still loading). */
  const [editorRequests, setEditorRequests] = createSignal<{ open: OpenRequest; n: number }[]>([]);
  let requested = 0;
  const openInEditor = (open: OpenRequest) => {
    setEditorOpen(true);
    setEditorRequests((all) => [...all.slice(-20), { open, n: ++requested }]);
  };
  /** The settings file, with a section for this repo, in the Manual editor. */
  const openRepoSettings = () =>
    core
      .openRepoSettings()
      .then((path) => openInEditor({ kind: "file", path }))
      .catch((err) => setError(String(err)));
  /** A file of the active Worktree, by its `/`-separated relative path (joined the way the
   *  Worktree's own path is written, which matters for `\\?\UNC\…` paths). */
  const worktreePath = (rel: string) => {
    const root = activeWorktree();
    return root.includes("\\") ? `${root.replace(/\\$/, "")}\\${rel.replaceAll("/", "\\")}` : `${root}/${rel}`;
  };
  const openWorktreeFile = (rel: string) => openInEditor({ kind: "file", path: worktreePath(rel) });
  const runCommand = (command: PaletteCommand) =>
    command.kind === "newSessionHere" ? void newSession() : setCreatingWorktree(true);
  /** The Worktree whose removal dialog is open. */
  const [removing, setRemoving] = createSignal<WorktreeTab | null>(null);
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
    setRecentOpen(false);
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

  /** Brings a closed session back in a new Tab and shows it. */
  const reopen = async (opening: Promise<SessionId | null>) => {
    setError("");
    setRecentOpen(false);
    try {
      const id = await opening;
      if (id !== null) await show(id); // (nothing to reopen is no error)
    } catch (err) {
      setError(String(err));
    }
  };

  const closeTab = (id: SessionId) => core.closeTab(id).catch((err) => setError(String(err)));

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
    openFile: async () => {
      // The file bench-memory.ps1 writes; reading it first makes a missing file fail the run.
      const path = worktreePath("src/main.rs");
      await core.readFile(path);
      openInEditor({ kind: "file", path });
      await new Promise((resolve) => setTimeout(resolve, 1500)); // (opened, grammar loaded)
    },
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
      } else if (event.kind === "sessionClosed") {
        // Closed (its × or a middle-click, or its Worktree is being removed): its Tab goes.
        const id = event.sessionId;
        const path = sessions[id]?.worktree;
        setOrder((ids) => ids.filter((other) => other !== id));
        setSessions(produce((all) => void delete all[id]));
        // Back to what's left of its Worktree, or to the main checkout if the Worktree's gone.
        if (activeId() === id && path) selectWorktree(worktrees().some((w) => w.path === path) ? path : props.workspace.root);
      } else if (event.kind === "popOutReturned") {
        // A popped-out window closed before it showed its file: back into the pane, unsaved work and all.
        if (event.window === "main") openInEditor({ kind: "poppedOut", file: event.file });
      } else if (event.kind === "filesChanged") {
        setFilesRevision(event.worktree, (n) => (n ?? 0) + 1);
      } else if (event.kind === "fileWatchFallback") {
        setNotice(event.message);
      } else if (event.kind === "autoSuspended") {
        setNotice(autoSuspendNotice(event.suspended));
      } else if (event.kind === "recentSessionsChanged") {
        setRecent(event.worktree, event.sessions);
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
      } else if (
        event.kind === "documentChangedOnDisk" ||
        event.kind === "documentConflicted" ||
        event.kind === "documentBackOnDisk" ||
        event.kind === "documentsChanged" ||
        event.kind === "editNotesChanged" ||
        event.kind === "gitStatusChanged"
      ) {
        return; // (the Manual editors', permission cards', Edit note chips' and Git drawer's business)
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
      // (Ctrl+P is never the browser's print, even with a dialog open.)
      if (e.ctrlKey && !e.shiftKey && e.key.toLowerCase() === "p") {
        e.preventDefault();
        if (!removing() && !creatingWorktree()) setPaletteOpen(true);
        return;
      }
      if (removing() || creatingWorktree() || paletteOpen()) return; // a dialog is open over the Tab
      if (e.ctrlKey && e.shiftKey && e.key.toLowerCase() === "e") {
        e.preventDefault();
        return void toggleDrawer("files");
      }
      // (Not when the editor took it: there it's find-previous.)
      if (e.ctrlKey && e.shiftKey && e.key.toLowerCase() === "g" && !e.defaultPrevented) {
        e.preventDefault();
        return void toggleDrawer("git");
      }
      if (e.ctrlKey && e.shiftKey && e.key.toLowerCase() === "t") {
        e.preventDefault();
        if (!e.repeat) void reopen(core.reopenLastClosed());
        return;
      }
      if (e.key === "Escape") setRecentOpen(false);
      const s = session();
      if (s?.state === "needsYou" && answerByKey(e, s.id, items)) e.preventDefault();
    };
    window.addEventListener("keydown", onKey);
    onCleanup(() => window.removeEventListener("keydown", onKey));
    // The Recent menu closes on a click anywhere else.
    const onClick = (e: MouseEvent) => !(e.target as Element | null)?.closest?.(".recent-menu") && setRecentOpen(false);
    window.addEventListener("click", onClick);
    onCleanup(() => window.removeEventListener("click", onClick));
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
    // The Tabs open when the editor last closed come back (Suspended until used).
    const restored = (await core.sessions()).filter((s) => !sessions[s.id]);
    batch(() => {
      for (const s of restored) setSessions(s.id, s);
      setOrder((ids) => [...restored.map((s) => s.id), ...ids]);
    });
    if (restored.length) {
      const last = await core.lastActiveSession();
      return void show(restored.find((s) => s.id === last)?.id ?? restored[0].id);
    }
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
        <button class="ghost" classList={{ on: filesOpen() }} onClick={() => toggleDrawer("files")} title="Files of this Worktree (Ctrl+Shift+E); Ctrl+P to find one">
          Files
        </button>
        <button class="ghost" classList={{ on: gitOpen() }} onClick={() => toggleDrawer("git")} title="Git: stage, commit, discard (Ctrl+Shift+G)">
          Git
        </button>
        <button class="ghost" onClick={openRepoSettings} title="Settings for this repo, e.g. its Worktree setup commands">
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
            <span class={`tab-wrap ${id === activeId() ? "active" : ""}`}>
              <button
                class={`tab ${id === activeId() ? "active" : ""}`}
                onClick={() => id !== activeId() && void show(id)}
                onAuxClick={(e) => e.button === 1 && void closeTab(id)}
              >
                <span class="agent-glyph">✦</span>
                <span class={`dot ${sessions[id].state}`} title={STATE_LABEL[sessions[id].state]} />
                {sessions[id].name}
                <Show when={id === activeId()} fallback={<Show when={sessions[id].unread}>{(n) => <span class="badge">{n()}</span>}</Show>}>
                  <span class="muted">{STATE_LABEL[sessions[id].state]}</span>
                </Show>
              </button>
              <button
                class="close-tab"
                aria-label={`Close ${sessions[id].name}`}
                title="Close (it stays in Recent sessions; Ctrl+Shift+T reopens)"
                onClick={() => void closeTab(id)}
              >
                ×
              </button>
            </span>
          )}
        </For>
        <button class="ghost add-tab" onClick={() => void newSession()} title="New Agent session in this Worktree" disabled={worktree()?.removed || settingUp()}>
          ＋ session
        </button>
        <Show when={recent[activeWorktree()]?.length}>
          <span class="recent-menu">
            <button class="ghost" onClick={() => setRecentOpen((open) => !open)} title="Closed sessions in this Worktree">
              Recent ▾
            </button>
            <Show when={recentOpen()}>
              <RecentList sessions={recent[activeWorktree()] ?? []} onReopen={(r) => void reopen(core.reopenSession(r.acpId))} />
            </Show>
          </span>
        </Show>
      </nav>
      <ContextBar
        worktree={worktree()}
        session={session()}
        stateLabel={STATE_LABEL}
        onRemove={() => setRemoving(worktree() ?? null)}
        onSuspend={(s) => core.suspendSession(s.id).catch((err) => setError(String(err)))}
        onResume={(s) => core.resumeSession(s.id).catch((err) => setError(String(err)))}
      />
      <Show when={paletteOpen()}>
        <CommandPalette
          worktree={activeWorktree()}
          onFile={(path) => {
            openWorktreeFile(path);
            // (and marked in the Files drawer, if that's open)
            if (filesOpen()) setRevealed((now) => ({ path, n: (now?.n ?? 0) + 1 }));
          }}
          revision={filesRevision[activeWorktree()] ?? 0}
          onCommand={runCommand}
          onClose={() => setPaletteOpen(false)}
        />
      </Show>
      <Show when={removing()}>
        {(w) => (
          <RemoveWorktreeDialog
            worktree={w().path}
            label={worktreeLabel(w())}
            sessionName={(id) => sessions[id]?.name ?? `Session ${id}`}
            onRemoved={(warning) => {
              const path = w().path;
              setRemoving(null);
              setNotice(warning ?? "");
              setSetups(produce((all) => void delete all[path]));
              selectWorktree(props.workspace.root);
            }}
            onClose={() => setRemoving(null)}
          />
        )}
      </Show>
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
      <div class="main-row">
      <div class="main-col">
      <Show
        when={session()}
        keyed
        fallback={
          <Show
            when={setups[activeWorktree()]?.status.kind !== "done" && setups[activeWorktree()]}
            fallback={
              <div class="center muted empty-worktree">
                <p>No Agent sessions open in this Worktree.</p>
                <button class="primary" onClick={() => void newSession()}>
                  ＋ session
                </button>
                <Show when={recent[activeWorktree()]?.length}>
                  <p class="small">Or reopen a Recent session:</p>
                  <RecentList sessions={recent[activeWorktree()] ?? []} onReopen={(r) => void reopen(core.reopenSession(r.acpId))} />
                </Show>
              </div>
            }
          >
            {(setup) => <Setup worktree={activeWorktree()} setup={setup()} onError={setError} onOpenSettings={openRepoSettings} />}
          </Show>
        }
      >
        {(s) => (
          <>
            <Transcript sessionId={s.id} items={items} start={start()} onLoadEarlier={loadEarlier} onError={setError} onOpenSnippet={(code, label) => openInEditor({ kind: "snippet", code, label })} />
            <Composer session={s} />
          </>
        )}
      </Show>
      </div>
      <Show when={editorOpen()}>
        <ManualEditor
          requests={editorRequests()}
          vim={settings()?.settings.editor?.vim ?? false}
          onEmpty={() => setEditorOpen(false)}
          onError={setError}
        />
      </Show>
      <Show when={filesOpen()}>
        <FilesDrawer
          worktree={activeWorktree()}
          revision={filesRevision[activeWorktree()] ?? 0}
          reveal={revealed()}
          onOpenFile={openWorktreeFile}
          onClose={() => setFilesOpen(false)}
        />
      </Show>
      <Show when={gitOpen()}>
        <GitDrawer worktree={activeWorktree()} onOpenFile={openWorktreeFile} onClose={() => setDrawer(null)} />
      </Show>
      </div>
    </div>
  );
}

const gb = (bytes: number) => `${(bytes / 1024 ** 3).toFixed(1)} GB`;

const andList = (names: string[]) =>
  names.length === 1 ? names[0] : `${names.slice(0, -1).join(", ")} and ${names.at(-1)}`;

/** The toast after Idle sessions were Suspended automatically (one check's, by reason). */
function autoSuspendNotice(suspended: { session: SessionInfo; reason: AutoSuspendReason }[]): string {
  const why = (reason: AutoSuspendReason) =>
    reason.kind === "memoryLimit"
      ? `to free memory: the Agents were using ${gb(reason.usedBytes)}, over the ${gb(reason.limitBytes)} limit`
      : reason.kind === "lowMemory"
        ? `because the computer is low on memory (${gb(reason.availableBytes)} free)`
        : `after ${reason.minutes} minutes Idle`;
  const byReason = new Map<string, { reason: AutoSuspendReason; names: string[] }>();
  for (const { session, reason } of suspended) {
    const key = JSON.stringify(reason);
    const entry = byReason.get(key) ?? { reason, names: [] };
    entry.names.push(session.name);
    byReason.set(key, entry);
  }
  const parts = [...byReason.values()].map(({ reason, names }) => `${andList(names)} ${why(reason)}`);
  return `Suspended ${parts.join("; and ")}. Sending a message resumes a session.`;
}

function RecentList(props: { sessions: RecentSession[]; onReopen: (session: RecentSession) => void }) {
  return (
    <div class="recent-list">
      <For each={props.sessions}>
        {(s) => (
          <button class="ghost" onClick={() => props.onReopen(s)} title="Reopen with its conversation">
            <span class="agent-glyph">✦</span> {s.name}
          </button>
        )}
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
  // A Suspended session takes a message too: sending it resumes the session.
  const disabled = () => props.session.state !== "idle" && props.session.state !== "suspended";
  let input!: HTMLTextAreaElement;
  onMount(() => input.focus());
  // Back to the composer once a turn (or a permission question) is over.
  createEffect(() => props.session.state === "idle" && input.focus());

  const setMode = async (select: HTMLSelectElement) => {
    setError("");
    try {
      await core.setPermissionMode(props.session.id, select.value as PermissionMode);
    } catch (err) {
      setError(String(err));
      select.value = props.session.permissionMode; // back to the mode it's really in
    }
  };

  const resume = async () => {
    setError("");
    try {
      await core.resumeSession(props.session.id);
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
      <EditNotes sessionId={props.session.id} />
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
            ? "The Agent exited. Resume it to carry on."
            : props.session.state === "suspended"
              ? "Suspended. Sending a message resumes it (Enter to send, Shift+Enter for a newline)."
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
          onChange={(e) => void setMode(e.currentTarget)}
        >
          <For each={Object.keys(MODE_LABEL) as PermissionMode[]}>
            {(mode) => <option value={mode}>{MODE_LABEL[mode]}</option>}
          </For>
        </select>
        <Show
          when={props.session.state === "exited"}
          fallback={
            <button class="primary" onClick={send} disabled={disabled() || !text().trim()}>
              Send
            </button>
          }
        >
          <button class="primary" onClick={() => void resume()} title="Bring the Agent back with this conversation">
            Resume
          </button>
        </Show>
      </div>
    </div>
  );
}

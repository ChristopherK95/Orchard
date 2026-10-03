// Walking skeleton view (ticket 01): prerequisite gate → open a Workspace → one Tab with a
// streaming transcript and a composer. It only renders core state and sends commands (ADR 0003).
import { batch, createEffect, createSignal, For, lazy, Match, on, onCleanup, onMount, Show, Switch } from "solid-js";
import { createStore, produce } from "solid-js/store";
import {
  core,
  type LoadedSettings,
  type MissingPrerequisite,
  type OpenDocument,
  type AutoSuspendReason,
  type RecentSession,
  type SessionId,
  type SessionInfo,
  type SessionState,
  TABS_SLOT,
  type WorkspaceInfo,
  type WorktreeInfo,
} from "./core";
import { type BenchDriver, runBenchmark } from "./benchmark";
import { answerByKey } from "./PermissionCard";
import { NewWorktreeDialog } from "./NewWorktreeDialog";
import { RemoveWorktreeDialog } from "./RemoveWorktreeDialog";
import { CommandPalette, type PaletteCommand } from "./CommandPalette";
import { FilesDrawer } from "./FilesDrawer";
import { GitDrawer } from "./GitDrawer";
import type { OpenRequest } from "./ManualEditor";
// CodeMirror loads with the first file opened, not at startup.
const ManualEditor = lazy(() => import("./ManualEditor").then((m) => ({ default: m.ManualEditor })));
import { Board } from "./Board";
import { Transcript } from "./Transcript";
import { ContextBar, removedWorktree, worktreeColour, worktreeLabel, WorktreeRow, type WorktreeTab } from "./Worktrees";
import { notify, onNotificationClicked } from "./notify";
import { keepOutput, Setup, type SetupView } from "./Setup";
import { StateDot } from "./StateDot";
import { Banner, Composer, RecentList, STATE_LABEL } from "./Chat";
import { Columns, COLUMNS_MIN_WIDTH } from "./Columns";
import { createTabView, type TabView } from "./TabView";
import { BareTitlebar, WindowControls } from "./WindowControls";
import { WorkspacePicker } from "./WorkspacePicker";
import { ChevronDown, GitBranch, Loader, PanelRight, Plus, Search, Settings, Sparkles, X } from "./icons";

export function App() {
  const [problems, setProblems] = createSignal<MissingPrerequisite[] | null>(null);
  const [workspace, setWorkspace] = createSignal<WorkspaceInfo | null>(null);

  onMount(async () => setProblems(await core.prerequisites()));

  return (
    <Show
      when={workspace()}
      keyed
      fallback={
        // Before a Workspace is open, a bare title bar (the window has no native one).
        <div class="startup-window">
          <BareTitlebar />
          <Switch
            fallback={
              <div class="center muted small">
                <Loader class="spin" />
                &nbsp;Checking prerequisites…
              </div>
            }
          >
            <Match when={problems()?.length}>
              <Prerequisites problems={problems()!} />
            </Match>
            <Match when={problems()}>
              <OpenWorkspace onOpened={setWorkspace} />
            </Match>
          </Switch>
        </div>
      }
    >
      {/* (Keyed: switching Workspace builds a fresh view, so nothing of the old one stays.) */}
      {(w) => <WorkspaceView workspace={w} onSwitched={setWorkspace} />}
    </Show>
  );
}

function AppMark() {
  return (
    <div class="app-mark">
      <span class="mark">
        <Sparkles />
      </span>
      Agent Editor
    </div>
  );
}

function Prerequisites(props: { problems: MissingPrerequisite[] }) {
  return (
    <div class="startup">
      <div class="startup-card">
        <AppMark />
        <h1>The editor can't start until these are installed:</h1>
        <div class="list-box">
          <For each={props.problems}>
            {(p) => (
              <div class="list-box-row">
                <X />
                <span>{p.message}</span>
              </div>
            )}
          </For>
        </div>
        <p class="muted small">Restart the editor afterwards.</p>
      </div>
    </div>
  );
}

/** The CLI argument's repo opens straight away; otherwise (or if it can't) the Workspace picker. */
function OpenWorkspace(props: { onOpened: (w: WorkspaceInfo) => void }) {
  const [picker, setPicker] = createSignal<{ text: string; error: string } | null>(null);
  onMount(async () => {
    const path = (await core.defaultWorkspacePath()) ?? "";
    if (!path) return setPicker({ text: "", error: "" });
    try {
      props.onOpened(await core.openWorkspace(path));
    } catch (err) {
      setPicker({ text: path, error: String(err) });
    }
  });
  return (
    <Show when={picker()}>
      {(p) => <WorkspacePicker header={<AppMark />} onOpened={props.onOpened} initialText={p().text} error={p().error} />}
    </Show>
  );
}

function WorkspaceView(props: { workspace: WorkspaceInfo; onSwitched: (w: WorkspaceInfo) => void }) {
  // Sessions as the core reports them; one of them is the Tabs view's visible Tab.
  const [sessions, setSessions] = createStore<Record<SessionId, SessionInfo>>({});
  const [order, setOrder] = createSignal<SessionId[]>([]);
  /** The Tabs view's visible Tab and its transcript (each column has its own; see Columns.tsx). */
  const tabs = createTabView(TABS_SLOT);
  const activeId = tabs.shown;
  const session = () => {
    const id = activeId();
    return id === null ? undefined : sessions[id];
  };
  const [error, setError] = createSignal("");
  /** The floating composer's height: the transcript's last item stays above it. */
  const [composerHeight, setComposerHeight] = createSignal(0);

  // Worktrees as the core reports them, the one whose sessions are shown, and the session last
  // used in each (so switching back returns to it).
  const [worktrees, setWorktrees] = createSignal<WorktreeInfo[]>([]);
  // In the Columns view the active Worktree is the focused column's.
  const [activeWorktree, setActiveWorktree] = createSignal(props.workspace.root);
  const [lastSessionIn, setLastSessionIn] = createStore<Record<string, SessionId>>({});
  const [creatingWorktree, setCreatingWorktree] = createSignal(false);
  /** Tabs or Columns, with the Board of every session by state over either (they stay as they are
   *  underneath). */
  const [mainView, setMainView] = createSignal<"tabs" | "columns">("tabs");
  const [boardOpen, setBoardOpen] = createSignal(false);
  const view = () => (boardOpen() ? "board" : mainView());
  /** The Worktrees pinned as columns, and those in Worktree row order. */
  const [pinned, setPinnedList] = createSignal<string[]>([]);
  const pinnedRow = () => rowWorktrees().filter((w) => pinned().includes(w.path));
  const setPinned = (path: string, on: boolean) =>
    core
      .setPinned(path, on)
      .then(setPinnedList)
      .catch((err) => setError(String(err)));
  /** Gives a Worktree a column and focuses it. */
  const pin = async (path: string) => {
    await setPinned(path, true);
    setActiveWorktree(path);
  };
  /** Closes a Worktree's column; focus moves to a neighbour. */
  const unpin = (path: string) => {
    const row = pinnedRow();
    const at = row.findIndex((w) => w.path === path);
    const next = row[at + 1] ?? row[at - 1];
    if (path === activeWorktree() && next) setActiveWorktree(next.path);
    void setPinned(path, false);
  };
  /** The title bar's "+ column" menu. */
  const [addColumnAt, setAddColumnAt] = createSignal<{ left: number; top: number } | null>(null);
  /** The Columns view is offered from COLUMNS_MIN_WIDTH up. */
  const [wide, setWide] = createSignal(window.innerWidth >= COLUMNS_MIN_WIDTH);
  /** Each column's view slot, by Worktree (Y / N answer the focused one's card). */
  const columnViews = new Map<string, TabView>();
  /** The session a column shows: its last-used one, else its first. */
  const shownIn = (path: string) => {
    const here = sessionsIn(path);
    const last = lastSessionIn[path];
    return here.some((s) => s.id === last) ? last : here[0]?.id;
  };
  /** Whether a session's transcript is on screen (so it shouldn't notify). */
  const onScreen = (id: SessionId) =>
    mainView() === "columns" ? pinnedRow().some((w) => shownIn(w.path) === id) : id === activeId();
  /** Switches between the Tabs and Columns views (closing the Board). */
  const chooseView = (next: "tabs" | "columns") => {
    setBoardOpen(false);
    if (next === mainView()) return;
    setMainView(next);
    if (next === "columns") {
      tabs.hide(); // (its columns stream instead)
      setRecentOpen(false);
      const focus = pinnedRow().find((w) => w.path === activeWorktree()) ?? pinnedRow()[0];
      if (focus) setActiveWorktree(focus.path);
    } else selectWorktree(activeWorktree());
  };
  /** The drawer can be parked on one Worktree in the Columns view; otherwise it follows focus. */
  const [drawerPin, setDrawerPin] = createSignal<string | null>(null);
  const drawerWorktree = () => (mainView() === "columns" && drawerPin()) || activeWorktree();
  const drawerTab = () => rowWorktrees().find((w) => w.path === drawerWorktree());
  /** The Board's Worktree filter (null: all), kept between visits. */
  const [boardOnly, setBoardOnly] = createSignal<string | null>(null);
  let titlebar!: HTMLElement;
  // Back on the Tabs, focus goes back where it was (the composer, say).
  let focusedBefore: Element | null = null;
  createEffect(
    on(
      view,
      (now) => {
        if (now === "board") focusedBefore = document.activeElement;
        else if (focusedBefore instanceof HTMLElement && focusedBefore.isConnected) focusedBefore.focus();
      },
      { defer: true },
    ),
  );
  const needYou = () => order().filter((id) => sessions[id]?.state === "needsYou").length;
  /** "New Worktree from this branch" (from the Git drawer's branch picker). */
  const [creatingFrom, setCreatingFrom] = createSignal<string | undefined>(undefined);
  /** Each Worktree's Recent sessions (closed Tabs), as the core last reported them. */
  const [recent, setRecent] = createStore<Record<string, RecentSession[]>>({});
  const [recentOpen, setRecentOpen] = createSignal(false);
  /** Where the Recent menu opens (it's fixed, so the scrolling session row doesn't clip it). */
  const [recentAt, setRecentAt] = createSignal({ left: 0, top: 0 });
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
  /** The drawer the title bar's button opens: the one last shown. */
  let lastDrawer: "files" | "git" = "files";
  const toggleDrawer = (which: "files" | "git") => {
    setBoardOpen(false); // (the drawers are beside the Tabs or columns)
    lastDrawer = which;
    setDrawer((now) => (now === which ? null : which));
  };
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
    setBoardOpen(false); // (the Manual editor is beside the Tabs or columns)
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
  const worktreePath = (rel: string, root = activeWorktree()) => {
    return root.includes("\\") ? `${root.replace(/\\$/, "")}\\${rel.replaceAll("/", "\\")}` : `${root}/${rel}`;
  };
  const openWorktreeFile = (rel: string, root = activeWorktree()) => openInEditor({ kind: "file", path: worktreePath(rel, root) });
  const runCommand = (command: PaletteCommand) =>
    command.kind === "newSessionHere" ? void newSession() : command.kind === "switchWorkspace" ? setSwitching(true) : setCreatingWorktree(true);

  /** The Workspace picker is open over this Workspace (ticket 30). */
  const [switching, setSwitching] = createSignal(false);
  /** Leaving this Workspace waits on the user: sessions mid-turn, or unsaved files. */
  const [leaving, setLeaving] = createSignal<{
    busy: string[];
    unsaved: OpenDocument[];
    answer: (go: "save" | "discard" | null) => void;
  } | null>(null);
  let editorControls: { saveAll: () => Promise<boolean> } | undefined;
  /** Asks (if anything would be lost) whether to leave this Workspace; saves first if told to. */
  const mayLeave = async () => {
    const busy = order()
      .map((id) => sessions[id])
      .filter((s) => s && (s.state === "working" || s.state === "needsYou"))
      .map((s) => s.name);
    const unsaved = (await core.openDocuments().catch(() => [])).filter((d) => d.dirty);
    if (!busy.length && !unsaved.length) return true;
    const go = await new Promise<"save" | "discard" | null>((answer) => setLeaving({ busy, unsaved, answer }));
    setLeaving(null);
    if (go === "save") return (await editorControls?.saveAll()) ?? true; // (a refused save says why in its banner)
    return go === "discard";
  };
  /** The picker's open: the one already open just closes it; another is switched to, if allowed. */
  const switchTo = async (path: string) => {
    const target = await core.workspaceFor(path);
    if (target.root === props.workspace.root) {
      setSwitching(false);
      return null;
    }
    if (!(await mayLeave())) return null;
    await core.closePopOuts();
    return core.openWorkspace(target.root);
  };
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

  /** Shows a session: in its column if the Columns view is up and its Worktree is pinned, else as
   *  the Tabs view's visible Tab (a notification, a reopened Tab, the palette, the Board). */
  const show = async (id: SessionId) => {
    const path = sessions[id]?.worktree;
    if (path) setLastSessionIn(path, id);
    if (path && mainView() === "columns" && pinned().includes(path)) {
      setBoardOpen(false);
      setActiveWorktree(path);
      return;
    }
    if (mainView() === "columns") setMainView("tabs"); // (its columns go, and stop streaming)
    setBoardOpen(false);
    if (path) setActiveWorktree(path);
    try {
      await tabs.show(id);
    } catch (err) {
      setError(String(err));
    }
  };

  /** Shows a Worktree: its last-used session, else its first, else an empty Tab row. */
  const selectWorktree = (path: string) => {
    setRecentOpen(false);
    if (mainView() === "columns" && pinned().includes(path)) return void setActiveWorktree(path);
    const id = shownIn(path);
    if (id !== undefined) return void show(id);
    if (mainView() === "columns") setMainView("tabs");
    setActiveWorktree(path);
    tabs.hide(); // nothing should stream into an empty Worktree's view
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
    newWorktree: async (name) => (await core.createWorktree({ kind: "newBranch", name, startPoint: null })).worktree.path,
    newSessionIn: async (worktree) => {
      const id = await newSession(worktree);
      if (id === undefined) throw new Error(error());
      return id;
    },
    showColumns: async (paths) => {
      for (const path of [props.workspace.root, ...paths]) if (!pinned().includes(path)) setPinnedList(await core.setPinned(path, true));
      chooseView("columns");
      await new Promise((resolve) => setTimeout(resolve, 1500)); // (each column shown and streaming)
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

  // A session you're not looking at (another Tab is visible, or the window is in the background)
  // raises an OS notification when it needs you (spec story 25).
  const onStateChanged = (id: SessionId, state: SessionState) => {
    const { name, state: previous } = sessions[id]; // read before the store updates
    setSessions(id, "state", state);
    onBenchState(id, state);
    const unseen = !onScreen(id) || boardOpen() || !document.hasFocus();
    if (!unseen) return;
    if (state === "needsYou") void notify(id, `${name} needs you`, "The Agent is waiting for your answer.");
    else if (settings()?.settings.notifications.turnFinished && previous === "working" && state === "idle")
      void notify(id, `${name} finished`, "The Agent's turn is done.");
  };

  let sawWorktreesEvent = false;
  let sawSettingsEvent = false;
  // Solid only ties an onCleanup to this view if it's registered before anything is awaited, and
  // the listeners below arrive after awaits: this one cleanup removes them all. Without it, a
  // Workspace switched away from would keep handling events (and hide the new view's Tab).
  const stops: (() => void)[] = [];
  let disposed = false;
  onCleanup(() => {
    disposed = true;
    stops.splice(0).forEach((stop) => stop());
  });
  /** Removed with the view (at once, if the view went while it was being added). */
  const untilGone = (stop: () => void) => (disposed ? stop() : void stops.push(stop));
  onMount(async () => {
    const unlisten = await core.onEvent((event) => {
      if (disposed) return; // (one already on its way when the view went)
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
        if (status.kind === "done" && mainView() === "tabs" && activeWorktree() === worktree && activeId() === null) void show(status.sessionId);
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
        event.kind === "availableCommandsChanged" ||
        event.kind === "gitStatusChanged"
      ) {
        return; // (the Manual editors', permission cards', Edit note chips', Git drawer's and composer's business)
      } else if (!sessions[event.sessionId]) return;
      else if (event.kind === "sessionStateChanged") onStateChanged(event.sessionId, event.state);
      else if (event.kind === "permissionModeChanged") setSessions(event.sessionId, "permissionMode", event.mode);
      else setSessions(event.sessionId, "unread", event.unread);
    });
    untilGone(unlisten);
    // Clicking a notification opens the session it was about.
    const unlistenClicks = await onNotificationClicked((id) => sessions[id] && (boardOpen() || !onScreen(id)) && void show(id));
    untilGone(unlistenClicks);
    // Y/N answer the oldest open permission card anywhere in the Tab (outside text fields).
    const onKey = (e: KeyboardEvent) => {
      // (Ctrl+P is never the browser's print, even with a dialog open.)
      if (e.ctrlKey && !e.shiftKey && e.key.toLowerCase() === "p") {
        e.preventDefault();
        if (!removing() && !creatingWorktree() && !switching()) setPaletteOpen(true);
        return;
      }
      if (e.ctrlKey && e.shiftKey && e.key.toLowerCase() === "o") {
        e.preventDefault();
        if (!removing() && !creatingWorktree() && !paletteOpen()) setSwitching(true);
        return;
      }
      if (removing() || creatingWorktree() || paletteOpen() || switching()) return; // a dialog is open over the Tab
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
      // (Ctrl+B is the editor's own when it takes it.)
      if (e.ctrlKey && !e.shiftKey && e.key.toLowerCase() === "b" && !e.defaultPrevented) {
        e.preventDefault();
        if (!e.repeat) setBoardOpen((open) => !open);
        return;
      }
      if (boardOpen()) {
        if (e.key === "Escape") setBoardOpen(false);
        return; // (Y/N answer the Tab's card, which isn't showing)
      }
      if (e.key === "Escape") setRecentOpen(false);
      // Alt+arrows: ←/→ the column on the left / right (Columns view), ↑/↓ the previous / next session.
      if (e.altKey && !e.ctrlKey && !e.shiftKey && e.key.startsWith("Arrow") && !e.defaultPrevented) {
        e.preventDefault();
        return void altArrow(e.key);
      }
      // Y/N answer the oldest open card of the Tab on screen (in the Columns view: the focused column's).
      const shown = mainView() === "columns" ? columnViews.get(activeWorktree()) : tabs;
      const id = shown?.shown();
      const s = id === null || id === undefined ? undefined : sessions[id];
      if (s?.state === "needsYou" && shown && answerByKey(e, s.id, shown.items)) e.preventDefault();
    };
    window.addEventListener("keydown", onKey);
    untilGone(() => window.removeEventListener("keydown", onKey));
    // Narrower than the Columns view allows: back to the Tabs.
    const onResize = () => {
      setWide(window.innerWidth >= COLUMNS_MIN_WIDTH);
      if (!wide() && mainView() === "columns") chooseView("tabs");
    };
    window.addEventListener("resize", onResize);
    untilGone(() => window.removeEventListener("resize", onResize));
    // The Recent menu closes on a click anywhere else.
    const onClick = (e: MouseEvent) => {
      if ((e.target as Element | null)?.closest?.(".recent-menu, .menu")) return;
      setRecentOpen(false);
      setAddColumnAt(null);
    };
    window.addEventListener("click", onClick);
    untilGone(() => window.removeEventListener("click", onClick));
    // Worktrees may have changed while the editor was in the background (the watcher covers the rest),
    // and they're fetched if it's due (the core keeps it to every 5 minutes at most).
    const onFocus = () => void core.windowFocused().catch(() => {});
    window.addEventListener("focus", onFocus);
    untilGone(() => window.removeEventListener("focus", onFocus));
    const loaded = await core.settings();
    if (disposed) return;
    if (!sawSettingsEvent) setSettings(loaded);
    // Setups started before this view (e.g. the webview reloaded); events since then win.
    for (const setup of await core.setups()) if (!setups[setup.worktree]) setSetups(setup.worktree, setup);
    const snapshot = await core.worktrees();
    if (!sawWorktreesEvent) setWorktrees(snapshot);
    setPinnedList(await core.pinnedWorktrees());
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

  /** Alt+←/→ move focus between columns; Alt+↑/↓ show the previous / next session of the focused
   *  column (or the Tabs view's Worktree). */
  const altArrow = (key: string) => {
    const path = activeWorktree();
    if (key === "ArrowLeft" || key === "ArrowRight") {
      if (mainView() !== "columns") return;
      const row = pinnedRow();
      const at = row.findIndex((w) => w.path === path);
      const next = row[at + (key === "ArrowLeft" ? -1 : 1)];
      if (next) setActiveWorktree(next.path);
      return;
    }
    const here = sessionsIn(path);
    const at = here.findIndex((s) => s.id === (mainView() === "columns" ? shownIn(path) : activeId()));
    const next = here[at + (key === "ArrowUp" ? -1 : 1)];
    if (next) void show(next.id);
  };

  /** The drawer head's "Pin to this Worktree" (Columns view only). */
  const drawerPinControl = () =>
    mainView() === "columns"
      ? { pinned: drawerPin() !== null, onToggle: () => setDrawerPin((now) => (now === null ? activeWorktree() : null)) }
      : undefined;

  const sharing = () => {
    const s = session();
    return s ? order().filter((id) => sessions[id].worktree === s.worktree).length : 0;
  };

  return (
    <div class="workspace" classList={{ "board-open": view() === "board" }}>
      {/* The window's title bar (no native one): drag it by any bare part; double-click maximises. */}
      <header class="titlebar" ref={titlebar} data-tauri-drag-region>
        <button class="name switch-workspace" onClick={() => setSwitching(true)} title="Switch repository (Ctrl+Shift+O)">
          {props.workspace.name}
          <ChevronDown />
        </button>
        <span class="path" title={props.workspace.root} data-tauri-drag-region>
          {props.workspace.root}
        </span>
        <span class="grow" data-tauri-drag-region />
        <div class="segmented">
          <button classList={{ on: view() === "tabs" }} onClick={() => chooseView("tabs")} title="One session at a time">
            Tabs
          </button>
          <button
            classList={{ on: view() === "columns" }}
            onClick={() => chooseView("columns")}
            disabled={!wide()}
            title={wide() ? "A column per pinned Worktree, side by side (Alt+←/→ move between them)" : "Columns needs a window at least 1 600 px wide"}
          >
            Columns
          </button>
          <button classList={{ on: view() === "board" }} onClick={() => setBoardOpen(true)} title="Every session by state (Ctrl+B)">
            Board
          </button>
        </div>
        <Show when={mainView() === "columns"}>
          <span class="recent-menu">
            <button
              class="add-worktree"
              onClick={(e) => {
                const r = e.currentTarget.getBoundingClientRect();
                setAddColumnAt((now) => (now ? null : { left: r.left, top: r.bottom + 4 }));
              }}
              title="Give another Worktree a column"
            >
              <Plus />
              column
            </button>
            <Show when={addColumnAt()}>
              {(at) => (
                <div class="menu" style={{ left: `${at().left}px`, top: `${at().top}px` }}>
                  <For each={rowWorktrees().filter((w) => !w.removed && !pinned().includes(w.path))}>
                    {(w) => (
                      <button
                        class="menu-item"
                        style={{ "--c": worktreeColour(w.path) }}
                        onClick={() => {
                          setAddColumnAt(null);
                          void pin(w.path);
                        }}
                      >
                        <GitBranch class="wt-glyph" />
                        <span class="mono">{worktreeLabel(w)}</span>
                        <Show when={sessionsIn(w.path).some((s) => s.state === "needsYou")}>
                          <span class="badge needs">{sessionsIn(w.path).filter((s) => s.state === "needsYou").length}</span>
                        </Show>
                      </button>
                    )}
                  </For>
                  <div class="menu-sep" />
                  <button
                    class="menu-item"
                    onClick={() => {
                      setAddColumnAt(null);
                      setCreatingWorktree(true);
                    }}
                  >
                    <Plus />
                    New Worktree…
                  </button>
                </div>
              )}
            </Show>
          </span>
        </Show>
        <Show when={needYou() > 0}>
          <button class="needs-you-button" onClick={() => setBoardOpen(true)} title="See them on the Board">
            {needYou()} need{needYou() === 1 ? "s" : ""} you
          </button>
        </Show>
        <button class="search-button" onClick={() => !removing() && !creatingWorktree() && setPaletteOpen(true)} title="Go to a file in this Worktree, or start a session">
          <Search />
          Search files
          <kbd>Ctrl+P</kbd>
        </button>
        <button
          class="ghost icon"
          classList={{ on: drawer() !== null }}
          onClick={() => (drawer() ? setDrawer(null) : toggleDrawer(lastDrawer))}
          title="Files and Git of this Worktree (Ctrl+Shift+E, Ctrl+Shift+G)"
          aria-label="Files and Git drawer"
        >
          <PanelRight />
        </button>
        <button class="ghost icon" onClick={openRepoSettings} title="Settings for this repo, e.g. its Worktree setup commands" aria-label="Repo settings">
          <Settings />
        </button>
        <WindowControls />
      </header>
      <Show when={view() === "board"}>
        <Board
          sessions={order().map((id) => sessions[id])}
          worktrees={rowWorktrees()}
          only={boardOnly()}
          onOnly={setBoardOnly}
          top={titlebar.offsetHeight}
          banners={
            <>
              <Show when={error()}>
                <Banner tone="error" onDismiss={() => setError("")}>
                  {error()}
                </Banner>
              </Show>
              <Show when={notice()}>
                <Banner tone="info" onDismiss={() => setNotice("")}>
                  {notice()}
                </Banner>
              </Show>
            </>
          }
          onOpen={(id) => {
            setBoardOpen(false);
            if (mainView() === "columns" || id !== activeId()) void show(id);
          }}
        />
      </Show>
      {/* (The Columns view has no Worktree row: its column headers say which Worktree is where.) */}
      <Show when={mainView() === "tabs"}>
        <WorktreeRow
          worktrees={rowWorktrees()}
          active={activeWorktree()}
          sessionsIn={sessionsIn}
          settingUp={(path) => !!setups[path] && setups[path].status.kind !== "done"}
          onSelect={selectWorktree}
          onNewSession={(path) => void newSession(path)}
          onNewWorktree={() => setCreatingWorktree(true)}
        />
      </Show>
      <Show when={creatingWorktree()}>
        <NewWorktreeDialog
          activeBranch={worktree()?.branch ?? null}
          existing={creatingFrom()}
          onCreated={async (created) => {
            // The Worktree exists now: close, and start its session in the main view, where a
            // failure leaves the (empty) Worktree selected with "＋ session" to retry. With a
            // setup, show it running; the core starts the session once it's done. In the Columns
            // view it gets a column first.
            setCreatingWorktree(false);
            setCreatingFrom(undefined);
            setNotice(created.warning ?? "");
            if (mainView() === "columns") await pin(created.worktree.path);
            const setup = created.setup;
            if (!setup) return void newSession(created.worktree.path);
            // Events may have got here first: keep their status and output.
            updateSetup(setup.worktree, (s) => (s.commands = setup.commands));
            selectWorktree(setup.worktree);
          }}
          onGoToWorktree={(path) => {
            setCreatingWorktree(false);
            setCreatingFrom(undefined);
            selectWorktree(path);
          }}
          onClose={() => {
            setCreatingWorktree(false);
            setCreatingFrom(undefined);
          }}
        />
      </Show>
      <Show when={mainView() === "tabs"}>
      <nav class="tabs" style={{ "--c": worktreeColour(activeWorktree()) }}>
        <For each={sessionsIn(activeWorktree()).map((s) => s.id)}>
          {(id) => (
            <span class={`tab-wrap ${id === activeId() ? "active" : ""}`}>
              <button
                class="tab"
                onClick={() => id !== activeId() && void show(id)}
                onAuxClick={(e) => e.button === 1 && void closeTab(id)}
              >
                <Sparkles />
                <StateDot state={sessions[id].state} title={STATE_LABEL[sessions[id].state]} />
                <span class="name">{sessions[id].name}</span>
                <Show when={id === activeId()} fallback={<Show when={sessions[id].unread}>{(n) => <span class="badge">{n()}</span>}</Show>}>
                  <span class="sep">·</span>
                  <span class={`state ${sessions[id].state}`}>{STATE_LABEL[sessions[id].state]}</span>
                </Show>
              </button>
              <button
                class="close-tab"
                aria-label={`Close ${sessions[id].name}`}
                title="Close (it stays in Recent sessions; Ctrl+Shift+T reopens)"
                onClick={() => void closeTab(id)}
              >
                <X />
              </button>
            </span>
          )}
        </For>
        <button class="ghost add-tab" onClick={() => void newSession()} title="New Agent session in this Worktree" disabled={worktree()?.removed || settingUp()}>
          <Plus />
          session
        </button>
        <Show when={recent[activeWorktree()]?.length}>
          <span class="recent-menu">
            <button
              class="ghost"
              onClick={(e) => {
                const r = e.currentTarget.getBoundingClientRect();
                setRecentAt({ left: r.left, top: r.bottom + 4 });
                setRecentOpen((open) => !open);
              }}
              title="Closed sessions in this Worktree"
            >
              Recent
              <ChevronDown />
            </button>
            <Show when={recentOpen()}>
              <div class="menu" style={{ left: `${recentAt().left}px`, top: `${recentAt().top}px` }}>
                <For each={recent[activeWorktree()] ?? []}>
                  {(r) => (
                    <button class="menu-item" onClick={() => void reopen(core.reopenSession(r.acpId))} title="Reopen with its conversation">
                      <Sparkles />
                      {r.name}
                    </button>
                  )}
                </For>
              </div>
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
      </Show>
      <Show when={switching()}>
        <WorkspacePicker
          overlay={{ current: props.workspace.root, onCancel: () => setSwitching(false) }}
          open={switchTo}
          onOpened={props.onSwitched}
        />
      </Show>
      <Show when={leaving()}>
        {(leave) => (
          <div class="modal-backdrop leave-workspace" onClick={(e) => e.target === e.currentTarget && leave().answer(null)}>
            <div class="modal" role="dialog" aria-label="Switch repository">
              <div class="modal-head">
                <b>Switch repository?</b>
              </div>
              <div class="modal-body">
                <Show when={leave().busy.length}>
                  <p>
                    {leave().busy.length === 1 ? "1 Agent session is working" : `${leave().busy.length} Agent sessions are working`} ({leave().busy.join(", ")}).
                    Switching stops them; they can be resumed when you come back.
                  </p>
                </Show>
                <Show when={leave().unsaved.length}>
                  <p>
                    Unsaved changes in <span class="mono">{leave().unsaved.map((d) => d.path.split(/[\\/]/).pop()).join(", ")}</span>
                    <Show when={leave().unsaved.some((d) => d.window !== "main")}> (some are in popped-out windows: save them there first, or they're discarded)</Show>.
                  </p>
                </Show>
              </div>
              <div class="modal-foot">
                <button class="ghost" onClick={() => leave().answer(null)}>
                  Cancel
                </button>
                <Show
                  when={leave().unsaved.length}
                  fallback={
                    <button class="primary" onClick={() => leave().answer("discard")}>
                      Switch
                    </button>
                  }
                >
                  <button onClick={() => leave().answer("discard")}>Discard and switch</button>
                  <Show when={leave().unsaved.every((d) => d.window === "main")}>
                    <button class="primary" onClick={() => leave().answer("save")}>
                      Save and switch
                    </button>
                  </Show>
                </Show>
              </div>
            </div>
          </div>
        )}
      </Show>
      <Show when={paletteOpen()}>
        <CommandPalette
          worktree={activeWorktree()}
          label={worktree() ? worktreeLabel(worktree()!) : ""}
          colour={worktreeColour(activeWorktree())}
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
              // In the Columns view, focus moves to another column rather than leaving the view.
              const other = mainView() === "columns" ? pinnedRow().find((p) => p.path !== path) : undefined;
              if (other) setActiveWorktree(other.path);
              else selectWorktree(props.workspace.root);
            }}
            onClose={() => setRemoving(null)}
          />
        )}
      </Show>
      <Show when={mainView() === "tabs" && sharing() > 1}>
        <Banner tone="warn">{sharing()} Agent sessions share this Worktree, so they can edit the same files.</Banner>
      </Show>
      <Show when={notice()}>
        <Banner tone="info" onDismiss={() => setNotice("")}>
          {notice()}
        </Banner>
      </Show>
      <Show when={settings()?.error}>
        {(e) => (
          <Banner
            tone="error"
            action={
              <button class="link" onClick={openRepoSettings}>
                Open settings
              </button>
            }
          >
            Settings not applied: {e()}
          </Banner>
        )}
      </Show>
      <Show when={error()}>
        <Banner tone="error" onDismiss={() => setError("")}>
          {error()}
        </Banner>
      </Show>
      <div class="main-row">
      <Show when={mainView() === "columns"}>
        <Columns
          worktrees={pinnedRow()}
          unpinned={rowWorktrees().filter((w) => !w.removed && !pinned().includes(w.path))}
          onPin={(path) => void pin(path)}
          onUnpin={unpin}
          focused={activeWorktree()}
          sessionsIn={sessionsIn}
          shownIn={shownIn}
          setupOf={(path) => setups[path]}
          recentIn={(path) => recent[path] ?? []}
          register={(path, v) => (v ? columnViews.set(path, v) : columnViews.delete(path))}
          onFocus={setActiveWorktree}
          onShow={(path, id) => {
            setLastSessionIn(path, id);
            setActiveWorktree(path);
          }}
          onNewSession={(path) => void newSession(path)}
          onCloseTab={(id) => void closeTab(id)}
          onReopen={(r) => void reopen(core.reopenSession(r.acpId))}
          onSuspend={(s) => core.suspendSession(s.id).catch((err) => setError(String(err)))}
          onResume={(s) => core.resumeSession(s.id).catch((err) => setError(String(err)))}
          onRemove={setRemoving}
          onOpenSettings={openRepoSettings}
          onOpenSnippet={(code, label) => openInEditor({ kind: "snippet", code, label })}
          onError={setError}
        />
      </Show>
      <Show when={mainView() === "tabs"}>
      <div class="main-col">
      <Show
        when={session()}
        keyed
        fallback={
          <Show
            when={setups[activeWorktree()]?.status.kind !== "done" && setups[activeWorktree()]}
            fallback={
              <div class="center empty-worktree" style={{ "--c": worktreeColour(activeWorktree()) }}>
                <GitBranch />
                <p>No Agent sessions in this Worktree yet.</p>
                <Show when={!worktree()?.removed}>
                  <button class="primary" onClick={() => void newSession()}>
                    <Sparkles />
                    New Agent session here
                  </button>
                </Show>
                <Show when={recent[activeWorktree()]?.length}>
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
            <Transcript
              sessionId={s.id}
              items={tabs.items}
              start={tabs.start()}
              onLoadEarlier={tabs.loadEarlier}
              onError={setError}
              onOpenSnippet={(code, label) => openInEditor({ kind: "snippet", code, label })}
              bottomInset={composerHeight}
            />
            <Composer session={s} onHeight={setComposerHeight} />
          </>
        )}
      </Show>
      </div>
      </Show>
      <Show when={editorOpen()}>
        <ManualEditor
          requests={editorRequests()}
          controls={(c) => (editorControls = c)}
          vim={settings()?.settings.editor?.vim ?? false}
          onEmpty={() => setEditorOpen(false)}
          onError={setError}
        />
      </Show>
      <Show when={filesOpen()}>
        <FilesDrawer
          worktree={drawerWorktree()}
          revision={filesRevision[drawerWorktree()] ?? 0}
          reveal={drawerWorktree() === activeWorktree() ? revealed() : null}
          onOpenFile={(rel) => openWorktreeFile(rel, drawerWorktree())}
          label={drawerTab() ? worktreeLabel(drawerTab()!) : ""}
          colour={worktreeColour(drawerWorktree())}
          pin={drawerPinControl()}
          onGit={() => toggleDrawer("git")}
          onClose={() => setFilesOpen(false)}
        />
      </Show>
      <Show when={gitOpen()}>
        <GitDrawer
          worktree={drawerWorktree()}
          onOpenFile={(rel) => openWorktreeFile(rel, drawerWorktree())}
          onGoToWorktree={selectWorktree}
          onOpenDiff={(diff) =>
            openInEditor({
              kind: "diff",
              key: [diff.worktree, diff.file.path, diff.split].join("\n"),
              title: `${diff.file.path.split("/").pop()} vs ${diff.base}`,
              name: diff.file.path,
              path: diff.file.change === "deleted" ? null : worktreePath(diff.file.path, diff.worktree),
              base: diff.base,
              renamedFrom: diff.file.renamedFrom,
              lines: diff.lines,
            })
          }
          onNewWorktreeFrom={(branch) => {
            setCreatingFrom(branch);
            setCreatingWorktree(true);
          }}
          label={drawerTab() ? worktreeLabel(drawerTab()!) : ""}
          colour={worktreeColour(drawerWorktree())}
          pin={drawerPinControl()}
          onFiles={() => toggleDrawer("files")}
          onClose={() => setDrawer(null)}
        />
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


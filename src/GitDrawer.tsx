// The Git drawer (ticket 18): the active Worktree's branch, how it stands against upstream, and its
// changed files, live as Agents work. Whole files stage and unstage (or all at once); commit with a
// message, or amend the last commit; discard a file's changes after asking. The core asks before
// committing while a session there is mid-turn, and before amending a commit that's already pushed.
// The branch name opens a picker to switch branches (ticket 20: not while a session there is
// mid-turn; branches checked out elsewhere lead to their Worktree); a merge or rebase in progress
// shows a banner with its conflicted files and Abort.
import { createEffect, createMemo, createResource, createSignal, For, on, onCleanup, Show } from "solid-js";
import { core, type BaseChange, type BaseChanges, type BranchInfo, type CommitRequest, type DiffLine, type GitFile, type GitStatus } from "./core";
import { hasUnsavedChangesUnder } from "./documents";
import { DrawerHead } from "./FilesDrawer";
import { ArrowDown, ArrowUp, ChevronDown, CircleMinus, CirclePlus, GitBranch, RefreshCw, TriangleAlert, Undo2 } from "./icons";

/** A path as the rows show it: the file name, then its folder, muted. */
function splitPath(path: string) {
  const slash = path.lastIndexOf("/");
  return { name: path.slice(slash + 1), dir: slash < 0 ? "" : path.slice(0, slash + 1) };
}

/** What the drawer is asking before it goes on, and about which Worktree. */
type Ask = { worktree: string } & (
  | { kind: "discard"; file: GitFile }
  | { kind: "working"; sessions: string[]; request: CommitRequest }
  | { kind: "pushed"; request: CommitRequest }
  | { kind: "pullWorking"; sessions: string[] }
  | { kind: "switch"; branch: BranchInfo }
  | { kind: "abort"; operation: Operation }
);

type Operation = NonNullable<GitStatus["operation"]>;
const OPERATION: Record<Operation, { name: string; git: string }> = {
  merge: { name: "merge", git: "merge" },
  rebase: { name: "rebase", git: "rebase" },
  cherryPick: { name: "cherry-pick", git: "cherry-pick" },
  revert: { name: "revert", git: "revert" },
  am: { name: "patch apply (git am)", git: "am" },
};

const LETTER_KIND: Record<string, string> = { "?": "untracked", A: "added", D: "deleted", U: "deleted", R: "renamed" };

export function GitDrawer(props: {
  worktree: string;
  onOpenFile: (path: string) => void;
  onGoToWorktree: (path: string) => void;
  onNewWorktreeFrom: (branch: string) => void;
  /** A file's change vs the Base, to show in the Manual editor. */
  onOpenDiff: (diff: { worktree: string; file: BaseChange; base: string; split: string; lines: DiffLine[] }) => void;
  label: string;
  colour: string;
  onFiles: () => void;
  onClose: () => void;
}) {
  const [status, setStatus] = createSignal<GitStatus | null>(null);
  const [error, setError] = createSignal("");
  const [message, setMessage] = createSignal("");
  const [amend, setAmend] = createSignal(false);
  const [ask, setAsk] = createSignal<Ask | null>(null);
  /** The Worktree an operation is running in (another one shown meanwhile isn't held up by it). */
  const [busyIn, setBusyIn] = createSignal<string | null>(null);
  const busy = () => busyIn() === props.worktree;
  /** What the last push, fetch or pull said, under their buttons. */
  const [notice, setNotice] = createSignal<{ text: string; tone: "ok" | "warn" | "error" } | null>(null);

  /** Status requests in flight: only the latest one's answer is shown. */
  let asked = 0;
  const refresh = async () => {
    const n = ++asked;
    const worktree = props.worktree;
    try {
      const next = await core.gitStatus(worktree);
      if (n !== asked || worktree !== props.worktree) return;
      setStatus(next); // (an operation's error stays up: the status is read after every one)
    } catch (err) {
      if (n === asked) setError(String(err));
    }
  };
  createEffect(
    on(
      () => props.worktree,
      () => {
        setStatus(null);
        setAsk(null);
        setPicking(false);
        setChanges(null);
        setChangesError("");
        if (mode() === "base") void refreshChanges();
        setAmend(false);
        setError("");
        setNotice(null);
        void refresh();
      },
    ),
  );
  let stop: (() => void) | undefined;
  let alive = true;
  onCleanup(() => {
    alive = false;
    stop?.();
  });
  void core
    .onEvent((event) => {
      if (event.kind === "gitStatusChanged" && event.worktree === props.worktree) {
        void refresh();
        if (mode() === "base") void refreshChanges();
      }
    })
    .then((unlisten) => (alive ? (stop = unlisten) : unlisten()));

  // "Changes vs base" (ticket 21): what the branch changed since it split from its Base.
  const [mode, setMode] = createSignal<"status" | "base">("status");
  const [changes, setChanges] = createSignal<BaseChanges | null>(null);
  const [changesError, setChangesError] = createSignal("");
  let askedChanges = 0;
  const refreshChanges = async () => {
    const n = ++askedChanges;
    const worktree = props.worktree;
    try {
      const next = await core.changesVsBase(worktree);
      if (n !== askedChanges || worktree !== props.worktree) return;
      setChanges(next);
      setChangesError("");
    } catch (err) {
      if (n !== askedChanges || worktree !== props.worktree) return;
      setChanges(null); // (not a list for a Base that no longer stands)
      setChangesError(String(err));
    }
  };
  createEffect(on(mode, () => mode() === "base" && void refreshChanges(), { defer: true }));
  /** Sets the Worktree's Base (empty: the default); a Base that isn't there leaves the list as it was. */
  const changeBase = async (typed: string) => {
    const worktree = props.worktree;
    try {
      await core.setBase(worktree, typed.trim() === "" ? null : typed.trim());
      if (worktree === props.worktree) void refreshChanges();
    } catch (err) {
      if (worktree === props.worktree) setChangesError(String(err));
    }
  };
  /** A file's diff, from the split the list was made at. */
  const openDiff = async (file: BaseChange) => {
    const worktree = props.worktree;
    const list = changes();
    if (!list) return;
    try {
      const lines = await core.diffVsBase(worktree, list.split, file);
      if (worktree === props.worktree) props.onOpenDiff({ worktree, file, base: list.base, split: list.split, lines });
    } catch (err) {
      if (worktree === props.worktree) setChangesError(String(err));
    }
  };

  // The branch picker: every branch, fetched first (as the New Worktree dialog lists them).
  const [picking, setPicking] = createSignal(false);
  const [pickFilter, setPickFilter] = createSignal("");
  const [branchList] = createResource(picking, () =>
    core.branches().catch((err) => ({ branches: [] as BranchInfo[], warning: `Couldn't list the branches: ${err}` })),
  );
  const picks = createMemo(() => {
    const words = pickFilter().toLowerCase();
    return (branchList()?.branches ?? []).filter((b) => b.name.toLowerCase().includes(words));
  });
  /** The local branch a row comes to (a remote one's local branch of the same name, if any). */
  const localOf = (branch: BranchInfo) => {
    if (!branch.remote) return branch;
    const name = branch.name.slice(branch.name.indexOf("/") + 1);
    return branchList()?.branches.find((b) => !b.remote && b.name === name) ?? null;
  };
  /** Where `branch` (or its local branch) is checked out, if that's another Worktree. */
  const elsewhere = (branch: BranchInfo) => {
    const at = localOf(branch)?.checkedOutIn;
    return at && at !== props.worktree ? at : null;
  };
  const current = (branch: BranchInfo) => {
    const local = localOf(branch);
    return !!local && local.name === status()?.branch;
  };
  /** What a switch said when it couldn't (shown in the picker). */
  const [pickError, setPickError] = createSignal("");
  const switchTo = (branch: BranchInfo) =>
    run(async (worktree) => {
      setAsk(null);
      setPickError("");
      try {
        await core.switchBranch(worktree, branch.name);
        setPicking(false);
      } catch (err) {
        if (worktree === props.worktree) setPickError(String(err));
      }
    });
  const togglePicker = () => {
    setPickFilter("");
    setPickError("");
    setPicking(!picking());
  };

  /** Runs a git operation in the shown Worktree, then shows the status after it. A remote one's
   *  failure goes under the Fetch/Pull/Push buttons; others' by the commit box. What comes back
   *  after another Worktree is shown is dropped. */
  const run = async (operation: (worktree: string) => Promise<unknown>, remote = false) => {
    const worktree = props.worktree;
    if (busyIn() !== null) return;
    setBusyIn(worktree);
    setError("");
    setNotice(null);
    try {
      await operation(worktree);
    } catch (err) {
      if (worktree !== props.worktree) return;
      if (remote) setNotice({ text: String(err), tone: "error" });
      else setError(String(err));
    } finally {
      setBusyIn(null);
      if (worktree === props.worktree) void refresh();
    }
  };

  const asking = <K extends Ask["kind"]>(kind: K) => {
    const a = ask();
    return a?.kind === kind ? (a as Extract<Ask, { kind: K }>) : undefined;
  };
  const staged = () => status()?.files.filter((f) => f.staged) ?? [];
  const unstaged = () => status()?.files.filter((f) => f.unstaged) ?? [];

  /** Commits in `worktree` (the one asked about, even if another is shown by the time it's confirmed). */
  const commit = (request: CommitRequest, worktree = props.worktree) =>
    run(async () => {
      setAsk(null);
      const outcome = await core.gitCommit(worktree, request);
      if (worktree !== props.worktree) return; // (switched away meanwhile: nothing to ask here)
      if (outcome.kind === "sessionsWorking") setAsk({ kind: "working", sessions: outcome.sessions, request, worktree });
      else if (outcome.kind === "alreadyPushed") setAsk({ kind: "pushed", request, worktree });
      else {
        setMessage("");
        setAmend(false);
      }
    });

  const commits = (n: number) => `${n} commit${n === 1 ? "" : "s"}`;
  const pull = (evenIfWorking = false) =>
    run(async (worktree) => {
      setAsk(null);
      const outcome = await core.gitPull(worktree, evenIfWorking);
      if (worktree !== props.worktree) return;
      if (outcome.kind === "sessionsWorking") setAsk({ kind: "pullWorking", sessions: outcome.sessions, worktree });
      else if (outcome.kind === "upToDate") setNotice({ text: "Already up to date.", tone: "ok" });
      else if (outcome.kind === "fastForwarded") setNotice({ text: `Pulled ${commits(outcome.commits)}.`, tone: "ok" });
      else
        setNotice({
          text: `This branch and its upstream have diverged (${commits(outcome.ahead)} here, ${commits(outcome.behind)} there). Pulling would take a merge or a rebase, which the editor leaves to you: ask an Agent, or do it in a terminal.`,
          tone: "warn",
        });
    }, true);
  const push = () =>
    run(async (worktree) => {
      const outcome = await core.gitPush(worktree);
      if (worktree !== props.worktree) return;
      if (outcome.kind === "pushed") setNotice({ text: `Pushed to ${outcome.to}.`, tone: "ok" });
      else
        setNotice({
          text: "The remote has commits this branch hasn't, so it turned the push down. Pull first; if the branch has diverged, ask an Agent to merge or rebase, or do it in a terminal.",
          tone: "warn",
        });
    }, true);
  const fetch = () =>
    run(async (worktree) => {
      await core.gitFetch(worktree);
      if (worktree === props.worktree) setNotice({ text: "Fetched.", tone: "ok" });
    }, true);

  /** A file in this Worktree has unsaved changes in some Manual editor. */
  const unsavedHere = () => hasUnsavedChangesUnder(props.worktree);

  const canCommit = () => !busy() && (amend() || (staged().length > 0 && message().trim() !== ""));
  const deleted = (file: GitFile) => file.staged === "D" || file.unstaged === "D";

  const row = (file: GitFile, side: "staged" | "unstaged") => {
    const letter = side === "staged" ? file.staged! : file.unstaged!;
    const { name, dir } = splitPath(file.path);
    return (
      <div class="git-file">
        <span class={`change change-${LETTER_KIND[letter] ?? "modified"}`}>{letter === "?" ? "U" : letter}</span>
        <button
          class="tree-name"
          disabled={deleted(file)}
          onClick={() => props.onOpenFile(file.path)}
          title={deleted(file) ? `${file.path} (deleted)` : file.renamedFrom ? `${file.renamedFrom} → ${file.path}` : file.path}
        >
          {name}
          <span class="dir">{dir}</span>
        </button>
        {/* (A conflict is resolved, not discarded: that's the conflict banner's business.) */}
        <Show when={!file.conflicted}>
          <button
            class="ghost row-action"
            title="Discard these changes"
            disabled={busy()}
            onClick={() => setAsk({ kind: "discard", file, worktree: props.worktree })}
          >
            <Undo2 />
          </button>
        </Show>
        <button
          class="ghost row-action"
          title={side === "staged" ? "Unstage" : "Stage"}
          disabled={busy()}
          onClick={() => run((worktree) => (side === "staged" ? core.gitUnstage : core.gitStage)(worktree, [file.path]))}
        >
          {side === "staged" ? <CircleMinus /> : <CirclePlus />}
        </button>
      </div>
    );
  };

  return (
    <aside class="files-drawer git-drawer">
      <DrawerHead label={props.label} colour={props.colour} tab="git" onTab={(tab) => tab === "files" && props.onFiles()} onClose={props.onClose} />
      <Show when={status()} fallback={<p class="muted center small">{error() || "Reading the status…"}</p>}>
        {(s) => (
          <>
            <div class="git-top" style={{ "--c": props.colour }}>
              <button class="branch-pick" classList={{ on: picking() }} onClick={togglePicker} title="Switch this Worktree to another branch">
                <GitBranch />
                {s().branch ?? "detached HEAD"}
                <span class="counts">
                  <Show when={s().upstream} fallback="no upstream">
                    <Show when={s().ahead !== null} fallback={`${s().upstream} is gone`}>
                      ↑{s().ahead} ↓{s().behind}
                    </Show>
                  </Show>
                </span>
                <ChevronDown class="chev" />
              </button>
              <div class="git-sync">
                <button disabled={busy()} onClick={() => void fetch()} title="Fetch from the remote">
                  <RefreshCw class={busy() ? "spin" : undefined} />
                  Fetch
                </button>
                <button
                  disabled={busy() || s().ahead === null}
                  onClick={() => void pull()}
                  title={s().ahead === null ? "Nothing to pull from: no upstream (or it's gone)" : "Fast-forward to the upstream (never merges or rebases)"}
                >
                  <ArrowDown />
                  Pull
                </button>
                <button disabled={busy() || !s().branch} onClick={() => void push()} title={`Push ${s().branch ?? ""} to the branch of its name on the remote`}>
                  <ArrowUp />
                  Push
                </button>
                <Show when={notice()?.tone === "ok"}>
                  <span class="note">{notice()!.text}</span>
                </Show>
              </div>
            </div>
            <Show when={notice()?.tone !== "ok" ? notice() : null}>
              {(n) => (
                <p class={`git-notice ${n().tone}`}>
                  <TriangleAlert />
                  {n().text}
                </p>
              )}
            </Show>
            <Show when={s().operation}>
              {(operation) => (
                <div class="git-operation">
                  <span>
                    <b>A {OPERATION[operation()].name} is in progress</b>
                    {s().files.some((f) => f.conflicted) ? ", with conflicts to resolve:" : "."}
                  </span>
                  <For each={s().files.filter((f) => f.conflicted)}>
                    {(file) => (
                      <button class="ghost mono conflicted-file" onClick={() => props.onOpenFile(file.path)} title="Open it to resolve">
                        {file.path}
                      </button>
                    )}
                  </For>
                  <div class="actions">
                    <button
                      class="danger"
                      disabled={busy()}
                      onClick={() => setAsk({ kind: "abort", operation: operation(), worktree: props.worktree })}
                      title={`git ${OPERATION[operation()].git} --abort: back to how it was before`}
                    >
                      Abort
                    </button>
                  </div>
                </div>
              )}
            </Show>
            <Show when={picking()}>
              <div
                class="branch-picker"
                onKeyDown={(e) => {
                  if (e.key === "Escape") {
                    e.stopPropagation();
                    setPicking(false);
                  }
                }}
              >
                <Show when={s().midTurn.length > 0}>
                  <p class="warning small">
                    {s().midTurn.join(", ")} {s().midTurn.length === 1 ? "is" : "are"} in the middle of a turn here: switch branches once{" "}
                    {s().midTurn.length === 1 ? "it's" : "they're"} done.
                  </p>
                </Show>
                <Show when={s().operation}>
                  {(operation) => <p class="warning small">Finish or abort the {OPERATION[operation()].name} first.</p>}
                </Show>
                <Show when={pickError()}>
                  <p class="error small">{pickError()}</p>
                </Show>
                <input placeholder="Filter branches" value={pickFilter()} onInput={(e) => setPickFilter(e.currentTarget.value)} autofocus />
                <Show when={branchList()?.warning}>{(w) => <p class="warning small">{w()}</p>}</Show>
                <div class="branch-list">
                  <Show when={!branchList.loading} fallback={<p class="muted">Fetching branches…</p>}>
                    <For each={picks()} fallback={<p class="muted">No matching branches.</p>}>
                      {(b) => (
                        <div class="branch-row" classList={{ taken: !!elsewhere(b) }}>
                          <button
                            class="branch"
                            disabled={busy() || current(b) || !!elsewhere(b) || s().midTurn.length > 0 || !!s().operation}
                            onClick={() => setAsk({ kind: "switch", branch: b, worktree: props.worktree })}
                            title={elsewhere(b) ? `Checked out in ${elsewhere(b)}` : current(b) ? "The branch this Worktree is on" : `Switch to ${b.name}`}
                          >
                            <span class="mono">{b.name}</span>
                            <Show when={b.remote}>
                              <span class="muted small">remote</span>
                            </Show>
                            <Show when={current(b)}>
                              <span class="muted small">current</span>
                            </Show>
                          </button>
                          <Show
                            when={elsewhere(b)}
                            fallback={
                              <Show when={!current(b)}>
                                <button class="ghost small" onClick={() => props.onNewWorktreeFrom(b.name)} title="A new Worktree on this branch, with its own session">
                                  New Worktree from this branch
                                </button>
                              </Show>
                            }
                          >
                            {(path) => (
                              <button class="ghost small" onClick={() => props.onGoToWorktree(path())}>
                                Go to that Worktree
                              </button>
                            )}
                          </Show>
                        </div>
                      )}
                    </For>
                  </Show>
                </div>
              </div>
            </Show>
            <div class="git-mode">
              <div class="segmented full">
                <button classList={{ on: mode() === "status" }} onClick={() => setMode("status")}>
                  Working changes
                </button>
                <button classList={{ on: mode() === "base" }} onClick={() => setMode("base")} title="What this branch changed since it split from its Base">
                  Changes vs base
                </button>
              </div>
            </div>
            <Show when={mode() === "base"}>
              <div class="drawer-tree">
                <form
                  class="git-base"
                  onSubmit={(e) => {
                    e.preventDefault();
                    void changeBase(new FormData(e.currentTarget).get("base")?.toString() ?? "");
                  }}
                >
                  <label class="grow">
                    Base
                    <input name="base" class="mono" value={changes()?.base ?? ""} placeholder="origin/<default>" spellcheck={false} />
                  </label>
                  <button class="ghost" type="submit" title="Compare this Worktree against this branch, tag or commit from now on">
                    Compare
                  </button>
                  <Show when={changes() && !changes()!.isDefault}>
                    <button class="ghost" type="button" onClick={() => void changeBase("")} title="Back to origin/<default>">
                      Use the default
                    </button>
                  </Show>
                </form>
                <Show when={changesError()}>
                  <p class="error small git-base-error">{changesError()}</p>
                </Show>
                <Show when={changes()} fallback={<p class="muted center">{changesError() ? "" : "Comparing…"}</p>}>
                  {(c) => (
                    <>
                      <div class="git-section">
                        Changed since {c().base}
                        <span class="badge">{c().files.length}</span>
                      </div>
                      <For each={c().files} fallback={<p class="muted center small">Nothing changed on this branch yet.</p>}>
                        {(file) => (
                          <div class="git-file">
                            <span class={`change change-${file.change}`}>{{ added: "A", modified: "M", deleted: "D", renamed: "R" }[file.change]}</span>
                            <button
                              class="tree-name"
                              onClick={() => void openDiff(file)}
                              title={file.renamedFrom ? `${file.renamedFrom} → ${file.path}: see the diff` : `${file.path}: see the diff`}
                            >
                              <Show when={file.renamedFrom} fallback={splitPath(file.path).name}>
                                {(from) => `${splitPath(from()).name} → ${splitPath(file.path).name}`}
                              </Show>
                              <span class="dir">{splitPath(file.path).dir}</span>
                            </button>
                          </div>
                        )}
                      </For>
                    </>
                  )}
                </Show>
              </div>
            </Show>
            <div class="drawer-tree" classList={{ hidden: mode() === "base" }}>
              <div class="git-section">
                Staged
                <span class="badge">{staged().length}</span>
                <Show when={staged().length > 0}>
                  <button class="link" disabled={busy()} onClick={() => run((worktree) => core.gitUnstageAll(worktree))} title="Unstage everything">
                    Unstage all
                  </button>
                </Show>
              </div>
              <For each={staged()}>{(file) => row(file, "staged")}</For>
              <div class="git-section">
                Changes
                <span class="badge">{unstaged().length}</span>
                <Show when={unstaged().length > 0}>
                  <button class="link" disabled={busy()} onClick={() => run((worktree) => core.gitStageAll(worktree))} title="Stage everything">
                    Stage all
                  </button>
                </Show>
              </div>
              <For each={unstaged()}>{(file) => row(file, "unstaged")}</For>
              <Show when={s().files.length === 0}>
                <p class="muted center small">Nothing to commit.</p>
              </Show>
            </div>
          </>
        )}
      </Show>
      <Show when={ask()}>
        <div
          class="git-ask"
          onKeyDown={(e) => {
            if (e.key === "Escape") {
              e.stopPropagation();
              setAsk(null);
            }
          }}
        >
          <Show when={asking("discard")}>
            {(d) => (
              <>
                <span>
                  Throw away {d().file.staged && d().file.unstaged ? "every change, staged or not, to " : "the changes to "}
                  <span class="mono">{d().file.path}</span>? This can't be undone.
                </span>
                <div class="actions">
                  <button
                    class="danger"
                    ref={(el) => queueMicrotask(() => el.focus())}
                    onClick={() => void run(async () => (setAsk(null), core.gitDiscard(d().worktree, d().file.path)))}
                  >
                    {d().file.unstaged === "?" || d().file.staged === "A" ? "Delete the file" : "Discard changes"}
                  </button>
                  <button class="ghost" onClick={() => setAsk(null)}>
                    Cancel
                  </button>
                </div>
              </>
            )}
          </Show>
          <Show when={asking("working")}>
            {(w) => (
              <>
                <span>
                  {w().sessions.join(", ")} {w().sessions.length === 1 ? "is" : "are"} in the middle of a turn in this Worktree and may
                  still change files. Commit anyway?
                </span>
                <div class="actions">
                  <button class="primary" ref={(el) => queueMicrotask(() => el.focus())} onClick={() => void commit({ ...w().request, evenIfWorking: true }, w().worktree)}>
                    Commit anyway
                  </button>
                  <button class="ghost" onClick={() => setAsk(null)}>
                    Cancel
                  </button>
                </div>
              </>
            )}
          </Show>
          <Show when={asking("switch")}>
            {(sw) => (
              <>
                <span>
                  Switch this Worktree to <span class="mono">{sw().branch.name}</span>? Its sessions stay with it and will see that
                  branch's files. Uncommitted changes come along if git can carry them.
                  <Show when={unsavedHere()}>
                    {" "}
                    Files with unsaved changes in the editor will ask which version to keep.
                  </Show>
                </span>
                <div class="actions">
                  <button class="primary" ref={(el) => queueMicrotask(() => el.focus())} onClick={() => void switchTo(sw().branch)}>
                    Switch
                  </button>
                  <button class="ghost" onClick={() => setAsk(null)}>
                    Cancel
                  </button>
                </div>
              </>
            )}
          </Show>
          <Show when={asking("abort")}>
            {(a) => (
              <>
                <span>
                  Abort the {OPERATION[a().operation].name}? Files go back to how they were before it started; conflicts already resolved
                  are lost.
                </span>
                <div class="actions">
                  <button class="danger" ref={(el) => queueMicrotask(() => el.focus())} onClick={() => void run(async () => (setAsk(null), core.abortOperation(a().worktree)))}>
                    Abort the {OPERATION[a().operation].name}
                  </button>
                  <button class="ghost" onClick={() => setAsk(null)}>
                    Cancel
                  </button>
                </div>
              </>
            )}
          </Show>
          <Show when={asking("pullWorking")}>
            {(w) => (
              <>
                <span>
                  {w().sessions.join(", ")} {w().sessions.length === 1 ? "is" : "are"} in the middle of a turn in this Worktree, and
                  pulling changes files under {w().sessions.length === 1 ? "it" : "them"}. Pull anyway?
                </span>
                <div class="actions">
                  <button class="primary" ref={(el) => queueMicrotask(() => el.focus())} onClick={() => void pull(true)}>
                    Pull anyway
                  </button>
                  <button class="ghost" onClick={() => setAsk(null)}>
                    Cancel
                  </button>
                </div>
              </>
            )}
          </Show>
          <Show when={asking("pushed")}>
            {(p) => (
              <>
                <span>The last commit is already pushed. Amending it rewrites it, and you'd have to force-push. Amend anyway?</span>
                <div class="actions">
                  <button class="danger" ref={(el) => queueMicrotask(() => el.focus())} onClick={() => void commit({ ...p().request, evenIfPushed: true }, p().worktree)}>
                    Amend anyway
                  </button>
                  <button class="ghost" onClick={() => setAsk(null)}>
                    Cancel
                  </button>
                </div>
              </>
            )}
          </Show>
        </div>
      </Show>
      <div class="git-commit" classList={{ hidden: mode() === "base" }}>
        <Show when={error() && status()}>
          <p class="error">{error()}</p>
        </Show>
        <textarea
          value={message()}
          onInput={(e) => setMessage(e.currentTarget.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter" && e.ctrlKey && canCommit()) {
              e.preventDefault();
              void commit({ message: message(), amend: amend() });
            }
          }}
          placeholder={amend() ? "New message (empty keeps the last one's)" : "Commit message (Ctrl+Enter to commit)"}
        />
        <label title={status()?.lastCommit ? `${status()!.lastCommit!.id} ${status()!.lastCommit!.subject}` : undefined}>
          <input type="checkbox" checked={amend()} disabled={!status()?.lastCommit} onChange={(e) => setAmend(e.currentTarget.checked)} />
          Amend last commit
        </label>
        <button class="primary" disabled={!canCommit()} onClick={() => void commit({ message: message(), amend: amend() })} title="Ctrl+Enter">
          {amend() ? "Amend" : "Commit"}
          <Show when={!amend() && staged().length}>
            <span class="badge accent">{staged().length}</span>
          </Show>
        </button>
      </div>
    </aside>
  );
}

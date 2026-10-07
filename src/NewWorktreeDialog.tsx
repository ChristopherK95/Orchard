// "＋ worktree" (ticket 07): a new branch (prefilled agent/task-N, starting from origin/<default>
// after a fetch, or from any other branch: the default first, then every local and remote one) or
// an existing branch, created next to the repo. Enter accepts the
// defaults. The dialog hands the new Worktree back as soon as it exists; the caller starts the
// Agent session (so a failure there doesn't leave the dialog stuck on an already-created branch).
import { createMemo, createResource, createSignal, For, onCleanup, onMount, Show } from "solid-js";
import { type BranchInfo, core, type CreatedWorktree, type NewWorktree } from "./core";
import { GitBranch, X } from "./icons";

/** "Start from": "" for the default (`origin/<default>`), a branch's name, or `OTHER`. */
const OTHER = "\0other";

export function NewWorktreeDialog(props: {
  /** Opened to check out this existing branch ("New Worktree from this branch"). */
  existing?: string;
  onCreated: (created: CreatedWorktree) => void;
  onGoToWorktree: (path: string) => void;
  onClose: () => void;
}) {
  const [mode, setMode] = createSignal<"new" | "existing">(props.existing ? "existing" : "new");
  const [name, setName] = createSignal("");
  const [typed, setTyped] = createSignal(false);
  const [startChoice, setStartChoice] = createSignal("");
  const [otherStart, setOtherStart] = createSignal("");
  const [filter, setFilter] = createSignal(props.existing ?? "");
  const [busy, setBusy] = createSignal("");
  const [error, setError] = createSignal("");
  const [taken, setTaken] = createSignal<BranchInfo | null>(null);
  const [defaultStart] = createResource(() => core.defaultStartPoint());
  // Fetching for the branch list can take a moment: "Start from" offers the default until it's in.
  const [branchList] = createResource(() => core.branches());
  // The suggested name, awaited by Enter if it hasn't arrived yet.
  const suggestion = core.suggestBranchName();
  let nameInput!: HTMLInputElement;

  // Registered before anything is awaited, so Solid ties the cleanup to this dialog.
  const onKey = (e: KeyboardEvent) => e.key === "Escape" && !busy() && props.onClose();
  window.addEventListener("keydown", onKey);
  onCleanup(() => window.removeEventListener("keydown", onKey));

  onMount(() => {
    nameInput?.focus(); // (not there when it opens on an existing branch)
    void suggestion
      .then((suggested) => {
        if (typed()) return; // don't overwrite what the user started typing
        setName(suggested);
        nameInput?.select(); // (not there when it opened on an existing branch)
      })
      .catch((err) => setError(String(err)));
  });

  /** Every branch but the default, its local twin (`master` for `origin/master`) first. */
  const startBranches = createMemo(() => {
    const def = defaultStart();
    const twin = def?.replace(/^origin\//, "");
    // (A listing that failed leaves just the default and "a tag or commit".)
    const listed = branchList.state === "ready" ? branchList().branches : [];
    const names = listed.map((b) => b.name).filter((n) => n !== def);
    return [...names.filter((n) => n === twin), ...names.filter((n) => n !== twin)];
  });

  const shown = createMemo(() => {
    const words = filter().toLowerCase();
    return (branchList()?.branches ?? []).filter((b) => b.name.toLowerCase().includes(words));
  });

  const create = async (spec: NewWorktree) => {
    if (busy()) return;
    setError("");
    setTaken(null);
    setBusy("Fetching origin and creating…");
    try {
      props.onCreated(await core.createWorktree(spec));
    } catch (err) {
      setError(String(err));
      setBusy("");
    }
  };

  const createNew = async (e: Event) => {
    e.preventDefault();
    const branch = name().trim() || (typed() ? "" : await suggestion.catch(() => ""));
    if (!branch) return setError("Give the branch a name.");
    const startPoint = startChoice() === "" ? null : startChoice() === OTHER ? otherStart().trim() || null : startChoice();
    if (startChoice() === OTHER && !startPoint) return setError("Name the branch, tag or commit to start from.");
    void create({ kind: "newBranch", name: branch, startPoint });
  };

  const pick = (b: BranchInfo) => {
    if (b.checkedOutIn) {
      setError("");
      setTaken(b); // explain why it can't be used, and offer to go there instead
    } else void create({ kind: "existingBranch", name: b.name });
  };

  return (
    <div class="modal-backdrop" onClick={(e) => e.target === e.currentTarget && !busy() && props.onClose()}>
      <div class="modal" role="dialog" aria-label="New Worktree">
        <div class="modal-head">
          <b>New Worktree</b>
          <div class="segmented">
            <button classList={{ on: mode() === "new" }} onClick={() => setMode("new")} disabled={!!busy()}>
              New branch
            </button>
            <button classList={{ on: mode() === "existing" }} onClick={() => setMode("existing")} disabled={!!busy()}>
              Existing branch
            </button>
          </div>
          <button class="ghost icon" onClick={() => props.onClose()} disabled={!!busy()} aria-label="Close" title="Close (Esc)">
            <X />
          </button>
        </div>

        <Show when={mode() === "new"}>
          <form class="modal-form" onSubmit={(e) => void createNew(e)}>
            <div class="modal-body">
            <label>
              Branch
              <input
                ref={nameInput}
                class="mono"
                value={name()}
                placeholder="agent/…"
                onInput={(e) => {
                  setTyped(true);
                  setName(e.currentTarget.value);
                }}
                spellcheck={false}
              />
            </label>
            <p class="hint">Prefilled and selected, so typing replaces it.</p>
            <label>
              Start from
              <select class="mono" value={startChoice()} onChange={(e) => setStartChoice(e.currentTarget.value)}>
                <option value="">
                  {defaultStart() ?? "origin/<default>"}
                  {defaultStart()?.startsWith("origin/") ? " (default, fetched first)" : " (default)"}
                </option>
                <For each={startBranches()}>{(b) => <option value={b}>{b}</option>}</For>
                <Show when={branchList.loading}>
                  <option disabled>Fetching branches…</option>
                </Show>
                <option value={OTHER}>A tag or commit…</option>
              </select>
            </label>
            <Show when={startChoice() === OTHER}>
              <input class="mono" placeholder="tag or commit" value={otherStart()} onInput={(e) => setOtherStart(e.currentTarget.value)} autofocus />
            </Show>
            <p class="hint">Created next to the repo, then an Agent session starts in it.</p>
            </div>
            <div class="modal-foot">
              <button type="button" onClick={() => props.onClose()} disabled={!!busy()}>
                Cancel <kbd>Esc</kbd>
              </button>
              <button type="submit" class="primary" disabled={!!busy()}>
                {busy() || "Create and start session"} <kbd>Enter</kbd>
              </button>
            </div>
          </form>
        </Show>

        <Show when={mode() === "existing"}>
          <div class="modal-body">
            <input placeholder="Filter branches" value={filter()} onInput={(e) => setFilter(e.currentTarget.value)} autofocus />
            <Show when={branchList()?.warning}>{(w) => <p class="warning small">{w()}</p>}</Show>
            <div class="branch-list">
              <Show when={!branchList.loading} fallback={<p class="muted">Fetching branches…</p>}>
                <For each={shown()} fallback={<p class="muted">No matching branches.</p>}>
                  {(b) => (
                    <button class="branch" classList={{ taken: !!b.checkedOutIn }} disabled={!!busy()} onClick={() => pick(b)}>
                      <GitBranch />
                      <span class="mono">{b.name}</span>
                      <Show when={b.remote}>
                        <span class="muted small">remote</span>
                      </Show>
                      <span class="grow" />
                      <span class="muted small">{b.checkedOutIn ? "checked out" : "check out"}</span>
                    </button>
                  )}
                </For>
              </Show>
            </div>
            <Show when={taken()}>
              {(b) => (
                <div class="taken-note">
                  <span>
                    <span class="mono">{b().name}</span> is already checked out in <span class="mono">{b().checkedOutIn}</span>, and
                    a branch can only be checked out in one Worktree.
                  </span>
                  <button onClick={() => props.onGoToWorktree(b().checkedOutIn!)}>Go to that Worktree</button>
                </div>
              )}
            </Show>
            <Show when={busy()}>
              <p class="muted">{busy()}</p>
            </Show>
          </div>
        </Show>

        <Show when={error()}>
          <p class="error modal-error">{error()}</p>
        </Show>
      </div>
    </div>
  );
}

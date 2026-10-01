// "＋ worktree" (ticket 07): a new branch (prefilled agent/task-N, base origin/<default> after a
// fetch, or another base) or an existing branch, created next to the repo. Enter accepts the
// defaults; the caller starts an Agent session in the new Worktree.
import { createMemo, createResource, createSignal, For, onCleanup, onMount, Show } from "solid-js";
import { type BranchInfo, core, type NewWorktree, type WorktreeInfo } from "./core";

type BaseChoice = "default" | "active" | "other";

export function NewWorktreeDialog(props: {
  /** The active Worktree's branch, offered as a base. */
  activeBranch: string | null;
  onCreated: (worktree: WorktreeInfo) => Promise<void>;
  onGoToWorktree: (path: string) => void;
  onClose: () => void;
}) {
  const [mode, setMode] = createSignal<"new" | "existing">("new");
  const [name, setName] = createSignal("");
  const [baseChoice, setBaseChoice] = createSignal<BaseChoice>("default");
  const [otherBase, setOtherBase] = createSignal("");
  const [filter, setFilter] = createSignal("");
  const [busy, setBusy] = createSignal("");
  const [error, setError] = createSignal("");
  const [defaultBase] = createResource(() => core.defaultBase());
  // Fetching for the branch list can take a moment; only start it when that mode is chosen.
  const [branches] = createResource(() => mode() === "existing", () => core.branches());
  let nameInput!: HTMLInputElement;

  onMount(async () => {
    setName(await core.suggestBranchName());
    nameInput.select();
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && !busy() && props.onClose();
    window.addEventListener("keydown", onKey);
    onCleanup(() => window.removeEventListener("keydown", onKey));
  });

  const shown = createMemo(() => {
    const words = filter().toLowerCase();
    return (branches() ?? []).filter((b) => b.name.toLowerCase().includes(words));
  });

  const create = async (spec: NewWorktree) => {
    if (busy()) return;
    setError("");
    setBusy(spec.kind === "newBranch" && spec.base === null ? "Fetching and creating…" : "Creating…");
    try {
      await props.onCreated(await core.createWorktree(spec));
      props.onClose();
    } catch (err) {
      setError(String(err));
    } finally {
      setBusy("");
    }
  };

  const createNew = (e: Event) => {
    e.preventDefault();
    const branch = name().trim();
    if (!branch) return setError("Give the branch a name.");
    const base = baseChoice() === "default" ? null : baseChoice() === "active" ? props.activeBranch : otherBase().trim() || null;
    if (baseChoice() === "other" && !base) return setError("Name the branch or commit to start from.");
    void create({ kind: "newBranch", name: branch, base });
  };

  const pick = (b: BranchInfo) =>
    b.checkedOutIn ? props.onGoToWorktree(b.checkedOutIn) : void create({ kind: "existingBranch", name: b.name });

  return (
    <div class="modal-backdrop" onClick={(e) => e.target === e.currentTarget && !busy() && props.onClose()}>
      <div class="modal" role="dialog" aria-label="New Worktree">
        <div class="modal-head">
          <b>New Worktree</b>
          <span class="grow" />
          <div class="segmented">
            <button classList={{ on: mode() === "new" }} onClick={() => setMode("new")}>
              New branch
            </button>
            <button classList={{ on: mode() === "existing" }} onClick={() => setMode("existing")}>
              Existing branch
            </button>
          </div>
        </div>

        <Show when={mode() === "new"}>
          <form class="modal-body" onSubmit={createNew}>
            <label>
              Branch
              <input ref={nameInput} class="mono" value={name()} onInput={(e) => setName(e.currentTarget.value)} spellcheck={false} />
            </label>
            <label>
              Start from
              <select value={baseChoice()} onChange={(e) => setBaseChoice(e.currentTarget.value as BaseChoice)}>
                <option value="default">{defaultBase() ?? "origin/<default>"} (fetched first)</option>
                <Show when={props.activeBranch}>{(b) => <option value="active">{b()} (this Worktree)</option>}</Show>
                <option value="other">Another branch or commit…</option>
              </select>
            </label>
            <Show when={baseChoice() === "other"}>
              <input class="mono" placeholder="branch, tag or commit" value={otherBase()} onInput={(e) => setOtherBase(e.currentTarget.value)} autofocus />
            </Show>
            <p class="muted small">Created next to the repo, then an Agent session starts in it.</p>
            <div class="modal-actions">
              <button type="button" onClick={() => props.onClose()} disabled={!!busy()}>
                Cancel
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
            <div class="branch-list">
              <Show when={!branches.loading} fallback={<p class="muted">Fetching branches…</p>}>
                <For each={shown()} fallback={<p class="muted">No matching branches.</p>}>
                  {(b) => (
                    <button class="branch" classList={{ taken: !!b.checkedOutIn }} disabled={!!busy()} onClick={() => pick(b)}>
                      <span class="mono">{b.name}</span>
                      <Show when={b.remote}>
                        <span class="muted small">remote</span>
                      </Show>
                      <span class="grow" />
                      <Show when={b.checkedOutIn} fallback={<span class="muted small">check out</span>}>
                        <span class="small">checked out · Go to that Worktree</span>
                      </Show>
                    </button>
                  )}
                </For>
              </Show>
            </div>
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

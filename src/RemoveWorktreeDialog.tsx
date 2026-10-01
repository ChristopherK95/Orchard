// Removing a Worktree (ticket 09): says what removal stops (its Agent sessions) and loses
// (uncommitted changes, commits nothing else has) before anything happens. Losing work takes an
// explicit "Discard and remove"; a clean Worktree needs one confirmation.
import { createResource, createSignal, For, onCleanup, onMount, Show } from "solid-js";
import { core, type SessionId } from "./core";

/** How many changed files the dialog lists before "and N more". */
const SHOWN = 20;

export function RemoveWorktreeDialog(props: {
  worktree: string;
  label: string;
  sessionName: (id: SessionId) => string;
  onRemoved: (warning: string | null) => void;
  onClose: () => void;
}) {
  const [check, { refetch }] = createResource(() => core.removalCheck(props.worktree));
  // Starts ticked only when the branch is merged into its Base; the user's choice wins after that.
  const [deleteChoice, setDeleteChoice] = createSignal<boolean | null>(null);
  const deleteBranch = () => !!check()?.branch && (deleteChoice() ?? check()!.merged);
  const [busy, setBusy] = createSignal(false);
  const [error, setError] = createSignal("");

  const needsDiscard = () => {
    const c = check();
    return !!c && (c.discardToRemove || (deleteBranch() && c.discardToDeleteBranch));
  };

  const remove = async () => {
    if (busy() || !check()) return;
    setBusy(true);
    setError("");
    try {
      const discard = needsDiscard() ? check()!.fingerprint : null;
      const removed = await core.removeWorktree(props.worktree, { discard, deleteBranch: deleteBranch() });
      props.onRemoved(removed.warning);
    } catch (err) {
      setError(String(err));
      setBusy(false);
      // Sessions may have been stopped and work changed meanwhile: show what's true now.
      void refetch();
    }
  };

  const onKey = (e: KeyboardEvent) => {
    if (e.key === "Escape" && !busy()) props.onClose();
    // Enter confirms only a removal that loses nothing; discarding work takes a deliberate click.
    // (On a focused button, Enter is that button's own click.)
    else if (e.key === "Enter" && !needsDiscard() && check() && !(e.target instanceof HTMLButtonElement)) {
      e.preventDefault();
      void remove();
    }
  };
  let dialog!: HTMLDivElement;
  // Off the "Remove Worktree…" button behind, so keys act on the dialog.
  onMount(() => dialog.focus());
  window.addEventListener("keydown", onKey);
  onCleanup(() => window.removeEventListener("keydown", onKey));

  return (
    <div class="modal-backdrop" onClick={(e) => e.target === e.currentTarget && !busy() && props.onClose()}>
      <div class="modal" role="dialog" aria-label="Remove Worktree" tabIndex={-1} ref={dialog}>
        <div class="modal-head">
          <b>
            Remove Worktree <span class="mono">{props.label}</span>
          </b>
        </div>
        <div class="modal-body">
          <span class="mono muted small">{props.worktree}</span>
          <Show when={check.error}>
            <p class="error">{String(check.error)}</p>
          </Show>
          <Show when={check()} fallback={<Show when={!check.error}><p class="muted">Checking for work that would be lost…</p></Show>}>
            {(c) => (
              <>
                <Show when={c().sessions.length}>
                  <p>
                    Stops {c().sessions.length === 1 ? "its Agent session" : `its ${c().sessions.length} Agent sessions`}:{" "}
                    {c().sessions.map(props.sessionName).join(", ")}.
                  </p>
                </Show>
                <Show when={c().changedCount}>
                  <div class="loss">
                    <b class="warning">
                      {c().changedCount === 1 ? "1 uncommitted change" : `${c().changedCount} uncommitted changes`} will be lost
                    </b>
                    <ul class="mono small">
                      <For each={c().changes.slice(0, SHOWN)}>{(line) => <li>{line}</li>}</For>
                      <Show when={c().changedCount > SHOWN}>
                        <li class="muted">and {c().changedCount - SHOWN} more</li>
                      </Show>
                    </ul>
                  </div>
                </Show>
                <Show when={c().unpushedCount}>
                  <div class="loss">
                    <b classList={{ warning: c().discardToRemove || deleteBranch() }}>
                      {c().unpushedCount === 1 ? "1 commit" : `${c().unpushedCount} commits`} not pushed or merged into{" "}
                      <span class="mono">{c().base}</span>
                    </b>
                    <ul class="mono small">
                      <For each={c().unpushed.slice(0, SHOWN)}>
                        {(commit) => (
                          <li>
                            <span class="muted">{commit.id}</span> {commit.subject}
                          </li>
                        )}
                      </For>
                      <Show when={c().unpushedCount > SHOWN}>
                        <li class="muted">and {c().unpushedCount - SHOWN} more</li>
                      </Show>
                    </ul>
                    <p class="muted small">
                      {c().branch
                        ? deleteBranch()
                          ? "Deleting the branch loses them."
                          : `They stay on ${c().branch}.`
                        : "HEAD is detached, so no branch keeps them: they'll be lost."}
                    </p>
                  </div>
                </Show>
                <Show when={c().ignoredCount}>
                  <p class="muted small">
                    Also deletes ignored files:{" "}
                    <span class="mono">
                      {c().ignored.slice(0, 8).join(", ")}
                      {c().ignoredCount > 8 ? `, and ${c().ignoredCount - 8} more` : ""}
                    </span>
                  </p>
                </Show>
                <Show when={c().branch}>
                  {(branch) => (
                    <label class="check">
                      <input type="checkbox" checked={deleteBranch()} onChange={(e) => setDeleteChoice(e.currentTarget.checked)} disabled={busy()} />
                      Delete branch <span class="mono">{branch()}</span> too
                      <Show when={c().merged}>
                        <span class="muted small">
                          (merged into <span class="mono">{c().base}</span>)
                        </span>
                      </Show>
                    </label>
                  )}
                </Show>
              </>
            )}
          </Show>
          <div class="modal-actions">
            <button onClick={() => props.onClose()} disabled={busy()}>
              Cancel
            </button>
            <Show
              when={needsDiscard()}
              fallback={
                <button class="primary" onClick={() => void remove()} disabled={busy() || !check()}>
                  {busy() ? "Removing…" : "Remove"} <kbd>Enter</kbd>
                </button>
              }
            >
              <button class="danger" onClick={() => void remove()} disabled={busy()}>
                {busy() ? "Removing…" : "Discard and remove"}
              </button>
            </Show>
          </div>
        </div>
        <Show when={error()}>
          <p class="error modal-error">{error()}</p>
        </Show>
      </div>
    </div>
  );
}

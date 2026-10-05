// Removing a Worktree (ticket 09): says what removal stops (its Agent sessions) and loses
// (uncommitted changes, commits nothing else has) before anything happens. Losing work takes an
// explicit "Discard and remove"; a clean Worktree needs one confirmation.
import { createResource, createSignal, For, onCleanup, onMount, Show } from "solid-js";
import { core, type SessionId } from "./core";
import { TriangleAlert, X } from "./icons";

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
          <b>Remove Worktree {props.label}?</b>
          <button class="ghost icon" onClick={() => props.onClose()} disabled={busy()} aria-label="Close" title="Close (Esc)">
            <X />
          </button>
        </div>
        <div class="modal-body">
          <Show when={needsDiscard()} fallback={<span class="mono muted">{props.worktree}</span>}>
            <div class="callout danger">
              <TriangleAlert />
              <span>
                This deletes work that git cannot get back. Its folder <span class="mono">{props.worktree}</span> will be deleted.
              </span>
            </div>
          </Show>
          <Show when={check.error}>
            <p class="error">{String(check.error)}</p>
          </Show>
          <Show when={check()} fallback={<Show when={!check.error}><p class="muted">Checking for work that would be lost…</p></Show>}>
            {(c) => (
              <>
                <Show when={c().sessions.length}>
                  <div class="loss sessions">
                    <div class="section-label">
                      {c().sessions.length === 1 ? "1 Agent session is running here and will be stopped" : `${c().sessions.length} Agent sessions are running here and will be stopped`}
                    </div>
                    <ul>
                      <For each={c().sessions}>{(id) => <li>{props.sessionName(id)}</li>}</For>
                    </ul>
                  </div>
                </Show>
                <Show when={c().changedCount}>
                  <div class="loss">
                    <div class="section-label">
                      Uncommitted changes <span class="badge">{c().changedCount}</span>
                    </div>
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
                    <div class="section-label">
                      {c().merged ? "Commits only this branch has" : `Not pushed or merged into ${c().base}`} <span class="badge">{c().unpushedCount}</span>
                    </div>
                    <ul>
                      <For each={c().unpushed.slice(0, SHOWN)}>
                        {(commit) => (
                          <li>
                            <span class="commit-id">{commit.id}</span> {commit.subject}
                          </li>
                        )}
                      </For>
                      <Show when={c().unpushedCount > SHOWN}>
                        <li class="muted">and {c().unpushedCount - SHOWN} more</li>
                      </Show>
                    </ul>
                    <p class="muted small">
                      {c().merged
                        ? `Their changes are already in ${c().mergedInto ?? c().base} (squashed or rebased), so deleting the branch loses nothing.`
                        : c().branch
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
                      <span>
                        Delete branch <span class="mono">{branch()}</span> too
                        <span class="sub">
                          {c().merged ? (c().mergedInto ? `merged into ${c().mergedInto}` : "nothing of its own") : `not merged into ${c().base}: leaving it unticked keeps the commits recoverable`}
                        </span>
                      </span>
                    </label>
                  )}
                </Show>
              </>
            )}
          </Show>
        </div>
        <div class="modal-foot">
          <button onClick={() => props.onClose()} disabled={busy()}>
            Cancel <kbd>Esc</kbd>
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
        <Show when={error()}>
          <p class="error modal-error">{error()}</p>
        </Show>
      </div>
    </div>
  );
}

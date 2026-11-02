// Where one of the user's PRs conflicts with the branch it goes into, from the Dashboard: each file
// that merging that branch in would leave conflicted (what git said about it on hover).
// An Agent session can be started on them, to merge the branch in, resolve them and commit (not push).
import { createResource, createSignal, For, Show } from "solid-js";
import { core, type MergeConflicts as Conflicts, type MyPullRequest } from "./core";
import { FileIcon, Loader, Sparkles, TriangleAlert, X } from "./icons";

export function MergeConflicts(props: {
  pr: MyPullRequest;
  load: () => Promise<Conflicts>;
  /** Starts an Agent session on the PR with this prompt (the Dashboard's row says how it goes). */
  onStart: (prompt: Promise<string>) => void;
  onClose: () => void;
}) {
  const [error, setError] = createSignal("");
  const [conflicts] = createResource(async () => {
    try {
      return await props.load();
    } catch (err) {
      setError(String(err));
      return null;
    }
  });
  const files = () => conflicts()?.files ?? [];
  const start = () => {
    const read = conflicts();
    if (!read || !read.files.length) return;
    props.onStart(core.mergeConflictPrompt(read));
    props.onClose();
  };

  return (
    <div class="modal-backdrop" onClick={(e) => e.target === e.currentTarget && props.onClose()}>
      <div class="modal code-rabbit" role="dialog" aria-label="Merge conflicts">
        <div class="modal-head code-rabbit-title">
          <div>
            <span class="code-rabbit-eyebrow">
              <TriangleAlert />
              <Show when={conflicts()} fallback="Conflicts with the branch it goes into">
                {(c) => (
                  <>
                    Conflicts with <span class="mono">{c().base}</span>
                  </>
                )}
              </Show>
            </span>
            <b>
              {props.pr.title} <span class="pr-number">#{props.pr.number}</span>
            </b>
          </div>
          <button class="ghost icon" onClick={() => props.onClose()} aria-label="Close">
            <X />
          </button>
        </div>
        <Show when={error()}>
          <p class="error small modal-error">{error()}</p>
        </Show>
        <div class="modal-body">
          <Show
            when={!conflicts.loading}
            fallback={
              <p class="dashboard-line muted">
                <Loader class="spin" />
                Fetching both branches…
              </p>
            }
          >
            <Show
              when={files().length}
              fallback={
                <Show when={conflicts()}>
                  {(c) => (
                    <p class="dashboard-line muted">
                      It merges cleanly with origin/{c().base} now: GitHub hasn't caught up yet
                    </p>
                  )}
                </Show>
              }
            >
              <ul class="merge-conflicts-list">
                <For each={files()}>
                  {(f) => (
                    <li class="mono" title={f.messages.join("\n")}>
                      <FileIcon />
                      {f.path}
                    </li>
                  )}
                </For>
              </ul>
            </Show>
          </Show>
        </div>
        <div class="modal-foot">
          <span class="hint">The Agent merges {conflicts()?.base ?? "it"} in and resolves them; you push</span>
          <button class="ghost" onClick={() => props.onClose()}>
            Cancel
          </button>
          <button class="primary" onClick={start} disabled={!files().length} title="A session on its branch, its composer filled with the conflicts (not sent)">
            <Sparkles />
            Start an Agent session
          </button>
        </div>
      </div>
    </div>
  );
}

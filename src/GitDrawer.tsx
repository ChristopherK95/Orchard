// The Git drawer (ticket 18): the active Worktree's branch, how it stands against upstream, and its
// changed files, live as Agents work. Whole files stage and unstage (or all at once); commit with a
// message, or amend the last commit; discard a file's changes after asking. The core asks before
// committing while a session there is mid-turn, and before amending a commit that's already pushed.
import { createEffect, createSignal, For, on, onCleanup, Show } from "solid-js";
import { core, type CommitRequest, type GitFile, type GitStatus } from "./core";

/** What the drawer is asking before it goes on, and about which Worktree. */
type Ask = { worktree: string } & (
  | { kind: "discard"; file: GitFile }
  | { kind: "working"; sessions: string[]; request: CommitRequest }
  | { kind: "pushed"; request: CommitRequest }
);

const LETTER_KIND: Record<string, string> = { "?": "added", A: "added", D: "deleted", U: "conflicted" };

export function GitDrawer(props: { worktree: string; onOpenFile: (path: string) => void; onClose: () => void }) {
  const [status, setStatus] = createSignal<GitStatus | null>(null);
  const [error, setError] = createSignal("");
  const [message, setMessage] = createSignal("");
  const [amend, setAmend] = createSignal(false);
  const [ask, setAsk] = createSignal<Ask | null>(null);
  const [busy, setBusy] = createSignal(false);

  /** Status requests in flight: only the latest one's answer is shown. */
  let asked = 0;
  const refresh = async () => {
    const n = ++asked;
    const worktree = props.worktree;
    try {
      const next = await core.gitStatus(worktree);
      if (n !== asked || worktree !== props.worktree) return;
      setStatus(next);
      setError("");
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
        setAmend(false);
        setError("");
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
    .onEvent((event) => event.kind === "gitStatusChanged" && event.worktree === props.worktree && void refresh())
    .then((unlisten) => (alive ? (stop = unlisten) : unlisten()));

  /** Runs a git operation, then shows the status after it. */
  const run = async (operation: () => Promise<unknown>) => {
    if (busy()) return;
    setBusy(true);
    setError("");
    try {
      await operation();
    } catch (err) {
      setError(String(err));
    } finally {
      setBusy(false);
      void refresh();
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

  const canCommit = () => !busy() && (amend() || (staged().length > 0 && message().trim() !== ""));
  const deleted = (file: GitFile) => file.staged === "D" || file.unstaged === "D";

  const row = (file: GitFile, side: "staged" | "unstaged") => {
    const letter = side === "staged" ? file.staged! : file.unstaged!;
    return (
      <div class="git-file">
        <button
          class="tree-name"
          disabled={deleted(file)}
          onClick={() => props.onOpenFile(file.path)}
          title={deleted(file) ? `${file.path} (deleted)` : file.renamedFrom ? `${file.renamedFrom} → ${file.path}` : file.path}
        >
          {file.path}
        </button>
        <span class={`change change-${LETTER_KIND[letter] ?? "modified"}`}>{letter}</span>
        {/* (A conflict is resolved, not discarded: that's the conflict banner's business.) */}
        <Show when={!file.conflicted}>
          <button
            class="ghost row-action"
            title="Discard these changes"
            disabled={busy()}
            onClick={() => setAsk({ kind: "discard", file, worktree: props.worktree })}
          >
            ↺
          </button>
        </Show>
        <button
          class="ghost row-action"
          title={side === "staged" ? "Unstage" : "Stage"}
          disabled={busy()}
          onClick={() => run(() => (side === "staged" ? core.gitUnstage : core.gitStage)(props.worktree, [file.path]))}
        >
          {side === "staged" ? "−" : "+"}
        </button>
      </div>
    );
  };

  return (
    <aside class="files-drawer git-drawer">
      <div class="drawer-head">
        <b>Git</b>
        <span class="grow" />
        <button class="ghost" onClick={() => props.onClose()} title="Close (Ctrl+Shift+G)">
          ×
        </button>
      </div>
      <Show when={status()} fallback={<p class="muted center">{error() || "Reading the status…"}</p>}>
        {(s) => (
          <>
            <div class="git-branch">
              <b class="mono">{s().branch ?? "detached HEAD"}</b>
              <Show when={s().upstream} fallback={<span class="muted">no upstream</span>}>
                <Show when={s().ahead !== null} fallback={<span class="muted">{s().upstream} is gone</span>}>
                  <span class="muted" title={`Against ${s().upstream}`}>
                    ↑{s().ahead} ↓{s().behind}
                  </span>
                </Show>
              </Show>
            </div>
            <div class="drawer-tree">
              <div class="git-section">
                <span class="grow">Staged ({staged().length})</span>
                <Show when={staged().length > 0}>
                  <button class="ghost" disabled={busy()} onClick={() => run(() => core.gitUnstageAll(props.worktree))} title="Unstage everything">
                    Unstage all
                  </button>
                </Show>
              </div>
              <For each={staged()}>{(file) => row(file, "staged")}</For>
              <div class="git-section">
                <span class="grow">Changes ({unstaged().length})</span>
                <Show when={unstaged().length > 0}>
                  <button class="ghost" disabled={busy()} onClick={() => run(() => core.gitStageAll(props.worktree))} title="Stage everything">
                    Stage all
                  </button>
                </Show>
              </div>
              <For each={unstaged()}>{(file) => row(file, "unstaged")}</For>
              <Show when={s().files.length === 0}>
                <p class="muted center">Nothing to commit.</p>
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
      <div class="git-commit">
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
        <button class="primary" disabled={!canCommit()} onClick={() => void commit({ message: message(), amend: amend() })}>
          {amend() ? "Amend" : `Commit${staged().length ? ` ${staged().length} file${staged().length === 1 ? "" : "s"}` : ""}`}
        </button>
      </div>
    </aside>
  );
}

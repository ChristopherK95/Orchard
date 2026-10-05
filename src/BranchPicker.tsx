// Switching a Worktree's branch (ticket 20): the list of branches the Git drawer shows inline, and
// the same list as a popover from the branch name in a column's header, the Tabs view's breadcrumb
// or the sidebar's Worktree row. Not while a session there is mid-turn or a merge or rebase is in
// progress; branches checked out in another Worktree lead to it.
import { createMemo, createResource, createSignal, For, onCleanup, Show } from "solid-js";
import { core, type BranchInfo, type GitStatus } from "./core";
import { hasUnsavedChangesUnder } from "./documents";

const OPERATION_NAME: Record<NonNullable<GitStatus["operation"]>, string> = {
  merge: "merge",
  rebase: "rebase",
  cherryPick: "cherry-pick",
  revert: "revert",
  am: "patch apply (git am)",
};

/** Every branch (fetched first, as the New Worktree dialog lists them), filtered, each with what
 *  can be done with it from `worktree`. */
export function BranchList(props: {
  worktree: string;
  status: GitStatus;
  busy?: boolean;
  error?: string;
  onPick: (branch: BranchInfo) => void;
  onGoToWorktree: (path: string) => void;
  onNewWorktreeFrom: (branch: string) => void;
  onClose: () => void;
}) {
  const [filter, setFilter] = createSignal("");
  const [list] = createResource(() =>
    core.branches().catch((err) => ({ branches: [] as BranchInfo[], warning: `Couldn't list the branches: ${err}` })),
  );
  const picks = createMemo(() => {
    const words = filter().toLowerCase();
    return (list()?.branches ?? []).filter((b) => b.name.toLowerCase().includes(words));
  });
  /** The local branch a row comes to (a remote one's local branch of the same name, if any). */
  const localOf = (branch: BranchInfo) => {
    if (!branch.remote) return branch;
    const name = branch.name.slice(branch.name.indexOf("/") + 1);
    return list()?.branches.find((b) => !b.remote && b.name === name) ?? null;
  };
  /** Where `branch` (or its local branch) is checked out, if that's another Worktree. */
  const elsewhere = (branch: BranchInfo) => {
    const at = localOf(branch)?.checkedOutIn;
    return at && at !== props.worktree ? at : null;
  };
  const current = (branch: BranchInfo) => {
    const local = localOf(branch);
    return !!local && local.name === props.status.branch;
  };
  const midTurn = () => props.status.midTurn;
  return (
    <div
      class="branch-picker"
      onKeyDown={(e) => {
        if (e.key === "Escape") {
          e.stopPropagation();
          props.onClose();
        }
      }}
    >
      <Show when={midTurn().length > 0}>
        <p class="warning small">
          {midTurn().join(", ")} {midTurn().length === 1 ? "is" : "are"} in the middle of a turn here: switch branches once{" "}
          {midTurn().length === 1 ? "it's" : "they're"} done.
        </p>
      </Show>
      <Show when={props.status.operation}>{(operation) => <p class="warning small">Finish or abort the {OPERATION_NAME[operation()]} first.</p>}</Show>
      <Show when={props.error}>
        <p class="error small">{props.error}</p>
      </Show>
      <input placeholder="Filter branches" value={filter()} onInput={(e) => setFilter(e.currentTarget.value)} autofocus />
      <Show when={list()?.warning}>{(w) => <p class="warning small">{w()}</p>}</Show>
      <div class="branch-list">
        <Show when={!list.loading} fallback={<p class="muted">Fetching branches…</p>}>
          <For each={picks()} fallback={<p class="muted">No matching branches.</p>}>
            {(b) => (
              <div class="branch-row" classList={{ taken: !!elsewhere(b) }}>
                <button
                  class="branch"
                  disabled={props.busy || current(b) || !!elsewhere(b) || midTurn().length > 0 || !!props.status.operation}
                  onClick={() => props.onPick(b)}
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
  );
}

/** What the switch confirmation says (the drawer's and the popover's). */
export function SwitchQuestion(props: { branch: string; worktree: string }) {
  return (
    <span>
      Switch this Worktree to <span class="mono">{props.branch}</span>? Its sessions stay with it and will see that branch's files. Uncommitted
      changes come along if git can carry them.
      <Show when={hasUnsavedChangesUnder(props.worktree)}> Files with unsaved changes in the editor will ask which version to keep.</Show>
    </span>
  );
}

/** Where a branch popover opens: under the element that opened it. */
export type BranchPopover = { worktree: string; left: number; top: number };

/** The branch list as a popover, asking before it switches. Closes on a click outside or Esc. */
export function BranchSwitcher(props: {
  at: BranchPopover;
  onGoToWorktree: (path: string) => void;
  onNewWorktreeFrom: (branch: string) => void;
  onClose: () => void;
}) {
  const [status] = createResource(
    () => props.at.worktree,
    (w) => core.gitStatus(w),
  );
  const [asking, setAsking] = createSignal<BranchInfo | null>(null);
  const [busy, setBusy] = createSignal(false);
  const [error, setError] = createSignal("");
  const outside = (e: MouseEvent) => !(e.target as Element | null)?.closest?.(".branch-popover, .opens-branches") && props.onClose();
  window.addEventListener("mousedown", outside);
  onCleanup(() => window.removeEventListener("mousedown", outside));
  const switchTo = async (branch: BranchInfo) => {
    setBusy(true);
    setError("");
    try {
      await core.switchBranch(props.at.worktree, branch.name);
      props.onClose();
    } catch (err) {
      setError(String(err));
      setAsking(null);
    } finally {
      setBusy(false);
    }
  };
  return (
    <div class="menu branch-popover" style={{ left: `${Math.max(8, Math.min(props.at.left, window.innerWidth - 368))}px`, top: `${props.at.top}px` }}>
      <Show when={status()} fallback={<p class="muted small">{status.error ? `Couldn't read its status: ${status.error}` : "Reading the status…"}</p>}>
        {(s) => (
          <Show
            when={asking()}
            fallback={
              <BranchList
                worktree={props.at.worktree}
                status={s()}
                busy={busy()}
                error={error()}
                onPick={setAsking}
                onGoToWorktree={(path) => {
                  props.onClose();
                  props.onGoToWorktree(path);
                }}
                onNewWorktreeFrom={(branch) => {
                  props.onClose();
                  props.onNewWorktreeFrom(branch);
                }}
                onClose={props.onClose}
              />
            }
          >
            {(branch) => (
              <div
                class="branch-confirm"
                onKeyDown={(e) => {
                  if (e.key === "Escape") {
                    e.stopPropagation();
                    setAsking(null);
                  }
                }}
              >
                <SwitchQuestion branch={branch().name} worktree={props.at.worktree} />
                <div class="actions">
                  <button class="primary" disabled={busy()} ref={(el) => queueMicrotask(() => el.focus())} onClick={() => void switchTo(branch())}>
                    Switch
                  </button>
                  <button class="ghost" onClick={() => setAsking(null)}>
                    Cancel
                  </button>
                </div>
              </div>
            )}
          </Show>
        )}
      </Show>
    </div>
  );
}

/** Where a popover opened by `el` goes. */
export function popoverUnder(worktree: string, el: Element): BranchPopover {
  const r = el.getBoundingClientRect();
  return { worktree, left: r.left, top: r.bottom + 4 };
}

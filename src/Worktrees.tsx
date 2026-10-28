// What the Worktree switcher, the columns and the header share about Worktrees (their colours,
// labels and badges), and the context strip saying exactly where your next prompt goes.
import { Match, Show, Switch } from "solid-js";
import type { SessionInfo, WorktreeInfo } from "./core";
import { newCommits } from "./reviewWatch";
import { shellsIn } from "./shells";
import { ArrowDownUp, CirclePause, CirclePlay, FileDiff, Folder, MessageSquareCode, SquareTerminal, Trash2 } from "./icons";

/** The design's eight Worktree colours (wt-1 … wt-8): muted, so state colours stay louder. */
const PALETTE = ["#6f87c4", "#a287c2", "#5d9cab", "#b89a66", "#6aa87a", "#c07d86", "#bd845c", "#848a96"];

/** A Worktree as the switcher and the columns show it: one git lists, or one that's gone but still has sessions. */
export type WorktreeTab = WorktreeInfo & { removed?: boolean };

/** A Worktree that disappeared (e.g. removed in a terminal) while sessions still run in it. */
export const removedWorktree = (path: string): WorktreeTab => ({
  path, branch: null, head: "", isMain: false, ahead: null, behind: null, changed: 0, review: null, removed: true,
});

/** A stable colour per Worktree path. */
export function worktreeColour(path: string): string {
  let hash = 0;
  for (const c of path) hash = (hash * 31 + c.charCodeAt(0)) | 0;
  return PALETTE[Math.abs(hash) % PALETTE.length];
}

export function worktreeLabel(w: WorktreeTab): string {
  if (w.removed) return `${w.path.split(/[\\/]/).pop()} (removed)`;
  return w.branch ?? `detached ${w.head}`;
}

/** The Worktree's running shells, on its switcher card: how many, and whether one runs something.
 *  Clicking it (the card handles that) shows its Terminal panel. */
export function ShellBadge(props: { path: string }) {
  const shells = () => shellsIn(props.path);
  const title = () =>
    `Shells running here (click to show the terminal):\n${shells()
      .map((t) => `• ${t.name}${t.busy ? " (running)" : ""}`)
      .join("\n")}`;
  return (
    <Show when={shells().length > 0}>
      <span class="shell-badge" classList={{ busy: shells().some((t) => t.busy) }} title={title()}>
        <SquareTerminal />
        <Show when={shells().length > 1}>{shells().length}</Show>
      </span>
    </Show>
  );
}

/** A dot on a Review badge (and the review's Tab) while new commits wait for a Refresh. */
function NewCommitsDot(props: { number: number }) {
  const count = () => newCommits.of(props.number);
  return (
    <Show when={count() > 0}>
      <span class="new-commits-dot" title={`${count()} new commit${count() === 1 ? "" : "s"} pushed to the PR since: Refresh in its Code review`} />
    </Show>
  );
}

/** A Review Worktree's badge, with its PR's number. With `onOpen` it's a button opening its Code review. */
export function ReviewBadge(props: { number: number; onOpen?: (number: number) => void }) {
  const title = () => `A Review Worktree: the head of PR #${props.number}, read-only${props.onOpen ? ". Open its Code review" : ""}`;
  return (
    <Show
      when={props.onOpen}
      fallback={
        <span class="review-badge" title={title()}>
          <MessageSquareCode />
          Review #{props.number}
          <NewCommitsDot number={props.number} />
        </span>
      }
    >
      {(open) => (
        <button class="review-badge" onClick={() => open()(props.number)} title={title()}>
          <MessageSquareCode />
          Review #{props.number}
          <NewCommitsDot number={props.number} />
        </button>
      )}
    </Show>
  );
}

/** Under the header in the Tabs view: the Worktree's folder, how far it is from its upstream and
 *  its uncommitted changes; and what can be done to the session and the Worktree. */
export function ContextStrip(props: {
  worktree: WorktreeTab | undefined;
  session: SessionInfo | undefined;
  onRemove: (path: string) => void;
  onSuspend: (session: SessionInfo) => void;
  onResume: (session: SessionInfo) => void;
}) {
  return (
    <Show when={props.worktree}>
      {(w) => (
        <div class="context-strip">
          <span class="meta path-meta" title={w().path}>
            <Folder />
            <span class="mono ellipsis">{w().path}</span>
          </span>
          <Show when={!w().removed} fallback={<span class="meta gone">folder removed — sessions still run</span>}>
            <Show when={w().ahead !== null}>
              <span class="meta" title="Commits ahead of / behind its upstream">
                <ArrowDownUp />
                <span class="mono">
                  ↑{w().ahead} ↓{w().behind}
                </span>
              </span>
            </Show>
            <span class="meta">
              <FileDiff />
              {w().changed === 0 ? "no changes" : `${w().changed} changed`}
            </span>
          </Show>
          <span class="grow" />
          <Show when={props.session}>
            {(s) => (
              <Switch>
                <Match when={s().state === "idle"}>
                  <button class="ghost small" onClick={() => props.onSuspend(s())} title="Stop this session's Agent to free memory; sending a message resumes it">
                    <CirclePause />
                    Suspend
                  </button>
                </Match>
                <Match when={s().state === "exited"}>
                  <button class="ghost small" onClick={() => props.onResume(s())} title="Bring the Agent back with this conversation">
                    <CirclePlay />
                    Resume
                  </button>
                </Match>
              </Switch>
            )}
          </Show>
          <Show when={!w().isMain && !w().removed}>
            <button class="ghost small" onClick={() => props.onRemove(w().path)} title="Remove this Worktree (asks first)">
              <Trash2 />
              Remove Worktree…
            </button>
          </Show>
        </div>
      )}
    </Show>
  );
}

// The Worktree row and context bar (ticket 06, layout A1): Worktrees on top, the active Worktree's
// Agent sessions in the row below, and a bar saying exactly where your next prompt goes.
import { For, Show } from "solid-js";
import type { SessionInfo, SessionState, WorktreeInfo } from "./core";

const PALETTE = ["#7aa2f7", "#c49cf0", "#6cc5d9", "#e5c07b", "#7fd18b", "#ef8f9a", "#f0a35e", "#9aa1ad"];

/** A Worktree as the row shows it: one git lists, or one that's gone but still has sessions. */
export type WorktreeTab = WorktreeInfo & { removed?: boolean };

/** A Worktree that disappeared (e.g. removed in a terminal) while sessions still run in it. */
export const removedWorktree = (path: string): WorktreeTab => ({
  path, branch: null, head: "", isMain: false, ahead: null, behind: null, changed: 0, removed: true,
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

export function WorktreeRow(props: {
  worktrees: WorktreeTab[];
  active: string | undefined;
  sessionsIn: (path: string) => SessionInfo[];
  onSelect: (path: string) => void;
  onNewSession: (path: string) => void;
  onNewWorktree: () => void;
}) {
  return (
    <nav class="worktree-row">
      <For each={props.worktrees}>
        {(w) => {
          const sessions = () => props.sessionsIn(w.path);
          const needsYou = () => sessions().filter((s) => s.state === "needsYou").length;
          return (
            <button
              class="worktree-tab"
              classList={{ on: w.path === props.active, dimmed: sessions().length === 0, removed: !!w.removed }}
              style={{ "--c": worktreeColour(w.path) }}
              title={w.removed ? `${w.path}: no longer a Worktree; its sessions still run` : w.path}
              onClick={() => props.onSelect(w.path)}
            >
              <span class="branch-glyph">⎇</span>
              <span class="mono">{worktreeLabel(w)}</span>
              <Show when={w.ahead}>{(n) => <span class="muted">↑{n()}</span>}</Show>
              <span class="mini-dots">
                <For each={sessions()}>{(s) => <span class={`dot mini ${s.state}`} />}</For>
              </span>
              <Show when={needsYou() > 0 && w.path !== props.active}>
                <span class="badge needs" title="Sessions here need you">
                  {needsYou()}
                </span>
              </Show>
              <Show when={sessions().length === 0 && !w.removed}>
                <span
                  class="new-session"
                  title="New Agent session in this Worktree"
                  onClick={(e) => {
                    e.stopPropagation();
                    props.onNewSession(w.path);
                  }}
                >
                  ＋ session
                </span>
              </Show>
            </button>
          );
        }}
      </For>
      <button class="ghost add-worktree" onClick={() => props.onNewWorktree()} title="New Worktree with an Agent session">
        ＋ worktree
      </button>
    </nav>
  );
}

export function ContextBar(props: {
  worktree: WorktreeTab | undefined;
  session: SessionInfo | undefined;
  stateLabel: Record<SessionState, string>;
}) {
  return (
    <Show when={props.worktree}>
      {(w) => (
        <div class="context-bar" style={{ "--c": worktreeColour(w().path) }}>
          <span class="branch-glyph">⎇</span>
          <b class="mono">{worktreeLabel(w())}</b>
          <span class="mono muted path">{w().path}</span>
          <Show when={!w().removed}>
            <Show when={w().ahead !== null}>
              <span class="muted">
                ↑{w().ahead} ↓{w().behind}
              </span>
            </Show>
            <span class="muted">{w().changed === 1 ? "1 changed" : `${w().changed} changed`}</span>
          </Show>
          <Show when={props.session}>
            {(s) => (
              <>
                <span class="sep">›</span>
                <span class="agent-glyph">✦</span>
                <span class={`dot ${s().state}`} />
                <b>{s().name}</b>
                <span class="muted">{props.stateLabel[s().state]}</span>
              </>
            )}
          </Show>
        </div>
      )}
    </Show>
  );
}

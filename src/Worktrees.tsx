// The Worktree row and context bar (ticket 06, layout A1): Worktrees on top, the active Worktree's
// Agent sessions in the row below, and a bar saying exactly where your next prompt goes.
import { For, Match, Show, Switch } from "solid-js";
import type { SessionInfo, SessionState, WorktreeInfo } from "./core";
import { CirclePause, CirclePlay, GitBranch, House, Loader, Plus, Sparkles, Trash2, X } from "./icons";
import { StateDot } from "./StateDot";

/** The design's eight Worktree colours (wt-1 … wt-8): muted, so state colours stay louder. */
const PALETTE = ["#6f87c4", "#a287c2", "#5d9cab", "#b89a66", "#6aa87a", "#c07d86", "#bd845c", "#848a96"];

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
  /** A Worktree setup is running there (its first session hasn't started yet). */
  settingUp: (path: string) => boolean;
  onSelect: (path: string) => void;
  onNewSession: (path: string) => void;
  onNewWorktree: () => void;
}) {
  return (
    <nav class="worktree-row">
      <div class="worktree-tabs">
        <For each={props.worktrees}>
          {(w) => {
            const sessions = () => props.sessionsIn(w.path);
            const needsYou = () => sessions().filter((s) => s.state === "needsYou").length;
            return (
              <button
                class="worktree-tab"
                classList={{ on: w.path === props.active, dimmed: sessions().length === 0 && !props.settingUp(w.path), removed: !!w.removed }}
                style={{ "--c": worktreeColour(w.path) }}
                title={w.removed ? `${w.path}: no longer a Worktree; its sessions still run` : w.path}
                onClick={() => props.onSelect(w.path)}
              >
                <Show when={w.isMain}>
                  <House class="home" />
                </Show>
                <GitBranch />
                <span class="label">{w.removed ? w.path.split(/[\\/]/).pop() : worktreeLabel(w)}</span>
                <Show when={w.removed}>
                  <span class="removed-note">(removed)</span>
                </Show>
                <Show when={w.ahead}>{(n) => <span class="ahead">↑{n()}</span>}</Show>
                <Show
                  when={!props.settingUp(w.path)}
                  fallback={
                    <span class="setting-up">
                      <Loader class="spin" />
                      setting up
                    </span>
                  }
                >
                  <span class="mini-dots">
                    <For each={sessions()}>{(s) => <StateDot state={s.state} />}</For>
                  </span>
                </Show>
                <Show when={needsYou() > 0 && w.path !== props.active}>
                  <span class="badge needs" title="Sessions here need you">
                    {needsYou()}
                  </span>
                </Show>
                <Show when={sessions().length === 0 && !w.removed && !props.settingUp(w.path)}>
                  <span
                    class="new-session"
                    title="New Agent session in this Worktree"
                    onClick={(e) => {
                      e.stopPropagation();
                      props.onNewSession(w.path);
                    }}
                  >
                    <Plus />
                    session
                  </span>
                </Show>
              </button>
            );
          }}
        </For>
      </div>
      <button class="add-worktree" onClick={() => props.onNewWorktree()} title="New Worktree with an Agent session">
        <Plus />
        worktree
      </button>
    </nav>
  );
}

export function ContextBar(props: {
  worktree: WorktreeTab | undefined;
  session: SessionInfo | undefined;
  stateLabel: Record<SessionState, string>;
  onRemove: (path: string) => void;
  /** In the Columns view: closes this column (unpins its Worktree). */
  onUnpin?: () => void;
  onSuspend: (session: SessionInfo) => void;
  onResume: (session: SessionInfo) => void;
}) {
  return (
    <Show when={props.worktree}>
      {(w) => (
        <div class="context-bar" style={{ "--c": worktreeColour(w().path) }}>
          <GitBranch />
          <span class="ctx-branch">{w().removed ? w().path.split(/[\\/]/).pop() : worktreeLabel(w())}</span>
          <span class="sep">·</span>
          <span class="path" title={w().path}>
            {w().path}
          </span>
          <Show when={!w().removed} fallback={<span class="gone">folder removed — sessions still run</span>}>
            <Show when={w().ahead !== null}>
              <span class="sep">·</span>
              <span class="counts">
                ↑{w().ahead} ↓{w().behind}
              </span>
            </Show>
            <span class="sep">·</span>
            <span class="changed">{w().changed === 0 ? "no changes" : `${w().changed} changed`}</span>
          </Show>
          <Show when={props.session} fallback={<span class="none">— no Agent session selected</span>}>
            {(s) => (
              <>
                <span class="inside">›</span>
                <Sparkles class="session-glyph" />
                <span class="session">{s().name}</span>
                <span class="sep">·</span>
                <StateDot state={s().state} />
                <span class={`state ${s().state}`}>{props.stateLabel[s().state]}</span>
              </>
            )}
          </Show>
          <span class="bar-actions">
            <Show when={props.session}>
              {(s) => (
                <Switch>
                  <Match when={s().state === "idle"}>
                    <button onClick={() => props.onSuspend(s())} title="Stop this session's Agent to free memory; sending a message resumes it">
                      <CirclePause />
                      Suspend
                    </button>
                  </Match>
                  <Match when={s().state === "exited"}>
                    <button onClick={() => props.onResume(s())} title="Bring the Agent back with this conversation">
                      <CirclePlay />
                      Resume
                    </button>
                  </Match>
                </Switch>
              )}
            </Show>
            <Show when={!w().isMain && !w().removed}>
              <button onClick={() => props.onRemove(w().path)} title="Remove this Worktree (asks first)">
                <Trash2 />
                Remove Worktree…
              </button>
            </Show>
            <Show when={props.onUnpin}>
              {(unpin) => (
                <button class="icon" onClick={() => unpin()()} title="Close this column (its sessions keep running)" aria-label="Close this column">
                  <X />
                </button>
              )}
            </Show>
          </span>
        </div>
      )}
    </Show>
  );
}

// The sidebar and context strip (ticket 06, redone in the shadcn style): the Workspace's Worktrees
// down the left with the active one's Agent sessions under it, folding to an icon rail, and a strip
// saying exactly where your next prompt goes.
import { For, type JSX, Match, Show, Switch } from "solid-js";
import type { SessionId, SessionInfo, WorktreeInfo } from "./core";
import {
  ArrowDownUp,
  ChevronsUpDown,
  CirclePause,
  CirclePlay,
  FileDiff,
  Folder,
  GitBranch,
  History,
  ListTree,
  Loader,
  Plus,
  Search,
  Settings,
  Trash2,
  X,
} from "./icons";
import { StateDot } from "./StateDot";

/** The design's eight Worktree colours (wt-1 … wt-8): muted, so state colours stay louder. */
const PALETTE = ["#6f87c4", "#a287c2", "#5d9cab", "#b89a66", "#6aa87a", "#c07d86", "#bd845c", "#848a96"];

/** A Worktree as the sidebar shows it: one git lists, or one that's gone but still has sessions. */
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

/** Where a Worktree's sessions stand, most urgent first, at the end of its sidebar row. */
function worktreeSignal(sessions: SessionInfo[], settingUp: boolean): JSX.Element {
  if (sessions.some((s) => s.state === "needsYou")) return <span class="needs-dot" title="A session here needs you" />;
  if (settingUp) return <Loader class="spin setting-up" aria-label="Setting up" />;
  if (sessions.some((s) => s.state === "working")) return <Loader class="spin working" aria-label="Working" />;
  if (sessions.some((s) => s.state === "exited")) return <StateDot state="exited" title="An Agent exited" />;
  return null;
}

export interface SidebarProps {
  workspaceName: string;
  workspaceRoot: string;
  /** Folded to the icon rail. */
  collapsed: boolean;
  /** The app mark (an SVG), for the Workspace switcher. */
  mark: string;
  worktrees: WorktreeTab[];
  active: string;
  /** The Columns view's pinned Worktrees (the others are dimmed; choosing one gives it a column);
   *  null in the Tabs view, where the active Worktree lists its sessions. */
  pinned: string[] | null;
  sessionsIn: (path: string) => SessionInfo[];
  /** The session the Tabs view shows. */
  activeSession: SessionId | null;
  /** A Worktree setup is running there (its first session hasn't started yet). */
  settingUp: (path: string) => boolean;
  onSelect: (path: string) => void;
  onShowSession: (id: SessionId) => void;
  onCloseSession: (id: SessionId) => void;
  onNewSession: (path: string) => void;
  /** Opens the Recent sessions menu under `anchor`. */
  onRecent: (anchor: DOMRect) => void;
  onNewWorktree: () => void;
  onOverview: () => void;
  onSwitchWorkspace: () => void;
  onSearch: () => void;
  onSettings: () => void;
}

export function Sidebar(props: SidebarProps) {
  const dimmed = (w: WorktreeTab) => !!props.pinned && !props.pinned.includes(w.path);
  const title = (w: WorktreeTab) =>
    w.removed ? `${w.path}: no longer a Worktree; its sessions still run` : dimmed(w) ? `${worktreeLabel(w)}: give it a column` : w.path;
  return (
    <Show
      when={!props.collapsed}
      fallback={
        <aside class="sidebar rail">
          <button class="logo-box" onClick={() => props.onSwitchWorkspace()} title={`${props.workspaceName}: switch repository (Ctrl+Shift+O)`}>
            <span class="logo" innerHTML={props.mark} />
          </button>
          <span class="rail-sep" />
          <For each={props.worktrees}>
            {(w) => (
              <button
                class="rail-item"
                classList={{ on: props.pinned ? !dimmed(w) : w.path === props.active, dimmed: dimmed(w), focused: !!props.pinned && w.path === props.active }}
                style={{ "--c": worktreeColour(w.path) }}
                title={title(w)}
                aria-label={worktreeLabel(w)}
                onClick={() => props.onSelect(w.path)}
              >
                <GitBranch />
                <Show when={props.sessionsIn(w.path).some((s) => s.state === "needsYou")}>
                  <span class="needs-dot" />
                </Show>
              </button>
            )}
          </For>
          <button class="rail-item add" onClick={() => props.onNewWorktree()} title="New Worktree with an Agent session" aria-label="New Worktree">
            <Plus />
          </button>
          <span class="grow" />
          <button class="rail-item" onClick={() => props.onSettings()} title="Settings (Ctrl+,)" aria-label="Settings">
            <Settings />
          </button>
        </aside>
      }
    >
      <aside class="sidebar">
        <button class="workspace-switcher" onClick={() => props.onSwitchWorkspace()} title="Switch repository (Ctrl+Shift+O)">
          <span class="logo-box">
            <span class="logo" innerHTML={props.mark} />
          </span>
          <span class="names">
            <span class="name">{props.workspaceName}</span>
            <span class="path">{props.workspaceRoot}</span>
          </span>
          <ChevronsUpDown />
        </button>
        <button class="sidebar-search" onClick={() => props.onSearch()} title="Go to a file in this Worktree, or start a session">
          <Search />
          <span class="grow">Search files…</span>
          <kbd>Ctrl P</kbd>
        </button>
        <div class="group-label">
          <span class="grow">Worktrees</span>
          <button class="ghost icon" onClick={() => props.onOverview()} title="Every Worktree, and which are merged and can be removed" aria-label="Worktrees overview">
            <ListTree />
          </button>
          <button class="ghost icon" onClick={() => props.onNewWorktree()} title="New Worktree with an Agent session" aria-label="New Worktree">
            <Plus />
          </button>
        </div>
        <nav class="sidebar-list">
          <For each={props.worktrees}>
            {(w) => {
              const sessions = () => props.sessionsIn(w.path);
              return (
                <>
                  <button
                    class="sidebar-item"
                    classList={{ on: w.path === props.active, dimmed: dimmed(w), removed: !!w.removed }}
                    style={{ "--c": worktreeColour(w.path) }}
                    title={title(w)}
                    onClick={() => props.onSelect(w.path)}
                  >
                    <GitBranch class="wt-glyph" />
                    <span class="label">{w.removed ? w.path.split(/[\\/]/).pop() : worktreeLabel(w)}</span>
                    {worktreeSignal(sessions(), props.settingUp(w.path))}
                  </button>
                  <Show when={!props.pinned && w.path === props.active}>
                    <SessionList {...props} worktree={w} sessions={sessions()} />
                  </Show>
                </>
              );
            }}
          </For>
        </nav>
        <button class="sidebar-footer" onClick={() => props.onSettings()} title="Settings (Ctrl+,)">
          <Settings />
          <span class="grow">Settings</span>
          <kbd>Ctrl ,</kbd>
        </button>
      </aside>
    </Show>
  );
}

/** The active Worktree's sessions, under it: each Tab, a new one, and the Recent menu. */
function SessionList(props: SidebarProps & { worktree: WorktreeTab; sessions: SessionInfo[] }) {
  const path = props.worktree.path;
  return (
    <div class="sub-menu">
      <For each={props.sessions}>
        {(s) => (
          <span class="sub-item-wrap" classList={{ on: s.id === props.activeSession }}>
            <button
              class="sub-item"
              onClick={() => s.id !== props.activeSession && props.onShowSession(s.id)}
              onAuxClick={(e) => e.button === 1 && props.onCloseSession(s.id)}
            >
              <span class="label">{s.name}</span>
              <Switch>
                <Match when={s.state === "needsYou"}>
                  <span class="state-badge needsYou">Needs you</span>
                </Match>
                <Match when={s.unread > 0 && s.id !== props.activeSession}>
                  <span class="count-badge">{s.unread} new</span>
                </Match>
                <Match when={s.state === "working"}>
                  <Loader class="spin working" aria-label="Working" />
                </Match>
                <Match when={s.state !== "idle"}>
                  <StateDot state={s.state} title={s.state === "exited" ? "Exited" : "Suspended"} />
                </Match>
              </Switch>
            </button>
            <button class="close-tab" aria-label={`Close ${s.name}`} title="Close (it stays in Recent sessions; Ctrl+Shift+T reopens)" onClick={() => props.onCloseSession(s.id)}>
              <X />
            </button>
          </span>
        )}
      </For>
      <Show when={!props.worktree.removed && !props.settingUp(path)}>
        <button class="sub-item quiet" onClick={() => props.onNewSession(path)} title="New Agent session in this Worktree">
          <Plus />
          New session
        </button>
      </Show>
      <Show when={!props.worktree.removed}>
        <span class="recent-menu">
          <button
            class="sub-item quiet"
            onClick={(e) => props.onRecent(e.currentTarget.getBoundingClientRect())}
            title="Closed sessions in this Worktree, and its conversations started elsewhere"
          >
            <History />
            Recent sessions
          </button>
        </span>
      </Show>
    </div>
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

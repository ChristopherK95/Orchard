// The Worktree switcher (ticket 48): every Worktree as a card in a grid over the window, each with
// its name in full. Ctrl+Space or the header's Worktrees button opens it; typing filters, the
// arrow keys move between cards, Enter picks, Esc closes.
import { createSignal, For, onCleanup, onMount, Show } from "solid-js";
import type { OtherConversation, RecentSession, SessionId, SessionInfo } from "./core";
import { FindOtherConversations } from "./Chat";
import { Columns2, GitBranch, History, LayoutGrid, Loader, Plus, Search, Sparkles, Trash2, X } from "./icons";
import { StateDot } from "./StateDot";
import { STATE_LABEL } from "./stateLabels";
import { ReviewBadge, ShellBadge, worktreeColour, worktreeLabel, type WorktreeTab } from "./Worktrees";

export interface WorktreeSwitcherProps {
  worktrees: WorktreeTab[];
  active: string;
  /** The session on screen (the Tabs view's Tab, or the focused column's), highlighted. */
  activeSession: SessionId | null;
  /** The Columns view's pinned Worktrees; null in the Tabs view. */
  pinned: string[] | null;
  sessionsIn: (path: string) => SessionInfo[];
  settingUp: (path: string) => boolean;
  /** A Worktree's Recent sessions, as last reported (asked for when its Recent list opens). */
  recentIn: (path: string) => RecentSession[];
  loadRecent: (path: string) => void;
  onPick: (path: string) => void;
  /** A Review Worktree's badge: its Code review, of PR `number`. */
  onCodeReview: (number: number) => void;
  onShowSession: (id: SessionId) => void;
  /** Picks the Worktree and shows its Terminal panel (its shell badge). */
  onShowTerminal: (path: string) => void;
  onCloseSession: (id: SessionId) => void;
  /** "Close all and forget": a removed Worktree's sessions closed, not kept in Recent sessions. */
  onForget: (path: string) => void;
  onNewSession: (path: string) => void;
  onReopen: (session: RecentSession) => void;
  onOpenConversation: (conversation: OtherConversation) => void;
  onPin: (path: string) => void;
  onUnpin: (path: string) => void;
  onNewWorktree: () => void;
  onCleanUp: () => void;
  onClose: () => void;
}

const folder = (path: string) => path.split(/[\\/]/).pop() ?? path;
const nameOf = (w: WorktreeTab) => (w.removed ? folder(w.path) : worktreeLabel(w));

/** Where a Worktree's sessions stand, most urgent first. */
function StatusBadge(props: { sessions: SessionInfo[]; settingUp: boolean }) {
  const has = (state: SessionInfo["state"]) => props.sessions.some((s) => s.state === state);
  return (
    <Show
      when={!has("needsYou")}
      fallback={<span class="state-badge needsYou">{STATE_LABEL.needsYou}</span>}
    >
      <Show when={!props.settingUp} fallback={<span class="state-badge"><Loader class="spin" />Setting up</span>}>
        <Show when={!has("working")} fallback={<span class="state-badge working"><span class="dot" />{STATE_LABEL.working}</span>}>
          <Show when={!has("idle") && !has("suspended")} fallback={<span class="state-badge idle">{STATE_LABEL.idle}</span>}>
            <Show when={has("exited")}>
              <span class="state-badge exited">{STATE_LABEL.exited}</span>
            </Show>
          </Show>
        </Show>
      </Show>
    </Show>
  );
}

export function WorktreeSwitcher(props: WorktreeSwitcherProps) {
  const [query, setQuery] = createSignal("");
  /** The cards shown: the Worktrees matching the filter, then "+ New Worktree" (index `shown().length`). */
  const shown = () => {
    const words = query().trim().toLowerCase();
    return words ? props.worktrees.filter((w) => nameOf(w).toLowerCase().includes(words) || folder(w.path).toLowerCase().includes(words)) : props.worktrees;
  };
  const startAt = () => Math.max(0, shown().findIndex((w) => w.path === props.active));
  const [selected, setSelected] = createSignal(startAt());
  const [recentOf, setRecentOf] = createSignal<string | null>(null);
  let input!: HTMLInputElement;
  let grid!: HTMLDivElement;

  /** How many cards a row of the grid holds (it fits as many ~300 px columns as there is room for). */
  const perRow = () => Math.max(1, getComputedStyle(grid).gridTemplateColumns.split(" ").filter(Boolean).length);
  const move = (by: number) => {
    const count = shown().length + 1;
    // (Down from a row with nothing below lands on the last card.)
    const next = Math.min(count - 1, selected() + by);
    if (next < 0) return;
    setSelected(next);
    grid.querySelectorAll(".switcher-card")[next]?.scrollIntoView({ block: "nearest" });
  };
  const pickSelected = () => {
    const w = shown()[selected()];
    if (w) props.onPick(w.path);
    else props.onNewWorktree();
  };
  const toggleRecent = (path: string) => {
    if (recentOf() === path) return setRecentOf(null);
    props.loadRecent(path);
    setRecentOf(path);
  };

  onMount(() => {
    input.focus();
    grid.querySelectorAll(".switcher-card")[selected()]?.scrollIntoView({ block: "nearest" });
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.preventDefault();
        props.onClose();
      }
    };
    window.addEventListener("keydown", onKey);
    onCleanup(() => window.removeEventListener("keydown", onKey));
  });

  return (
    // The panel has no background of its own, so a click anywhere but a card or a control closes it.
    <div class="modal-backdrop switcher-backdrop" onClick={(e) => !(e.target as Element).closest(".switcher-card, .switcher-search, button") && props.onClose()}>
      <div class="modal worktree-switcher" role="dialog" aria-label="Worktree switcher">
        <div class="switcher-scroll">
          {/* A square-ish grid (4 cards 2×2, 9 cards 3×3), as wide as the window allows, with the
              filter and buttons just above it. */}
          <div class="switcher-body" style={{ "--cols": Math.max(1, Math.ceil(Math.sqrt(shown().length))) }}>
            <div class="switcher-head">
              <LayoutGrid />
              <b>Worktrees</b>
              <span class="switcher-search">
                <Search />
                <input
                  ref={input}
                  placeholder="Filter by name…"
                  value={query()}
                  onInput={(e) => {
                    setQuery(e.currentTarget.value);
                    setSelected(startAt());
                  }}
                  onKeyDown={(e) => {
                    const by = { ArrowLeft: -1, ArrowRight: 1, ArrowUp: -perRow(), ArrowDown: perRow() }[e.key];
                    if (by !== undefined) {
                      e.preventDefault();
                      move(by);
                    } else if (e.key === "Enter") {
                      e.preventDefault();
                      pickSelected();
                    }
                  }}
                  spellcheck={false}
                />
              </span>
              <span class="grow" />
              <button class="ghost small" onClick={() => props.onCleanUp()} title="Every Worktree, and which are merged and can be removed">
                <Trash2 />
                Clean up…
              </button>
              <button class="ghost icon" onClick={() => props.onClose()} title="Close (Esc)" aria-label="Close">
                <X />
              </button>
            </div>
            <div class="switcher-grid" ref={grid}>
              <For each={shown()}>
                {(w, i) => {
                  const sessions = () => props.sessionsIn(w.path);
                  const hasColumn = () => !!props.pinned?.includes(w.path);
                  const canStart = () => !w.removed && w.review === null;
                  return (
                    <div
                      class="switcher-card"
                      classList={{
                        on: w.path === props.active,
                        selected: i() === selected(),
                        removed: !!w.removed,
                        "needs-you": sessions().some((s) => s.state === "needsYou"),
                      }}
                      style={{ "--c": worktreeColour(w.path) }}
                      onMouseEnter={() => setSelected(i())}
                    >
                      <button class="switcher-card-main" onClick={() => props.onPick(w.path)} title={w.path}>
                        <GitBranch class="wt-glyph" />
                        <span class="switcher-name">{nameOf(w)}</span>
                      </button>
                      <div class="switcher-badges" onClick={(e) => (e.target as Element).closest(".shell-badge") && props.onShowTerminal(w.path)}>
                        <StatusBadge sessions={sessions()} settingUp={props.settingUp(w.path)} />
                        <Show when={w.review}>{(n) => <ReviewBadge number={n()} onOpen={props.onCodeReview} />}</Show>
                        <ShellBadge path={w.path} />
                        <Show when={w.removed}>
                          <span class="switcher-gone">folder removed</span>
                        </Show>
                        <Show when={props.pinned && !w.removed}>
                          <span class="switcher-column" classList={{ has: hasColumn() }}>
                            <Columns2 />
                            {hasColumn() ? "Has a column" : "No column"}
                          </span>
                        </Show>
                      </div>
                      <Show when={sessions().length}>
                        <div class="switcher-sessions">
                          <For each={sessions()}>
                            {(s) => (
                              <div class={`switcher-session ${s.state}`} classList={{ on: s.id === props.activeSession }}>
                                <button class="switcher-session-main" onClick={() => props.onShowSession(s.id)} title={`Go to ${s.name}`}>
                                  <Show when={s.state === "working"} fallback={<StateDot state={s.state} />}>
                                    <Loader class="spin" />
                                  </Show>
                                  <span class="name">{s.name}</span>
                                  <span class="state">{STATE_LABEL[s.state]}</span>
                                  <Show when={s.unread > 0}>
                                    <span class="count-badge">{s.unread} new</span>
                                  </Show>
                                </button>
                                <Show when={w.removed}>
                                  <button class="ghost icon close-tab" onClick={() => props.onCloseSession(s.id)} title="Close" aria-label={`Close ${s.name}`}>
                                    <X />
                                  </button>
                                </Show>
                              </div>
                            )}
                          </For>
                        </div>
                      </Show>
                      <div class="switcher-actions">
                        <Show when={canStart() && !props.settingUp(w.path)}>
                          <button class="ghost small" onClick={() => props.onNewSession(w.path)} title="New Agent session in this Worktree">
                            <Plus />
                            New session
                          </button>
                        </Show>
                        <Show when={canStart()}>
                          <button
                            class="ghost small"
                            classList={{ on: recentOf() === w.path }}
                            onClick={() => toggleRecent(w.path)}
                            title="Closed sessions in this Worktree, and its conversations started elsewhere"
                            aria-expanded={recentOf() === w.path}
                          >
                            <History />
                            Recent
                          </button>
                        </Show>
                        <Show when={props.pinned && !w.removed}>
                          <Show
                            when={hasColumn()}
                            fallback={
                              <button class="ghost small" onClick={() => props.onPin(w.path)} title="Give it a column in the Columns view">
                                <Columns2 />
                                Give it a column
                              </button>
                            }
                          >
                            <button class="ghost small" onClick={() => props.onUnpin(w.path)} title="Close its column (its sessions keep running)">
                              <X />
                              Close column
                            </button>
                          </Show>
                        </Show>
                        <Show when={w.removed && sessions().length}>
                          <button class="ghost small switcher-forget" onClick={() => props.onForget(w.path)} title="Close its sessions without keeping them in Recent sessions">
                            <Trash2 />
                            Close all and forget
                          </button>
                        </Show>
                      </div>
                      <Show when={recentOf() === w.path}>
                        <div class="switcher-recent">
                          <For each={props.recentIn(w.path)} fallback={<span class="note">No Recent sessions.</span>}>
                            {(r) => (
                              <button class="switcher-recent-row" onClick={() => props.onReopen(r)} title="Reopen with its conversation">
                                <Sparkles />
                                {r.name}
                              </button>
                            )}
                          </For>
                          <FindOtherConversations worktree={w.path} onOpen={props.onOpenConversation} />
                        </div>
                      </Show>
                    </div>
                  );
                }}
              </For>
              <button class="switcher-card switcher-new" title="New Worktree with an Agent session (Ctrl+N)" classList={{ selected: selected() === shown().length }} onClick={() => props.onNewWorktree()} onMouseEnter={() => setSelected(shown().length)}>
                <Plus />
                New Worktree
              </button>
            </div>
          </div>
        </div>
      </div>
    </div>
  );
}

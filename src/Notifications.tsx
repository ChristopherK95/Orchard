// The sticky Notifications (ticket 50): a stack of cards at the bottom right for the sessions out of
// sight, newest at the bottom. They stay until closed, until their session is opened, or until the
// Worktree switcher is opened (App.tsx clears them); nothing clears them by time.
import { createEffect, createSignal, For, onCleanup, untrack } from "solid-js";
import { GitBranch, X } from "./icons";
import type { Notification, NotificationEvent } from "./notificationData";
import { worktreeColour } from "./Worktrees";

const EVENT_TEXT: Record<NotificationEvent, string> = {
  needsYou: "Needs you",
  finished: "Finished",
  exited: "Exited unexpectedly",
  setupFailed: "Worktree setup failed",
};

/** The state badge each event wears. */
const BADGE: Record<NotificationEvent, string> = { needsYou: "needsYou", finished: "idle", exited: "exited", setupFailed: "exited" };

/** A card on screen; `leaving` while its fade plays (it's already gone from the list). */
interface Row {
  n: Notification;
  leaving: boolean;
}

const STACK_WIDTH = 340;
/** A leaving card goes after this even if its fade's `animationend` never comes (it's 280 ms). */
const LEAVE_FALLBACK_MS = 400;
const EDGE = 16;
const reducedMotion = () => window.matchMedia("(prefers-reduced-motion: reduce)").matches;

export function Notifications(props: {
  notifications: Notification[];
  /** The Worktree's full name (its branch), for the card. */
  worktreeName: (path: string) => string;
  /** Clicking a card: goes to its session (or its Worktree, for a failed setup). */
  onOpen: (n: Notification) => void;
  onClose: (key: string) => void;
}) {
  // The rows lag the list by the closed ones' fade, each kept where it was meanwhile.
  const [rows, setRows] = createSignal<Row[]>([]);
  const gone = (row: Row) => setRows((all) => all.filter((r) => r !== row));
  const fallbacks = new Set<number>();
  const leave = (n: Notification): Row => {
    const row = { n, leaving: true };
    const timer = window.setTimeout(() => {
      fallbacks.delete(timer);
      gone(row);
    }, LEAVE_FALLBACK_MS);
    fallbacks.add(timer);
    return row;
  };
  onCleanup(() => fallbacks.forEach((timer) => clearTimeout(timer)));
  createEffect(() => {
    const list = props.notifications;
    const live = new Set(list.map((n) => n.key));
    const before = untrack(rows);
    const kept = new Map(before.filter((r) => !r.leaving).map((r) => [r.n, r]));
    const next: Row[] = list.map((n) => kept.get(n) ?? { n, leaving: false });
    // A cleared one fades where it was (one replaced by a newer event just goes: the newer slides in).
    if (!reducedMotion())
      before.forEach((r, i) => {
        if (r.leaving || !live.has(r.n.key)) next.splice(Math.min(i, next.length), 0, r.leaving ? r : leave(r.n));
      });
    setRows(next);
  });

  // Kept above whichever is higher under its strip, the composers or the Terminal panel at the
  // bottom (one at the side runs the full height, so there is no above it), so their
  // send buttons and prompts are never covered.
  const [bottom, setBottom] = createSignal(EDGE);
  const measure = () => {
    const right = window.innerWidth - EDGE;
    const left = right - STACK_WIDTH;
    let top = window.innerHeight;
    for (const box of document.querySelectorAll(".composer-box, .terminal-panel:not(.side)")) {
      const r = box.getBoundingClientRect();
      if (r.width > 0 && r.right > left && r.left < right && r.top < top) top = r.top;
    }
    setBottom(Math.max(EDGE, window.innerHeight - top + 8));
  };
  const showing = () => rows().length > 0;
  createEffect(() => {
    rows();
    if (showing()) measure();
  });
  // Listening only while there are cards. (A composer grows as it's typed in, and the views and
  // panels around it change on clicks.)
  createEffect(() => {
    if (!showing()) return;
    const events = ["resize", "input", "click", "keyup", "transitionend"] as const;
    const again = () => requestAnimationFrame(measure);
    events.forEach((e) => window.addEventListener(e, again, true));
    onCleanup(() => events.forEach((e) => window.removeEventListener(e, again, true)));
  });

  const time = (at: number) => new Date(at).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });

  return (
    <div class="notifications" style={{ bottom: `${bottom()}px`, width: `${STACK_WIDTH}px` }} aria-live="polite">
      <For each={rows()}>
        {(row) => (
          <div class="notification-slot" classList={{ leaving: row.leaving }} onAnimationEnd={(e) => row.leaving && e.target === e.currentTarget && gone(row)}>
            <div class="notification" classList={{ "needs-you": row.n.event === "needsYou" }} style={{ "--c": worktreeColour(row.n.worktree) }}>
              <button class="notification-main" disabled={row.leaving} onClick={() => props.onOpen(row.n)} title="Go to it">
                <span class="notification-worktree">
                  <GitBranch class="wt-glyph" />
                  <span class="notification-worktree-name">{props.worktreeName(row.n.worktree)}</span>
                </span>
                <span class="notification-session">{row.n.name}</span>
                <span class="notification-event">
                  <span class={`state-badge ${BADGE[row.n.event]}`}>{EVENT_TEXT[row.n.event]}</span>
                  <span class="notification-time">{time(row.n.at)}</span>
                </span>
              </button>
              <button class="ghost icon notification-close" disabled={row.leaving} onClick={() => props.onClose(row.n.key)} title="Close" aria-label="Close">
                <X />
              </button>
            </div>
          </div>
        )}
      </For>
    </div>
  );
}

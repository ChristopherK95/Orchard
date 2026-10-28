// The rules for the sticky in-app Notifications (ticket 50): which session events raise one, one
// per session, at most `MAX_NOTIFICATIONS`. Kept apart from the UI so they're easy to reason about.
import type { SessionId, SessionState } from "./core";

export type NotificationEvent = "needsYou" | "finished" | "exited" | "setupFailed";

/** Which event (if any) a session's state change raises a Notification for. Closing a session
 *  doesn't pass through `exited` (it's a `sessionClosed`), so any exit is unexpected. */
export function eventOf(previous: SessionState, state: SessionState, turnFinishedOn: boolean): NotificationEvent | null {
  if (state === previous) return null;
  if (state === "needsYou") return "needsYou";
  if (state === "exited") return "exited";
  if (turnFinishedOn && previous === "working" && state === "idle") return "finished";
  return null;
}

export const MAX_NOTIFICATIONS = 10;

export interface Notification {
  /** One per key: a session's id, or `setup:<path>` for a Worktree setup that failed. */
  key: string;
  worktree: string;
  /** Null for a failed Worktree setup (no session started). */
  session: SessionId | null;
  /** The session's name (or "Worktree setup"), as it was when raised. */
  name: string;
  event: NotificationEvent;
  /** When it was raised (ms since the epoch). */
  at: number;
}

export const sessionKey = (id: SessionId) => String(id);
export const setupKey = (worktree: string) => `setup:${worktree}`;

/** Adds `n` at the bottom (newest), replacing any older one with its key and dropping the oldest
 *  past the cap. */
export function raise(list: readonly Notification[], n: Notification): Notification[] {
  const next = [...list.filter((other) => other.key !== n.key), n];
  return next.slice(Math.max(0, next.length - MAX_NOTIFICATIONS));
}

export function clear(list: readonly Notification[], key: string): Notification[] {
  return list.filter((n) => n.key !== key);
}

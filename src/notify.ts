// OS notifications for Tabs you're not looking at (ticket 04). The shell shows them natively;
// clicking one emits `show-session` with the session to open.
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type { SessionId } from "./core";

/** Spec story 26's optional notification; off until the settings file (ticket 08) can switch it on. */
export const NOTIFY_WHEN_BACKGROUND_TURN_FINISHES = false;

export function notify(sessionId: SessionId, title: string, body: string) {
  return invoke<void>("notify_session", { sessionId, title, body });
}

/** Called with the session whose notification was clicked. */
export function onNotificationClicked(handler: (sessionId: SessionId) => void): Promise<UnlistenFn> {
  return listen<SessionId>("show-session", (e) => handler(e.payload));
}

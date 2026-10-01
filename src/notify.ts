// OS notifications for Tabs you're not looking at (ticket 04).
import { isPermissionGranted, requestPermission, sendNotification } from "@tauri-apps/plugin-notification";

/** Spec story 26's optional notification; off until the settings file (ticket 08) can switch it on. */
export const NOTIFY_WHEN_BACKGROUND_TURN_FINISHES = false;

let allowed: Promise<boolean> | undefined;

export async function notify(title: string, body: string) {
  allowed ??= (async () => (await isPermissionGranted()) || (await requestPermission()) === "granted")();
  if (await allowed) sendNotification({ title, body });
}

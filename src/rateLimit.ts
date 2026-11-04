// GitHub's rate limit, while the core knows it's used up: the Dashboard and the PR board count down
// to when it resets, and what it refused is asked for again then.
import { createSignal } from "solid-js";
import { core } from "./core";

/** When the rate limit resets (ms since the epoch); null while it isn't used up. */
const [until, setUntil] = createSignal<number | null>(null);
const [now, setNow] = createSignal(Date.now());
let ticker: number | undefined;
let lift: number | undefined;
/** What the rate limit refused, asked for again once it's reset. */
const retries = new Set<() => void>();

/** After `retry` failed to ask GitHub: if the rate limit refused it, counts down to its reset and
 *  calls `retry` then (a second after, so the core's own wait is over too). */
export async function afterGitHubFailed(retry: () => void) {
  const at = await core.githubRateLimitedUntil().catch(() => null);
  if (at === null || at <= Date.now()) return;
  retries.add(retry);
  const latest = Math.max(at, until() ?? 0);
  setUntil(latest);
  setNow(Date.now());
  clearInterval(ticker);
  ticker = window.setInterval(() => setNow(Date.now()), 1000);
  clearTimeout(lift);
  lift = window.setTimeout(() => {
    clearInterval(ticker);
    setUntil(null);
    const asked = [...retries];
    retries.clear();
    asked.forEach((retry) => retry());
  }, latest - Date.now() + 1000);
}

/** `error` as shown: a rate limit's with a live countdown to its reset, in place of its minutes. */
export function withCountdown(error: string): string {
  const at = until();
  if (at === null || !error.includes("rate limit")) return error;
  const secs = Math.max(0, Math.ceil((at - now()) / 1000));
  const left = `${Math.floor(secs / 60)}:${String(secs % 60).padStart(2, "0")}`;
  return `GitHub's API rate limit is used up (it's shared by everything using gh as you, Agent sessions too). Asking again in ${left}.`;
}

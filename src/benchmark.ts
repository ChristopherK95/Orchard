// The memory-budget benchmark's scenario (ticket 05), run by the app itself when launched with
// AGENT_EDITOR_BENCH set, so no keystrokes or clicks are needed. `scripts/bench-memory.ps1` starts
// the app against the fake ACP agent and measures at each phase marker:
//   one-tab   : one Tab, after a short turn, idle
//   five-tabs : five Tabs, after a long transcript streamed into Tab 1 and a visit to every Tab, idle
//   done      : the script can stop the app
import { invoke } from "@tauri-apps/api/core";
import type { SessionId, SessionState } from "./core";

export interface BenchDriver {
  newSession(): Promise<SessionId>;
  show(id: SessionId): Promise<void>;
  send(id: SessionId, text: string): Promise<void>;
  state(id: SessionId): SessionState | undefined;
}

/** How long each measured phase holds still, so the script can sample it. */
const HOLD_MS = 15_000;
const SETTLE_MS = 3_000;

export const benchMode = () => invoke<boolean>("bench_mode");
const mark = (phase: string) => invoke<void>("bench_mark", { phase });
const sleep = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms));

async function turn(driver: BenchDriver, id: SessionId, text: string) {
  await driver.send(id, text);
  while (driver.state(id) !== "idle") await sleep(50);
}

export async function runBenchmark(driver: BenchDriver, first: SessionId) {
  await turn(driver, first, "short reply please");
  await sleep(SETTLE_MS);
  await mark("one-tab");
  await sleep(HOLD_MS);

  const others: SessionId[] = [];
  for (let i = 0; i < 4; i++) others.push(await driver.newSession());
  await driver.show(first);
  await turn(driver, first, "now a long transcript");
  for (const id of others) {
    await driver.show(id);
    await sleep(300);
  }
  await driver.show(first);
  await sleep(SETTLE_MS);
  await mark("five-tabs");
  await sleep(HOLD_MS);
  await mark("done");
}

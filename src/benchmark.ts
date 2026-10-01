// The memory-budget benchmark's scenario (ticket 05), run by the app itself when launched with
// AGENT_EDITOR_BENCH set, so no keystrokes or clicks are needed. `scripts/bench-memory.ps1` starts
// the app against the fake ACP agent and measures at each phase marker. Keep the marker names in
// sync with the script.
import { core, type SessionId } from "./core";

export const PHASES = {
  /** One Tab, after a short turn, idle. */
  oneTab: "one-tab",
  /** Five Tabs, each after a short turn, idle: the per-Tab cost. */
  fiveTabs: "five-tabs",
  /** Then a long transcript streamed into Tab 1 and a visit to every Tab, idle: the total and idle CPU. */
  longTranscript: "long-transcript",
  /** The script may stop the app. */
  done: "done",
} as const;

export interface BenchDriver {
  newSession(): Promise<SessionId>;
  show(id: SessionId): Promise<void>;
  /** Sends a prompt and resolves when that turn has finished. */
  turn(id: SessionId, text: string): Promise<void>;
}

/** How long each measured phase holds still, so the script can sample it (incl. a 20 s CPU window). */
const HOLD_MS = 35_000;
const SETTLE_MS = 3_000;

const sleep = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms));

async function phase(name: string) {
  await sleep(SETTLE_MS);
  await core.benchMark(name);
  await sleep(HOLD_MS);
}

export async function runBenchmark(driver: BenchDriver, first: SessionId) {
  await driver.turn(first, "short reply please");
  await phase(PHASES.oneTab);

  const tabs = [first];
  for (let i = 0; i < 4; i++) {
    const id = await driver.newSession();
    await driver.turn(id, "short reply please");
    tabs.push(id);
  }
  await phase(PHASES.fiveTabs);

  await driver.show(first);
  await driver.turn(first, "now a long transcript");
  for (const id of tabs) {
    await driver.show(id);
    await sleep(300);
  }
  await driver.show(first);
  await phase(PHASES.longTranscript);
  await core.benchMark(PHASES.done);
}

// The memory-budget benchmark's scenario (ticket 05), run by the app itself when launched with
// ORCHARD_BENCH set, so no keystrokes or clicks are needed. `scripts/bench-memory.ps1` starts
// the app against the fake ACP agent and measures at each phase marker. Keep the marker names in
// sync with the script.
import { core, type SessionId } from "./core";

export const PHASES = {
  /** One Tab, after a short turn, idle. */
  oneTab: "one-tab",
  /** Five Tabs, each after a short turn, idle: the per-Tab cost. */
  fiveTabs: "five-tabs",
  /** Then a long transcript streamed into Tab 1, a visit to every Tab and a file open in the Manual
   *  editor, idle: the total and idle CPU. */
  longTranscript: "long-transcript",
  /** Then the Columns view (ticket 28): 3 new Worktrees, each with a session after a short turn,
   *  pinned with the main checkout, so 4 columns stream at once, idle. */
  columns: "columns",
  /** The script may stop the app. */
  done: "done",
} as const;

export interface BenchDriver {
  newSession(): Promise<SessionId>;
  show(id: SessionId): Promise<void>;
  /** Sends a prompt and resolves when that turn has finished. */
  turn(id: SessionId, text: string): Promise<void>;
  /** Opens a file of the Workspace in the Manual editor. */
  openFile(): Promise<void>;
  /** Creates a Worktree on a new branch; resolves to its path. */
  newWorktree(name: string): Promise<string>;
  newSessionIn(worktree: string): Promise<SessionId>;
  /** Pins the main checkout and these Worktrees and shows the Columns view (whatever the width). */
  showColumns(worktrees: string[]): Promise<void>;
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
  await driver.openFile();
  await phase(PHASES.longTranscript);

  const worktrees: string[] = [];
  for (let i = 1; i <= 3; i++) {
    const worktree = await driver.newWorktree(`bench/column-${i}`);
    const id = await driver.newSessionIn(worktree);
    await driver.turn(id, "short reply please");
    worktrees.push(worktree);
  }
  await driver.showColumns(worktrees);
  await phase(PHASES.columns);
  await core.benchMark(PHASES.done);
}

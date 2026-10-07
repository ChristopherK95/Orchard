// The running shells, as the core last reported them: the Terminal panel's tabs, and the green dot
// on the Terminal button and tab. Not in TerminalPanel.tsx, which loads late with xterm.js.
import { createSignal } from "solid-js";
import { core, type TerminalInfo } from "./core";

const [all, setAll] = createSignal<TerminalInfo[]>([]);

/** From the core's `terminalsChanged` event (and its list, once at the start). */
export const setShells = setAll;

/** The Worktree's running shells, oldest first. */
export const shellsIn = (worktree: string) => all().filter((t) => t.worktree === worktree);

export const shellRunning = (worktree: string) => all().some((t) => t.worktree === worktree);

/** A shell a Terminal panel on its Worktree should show next (an Action's, just started). `n`
 *  tells one request from the next. */
const [wanted, setWanted] = createSignal<{ worktree: string; id: number; n: number } | null>(null);
export { wanted as wantedShell };
export const showShellOf = (worktree: string, id: number) => setWanted((now) => ({ worktree, id, n: (now?.n ?? 0) + 1 }));

/** Looks again for the shells' names (what runs in the foreground changes with no event). */
export function refreshShells() {
  void core.terminals().then(setAll, () => {});
}

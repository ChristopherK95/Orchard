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

/** Looks again for the shells' names (what runs in the foreground changes with no event). */
export function refreshShells() {
  void core.terminals().then(setAll, () => {});
}

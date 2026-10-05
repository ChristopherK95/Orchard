// Which Worktrees have a shell running (the green dot on the Terminal button and tab). Started by a
// Terminal panel, ended when the shell exits or is stopped. Not in TerminalPanel.tsx, which loads
// late with xterm.js.
import { createSignal } from "solid-js";

const [live, setLive] = createSignal<ReadonlySet<string>>(new Set());

export const shellRunning = (worktree: string) => live().has(worktree);

export function setShellRunning(worktree: string, running: boolean) {
  if (live().has(worktree) === running) return;
  setLive((now) => {
    const next = new Set(now);
    if (running) next.add(worktree);
    else next.delete(worktree);
    return next;
  });
}

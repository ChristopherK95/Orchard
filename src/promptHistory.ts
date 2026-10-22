const KEY = "orchard.promptHistory";
/** Well above the 10 prompts people reach for, yet small enough to keep in the webview. */
const LIMIT = 50;

function load(): Record<string, string[]> {
  try {
    return JSON.parse(localStorage.getItem(KEY) ?? "{}");
  } catch {
    return {};
  }
}

/** The prompts sent in `worktree`, oldest first. */
export function promptHistory(worktree: string): string[] {
  return load()[worktree] ?? [];
}

/** Remembers a sent prompt for its Worktree; sending the same prompt twice in a row counts once. */
export function rememberPrompt(worktree: string, prompt: string) {
  const all = load();
  const list = all[worktree] ?? [];
  if (list[list.length - 1] === prompt) return;
  all[worktree] = [...list, prompt].slice(-LIMIT);
  try {
    localStorage.setItem(KEY, JSON.stringify(all));
  } catch {
    // Storage full or unavailable: history is a convenience, so losing it is fine.
  }
}

/** Steps through a Worktree's history with Up / Down, keeping the draft that was being typed. */
export class HistoryCursor {
  private index: number | null = null;
  private shown = "";
  private draft = "";

  /** Whether `text` is still the history entry this cursor put there. */
  browsing(text: string) {
    return this.index !== null && text === this.shown;
  }

  reset() {
    this.index = null;
  }

  older(worktree: string, current: string): string | null {
    const list = promptHistory(worktree);
    if (this.index === null) {
      if (!list.length) return null;
      this.draft = current;
      this.index = list.length;
    }
    if (this.index === 0) return null;
    this.index -= 1;
    return (this.shown = list[this.index]);
  }

  newer(worktree: string): string | null {
    if (this.index === null) return null;
    const list = promptHistory(worktree);
    this.index += 1;
    if (this.index >= list.length) {
      this.index = null;
      return (this.shown = this.draft);
    }
    return (this.shown = list[this.index]);
  }
}

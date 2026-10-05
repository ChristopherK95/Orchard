// "Open in editor" on a permission card (ticket 38): the change an Agent asks to make, as a diff of
// the whole file. The card's diff is of what the Agent sent (often just the snippet it replaces),
// so its hunks are found in the file on disk and applied there. A proposal settles once its card is
// answered or the turn ends, and its tab in the Manual editor then says it's out of date.
import { createSignal } from "solid-js";
import { core, type DiffLine, type SessionId } from "./core";

export const proposalKey = (sessionId: SessionId, toolCallId: string) => `${sessionId}\n${toolCallId}`;

/** Proposals open in a Manual editor, by session, until they settle. */
const watched = new Map<SessionId, Set<string>>();
const [settled, setSettled] = createSignal<ReadonlySet<string>>(new Set());
let following = false;

function follow() {
  if (following) return;
  following = true;
  // A session that stops needing you has no open card left: answered, or the turn ended.
  void core.onEvent((event) => {
    if ((event.kind === "sessionStateChanged" && event.state !== "needsYou") || event.kind === "sessionClosed")
      for (const key of watched.get(event.sessionId) ?? []) settle(event.sessionId, key);
  });
}

function settle(sessionId: SessionId, key: string) {
  const keys = watched.get(sessionId);
  if (!keys?.delete(key)) return;
  if (keys.size === 0) watched.delete(sessionId);
  setSettled((all) => new Set([...all, key]));
}

/** A proposal opened in a Manual editor: told when it settles. */
export function watchProposal(sessionId: SessionId, toolCallId: string) {
  follow();
  const keys = watched.get(sessionId) ?? new Set();
  keys.add(proposalKey(sessionId, toolCallId));
  watched.set(sessionId, keys);
}

/** The card was answered (or the turn ended). */
export const settleProposal = (sessionId: SessionId, toolCallId: string) => settle(sessionId, proposalKey(sessionId, toolCallId));

/** Whether the card behind `key` has been answered (reactive). */
export const isSettled = (key: string) => settled().has(key);

type Hunk = { oldStart: number; old: string[]; new: string[] };

/** The hunks; null if the diff was cut short (its last "hunk" is "… N more lines"). */
function hunksOf(diff: DiffLine[]): Hunk[] | null {
  const hunks: Hunk[] = [];
  for (const line of diff) {
    if (line.kind === "hunk") {
      const m = /^@@ -(\d+)/.exec(line.text);
      if (!m) return null;
      hunks.push({ oldStart: Number(m[1]), old: [], new: [] });
      continue;
    }
    const hunk = hunks.at(-1);
    if (!hunk) return null;
    if (line.kind !== "added") hunk.old.push(line.text);
    if (line.kind !== "removed") hunk.new.push(line.text);
  }
  return hunks;
}

const matchesAt = (lines: string[], want: string[], at: number) => want.every((line, i) => lines[at + i] === line);

function find(lines: string[], want: string[], from: number) {
  for (let at = from; at + want.length <= lines.length; at++) if (matchesAt(lines, want, at)) return at;
  return -1;
}

/**
 * `text` ("\n" lines) as it would be with the change. Each hunk's old lines (context and removed)
 * are looked for where its header says, counted from where its block's first hunk was found, else
 * anywhere after the last hunk; they're replaced with its new lines. A block (one of the Agent's
 * edits) starts where the hunk numbers do again, and is applied to the result of the one before.
 * A lone hunk with no old lines is a whole file. Null if a hunk can't be placed.
 */
export function applyDiff(text: string, diff: DiffLine[]): string | null {
  const hunks = hunksOf(diff);
  if (!hunks?.length) return null;
  if (hunks.length === 1 && hunks[0].old.length === 0) return hunks[0].new.join("\n") + "\n";
  const lines = text.split("\n");
  let from = 0;
  /** Where the block's line 1 is in `lines` now. */
  let shift: number | null = null;
  let lastStart = 0;
  for (const hunk of hunks) {
    if (hunk.old.length === 0) return null;
    if (hunk.oldStart <= lastStart) {
      shift = null;
      from = 0;
    }
    lastStart = hunk.oldStart;
    const expected: number = shift === null ? -1 : shift + hunk.oldStart - 1;
    const at: number = expected >= from && matchesAt(lines, hunk.old, expected) ? expected : find(lines, hunk.old, from);
    if (at < 0) return null;
    shift = (shift ?? at - (hunk.oldStart - 1)) + hunk.new.length - hunk.old.length;
    lines.splice(at, hunk.old.length, ...hunk.new);
    from = at + hunk.new.length;
  }
  return lines.join("\n");
}

/** The diff the other way round (new to old), each change's removed lines still before its added. */
export function invertDiff(diff: DiffLine[]): DiffLine[] {
  const out: DiffLine[] = [];
  let removed: DiffLine[] = [];
  let added: DiffLine[] = [];
  const flush = () => {
    out.push(...removed, ...added);
    removed = [];
    added = [];
  };
  for (const line of diff) {
    if (line.kind === "added") removed.push({ kind: "removed", text: line.text });
    else if (line.kind === "removed") added.push({ kind: "added", text: line.text });
    else {
      flush();
      out.push(line.kind === "hunk" ? { kind: "hunk", text: line.text.replace(/^@@ -(\S+) \+(\S+) @@/, "@@ -$2 +$1 @@") } : line);
    }
  }
  flush();
  return out;
}

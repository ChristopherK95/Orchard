// The composer's `@` menu: typing `@` and part of a name lists the Worktree's files that match it
// (fuzzy like Ctrl+P, but without typos, so typing more only narrows the list). Picking one
// completes the mention to `@path/of/the/file`, which the Agent reads as a reference to that file.
// Mentions of files that exist are marked in the input.
import { createEffect, createSignal, For, on } from "solid-js";
import { createStore } from "solid-js/store";
import { Highlighted } from "./CommandPalette";
import { core, type FileMatch } from "./core";
import { FileIcon } from "./icons";

/** How many files are offered. */
const LIMIT = 20;

/** The mention being typed: `@` (at the start or after a space) and at least one character, up to
 *  the caret; else null. */
export function mentionAt(text: string, caret: number): { start: number; query: string } | null {
  const found = /(?:^|\s)@([^\s@]+)$/.exec(text.slice(0, caret));
  if (!found) return null;
  return { start: caret - found[1].length - 1, query: found[1] };
}

/** How a file is written in the prompt (quoted if its path has a space). */
export const mention = (path: string) => (/\s/.test(path) ? `@"${path}"` : `@${path}`);

/** Whether each mentioned path is a file of its Worktree (keyed `worktree\0path`), as asked so far. */
const [known, setKnown] = createStore<Record<string, boolean>>({});

/** Whether `path` is a file of the Worktree (reactive; false until known). */
function isFile(worktree: string, path: string): boolean {
  const key = `${worktree}\0${path}`;
  if (!(key in known)) {
    setKnown(key, false);
    void core
      .findFiles(worktree, path, 10, false)
      .then((found) => found.some((m) => m.path === path) && setKnown(key, true))
      .catch(() => {});
  }
  return known[key];
}

/** `@path` or `@"a path"`, at the start or after a space (punctuation after it isn't part of it). */
const MENTION = /(^|\s)(@(?:"([^"\n]+)"|([^\s@"]+?)))(?=[.,;:!?)]*(?:\s|$))/g;

/** The text cut into plain runs and mentions of files that exist (reactive). */
export function markMentions(text: string, worktree: string): { text: string; file: boolean }[] {
  const parts: { text: string; file: boolean }[] = [];
  let from = 0;
  for (const m of text.matchAll(MENTION)) {
    const start = m.index + m[1].length;
    if (!isFile(worktree, m[3] ?? m[4])) continue;
    parts.push({ text: text.slice(from, start), file: false }, { text: m[2], file: true });
    from = start + m[2].length;
  }
  parts.push({ text: text.slice(from), file: false });
  return parts;
}

/** Behind the composer's input, the same text laid out the same way, invisible but for a
 *  background under each mention of a file (a textarea can't style its own text). */
export function MentionMarks(props: { text: string; worktree: string; ref: (el: HTMLDivElement) => void }) {
  return (
    <div class="mention-marks" ref={props.ref} aria-hidden="true">
      <For each={markMentions(props.text, props.worktree)}>{(part) => (part.file ? <mark>{part.text}</mark> : part.text)}</For>
      {/* (a last empty line still takes its height) */}
      {"\u200b"}
    </div>
  );
}

export function MentionMenu(props: {
  items: FileMatch[];
  selected: number;
  onHover: (index: number) => void;
  onPick: (file: FileMatch) => void;
}) {
  return (
    <div class="slash-menu mention-menu" role="listbox" aria-label="Files">
      <div class="slash-head">
        Files
        <span class="note">↑↓ choose · Enter or Tab complete · Esc close</span>
      </div>
      <div class="slash-list">
        <For each={props.items} fallback={<p class="slash-empty">No file matches.</p>}>
          {(file, i) => (
            <button
              class="slash-item mention-item"
              classList={{ on: i() === props.selected }}
              role="option"
              aria-selected={i() === props.selected}
              ref={(row) => createEffect(() => i() === props.selected && row.scrollIntoView({ block: "nearest" }))}
              onMouseEnter={() => props.onHover(i())}
              // (mousedown, so the textarea keeps focus)
              onMouseDown={(e) => {
                e.preventDefault();
                props.onPick(file);
              }}
            >
              <FileIcon />
              <span class="mention-text">
                <span class="mention-name">
                  <Name path={file.path} indices={file.indices} />
                </span>
                <span class="mention-path">{file.path}</span>
              </span>
            </button>
          )}
        </For>
      </div>
    </div>
  );
}

/** The file's name, its matched characters marked. */
function Name(props: { path: string; indices: number[] }) {
  const dir = () => props.path.slice(0, props.path.lastIndexOf("/") + 1);
  // (Offsets are bytes into the whole path.)
  const shift = () => new TextEncoder().encode(dir()).length;
  return <Highlighted text={props.path.slice(dir().length)} indices={props.indices.map((i) => i - shift()).filter((i) => i >= 0)} />;
}

/** The menu's state for a composer: open while a mention is being typed, unless dismissed with Esc. */
export function createMentionMenu(text: () => string, caret: () => number, worktree: () => string) {
  const [selected, setSelected] = createSignal(0);
  const [dismissed, setDismissed] = createSignal<string | null>(null);
  const [items, setItems] = createSignal<FileMatch[]>([]);
  const at = () => mentionAt(text(), caret());
  const query = () => at()?.query ?? null;
  const open = () => query() !== null && dismissed() !== text();
  let asked = 0;
  createEffect(
    on([query, worktree], async ([q, dir]) => {
      setSelected(0);
      const mine = ++asked;
      if (q === null) return setItems([]);
      try {
        const found = await core.findFiles(dir, q, LIMIT, false);
        if (mine === asked) setItems(found); // (a later keystroke's answer wins)
      } catch {
        if (mine === asked) setItems([]);
      }
    }),
  );
  return {
    open,
    items,
    selected,
    setSelected,
    /** Where the mention being typed starts and ends, to replace it. */
    span: () => {
      const a = at();
      return a && { start: a.start, end: caret() };
    },
    /** Handles a key in the composer; true if the menu used it. */
    key(e: KeyboardEvent, complete: (file: FileMatch) => void): boolean {
      if (!open()) return false;
      const count = items().length;
      if (e.key === "ArrowDown" && count) setSelected((i) => (i + 1) % count);
      else if (e.key === "ArrowUp" && count) setSelected((i) => (i - 1 + count) % count);
      else if ((e.key === "Enter" || e.key === "Tab") && !e.shiftKey && count) complete(items()[selected()]);
      else if (e.key === "Escape") setDismissed(text());
      else return false;
      e.preventDefault();
      return true;
    },
  };
}

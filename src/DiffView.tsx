// A read-only diff (ticket 16): the Manual editor's text against the file on disk, a file's change
// in the Git drawer, an Agent's proposed edit. Laid out like GitHub's: unified by default, with the
// old and new line numbers in gutters; side-by-side pairs each removed run with the added run after
// it, each side with its own numbers.
import { createSignal, For, type JSX, onCleanup, Show, untrack } from "solid-js";
import { Dynamic } from "solid-js/web";
import type { DiffLine, DiffSide } from "./core";
import { highlightCode } from "./highlight";

const SIGN = { context: " ", added: "+", removed: "−" } as const;

type Kind = keyof typeof SIGN;
/** A line with its numbers in the old and new text (each side's only where it has the line). */
type Numbered = { kind: Kind; text: string; old: number | null; now: number | null };
type Cell = { kind: Kind; text: string; no: number | null } | null;
type Row = { hunk: string } | { left: Cell; right: Cell };

/** The lines with their old and new line numbers, counted from the hunk headers. */
function numbered(lines: DiffLine[]): (Numbered | { hunk: string })[] {
  let old: number | null = null;
  let now: number | null = null;
  const next = (n: number | null) => (n === null ? null : n + 1);
  return lines.map((line) => {
    if (line.kind === "hunk") {
      const m = /^@@ -(\d+)(?:,\d+)? \+(\d+)/.exec(line.text);
      old = m ? Number(m[1]) : null;
      now = m ? Number(m[2]) : null;
      return { hunk: line.text };
    }
    const row: Numbered = {
      kind: line.kind,
      text: line.text,
      old: line.kind === "added" ? null : old,
      now: line.kind === "removed" ? null : now,
    };
    if (line.kind !== "added") old = next(old);
    if (line.kind !== "removed") now = next(now);
    return row;
  });
}

/** The unified lines as side-by-side rows. */
function sideBySide(lines: DiffLine[]): Row[] {
  const rows: Row[] = [];
  let removed: Numbered[] = [];
  let added: Numbered[] = [];
  const flush = () => {
    for (let i = 0; i < Math.max(removed.length, added.length); i++) {
      const left = removed[i];
      const right = added[i];
      rows.push({
        left: left ? { kind: "removed", text: left.text, no: left.old } : null,
        right: right ? { kind: "added", text: right.text, no: right.now } : null,
      });
    }
    removed = [];
    added = [];
  };
  for (const line of numbered(lines)) {
    if ("hunk" in line) {
      flush();
      rows.push(line);
    } else if (line.kind === "removed") {
      if (added.length) flush(); // (a removal after additions starts a new pair)
      removed.push(line);
    } else if (line.kind === "added") added.push(line);
    else {
      flush();
      rows.push({ left: { kind: "context", text: line.text, no: line.old }, right: { kind: "context", text: line.text, no: line.now } });
    }
  }
  flush();
  return rows;
}

/** A diff with its head: what the two sides are, Unified / Side by side, and "Edit file". */
export function DiffPanel(props: {
  legend: JSX.Element;
  lines: DiffLine[];
  language: string | undefined;
  sideBySide: boolean;
  onSideBySide: (on: boolean) => void;
  /** Said above the diff (a rename with no changes, say). */
  note?: string | null;
  /** Said instead of a diff with no lines. */
  empty?: string;
  editTitle: string;
  editDisabled?: boolean;
  onEdit: () => void;
}) {
  return (
    <>
      <div class="diff-head">
        <span class="grow muted">{props.legend} (read-only)</span>
        <button class="ghost" classList={{ on: !props.sideBySide }} onClick={() => props.onSideBySide(false)}>
          Unified
        </button>
        <button class="ghost" classList={{ on: props.sideBySide }} onClick={() => props.onSideBySide(true)}>
          Side by side
        </button>
        <button disabled={props.editDisabled} onClick={() => props.onEdit()} title={props.editTitle}>
          Edit file
        </button>
      </div>
      <Show when={props.note}>{(note) => <div class="editor-banner muted">{note()}</div>}</Show>
      <DiffView lines={props.lines} sideBySide={props.sideBySide} language={props.language} empty={props.empty} />
    </>
  );
}

/** Lines picked in a diff: `line` alone, or `startLine` to `line`, on one side. */
export interface DiffSelection {
  side: DiffSide;
  startLine: number | null;
  line: number;
}

export function DiffView(props: {
  lines: DiffLine[];
  sideBySide: boolean;
  language: string | undefined;
  empty?: string;
  /** Lines can be picked by their numbers: a click, then shift-click or a drag for a range (on one
   *  side; in the unified layout the side of the number column first clicked). */
  selectable?: boolean;
  /** The lines shown picked; without this, the last ones picked. */
  selection?: DiffSelection | null;
  /** Lines were picked; `rect` is the last line's, for a menu beside it. */
  onSelect?: (selection: DiffSelection & { rect: DOMRect }) => void;
  /** Shown across the diff under a line (threads, comments): called once per line on each side it
   *  has, untracked, so it should return JSX that updates itself (empty when there's nothing). */
  inline?: (side: DiffSide, line: number) => JSX.Element | undefined;
  /** Shown in a line's number column (a marker), called likewise. */
  gutter?: (side: DiffSide, line: number) => JSX.Element | undefined;
}) {
  // (These don't change for a diff's life, and leaving them out renders exactly as before.)
  const selectable = !!props.selectable;
  const inline = props.inline;
  const gutter = props.gutter;
  const code = (text: string) => <span class="code" innerHTML={highlightCode(text, props.language)} />;
  const no = (n: number | null, side?: DiffSide) => (
    <span class="no" data-side={selectable ? side : undefined}>
      {gutter && side && n !== null && untrack(() => gutter(side, n))}
      {n ?? ""}
    </span>
  );
  const below = (side: DiffSide, n: number | null) =>
    inline && n !== null && <div class="diff-inline wide">{untrack(() => inline(side, n))}</div>;

  // Picking lines: a drag in progress, and (when the caller doesn't hold it) the last pick.
  const [drag, setDrag] = createSignal<{ side: DiffSide; anchor: number; at: number } | null>(null);
  const [picked, setPicked] = createSignal<DiffSelection | null>(null);
  let lastLine: HTMLElement | null = null;
  const range = (side: DiffSide, a: number, b: number): DiffSelection => ({ side, startLine: a === b ? null : Math.min(a, b), line: Math.max(a, b) });
  const shownSelection = () => {
    const d = drag();
    if (d) return range(d.side, d.anchor, d.at);
    return props.selection !== undefined ? props.selection : picked();
  };
  const isPicked = (side: DiffSide, n: number | null) => {
    const sel = shownSelection();
    return !!sel && n !== null && sel.side === side && n >= (sel.startLine ?? sel.line) && n <= sel.line;
  };
  /** The line number on `side` of the line `el` is in, and that line's element. */
  const lineOf = (el: EventTarget | null, side: DiffSide) => {
    const line = el instanceof Element ? el.closest<HTMLElement>(".line") : null;
    const n = line?.dataset[side === "left" ? "l" : "r"];
    return line && n !== undefined ? { line, n: Number(n) } : null;
  };
  const finish = (side: DiffSide, anchor: number, at: number) => {
    const sel = range(side, anchor, at);
    setPicked(sel);
    if (lastLine) props.onSelect?.({ ...sel, rect: lastLine.getBoundingClientRect() });
  };
  const onMouseDown = (e: MouseEvent) => {
    if (e.button !== 0 || !(e.target instanceof Element)) return;
    const column = e.target.closest<HTMLElement>(".no[data-side]");
    if (!column) return;
    const clicked = column.dataset.side as DiffSide;
    // (A removed line has no new number, an added one no old: the side it has, then.)
    const side = lineOf(column, clicked) ? clicked : clicked === "left" ? "right" : "left";
    const at = lineOf(column, side);
    if (!at) return;
    e.preventDefault();
    lastLine = at.line;
    const current = shownSelection();
    if (e.shiftKey && current && current.side === side) return finish(side, current.startLine ?? current.line, at.n);
    setDrag({ side, anchor: at.n, at: at.n });
  };
  const onMouseOver = (e: MouseEvent) => {
    const d = drag();
    const at = d && lineOf(e.target, d.side);
    if (!d || !at) return;
    lastLine = at.line;
    setDrag({ ...d, at: at.n });
  };
  const onMouseUp = () => {
    const d = drag();
    if (!d) return;
    setDrag(null);
    finish(d.side, d.anchor, d.at);
  };
  if (selectable) {
    window.addEventListener("mouseup", onMouseUp);
    onCleanup(() => window.removeEventListener("mouseup", onMouseUp));
  }
  const mark = (side: DiffSide, n: number | null | undefined, on: DiffSide) => (selectable && side === on ? (n ?? undefined) : undefined);

  const cell = (c: Cell, side: DiffSide) => (
    <span
      class={`line ${c?.kind ?? "empty"}`}
      classList={selectable ? { picked: isPicked(side, c?.no ?? null) } : undefined}
      data-l={mark(side, c?.no, "left")}
      data-r={mark(side, c?.no, "right")}
    >
      {no(c?.no ?? null, side)}
      <span class="sign">{c ? SIGN[c.kind] : ""}</span>
      {c && code(c.text)}
    </span>
  );
  return (
    <Show
      when={props.lines.length > 0}
      fallback={<div class="center muted editor-placeholder">{props.empty ?? "No differences: your text is what's on disk."}</div>}
    >
      <Show
        when={props.sideBySide}
        fallback={
          // (A div when lines have rows under them, which a pre can't hold.)
          <Dynamic
            component={inline ? "div" : "pre"}
            class="diff diff-view"
            classList={{ selectable }}
            onMouseDown={selectable ? onMouseDown : undefined}
            onMouseOver={selectable ? onMouseOver : undefined}
          >
            <For each={numbered(props.lines)}>
              {(line) =>
                "hunk" in line ? (
                  <span class="line hunk">
                    {no(null)}
                    {no(null)}
                    <span class="sign" />
                    {line.hunk}
                  </span>
                ) : (
                  <>
                    <span
                      class={`line ${line.kind}`}
                      classList={selectable ? { picked: isPicked("left", line.old) || isPicked("right", line.now) } : undefined}
                      data-l={selectable ? (line.old ?? undefined) : undefined}
                      data-r={selectable ? (line.now ?? undefined) : undefined}
                    >
                      {no(line.old, "left")}
                      {no(line.now, "right")}
                      <span class="sign">{SIGN[line.kind]}</span>
                      {code(line.text)}
                    </span>
                    {below("left", line.old)}
                    {below("right", line.now)}
                  </>
                )
              }
            </For>
          </Dynamic>
        }
      >
        <div
          class="diff diff-view side-by-side"
          classList={{ selectable }}
          onMouseDown={selectable ? onMouseDown : undefined}
          onMouseOver={selectable ? onMouseOver : undefined}
        >
          <For each={sideBySide(props.lines)}>
            {(row) =>
              "hunk" in row ? (
                <span class="line hunk wide">
                  {no(null)}
                  <span class="sign" />
                  {row.hunk}
                </span>
              ) : (
                <>
                  {cell(row.left, "left")}
                  {cell(row.right, "right")}
                  {below("left", row.left?.no ?? null)}
                  {below("right", row.right?.no ?? null)}
                </>
              )
            }
          </For>
        </div>
      </Show>
    </Show>
  );
}

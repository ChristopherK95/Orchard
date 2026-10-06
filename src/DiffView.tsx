// A read-only diff (ticket 16): the Manual editor's text against the file on disk, a file's change
// in the Git drawer, an Agent's proposed edit. Laid out like GitHub's: unified by default, with the
// old and new line numbers in gutters; side-by-side pairs each removed run with the added run after
// it, each side with its own numbers.
import { For, type JSX, Show } from "solid-js";
import type { DiffLine } from "./core";
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

export function DiffView(props: { lines: DiffLine[]; sideBySide: boolean; language: string | undefined; empty?: string }) {
  const code = (text: string) => <span class="code" innerHTML={highlightCode(text, props.language)} />;
  const no = (n: number | null) => <span class="no">{n ?? ""}</span>;
  const cell = (c: Cell) => (
    <span class={`line ${c?.kind ?? "empty"}`}>
      {no(c?.no ?? null)}
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
          <pre class="diff diff-view">
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
                  <span class={`line ${line.kind}`}>
                    {no(line.old)}
                    {no(line.now)}
                    <span class="sign">{SIGN[line.kind]}</span>
                    {code(line.text)}
                  </span>
                )
              }
            </For>
          </pre>
        }
      >
        <div class="diff diff-view side-by-side">
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
                  {cell(row.left)}
                  {cell(row.right)}
                </>
              )
            }
          </For>
        </div>
      </Show>
    </Show>
  );
}

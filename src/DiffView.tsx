// A read-only diff (ticket 16): the Manual editor's text against the file on disk. Unified by
// default; side-by-side pairs each removed run with the added run after it.
import { For, Show } from "solid-js";
import type { DiffLine } from "./core";
import { highlightCode } from "./highlight";

const PREFIX = { hunk: "", context: " ", added: "+", removed: "−" } as const;

type Cell = { kind: "context" | "added" | "removed"; text: string } | null;
type Row = { hunk: string } | { left: Cell; right: Cell };

/** The unified lines as side-by-side rows. */
function sideBySide(lines: DiffLine[]): Row[] {
  const rows: Row[] = [];
  let removed: DiffLine[] = [];
  let added: DiffLine[] = [];
  const flush = () => {
    for (let i = 0; i < Math.max(removed.length, added.length); i++) {
      const left = removed[i];
      const right = added[i];
      rows.push({
        left: left ? { kind: "removed", text: left.text } : null,
        right: right ? { kind: "added", text: right.text } : null,
      });
    }
    removed = [];
    added = [];
  };
  for (const line of lines) {
    if (line.kind === "removed") {
      if (added.length) flush(); // (a removal after additions starts a new pair)
      removed.push(line);
    } else if (line.kind === "added") added.push(line);
    else {
      flush();
      if (line.kind === "hunk") rows.push({ hunk: line.text });
      else rows.push({ left: { kind: "context", text: line.text }, right: { kind: "context", text: line.text } });
    }
  }
  flush();
  return rows;
}

export function DiffView(props: { lines: DiffLine[]; sideBySide: boolean; language: string | undefined }) {
  const code = (text: string) => <span innerHTML={highlightCode(text, props.language)} />;
  const cell = (c: Cell) => (
    <span class={`line ${c?.kind ?? "empty"}`}>
      {c ? PREFIX[c.kind] : " "}
      {c && code(c.text)}
    </span>
  );
  return (
    <Show
      when={props.lines.length > 0}
      fallback={<div class="center muted editor-placeholder">No differences: your text is what's on disk.</div>}
    >
      <Show
        when={props.sideBySide}
        fallback={
          <pre class="diff diff-view">
            <For each={props.lines}>
              {(line) =>
                line.kind === "hunk" ? (
                  <span class="line hunk">{line.text}</span>
                ) : (
                  <span class={`line ${line.kind}`}>
                    {PREFIX[line.kind]}
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
                <span class="line hunk wide">{row.hunk}</span>
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

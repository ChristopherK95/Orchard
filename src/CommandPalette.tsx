// Ctrl+P (ticket 13): fuzzy-find a file in the active Worktree (typos allowed), plus the commands to
// start a new Agent session here or in a fresh Worktree. Arrow keys move, Enter picks, Esc closes.
import { createEffect, createSignal, For, on, onCleanup, onMount, Show } from "solid-js";
import { core, type FileMatch } from "./core";

export type PaletteCommand = { kind: "newSessionHere" } | { kind: "newSessionInFreshWorktree" };

type Item = { kind: "command"; command: PaletteCommand; label: string } | { kind: "file"; file: FileMatch };

const COMMANDS: { command: PaletteCommand; label: string }[] = [
  { command: { kind: "newSessionInFreshWorktree" }, label: "New Agent session in a fresh worktree" },
  { command: { kind: "newSessionHere" }, label: "New Agent session in this worktree" },
];

/** How many files are shown. */
const LIMIT = 50;

export function CommandPalette(props: {
  worktree: string;
  /** Bumped when the Worktree's files change (or its first index is ready): search again. */
  revision: number;
  onFile: (path: string) => void;
  onCommand: (command: PaletteCommand) => void;
  onClose: () => void;
}) {
  const [query, setQuery] = createSignal("");
  const [files, setFiles] = createSignal<FileMatch[]>([]);
  const [selected, setSelected] = createSignal(0);
  let input!: HTMLInputElement;
  let asked = 0;

  const items = (): Item[] => {
    const words = query().trim().toLowerCase();
    const commands = COMMANDS.filter((c) => !words || c.label.toLowerCase().includes(words)).map(
      (c) => ({ kind: "command", ...c }) as Item,
    );
    return [...commands, ...files().map((file) => ({ kind: "file", file }) as Item)];
  };

  const search = async (text: string) => {
    setQuery(text);
    setSelected(0);
    const mine = ++asked;
    try {
      const found = await core.findFiles(props.worktree, text, LIMIT);
      if (mine === asked) setFiles(found); // (a later keystroke's answer wins)
    } catch {
      if (mine === asked) setFiles([]);
    }
  };

  const pick = (item: Item | undefined) => {
    if (!item) return;
    props.onClose();
    if (item.kind === "command") props.onCommand(item.command);
    else props.onFile(item.file.path);
  };

  onMount(() => {
    input.focus();
    void search("");
  });
  createEffect(on(() => props.revision, () => void search(query()), { defer: true }));
  const onKey = (e: KeyboardEvent) => e.key === "Escape" && props.onClose();
  window.addEventListener("keydown", onKey);
  onCleanup(() => window.removeEventListener("keydown", onKey));

  return (
    <div class="modal-backdrop" onClick={(e) => e.target === e.currentTarget && props.onClose()}>
      <div class="modal palette" role="dialog" aria-label="Go to file">
        <input
          ref={input}
          class="palette-input"
          placeholder="Go to file, or start an Agent session…"
          value={query()}
          onInput={(e) => void search(e.currentTarget.value)}
          onKeyDown={(e) => {
            const count = items().length;
            if (e.key === "ArrowDown") {
              e.preventDefault();
              setSelected((i) => (count ? (i + 1) % count : 0));
            } else if (e.key === "ArrowUp") {
              e.preventDefault();
              setSelected((i) => (count ? (i - 1 + count) % count : 0));
            } else if (e.key === "Enter") {
              e.preventDefault();
              pick(items()[selected()]);
            }
          }}
          spellcheck={false}
        />
        <div class="palette-list">
          <For each={items()} fallback={<p class="muted small palette-empty">No matching files.</p>}>
            {(item, i) => (
              <button
                class="palette-item"
                classList={{ on: i() === selected() }}
                ref={(row) => createEffect(() => i() === selected() && row.scrollIntoView({ block: "nearest" }))}
                onMouseEnter={() => setSelected(i())}
                onClick={() => pick(item)}
              >
                <Show when={item.kind === "file" ? item.file : null} fallback={<span>✦ {(item as { label: string }).label}</span>}>
                  {(file) => <Highlighted text={file().path} indices={file().indices} />}
                </Show>
              </button>
            )}
          </For>
        </div>
      </div>
    </div>
  );
}

/** The path with its matched characters bold, and its file name brighter than its folder. */
function Highlighted(props: { text: string; indices: number[] }) {
  const parts = () => {
    const marked = new Set(props.indices);
    const slash = props.text.lastIndexOf("/");
    const out: { text: string; hit: boolean; dir: boolean }[] = [];
    // Offsets are bytes; paths here are mostly ASCII, so walk UTF-8 to be exact.
    let byte = 0;
    for (const ch of props.text) {
      const hit = marked.has(byte);
      const dir = byte <= slash;
      const last = out.at(-1);
      if (last && last.hit === hit && last.dir === dir) last.text += ch;
      else out.push({ text: ch, hit, dir });
      byte += new TextEncoder().encode(ch).length;
    }
    return out;
  };
  return (
    <span class="mono">
      <For each={parts()}>{(p) => <span classList={{ hit: p.hit, muted: p.dir && !p.hit }}>{p.text}</span>}</For>
    </span>
  );
}

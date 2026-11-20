// The file list of a Review (ticket 42) and of a Code review (issue 44): each changed file with its
// Viewed tick, how many have been viewed, and a filter box. Also the J/K/V(/Esc) keys both share.
import { children, createSignal, For, onCleanup, onMount, Show, type JSX } from "solid-js";
import type { BaseChange } from "./core";
import { Check, ChevronDown, ChevronRight, Loader } from "./icons";

export const LETTER = { added: "A", modified: "M", deleted: "D", renamed: "R" } as const;

export function splitPath(path: string) {
  const slash = path.lastIndexOf("/");
  return { name: path.slice(slash + 1), dir: slash < 0 ? "" : path.slice(0, slash + 1) };
}

/** What the list needs of a file. */
export interface ListedFile {
  path: string;
  change: BaseChange["change"];
  renamedFrom: string | null;
}

/** A folder of the file tree: its entries in the order the files first appear in the list, so the
 *  J/K order of the files stays the order they are drawn in. */
interface Dir<F> {
  path: string;
  name: string;
  entries: (Dir<F> | F)[];
}

function isDir<F>(entry: Dir<F> | F): entry is Dir<F> {
  return "entries" in (entry as object);
}

function buildTree<F extends ListedFile>(files: F[]): Dir<F> {
  const root: Dir<F> = { path: "", name: "", entries: [] };
  const dirs = new Map<string, Dir<F>>([["", root]]);
  for (const file of files) {
    const parts = file.path.split("/");
    let parent = root;
    for (let i = 1; i < parts.length; i++) {
      const path = parts.slice(0, i).join("/");
      let dir = dirs.get(path);
      if (!dir) {
        dir = { path, name: parts[i - 1], entries: [] };
        dirs.set(path, dir);
        parent.entries.push(dir);
      }
      parent = dir;
    }
    parent.entries.push(file);
  }
  return root;
}

export function ReviewFiles<F extends ListedFile>(props: {
  files: F[];
  /** The index (into `files`) of the file shown. */
  current: number;
  isViewed: (file: F) => boolean;
  onSelect: (index: number) => void;
  /** Clicking a file's tick unticks it; without this the tick is only shown. */
  onToggleViewed?: (file: F) => void;
  /** Viewed out of how many: the bar at the top. Not shown when `head` is given. */
  progress?: { viewed: number; total: number };
  /** In place of the bar (a commit the list is narrowed to, say). */
  head?: JSX.Element;
  /** Viewed ticks aren't shown (a commit's files are only part of each file's change). */
  hideViewed?: boolean;
  /** The files are still being read: a spinner in place of the list. */
  loading?: boolean;
  /** What an empty list says. */
  empty: string;
  /** Shown after a file's name (an uncommitted dot, a count of comments). */
  extra?: (file: F) => JSX.Element;
  /** Under the list (the commits). */
  children?: JSX.Element;
}) {
  const [filter, setFilter] = createSignal("");
  const shown = () => {
    const query = filter().trim().toLowerCase();
    return query ? props.files.filter((file) => file.path.toLowerCase().includes(query) || !!file.renamedFrom?.toLowerCase().includes(query)) : props.files;
  };
  const tree = () => buildTree(shown());
  // Folders the user collapsed, by path. A filter shows every match, so it ignores these.
  const [collapsed, setCollapsed] = createSignal<ReadonlySet<string>>(new Set());
  const toggle = (path: string) =>
    setCollapsed((set) => {
      const next = new Set(set);
      if (!next.delete(path)) next.add(path);
      return next;
    });
  const isOpen = (path: string) => !!filter().trim() || !collapsed().has(path);
  // (Resolved once: reading `props.head` again would make its elements again.)
  const head = children(() => props.head);

  const TreeEntries = (p: { dir: Dir<F>; depth: number }): JSX.Element => (
    <For each={p.dir.entries}>
      {(entry) =>
        isDir(entry) ? (
          <>
            <button class="review-dir" style={{ "padding-left": `${8 + p.depth * 12}px` }} onClick={() => toggle(entry.path)} aria-expanded={isOpen(entry.path)}>
              {isOpen(entry.path) ? <ChevronDown class="review-dir-chevron" /> : <ChevronRight class="review-dir-chevron" />}
              <span class="review-dir-name">{entry.name}</span>
            </button>
            <Show when={isOpen(entry.path)}>
              <TreeEntries dir={entry} depth={p.depth + 1} />
            </Show>
          </>
        ) : (
          <FileRow file={entry} depth={p.depth} />
        )
      }
    </For>
  );
  const FileRow = (p: { file: F; depth: number }): JSX.Element => {
    const file = p.file;
    // (Its place in `files`, which `current` and `onSelect` speak in, not in the tree.)
    const index = () => props.files.indexOf(file);
    return (
      <button
        class="review-file"
        classList={{ on: index() === props.current, viewed: !props.hideViewed && props.isViewed(file) }}
        style={{ "padding-left": `${8 + p.depth * 12}px` }}
        onClick={() => props.onSelect(index())}
        title={file.renamedFrom ? `${file.renamedFrom} → ${file.path}` : file.path}
      >
        <span class={`change change-${file.change}`}>{LETTER[file.change]}</span>
        <span class="review-file-name">{splitPath(file.path).name}</span>
        {props.extra?.(file)}
        <Show when={!props.hideViewed && props.isViewed(file)}>
          <Check
            class="viewed-check"
            onClick={(e: MouseEvent) => {
              if (!props.onToggleViewed) return;
              e.stopPropagation();
              props.onToggleViewed(file);
            }}
          />
        </Show>
      </button>
    );
  };
  return (
    <nav class="review-files">
      <Show when={head()} fallback={<Show when={props.progress}>{(p) => <Progress viewed={p().viewed} total={p().total} />}</Show>}>
        {head()}
      </Show>
      <Show when={props.files.length > 1 || filter()}>
        <input class="review-filter" placeholder="Filter files" value={filter()} onInput={(e) => setFilter(e.currentTarget.value)} spellcheck={false} />
      </Show>
      <Show
        when={!props.loading}
        fallback={
          <p class="muted small center">
            <Loader class="spin" />
          </p>
        }
      >
        <Show when={shown().length} fallback={<p class="muted small center">{filter() && props.files.length ? "No file matches the filter." : props.empty}</p>}>
          <TreeEntries dir={tree()} depth={0} />
        </Show>
      </Show>
      {props.children}
    </nav>
  );
}

function Progress(props: { viewed: number; total: number }) {
  return (
    <div class="review-progress">
      <span>
        {props.viewed} / {props.total} viewed
      </span>
      <span class="review-bar">
        <span style={{ width: `${props.total ? (100 * props.viewed) / props.total : 0}%` }} />
      </span>
    </div>
  );
}

/** J / → next file, K / ← previous, V ticks Viewed, Esc `close` if given (always, typing or not).
 *  The other keys only while `active`, not typing in a field, and (given a `scope`) while the focus
 *  is inside it. Captured, so they come before the diff's own. */
export function useReviewKeys(keys: {
  next: () => void;
  previous: () => void;
  toggleViewed: () => void;
  close?: (e: KeyboardEvent) => void;
  active?: () => boolean;
  scope?: () => HTMLElement | undefined;
}) {
  const onKey = (e: KeyboardEvent) => {
    if (e.key === "Escape" && keys.close) {
      e.preventDefault();
      keys.close(e);
      return;
    }
    if ((keys.active && !keys.active()) || e.ctrlKey || e.metaKey || e.altKey) return;
    if (keys.scope && !keys.scope()?.contains(document.activeElement)) return;
    const typing = e.target instanceof HTMLElement && e.target.closest("input, textarea, select");
    if (typing) return;
    if (e.key === "j" || e.key === "ArrowRight") (e.preventDefault(), keys.next());
    else if (e.key === "k" || e.key === "ArrowLeft") (e.preventDefault(), keys.previous());
    else if (e.key === "v") (e.preventDefault(), keys.toggleViewed());
  };
  onMount(() => window.addEventListener("keydown", onKey, true));
  onCleanup(() => window.removeEventListener("keydown", onKey, true));
}

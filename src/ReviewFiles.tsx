// The file list of a Review (ticket 42) and of a Code review (issue 44): each changed file with its
// Viewed tick, how many have been viewed, and a filter box. Also the J/K/V/Esc keys both share.
import { children, createSignal, For, onCleanup, onMount, Show, type JSX } from "solid-js";
import type { BaseChange } from "./core";
import { Check, Loader } from "./icons";

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
  // (Resolved once: reading `props.head` again would make its elements again.)
  const head = children(() => props.head);
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
        <For each={shown()} fallback={<p class="muted small center">{filter() && props.files.length ? "No file matches the filter." : props.empty}</p>}>
          {(file) => {
            // (Its place in `files`, which `current` and `onSelect` speak in, not in the filtered list.)
            const index = () => props.files.indexOf(file);
            return (
              <button
                class="review-file"
                classList={{ on: index() === props.current, viewed: !props.hideViewed && props.isViewed(file) }}
                onClick={() => props.onSelect(index())}
                title={file.renamedFrom ? `${file.renamedFrom} → ${file.path}` : file.path}
              >
                <span class={`change change-${file.change}`}>{LETTER[file.change]}</span>
                <span class="review-file-name">
                  {splitPath(file.path).name}
                  <span class="dir">{splitPath(file.path).dir}</span>
                </span>
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
          }}
        </For>
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

/** J / → next file, K / ← previous, V ticks Viewed, Esc `close` (always, typing or not). The other
 *  keys only while `active` and not typing in a field. Captured, so they come before the diff's own. */
export function useReviewKeys(keys: {
  next: () => void;
  previous: () => void;
  toggleViewed: () => void;
  close: (e: KeyboardEvent) => void;
  active?: () => boolean;
}) {
  const onKey = (e: KeyboardEvent) => {
    if (e.key === "Escape") {
      e.preventDefault();
      keys.close(e);
      return;
    }
    if ((keys.active && !keys.active()) || e.ctrlKey || e.metaKey || e.altKey) return;
    const typing = e.target instanceof HTMLElement && e.target.closest("input, textarea, select");
    if (typing) return;
    if (e.key === "j" || e.key === "ArrowRight") (e.preventDefault(), keys.next());
    else if (e.key === "k" || e.key === "ArrowLeft") (e.preventDefault(), keys.previous());
    else if (e.key === "v") (e.preventDefault(), keys.toggleViewed());
  };
  onMount(() => window.addEventListener("keydown", onKey, true));
  onCleanup(() => window.removeEventListener("keydown", onKey, true));
}

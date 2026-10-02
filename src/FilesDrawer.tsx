// The Files drawer (ticket 13): the active Worktree's files as a tree, folders opened on demand,
// changed files marked with their git status. It re-reads the open folders when the core says the
// Worktree's files changed.
import { createEffect, createResource, createSignal, Index, on, Show } from "solid-js";
import { core, type DirEntry } from "./core";

export function FilesDrawer(props: {
  worktree: string;
  /** Bumped when the Worktree's files change, to re-read what's open. */
  revision: number;
  /** A file to reveal (opening its folders) and mark; `n` changes each time it's asked for. */
  reveal: { path: string; n: number } | null;
  onClose: () => void;
}) {
  const [open, setOpen] = createSignal<Set<string>>(new Set());
  // Opening the folders of a file to reveal.
  createEffect(
    on(
      () => props.reveal,
      (reveal) => {
        if (!reveal) return;
        const folders = reveal.path.split("/").slice(0, -1);
        setOpen((now) => {
          const next = new Set(now);
          folders.forEach((_, i) => next.add(folders.slice(0, i + 1).join("/")));
          return next;
        });
      },
    ),
  );
  // A different Worktree starts closed.
  createEffect(on(() => props.worktree, () => setOpen(new Set<string>()), { defer: true }));
  const toggle = (path: string) =>
    setOpen((now) => {
      const next = new Set(now);
      if (!next.delete(path)) next.add(path);
      return next;
    });

  return (
    <aside class="files-drawer">
      <div class="drawer-head">
        <b>Files</b>
        <span class="grow" />
        <button class="ghost" onClick={() => props.onClose()} title="Close (Ctrl+Shift+E)">
          ×
        </button>
      </div>
      <div class="drawer-tree">
        <Folder worktree={props.worktree} dir="" depth={0} open={open()} revision={props.revision} reveal={props.reveal?.path ?? null} onToggle={toggle} />
      </div>
    </aside>
  );
}

function Folder(props: {
  worktree: string;
  dir: string;
  depth: number;
  open: Set<string>;
  revision: number;
  reveal: string | null;
  onToggle: (path: string) => void;
}) {
  const [entries] = createResource(
    () => [props.worktree, props.dir, props.revision] as const,
    ([worktree, dir]) => core.listDir(worktree, dir).catch(() => [] as DirEntry[]),
  );
  // Rows by position (not by object, which is new on every re-read), so a change in the Worktree
  // updates them in place instead of remounting open folders.
  return (
    <Index each={entries.latest ?? []}>
      {(entry) => (
        <>
          <button
            class="tree-row"
            classList={{ revealed: entry().path === props.reveal, changed: !!entry().change || entry().hasChanges }}
            style={{ "padding-left": `${8 + props.depth * 14}px` }}
            onClick={() => entry().isDir && props.onToggle(entry().path)}
            ref={(row) => createEffect(() => entry().path === props.reveal && row.scrollIntoView({ block: "nearest" }))}
            title={entry().path}
          >
            <span class="tree-icon">{entry().isDir ? (props.open.has(entry().path) ? "▾" : "▸") : ""}</span>
            <span class="tree-name">{entry().name}</span>
            <Show when={entry().change} fallback={<Show when={entry().hasChanges}><span class="change-dot">•</span></Show>}>
              {(change) => <span class={`change change-${changeKind(change())}`}>{change()}</span>}
            </Show>
          </button>
          <Show when={entry().isDir && props.open.has(entry().path)}>
            <Folder {...props} dir={entry().path} depth={props.depth + 1} />
          </Show>
        </>
      )}
    </Index>
  );
}

function changeKind(code: string): string {
  if (code === "??" || code.includes("A")) return "added";
  if (code.includes("D")) return "deleted";
  return "modified";
}

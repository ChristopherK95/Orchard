// The Files drawer (ticket 13): the active Worktree's files as a tree, folders opened on demand,
// changed files marked with their git status. It re-reads the open folders when the core says the
// Worktree's files changed.
import { createEffect, createResource, createSignal, Index, on, Show } from "solid-js";
import { core, type DirEntry } from "./core";
import { ChevronDown, ChevronRight, FileIcon, Folder as FolderIcon, FolderOpen, GitBranch, Pin, PinOff, X } from "./icons";

/** The Columns view's "Pin to this Worktree": the drawer stays on one Worktree instead of following focus. */
export type DrawerPin = { pinned: boolean; onToggle: () => void };

/** The drawer's head, shared by its two tabs: whose files these are, Files / Git, and close. */
export function DrawerHead(props: {
  /** The Worktree's branch (or label) and colour. */
  label: string;
  colour: string;
  tab: "files" | "git";
  onTab: (tab: "files" | "git") => void;
  pin?: DrawerPin;
  onClose: () => void;
}) {
  return (
    <div class="drawer-head" style={{ "--c": props.colour }}>
      <GitBranch />
      <span class="drawer-branch">{props.label}</span>
      <div class="segmented">
        <button classList={{ on: props.tab === "files" }} onClick={() => props.onTab("files")} title="Files of this Worktree (Ctrl+Shift+E)">
          Files
        </button>
        <button classList={{ on: props.tab === "git" }} onClick={() => props.onTab("git")} title="Git: stage, commit, discard (Ctrl+Shift+G)">
          Git
        </button>
      </div>
      <Show when={props.pin}>
        {(pin) => (
          <button
            class="ghost icon"
            classList={{ on: pin().pinned }}
            onClick={() => pin().onToggle()}
            title={pin().pinned ? "Pinned to this Worktree: follow the focused column again" : "Pin to this Worktree (it follows the focused column)"}
            aria-label={pin().pinned ? "Follow the focused column" : "Pin to this Worktree"}
          >
            {pin().pinned ? <Pin /> : <PinOff />}
          </button>
        )}
      </Show>
      <button class="ghost icon" onClick={() => props.onClose()} title="Close the drawer" aria-label="Close the drawer">
        <X />
      </button>
    </div>
  );
}

export function FilesDrawer(props: {
  worktree: string;
  /** Bumped when the Worktree's files change, to re-read what's open. */
  revision: number;
  /** A file to reveal (opening its folders) and mark; `n` changes each time it's asked for. */
  reveal: { path: string; n: number } | null;
  /** A file was clicked (its path relative to the Worktree). */
  onOpenFile: (path: string) => void;
  label: string;
  colour: string;
  pin?: DrawerPin;
  onGit: () => void;
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
      <DrawerHead label={props.label} colour={props.colour} tab="files" onTab={(tab) => tab === "git" && props.onGit()} pin={props.pin} onClose={props.onClose} />
      <div class="drawer-tree">
        <Folder worktree={props.worktree} dir="" depth={0} open={open()} revision={props.revision} reveal={props.reveal?.path ?? null} onToggle={toggle} onOpenFile={props.onOpenFile} />
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
  onOpenFile: (path: string) => void;
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
            style={{ "padding-left": `${8 + props.depth * 16}px` }}
            onClick={() => (entry().isDir ? props.onToggle(entry().path) : props.onOpenFile(entry().path))}
            ref={(row) => createEffect(() => entry().path === props.reveal && row.scrollIntoView({ block: "nearest" }))}
            title={entry().path}
          >
            <Show when={entry().isDir} fallback={<span class="spacer" />}>
              {props.open.has(entry().path) ? <ChevronDown class="chev" /> : <ChevronRight class="chev" />}
            </Show>
            <Show when={entry().isDir} fallback={<FileIcon class="kind" />}>
              {props.open.has(entry().path) ? <FolderOpen class="kind" /> : <FolderIcon class="kind" />}
            </Show>
            <span class="tree-name">{entry().name}</span>
            <Show when={entry().change} fallback={<Show when={entry().hasChanges}><span class="change-dot" title="Changes inside" /></Show>}>
              {(change) => <span class={`change change-${changeKind(change())}`}>{letter(change())}</span>}
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

/** One letter for a status code (`??` is U, untracked). */
const letter = (code: string) => (code === "??" ? "U" : code.trim().charAt(0));

function changeKind(code: string): string {
  if (code === "??") return "untracked";
  if (code.includes("A")) return "added";
  if (code.includes("D")) return "deleted";
  return "modified";
}

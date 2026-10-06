// The Manual editor pane (ticket 14): CodeMirror 6 to the right of the chat, with its own file tabs.
// Lezer highlighting loads per language on first use. Find/replace (Ctrl+F), go to line (Ctrl+G),
// soft-wrap, bracket matching, auto-indent (Enter keeps the line's indentation), multiple cursors
// (Ctrl+click, Alt+drag, Ctrl+D); vim with `editor.vim`, where `:w` saves and `:q` closes the file
// tab. One editor view is shared by the tabs (each keeps its own state, undo history included),
// which keeps the pane light.
//
// Every save says what it may write over: the version the file was read at. If it changed on disk
// since, the core refuses and the pane asks before overwriting. The editor works in "\n" text; the
// core puts the file's own line endings back. Files over ~5 MB are read-only and unhighlighted,
// binaries and non-UTF-8 text are a placeholder, minified files soft-wrap.
import { defaultKeymap, history, historyKeymap, indentWithTab } from "@codemirror/commands";
import { bracketMatching, defineLanguageFacet, indentOnInput, Language, syntaxHighlighting } from "@codemirror/language";
import { gotoLine, highlightSelectionMatches, searchKeymap } from "@codemirror/search";
import { Compartment, EditorState, type Extension, type Text } from "@codemirror/state";
import {
  crosshairCursor,
  drawSelection,
  dropCursor,
  EditorView,
  highlightActiveLine,
  highlightActiveLineGutter,
  keymap,
  lineNumbers,
  rectangularSelection,
} from "@codemirror/view";
import { classHighlighter } from "@lezer/highlight";
import { createEffect, createSignal, For, on, onCleanup, onMount, Show } from "solid-js";
import { createStore, produce } from "solid-js/store";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { core, type DiffLine, type OpenedFile, type PermissionRequest, type PoppedOutFile, type SessionId } from "./core";
import { DiffPanel } from "./DiffView";
import { languageOf, languageOfPath, parserFor } from "./highlight";
import { ExternalLink, TextWrap, X } from "./icons";
import { applyDiff, invertDiff, isSettled, proposalKey, watchProposal } from "./proposals";

/** What to open: a file (absolute path), a chat code block as an unsaved snippet, or a file popped
 *  out of the pane (in its new window). */
export type OpenRequest =
  | { kind: "file"; path: string }
  | { kind: "snippet"; code: string; label: string }
  | { kind: "poppedOut"; file: PoppedOutFile }
  /** A read-only diff to review (ticket 21: vs the Base; or a staged or unstaged change): one tab
   *  per `key` (Worktree, file and which change); `name` is the file relative to its Worktree;
   *  `path`, if the file is there, is what "Edit file" opens; `before` and `after` name the sides. */
  | {
      kind: "diff";
      key: string;
      title: string;
      name: string;
      path: string | null;
      before: string;
      after: string;
      renamedFrom: string | null;
      lines: DiffLine[];
    }
  /** A change an Agent asks to make (ticket 38), from its permission card: one tab per card. */
  | { kind: "proposal"; sessionId: SessionId; request: PermissionRequest };

interface Tab {
  id: number;
  title: string;
  /** Absolute and canonical (as the core gave it); none for a snippet, which isn't saved. */
  path: string | null;
  /** The version read (or last saved), sent with the next save. */
  version: string | null;
  /** The file's line ending, put back on save. */
  lineEnding: string;
  /** Text tabs have a state; binaries and too-big files a placeholder. */
  placeholder: string | null;
  /** Something to know about the file (shown while its tab is active). */
  note: string | null;
  readOnly: boolean;
  wrap: boolean;
  /** The text as saved, to tell whether there are unsaved changes. */
  saved: Text | null;
  dirty: boolean;
  /** The file changed on disk (or was deleted) while it had unsaved changes: the banner's showing. */
  onDisk: "changed" | "deleted" | null;
  /** "Keep mine": the next save writes over whatever is on disk. */
  overwrite: boolean;
  /** A read-only diff tab (Changes vs base). */
  review?: Omit<Extract<OpenRequest, { kind: "diff" }>, "kind" | "title">;
  /** A change an Agent asks to make: the file as it is against the result. Once its card is
   *  answered it's `stale`, or closed if it wasn't `touched` (scrolled, clicked, come back to). */
  proposal?: { key: string; file: string; name: string; lines: DiffLine[]; note: string | null; stale: boolean; touched: boolean };
}

type TabFields = Omit<Tab, "id" | "saved" | "dirty" | "onDisk" | "overwrite">;

const vimMode = new Compartment();
const wrapping = new Compartment();
const language = new Compartment();

// The vim keymap is loaded only once vim is switched on.
let vimExtension: Promise<{ ext: () => Extension; defineEx: (name: string, prefix: string, run: () => void) => void }> | undefined;
function loadVim() {
  vimExtension ??= import("@replit/codemirror-vim").then((m) => ({
    ext: () => m.vim(),
    defineEx: (name, prefix, run) => m.Vim.defineEx(name, prefix, run),
  }));
  return vimExtension;
}

const megabytes = (bytes: number) => `${(bytes / 1024 / 1024).toFixed(1)} MB`;

/** A carried selection, kept inside the text it lands in. */
const clampSelection = (s: { anchor: number; head: number }, length: number) => ({
  anchor: Math.min(s.anchor, length),
  head: Math.min(s.head, length),
});

export function ManualEditor(props: {
  /** Open requests in order; each has a fresh `n`, so the same file can be asked for twice. */
  requests: { open: OpenRequest; n: number }[];
  vim: boolean;
  /** In a popped-out window (no pop-out button there). */
  poppedOut?: boolean;
  onEmpty: () => void;
  onError: (message: string) => void;
  /** Whether any file tab has unsaved changes (for closing the window). */
  onDirtyChange?: (dirty: boolean) => void;
  /** Hands the window a way to save every file tab (for "Save and close"). */
  controls?: (controls: { saveAll: () => Promise<boolean> }) => void;
}) {
  const [tabs, setTabs] = createStore<Tab[]>([]);
  const [activeId, setActiveId] = createSignal<number | null>(null);
  /** The tab whose save the core refused (the file changed on disk). */
  const [conflict, setConflict] = createSignal<number | null>(null);
  const [closing, setClosing] = createSignal<number | null>(null);
  const [notice, setNotice] = createSignal("");
  /** The diff shown in place of the editor: the tab's text against the file on disk. */
  const [diff, setDiff] = createSignal<{ tabId: number; lines: DiffLine[] } | null>(null);
  const [sideBySide, setSideBySide] = createSignal(false);
  const states = new Map<number, EditorState>();
  /** Saves in flight, one per tab (a second Ctrl+S waits for the first rather than racing it). */
  const saving = new Map<number, Promise<boolean>>();
  /** Files being opened (by the path asked for), so a double-click opens one tab. */
  const opening = new Set<string>();
  let handled = 0;
  let nextId = 1;
  let host!: HTMLDivElement;
  let view: EditorView | undefined;
  let alive = true;

  const active = () => tabs.find((t) => t.id === activeId());
  const tab = (id: number) => tabs.find((t) => t.id === id);
  const currentState = (id: number) => (activeId() === id && view ? view.state : states.get(id));

  const markDirty = (tabId: number, doc: Text) => {
    const t = tab(tabId);
    if (!t) return;
    const dirty = !!t.saved && !t.saved.eq(doc);
    if (dirty === t.dirty) return;
    setTabs((x) => x.id === tabId, "dirty", dirty);
    // The core tracks what's open, and unsaved, in every window.
    if (t.path) void core.documentChanged(t.path, dirty).catch(() => {});
  };

  /** In a popped-out window: the tab that came with it, kept current in the core (debounced), so
   *  reloading the window loses nothing. */
  let poppedTab: number | null = null;
  let snapshotTimer: ReturnType<typeof setTimeout> | undefined;
  const snapshot = (tabId: number) => {
    if (!props.poppedOut || tabId !== poppedTab) return;
    clearTimeout(snapshotTimer);
    snapshotTimer = setTimeout(() => {
      const t = tab(tabId);
      const state = currentState(tabId);
      if (t?.saved && state) void core.updatePopOut(null, state.doc.toString(), t.saved.toString(), t.version ?? "").catch(() => {});
    }, 500);
  };
  /** Pop-outs in flight (one per tab). */
  const popping = new Set<number>();
  createEffect(() => props.onDirtyChange?.(tabs.some((t) => t.dirty)));

  const extensions = (tabId: number, readOnly: boolean, wrap: boolean): Extension[] => [
    vimMode.of([]),
    lineNumbers(),
    highlightActiveLineGutter(),
    history(),
    drawSelection(),
    dropCursor(),
    EditorState.allowMultipleSelections.of(true),
    indentOnInput(),
    bracketMatching(),
    rectangularSelection(),
    crosshairCursor(),
    highlightActiveLine(),
    highlightSelectionMatches(),
    keymap.of([
      { key: "Mod-s", run: () => (void save(tabId), true), preventDefault: true },
      { key: "Mod-g", run: gotoLine, preventDefault: true },
      ...defaultKeymap,
      ...searchKeymap,
      ...historyKeymap,
      indentWithTab,
    ]),
    wrapping.of(wrap ? EditorView.lineWrapping : []),
    language.of([]),
    syntaxHighlighting(classHighlighter),
    EditorState.readOnly.of(readOnly),
    EditorView.editable.of(!readOnly),
    EditorView.updateListener.of((update) => {
      if (!update.docChanged) return;
      markDirty(tabId, update.state.doc);
      snapshot(tabId);
    }),
  ];

  /** Highlighting for the tab, once its language's grammar has loaded (not for big files). */
  const highlight = async (tabId: number, lang: string | undefined) => {
    if (!lang) return;
    const parser = await parserFor(lang);
    const state = states.get(tabId);
    if (!parser || !state || !alive) return;
    // (A plain Language, as not every grammar is an LR one: markdown's isn't.)
    const effect = language.reconfigure(new Language(defineLanguageFacet(), parser, [], lang));
    if (activeId() === tabId && view) view.dispatch({ effects: effect });
    else states.set(tabId, state.update({ effects: effect }).state);
  };

  const addTab = (
    fields: TabFields,
    text: string | null,
    lang?: string,
    carried?: { savedText: string; selection: { anchor: number; head: number } },
  ) => {
    const id = nextId++;
    const full: Tab = { ...fields, id, saved: null, dirty: false, onDisk: null, overwrite: false };
    if (text !== null) {
      const state = EditorState.create({
        doc: text,
        selection: carried && clampSelection(carried.selection, text.length),
        extensions: extensions(id, fields.readOnly, fields.wrap),
      });
      full.saved = carried ? EditorState.create({ doc: carried.savedText }).doc : state.doc;
      full.dirty = !full.saved.eq(state.doc);
      states.set(id, state);
      if (!fields.readOnly) void highlight(id, lang);
    }
    setTabs(tabs.length, full);
    show(id);
  };

  /** A popped-out file arriving in this window (or coming back to it), with its unsaved text and
   *  cursor. */
  const openPoppedOut = (file: PoppedOutFile) => {
    const title = file.path.split(/[\\/]/).pop() ?? file.path;
    addTab(
      { title, path: file.path, version: file.version, lineEnding: file.lineEnding, placeholder: null, note: null, readOnly: false, wrap: file.wrap },
      file.text,
      languageOfPath(file.path),
      { savedText: file.savedText, selection: { anchor: file.anchor, head: file.cursor } },
    );
    poppedTab = activeId();
  };

  /** Moves a file tab into a window of its own, unsaved changes and cursor included. */
  const popOut = async (tabId: number) => {
    const t = tab(tabId);
    const state = currentState(tabId);
    if (!t?.path || !state || !t.saved || popping.has(tabId)) return;
    popping.add(tabId);
    const range = state.selection.main;
    const sent = state.doc;
    try {
      const label = await core.popOut({
        path: t.path,
        text: sent.toString(),
        savedText: t.saved.toString(),
        cursor: range.head,
        anchor: range.anchor,
        version: t.version ?? "",
        lineEnding: t.lineEnding,
        wrap: t.wrap,
      });
      // Typed while the window was opening: that goes along too.
      const now = currentState(tabId)?.doc;
      if (now && !now.eq(sent)) await core.updatePopOut(label, now.toString(), t.saved.toString(), t.version ?? "");
    } catch (err) {
      return props.onError(String(err));
    } finally {
      popping.delete(tabId);
    }
    close(tabId, true);
  };
  const openFile = async (asked: string) => {
    const existing = tabs.find((t) => t.path === asked);
    if (existing) return show(existing.id);
    if (opening.has(asked)) return;
    opening.add(asked);
    let opened: OpenedFile;
    try {
      // Open in another window (popped out): that window comes forward instead of a second copy.
      if (await core.showWhereOpen(asked)) return;
      opened = await core.readFile(asked);
    } catch (err) {
      props.onError(String(err));
      return;
    } finally {
      opening.delete(asked);
      if (!alive) return;
    }
    // The same file asked for by another spelling of its path: one tab.
    const same = tabs.find((t) => t.path === opened.path);
    if (same) return show(same.id);
    const path = opened.path;
    const title = path.split(/[\\/]/).pop() ?? path;
    const content = opened.content;
    const base = { title, path, version: opened.version, lineEnding: "\n", note: null, wrap: false };
    void core.documentOpened(path, opened.version).catch(() => {});
    if (content.kind !== "text") {
      const placeholder =
        content.kind === "binary"
          ? `Binary file — ${megabytes(content.bytes)}, not shown\nOrchard does not open binary files. Reveal it in your file manager if you need it.`
          : content.kind === "notUtf8"
            ? "This file isn't UTF-8 text, so it can't be shown or edited here."
            : `Too big to open here (${megabytes(content.bytes)}).`;
      return addTab({ ...base, placeholder, readOnly: true }, null);
    }
    addTab(
      {
        ...base,
        title: content.readOnly ? `${title} (read-only)` : title,
        lineEnding: content.lineEnding,
        placeholder: null,
        readOnly: content.readOnly,
        wrap: content.minified,
        note: content.mixedLineEndings
          ? `This file mixes line endings; saving makes them all ${content.lineEnding === "\r\n" ? "CRLF" : "LF"}.`
          : null,
      },
      content.text,
      content.readOnly ? undefined : languageOfPath(path),
    );
  };

  /** A diff to review, in a tab of its own (the same one again if it's open: brought up to date). */
  const openDiff = ({ kind: _, title, ...review }: Extract<OpenRequest, { kind: "diff" }>) => {
    const existing = tabs.find((t) => t.review?.key === review.key);
    if (existing) {
      setTabs((x) => x.id === existing.id, "review", review);
      return show(existing.id);
    }
    addTab({ title, path: null, version: null, lineEnding: "\n", placeholder: null, note: null, readOnly: true, wrap: false, review }, null);
  };

  /** The change a permission card asks for, in a tab of its own (the same one again if it's open). */
  const openProposal = async (sessionId: SessionId, request: PermissionRequest) => {
    const key = proposalKey(sessionId, request.toolCallId);
    const existing = tabs.find((t) => t.proposal?.key === key);
    if (existing) return show(existing.id);
    const { file, diff: card } = request;
    if (!file || !card || opening.has(key)) return;
    opening.add(key);
    watchProposal(sessionId, request.toolCallId);
    let lines = card;
    let note: string | null = null;
    try {
      // No file yet: the card's diff is the whole of it.
      const opened = await core.readFile(file).catch(() => null);
      if (opened) {
        const after = opened.content.kind === "text" ? applyDiff(opened.content.text, card) : null;
        if (after === null) note = "This is the change as the Agent sent it: it couldn't be placed in the file as it is now.";
        else lines = invertDiff(await core.diffWithDisk(opened.path, after));
      }
    } catch (err) {
      props.onError(String(err));
    } finally {
      opening.delete(key);
    }
    if (!alive || isSettled(key)) return; // (answered meanwhile)
    const name = request.target ?? file;
    const proposal = { key, file, name, lines, note, stale: false, touched: false };
    const title = `${file.split(/[\\/]/).pop()} (proposed)`;
    addTab({ title, path: null, version: null, lineEnding: "\n", placeholder: null, note: null, readOnly: true, wrap: false, proposal }, null);
  };
  const touch = (tabId: number) => setTabs((x) => x.id === tabId && !!x.proposal, "proposal", "touched", true);
  // An answered card's tab: out of date, or gone if it wasn't looked at.
  createEffect(() => {
    for (const t of tabs) {
      if (!t.proposal || t.proposal.stale || !isSettled(t.proposal.key)) continue;
      if (t.proposal.touched) setTabs((x) => x.id === t.id, "proposal", "stale", true);
      else close(t.id);
    }
  });

  const openSnippet = (code: string, label: string) =>
    addTab(
      { title: `snippet${label ? `.${label}` : ""}`, path: null, version: null, lineEnding: "\n", placeholder: null, note: null, readOnly: false, wrap: false },
      code,
      languageOf(label),
    );

  /** Shows tab `id` in the shared view, keeping the previous tab's state (and history). */
  const show = (id: number) => {
    const previous = activeId();
    if (view && previous !== null && previous !== id && states.has(previous)) states.set(previous, view.state);
    setActiveId(id);
    const state = states.get(id);
    if (view && state) {
      view.setState(state);
      void syncVim();
      view.focus();
    } else if (host.contains(document.activeElement)) (document.activeElement as HTMLElement).blur(); // (so Y / N answer cards)
  };

  /** Saves the tab (one save at a time per tab); whether it was saved. */
  const save = (tabId: number, overwrite = false): Promise<boolean> => {
    const running = saving.get(tabId);
    if (running) return running;
    const attempt = (async () => {
      const t = tab(tabId);
      if (!t || t.readOnly) return false;
      if (!t.path) {
        setNotice("A snippet isn't saved anywhere; copy what you need into a file.");
        return false;
      }
      const state = currentState(tabId);
      if (!state) return false;
      const sent = state.doc;
      try {
        const outcome = await core.saveFile(
          t.path,
          sent.toString(),
          t.lineEnding,
          overwrite || t.overwrite ? { kind: "anything" } : { kind: "version", version: t.version! },
        );
        if (outcome.kind === "changedOnDisk") {
          setConflict(tabId);
          return false;
        }
        // Anything typed while it saved is still unsaved.
        const now = currentState(tabId)?.doc ?? sent;
        setTabs(
          (x) => x.id === tabId,
          produce((x) => {
            x.version = outcome.version;
            x.saved = sent;
            x.dirty = !sent.eq(now);
            x.overwrite = false;
            x.onDisk = null;
          }),
        );
        if (diff()?.tabId === tabId) setDiff(null);
        void core.documentChanged(t.path, !sent.eq(now)).catch(() => {});
        snapshot(tabId);
        if (conflict() === tabId) setConflict(null);
        return true;
      } catch (err) {
        props.onError(String(err));
        return false;
      }
    })();
    saving.set(tabId, attempt);
    void attempt.finally(() => saving.delete(tabId));
    return attempt;
  };

  const close = (tabId: number, force = false) => {
    const t = tab(tabId);
    if (!t) return;
    if (t.dirty && !force) return void setClosing(tabId);
    setClosing(null);
    if (conflict() === tabId) setConflict(null);
    if (diff()?.tabId === tabId) setDiff(null);
    if (t.path) void core.documentClosed(t.path).catch(() => {});
    const at = tabs.findIndex((x) => x.id === tabId);
    states.delete(tabId);
    setTabs((all) => all.filter((x) => x.id !== tabId));
    if (activeId() !== tabId) return;
    const next = tabs[Math.min(at, tabs.length - 1)];
    if (next) show(next.id);
    else {
      setActiveId(null);
      if (opening.size === 0) props.onEmpty(); // (unless a file is still on its way)
    }
  };

  /** Reads the tab's file again and puts it in the editor (an undoable change, cursor kept). Unless
   *  `dropMine`, it's left alone if typed into meanwhile: the banner asks instead. */
  const reload = async (tabId: number, dropMine = false) => {
    const t = tab(tabId);
    if (!t?.path) return;
    const before = currentState(tabId)?.doc;
    let opened: OpenedFile;
    try {
      opened = await core.readFile(t.path);
    } catch (err) {
      // (Gone meanwhile, most likely: the core says so next.)
      return props.onError(String(err));
    }
    const state = currentState(tabId);
    if (!alive || !tab(tabId)) return;
    const content = opened.content;
    const settle = () => {
      setTabs(
        (x) => x.id === tabId,
        produce((x) => {
          x.version = opened.version;
          x.onDisk = null;
          x.overwrite = false;
        }),
      );
      if (diff()?.tabId === tabId) setDiff(null);
      if (conflict() === tabId) setConflict(null);
      void core.documentOpened(opened.path, opened.version).catch(() => {});
    };
    // A placeholder tab: nothing to put in the editor, just the new version.
    if (!state) return settle();
    if (content.kind !== "text") {
      // Not text any more: the tab keeps its text, and saving it would write over what's there now.
      setTabs((x) => x.id === tabId, "onDisk", "changed");
      return setNotice(`${t.title} isn't text on disk any more; close it and open it again to see what it is.`);
    }
    if (!dropMine && before && !before.eq(state.doc)) {
      setTabs((x) => x.id === tabId, "onDisk", "changed");
      return;
    }
    // The new saved text first, so the change below leaves the tab clean.
    setTabs(
      (x) => x.id === tabId,
      produce((x) => {
        x.saved = EditorState.create({ doc: content.text }).doc;
        x.lineEnding = content.lineEnding;
      }),
    );
    const change = {
      changes: { from: 0, to: state.doc.length, insert: content.text },
      selection: clampSelection(state.selection.main, content.text.length),
    };
    if (activeId() === tabId && view) view.dispatch(change);
    else states.set(tabId, state.update(change).state);
    markDirty(tabId, currentState(tabId)!.doc);
    snapshot(tabId);
    settle();
  };

  /** The core says `path` changed on disk: a clean tab reloads, one with unsaved changes asks. (The
   *  core decided which, but the tab looks again: it may have been typed into since.) An open diff
   *  of it is brought up to date. */
  const changedOnDisk = (path: string, deleted: boolean) => {
    for (const t of tabs.filter((x) => x.path === path)) {
      if (!deleted && !t.dirty) void reload(t.id);
      else {
        setTabs((x) => x.id === t.id, "onDisk", deleted ? "deleted" : "changed");
        if (diff()?.tabId === t.id) void (deleted ? setDiff(null) : showDiff(t.id));
      }
    }
  };

  /** The file is back as the tab has it (the Agent undid its change): nothing to ask any more. */
  const backOnDisk = (path: string) => {
    for (const t of tabs.filter((x) => x.path === path && x.onDisk)) {
      setTabs((x) => x.id === t.id, "onDisk", null);
      if (diff()?.tabId === t.id) setDiff(null);
    }
  };

  /** "Keep mine": the banner goes, and the next save writes over what's on disk. */
  const keepMine = (tabId: number) => {
    setTabs(
      (x) => x.id === tabId,
      produce((x) => {
        x.onDisk = null;
        x.overwrite = true;
      }),
    );
    if (diff()?.tabId === tabId) setDiff(null);
  };

  /** Shows the tab's text against the file on disk. */
  const showDiff = async (tabId: number) => {
    const t = tab(tabId);
    const mine = currentState(tabId)?.doc.toString();
    if (!t?.path || mine === undefined) return;
    try {
      const lines = await core.diffWithDisk(t.path, mine);
      if (alive && tab(tabId)) setDiff({ tabId, lines });
    } catch (err) {
      props.onError(String(err));
    }
  };

  const myWindow = getCurrentWindow().label;
  let stopEvents: (() => void) | undefined;
  void core
    .onEvent((event) => {
      if (!("window" in event) || event.window !== myWindow) return;
      if (event.kind === "documentChangedOnDisk") changedOnDisk(event.path, false);
      else if (event.kind === "documentConflicted") changedOnDisk(event.path, event.deleted);
      else if (event.kind === "documentBackOnDisk") backOnDisk(event.path);
    })
    .then((stop) => (alive ? (stopEvents = stop) : stop()));

  /** Vim on or off in the shown tab, as the setting says (applied live). */
  const syncVim = async () => {
    if (!view) return;
    if (!props.vim) return view.dispatch({ effects: vimMode.reconfigure([]) });
    const vim = await loadVim();
    if (!props.vim || !view || !alive) return; // (switched off again meanwhile)
    vim.defineEx("write", "w", () => void (activeId() !== null && save(activeId()!)));
    vim.defineEx("quit", "q", () => activeId() !== null && close(activeId()!));
    vim.defineEx("wq", "wq", () => {
      const id = activeId();
      if (id !== null) void save(id).then((saved) => saved && close(id));
    });
    view.dispatch({ effects: vimMode.reconfigure(vim.ext()) });
  };
  createEffect(on(() => props.vim, () => void syncVim(), { defer: true }));

  const toggleWrap = () => {
    const t = active();
    if (!t) return;
    const wrap = !t.wrap;
    setTabs((x) => x.id === t.id, "wrap", wrap);
    view?.dispatch({ effects: wrapping.reconfigure(wrap ? EditorView.lineWrapping : []) });
  };

  // Every request, in order (none dropped while the pane was still loading).
  createEffect(
    on(
      () => props.requests.length,
      () => {
        for (const request of props.requests.filter((r) => r.n > handled)) {
          handled = request.n;
          const open = request.open;
          if (open.kind === "file") void openFile(open.path);
          else if (open.kind === "poppedOut") openPoppedOut(open.file);
          else if (open.kind === "diff") openDiff(open);
          else if (open.kind === "proposal") void openProposal(open.sessionId, open.request);
          else openSnippet(open.code, open.label);
        }
      },
    ),
  );

  props.controls?.({
    saveAll: async () => {
      const results = await Promise.all(tabs.filter((t) => t.dirty).map((t) => save(t.id)));
      return results.every(Boolean);
    },
  });

  onMount(() => {
    view = new EditorView({ parent: host, state: states.get(activeId() ?? -1) ?? EditorState.create() });
    const id = activeId();
    if (id !== null) show(id);
  });
  onCleanup(() => {
    alive = false;
    stopEvents?.();
    clearTimeout(snapshotTimer);
    view?.destroy();
  });

  return (
    <section class="manual-editor">
      <div class="editor-tabs">
        <For each={tabs}>
          {(t) => (
            <span class="editor-tab" classList={{ on: t.id === activeId() }}>
              <button
                class="editor-tab-name"
                onClick={() => {
                  if (t.id !== activeId()) touch(t.id);
                  show(t.id);
                }}
                onAuxClick={(e) => e.button === 1 && close(t.id)}
                title={t.review?.name ?? t.proposal?.name ?? t.path ?? "Not saved anywhere"}
              >
                {t.title}
                <Show when={t.dirty}>
                  <span class="dirty" title="Unsaved changes" />
                </Show>
                <Show when={t.onDisk}>
                  <span class="on-disk" title={t.onDisk === "deleted" ? "Deleted on disk" : "Changed on disk"}>
                    !
                  </span>
                </Show>
              </button>
              <button class="close-tab" aria-label={`Close ${t.title}`} onClick={() => close(t.id)}>
                <X />
              </button>
            </span>
          )}
        </For>
        <span class="grow" />
        <Show when={active() && !active()!.placeholder && !active()!.review && !active()!.proposal}>
          <button class="ghost" classList={{ on: !!active()?.wrap }} onClick={toggleWrap} title="Soft-wrap long lines">
            <TextWrap />
            Wrap
          </button>
        </Show>
        <Show when={active()?.path && !active()?.readOnly}>
          <button class="ghost" onClick={() => void save(activeId()!)} title="Save (Ctrl+S)" disabled={!active()?.dirty && !active()?.overwrite}>
            Save
          </button>
          <Show when={!props.poppedOut}>
            <button class="ghost" onClick={() => void popOut(activeId()!)} title="Move this file into a window of its own (its undo history stays behind)">
              <ExternalLink />
              Pop out
            </button>
          </Show>
        </Show>
      </div>
      <Show when={conflict() !== null ? tab(conflict()!) : undefined}>
        {(t) => (
          <div class="editor-banner warning">
            <span class="grow">
              <span class="mono">{t().title}</span> changed on disk since you opened it. Saving would overwrite that.
            </span>
            <button onClick={() => void save(t().id, true)}>Overwrite</button>
            <button class="ghost" onClick={() => setConflict(null)}>
              Cancel
            </button>
          </div>
        )}
      </Show>
      <Show when={active()?.onDisk ? active() : undefined}>
        {(t) => (
          <div class="editor-banner warning">
            <Show
              when={t().onDisk === "changed"}
              fallback={
                <>
                  <span class="grow">
                    <span class="mono">{t().title}</span> was deleted on disk. Saving puts your text back.
                  </span>
                  <button onClick={() => keepMine(t().id)}>Keep mine</button>
                  <button class="ghost" onClick={() => close(t().id)}>
                    Close it
                  </button>
                </>
              }
            >
              <span class="grow">Agent changed this file. Your unsaved changes are still here.</span>
              <button
                classList={{ on: diff()?.tabId === t().id }}
                onClick={() => (diff()?.tabId === t().id ? setDiff(null) : void showDiff(t().id))}
              >
                Show diff
              </button>
              <button onClick={() => void reload(t().id, true)}>Reload (drop mine)</button>
              <button onClick={() => keepMine(t().id)}>Keep mine</button>
            </Show>
          </div>
        )}
      </Show>
      <Show when={closing() !== null ? tab(closing()!) : undefined}>
        {(t) => (
          <div class="editor-banner warning">
            <span class="grow">
              <span class="mono">{t().title}</span> has unsaved changes. Close anyway?
            </span>
            <button onClick={() => close(t().id, true)}>Close without saving</button>
            <button class="ghost" onClick={() => setClosing(null)}>
              Cancel
            </button>
          </div>
        )}
      </Show>
      <Show when={notice()}>
        <div class="editor-banner muted" onClick={() => setNotice("")} title="Click to dismiss">
          {notice()}
        </div>
      </Show>
      <Show when={active()?.note}>{(note) => <div class="editor-banner muted">{note()}</div>}</Show>
      <Show when={active()?.placeholder}>
        {(text) => (
          <div class="center muted editor-placeholder">
            {/* (A second line, if any, is the smaller explanation under it.) */}
            <div>{text().split("\n")[0]}</div>
            <Show when={text().split("\n")[1]}>{(sub) => <div class="small">{sub()}</div>}</Show>
          </div>
        )}
      </Show>
      <Show when={diff()?.tabId === activeId() ? diff() : undefined}>
        {(d) => (
          <DiffPanel
            legend={
              <>
                <span class="removed-key">− yours</span> · <span class="added-key">+ on disk</span>
              </>
            }
            lines={d().lines}
            language={languageOfPath(active()?.path)}
            sideBySide={sideBySide()}
            onSideBySide={setSideBySide}
            editTitle="Back to the editor"
            onEdit={() => setDiff(null)}
          />
        )}
      </Show>
      <Show when={active()?.review}>
        {(review) => (
          <DiffPanel
            legend={
              <>
                <span class="removed-key">− {review().before}</span> · <span class="added-key">+ {review().after}</span>
              </>
            }
            lines={review().lines}
            language={languageOfPath(review().name)}
            empty="No differences in the text."
            sideBySide={sideBySide()}
            onSideBySide={setSideBySide}
            note={
              review().renamedFrom
                ? `Renamed from ${review().renamedFrom}${review().lines.length === 0 ? ", with no changes." : "."}`
                : null
            }
            editTitle={review().path ? "Open the file itself (as it is now, uncommitted changes and all)" : "It's deleted"}
            editDisabled={!review().path}
            onEdit={() => review().path && void openFile(review().path!)}
          />
        )}
      </Show>
      <Show when={active()?.proposal}>
        {(proposal) => (
          <div class="contents" onPointerDown={() => touch(activeId()!)} onWheel={() => touch(activeId()!)}>
            <DiffPanel
              legend={
                <>
                  <span class="removed-key">− as it is</span> · <span class="added-key">+ if you allow it</span>
                </>
              }
              lines={proposal().lines}
              language={languageOfPath(proposal().file)}
              sideBySide={sideBySide()}
              onSideBySide={setSideBySide}
              note={
                proposal().stale
                  ? "Out of date: the card was answered, or the turn ended. This is the change as it was asked for."
                  : proposal().note
              }
              editTitle="Open the file itself (as it is now)"
              onEdit={() => void openFile(proposal().file)}
            />
          </div>
        )}
      </Show>
      <div
        class="editor-host"
        ref={host}
        classList={{ hidden: !!active()?.placeholder || !active() || !!active()?.review || !!active()?.proposal || diff()?.tabId === activeId() }}
      />
    </section>
  );
}

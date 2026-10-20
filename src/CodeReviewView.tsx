// Code review (issue 44): going through a PR the user was asked to review, in its Review Worktree.
// It is the review Agent session's Tab: the PR's files with GitHub's Viewed ticks, each file's diff
// (all of it, or only what changed since the user's last review), the PR's threads under their lines
// with Reply, and the Pending review's comments, added by picking lines, with Claude in a column on
// the right. Submit sends them in one go with Approve, Request changes or Comment. New commits pushed
// meanwhile are offered with Refresh.
import { openUrl } from "@tauri-apps/plugin-opener";
import { batch, createEffect, createMemo, createSignal, For, Match, on, onCleanup, onMount, Show, Switch, type JSX } from "solid-js";
import { createStore, reconcile, unwrap } from "solid-js/store";
import { ago } from "./Chat";
import {
  core,
  setupOver,
  type Attachment,
  type CodeReview,
  type CodeReviewFile,
  type CodeThread,
  type DiffLine,
  type DiffSide,
  type PendingComment,
  type Question,
  type SessionId,
  type SessionInfo,
  type SubmitError,
  type Verdict,
} from "./core";
import { DiffView, type DiffSelection } from "./DiffView";
import { languageOfPath } from "./highlight";
import { ChevronLeft, ChevronRight, CircleCheck, GitPullRequest, Loader, MessageSquare, MoveRight, PanelRight, Pencil, RefreshCw, Send, Sparkles, Trash2, TriangleAlert, X } from "./icons";
import { handleTranscriptClick, renderMarkdown } from "./markdown";
import { LETTER, ReviewFiles, useReviewKeys } from "./ReviewFiles";
import { Setup, type SetupView } from "./Setup";

const plural = (n: number, thing: string) => `${n} ${thing}${n === 1 ? "" : "s"}`;
const markdown = (text: string) => renderMarkdown(text.replace(/<!--[\s\S]*?-->/g, "").trim());

type Loaded = { lines: DiffLine[] } | { error: string };

/** Where Claude is docked in the Code review (ReviewChat): what it's given once the review has an
 *  Agent session. */
export interface ReviewDock {
  number: number;
  worktree: string;
  session: SessionId;
  /** "Ask Claude"'s attachments not taken yet, oldest first. */
  asked: () => Attachment[];
  /** Takes them (into the composer): they're cleared here. */
  takeAsked: () => Attachment[];
  /** Questions about lines, for "Use as comment" and the gutter markers. */
  questions: () => Question[];
  /** "Use as comment": opens a comment box on the question's lines with `body` in it. */
  draftComment: (question: Question, body: string) => void;
  /** The question whose gutter marker was clicked last (set again on each click): scrolled to. */
  question: () => Question | null;
  /** Collapses the dock. */
  onCollapse: () => void;
}

/** What a Code review's Tab had when it was last shown, by PR: switching Tabs and back finds it
 *  as it was (and doesn't fetch the PR again). */
interface Kept {
  review: CodeReview;
  threads: CodeThread[];
  since: boolean;
  at: number;
  sideBySide: boolean;
  /** How far the diff was scrolled. */
  scrollTop: number;
  dockOpen: boolean;
}
const kept = new Map<number, Kept>();

/** Openings under way, by PR: the way in and the Tab it shows ask once between them. */
const opening = new Map<number, Promise<CodeReview>>();

/** Opens PR `number`'s Code review (`core.openCodeReview`), keeping what it says for its Tab. */
export function openCodeReview(number: number): Promise<CodeReview> {
  const already = opening.get(number);
  if (already) return already;
  const open = core
    .openCodeReview(number)
    .then((r) => {
      const before = kept.get(number);
      kept.set(number, before ? { ...before, review: r } : { review: r, threads: [], since: !!r.sinceLastReview, at: 0, sideBySide: false, scrollTop: 0, dockOpen: dockPrefs().open });
      return r;
    })
    .finally(() => opening.delete(number));
  opening.set(number, open);
  return open;
}

/** Claude's column: its width and whether it's open, the same for every Code review. */
const DOCK_KEY = "orchard.codeReview.dock";
const DOCK_WIDTH = 520;
const DOCK_MIN = 380;
function dockPrefs(): { width: number; open: boolean } {
  try {
    const saved = JSON.parse(localStorage.getItem(DOCK_KEY) ?? "{}");
    return { width: typeof saved.width === "number" ? saved.width : DOCK_WIDTH, open: saved.open !== false };
  } catch {
    return { width: DOCK_WIDTH, open: true };
  }
}
const saveDockPrefs = (prefs: { width: number; open: boolean }) => localStorage.setItem(DOCK_KEY, JSON.stringify(prefs));

/** What GitHub's (or the core's) refusal of a submit says, whatever form it came in. */
function submitError(err: unknown): SubmitError {
  if (err && typeof err === "object" && "kind" in err && "message" in err) return err as SubmitError;
  return { kind: "other", message: String(err), paths: [], count: 0 };
}

const lines = (c: { side: DiffSide; startLine: number | null; line: number }) =>
  `${c.startLine !== null && c.startLine !== c.line ? `lines ${c.startLine}–${c.line}` : `line ${c.line}`}${c.side === "left" ? " (old)" : ""}`;

export function CodeReviewView(props: {
  number: number;
  /** The review Agent session, whose Tab this is; none until the Review Worktree's setup is over. */
  session: SessionInfo | undefined;
  /** Its Tab is the one shown: only then do J / K / V work. */
  active: boolean;
  colourOf: (worktree: string) => string;
  /** The Review Worktree's setup as the core's events tell it (fresher than what opening said). */
  setupOf: (worktree: string) => SetupView | undefined;
  onOpenSettings: () => void;
  /** "Remove Worktree" after Submit: the removal dialog for the Review Worktree. */
  onRemoveWorktree: (worktree: string) => void;
  /** Claude's column (ReviewChat), once the review has its session. */
  chat: (dock: ReviewDock) => JSX.Element;
  /** A gutter marker was clicked: the question about those lines (the column opens and scrolls to it either way). */
  onQuestion?: (question: Question) => void;
}) {
  const before = kept.get(props.number);
  const [review, setReview] = createSignal<CodeReview | null>(null);
  const [loadError, setLoadError] = createSignal("");
  /** Said under the header: a failure (V, a reply, Refresh), or what just happened. */
  const [notice, setNotice] = createSignal<{ text: string; error?: boolean; action?: { label: string; run: () => void } } | null>(null);
  const fail = (what: string) => (err: unknown) => setNotice({ text: `${what}: ${err}`, error: true });
  /** Only the changes since the user's last review (a re-requested review), or the PR's whole diff. */
  const [since, setSince] = createSignal(before?.since ?? false);
  const [at, setAt] = createSignal(before?.at ?? 0);
  const [sideBySide, setSideBySide] = createSignal(before?.sideBySide ?? false);
  const [diffs, setDiffs] = createStore<Record<string, Loaded>>({});
  /** Viewed ticks changed here since the files were read, by path. */
  const [viewedNow, setViewedNow] = createStore<Record<string, boolean>>({});
  const [threads, setThreads] = createSignal<CodeThread[]>([]);
  const [pending, setPending] = createStore<{ comments: PendingComment[]; questions: Question[] }>({ comments: [], questions: [] });
  const [newCommits, setNewCommits] = createSignal(0);
  const [refreshing, setRefreshing] = createSignal(false);

  const loadThreads = (r: CodeReview) =>
    void core
      .pullRequestDetails(r.worktree, r.number)
      .then((d) => {
        setThreads(d?.threads ?? []);
        if (d?.threadsError) setNotice({ text: `Couldn't read the PR's comment threads: ${d.threadsError}`, error: true });
      })
      .catch(fail("Couldn't read the PR's comment threads"));
  const apply = (r: CodeReview) =>
    batch(() => {
      setReview(r);
      setPending(reconcile({ comments: r.pending.comments, questions: r.pending.questions }));
      setDiffs(reconcile({}));
      setViewedNow(reconcile({}));
      setAt((i) => Math.min(i, Math.max(0, files().length - 1)));
    });
  onMount(async () => {
    if (before) {
      // (Pending comments and Viewed ticks as they were here: `kept` has them up to date.)
      batch(() => {
        setReview(before.review);
        setPending(reconcile({ comments: before.review.pending.comments, questions: before.review.pending.questions }));
        setThreads(before.threads);
      });
      if (!before.threads.length) loadThreads(before.review);
      return;
    }
    try {
      const r = await openCodeReview(props.number);
      setSince(!!r.sinceLastReview);
      apply(r);
      loadThreads(r);
    } catch (err) {
      setLoadError(String(err));
    }
  });
  onCleanup(() => {
    const r = review();
    if (!r) return;
    const ticked = (files: CodeReviewFile[]) => files.map((f) => ({ ...f, viewed: isViewed(f) }));
    const saved: CodeReview = {
      ...r,
      files: ticked(r.files),
      sinceLastReview: r.sinceLastReview && { ...r.sinceLastReview, files: ticked(r.sinceLastReview.files) },
      pending: structuredClone(unwrap(pending)),
    };
    kept.set(props.number, { review: saved, threads: threads(), since: since(), at: at(), sideBySide: sideBySide(), scrollTop, dockOpen: dockOpen() });
  });

  // New commits pushed to the PR: asked every minute (quietly: offline is no news).
  const poll = window.setInterval(() => {
    if (review() && !refreshing()) core.codeReviewNewCommits(props.number).then(setNewCommits, () => {});
  }, 60_000);
  onCleanup(() => clearInterval(poll));
  const refresh = async () => {
    setRefreshing(true);
    try {
      const r = await core.refreshCodeReview(props.number);
      apply(r);
      setNewCommits(0);
      loadThreads(r);
      const outdated = r.pending.comments.filter((c) => c.outdated).length;
      setNotice({ text: outdated ? `Moved to the PR's latest commits. ${plural(outdated, "comment")} lost ${outdated === 1 ? "its line" : "their lines"}: move or remove ${outdated === 1 ? "it" : "them"} (at the top of the file).` : "Moved to the PR's latest commits." });
    } catch (err) {
      fail("Couldn't refresh")(err);
    } finally {
      setRefreshing(false);
    }
  };

  const files = (): CodeReviewFile[] => {
    const r = review();
    return (since() && r?.sinceLastReview?.files) || r?.files || [];
  };
  const current = () => files()[at()] as CodeReviewFile | undefined;
  const setScope = (onlySince: boolean) => {
    if (onlySince === since()) return;
    const path = current()?.path;
    leaveLines();
    setSince(onlySince);
    const i = files().findIndex((f) => f.path === path);
    setAt(i < 0 ? 0 : i);
  };
  const keyOf = (file: CodeReviewFile) => `${since() ? "since" : "all"}\n${file.path}`;
  const diffOf = (file: CodeReviewFile) => diffs[keyOf(file)];
  const fetchDiff = (file: CodeReviewFile | undefined) => {
    if (!file || !review() || diffOf(file)) return;
    const key = keyOf(file);
    core
      .codeReviewDiff(props.number, file.path, since())
      .then((lines) => setDiffs(key, { lines }))
      .catch((err) => setDiffs(key, { error: String(err) }));
  };
  // The file shown, and the next one ahead of time.
  createEffect(() => {
    fetchDiff(current());
    fetchDiff(files()[at() + 1]);
  });
  /** Another file: what was being picked or written on this one's lines goes (a comment box's text too). */
  const leaveLines = () =>
    batch(() => {
      setMenuAt(null);
      setDrafting(null);
      setSelection(null);
      setMoving(null);
    });
  const go = (i: number) => {
    const to = Math.max(0, Math.min(files().length - 1, i));
    if (to !== at()) leaveLines();
    setAt(to);
  };
  let diffBox: HTMLDivElement | undefined;
  /** The diff's scroller (DiffView's), and how far down it is. */
  const scroller = () => diffBox?.querySelector<HTMLElement>(".diff-view") ?? undefined;
  let scrollTop = before?.scrollTop ?? 0;
  /** Where the diff goes back to once it has loaded again (the Tab shown again). */
  let restoreTop = scrollTop;
  createEffect(
    on(
      current,
      () => {
        restoreTop = 0;
        scrollTop = 0;
        scroller()?.scrollTo({ top: 0 });
      },
      { defer: true },
    ),
  );
  createEffect(() => {
    const file = current();
    const loaded = file && diffOf(file);
    if (!loaded || !("lines" in loaded) || !restoreTop) return;
    const top = restoreTop;
    restoreTop = 0;
    requestAnimationFrame(() => scroller()?.scrollTo({ top }));
  });
  // (A scroll doesn't bubble: it's caught on its way down.)
  const onDiffScroll = () => {
    scrollTop = scroller()?.scrollTop ?? 0;
    if (menuAt()) closeMenu();
  };

  // Viewed ticks are GitHub's: set there, and put back here if that fails.
  const isViewed = (file: CodeReviewFile) => viewedNow[file.path] ?? file.viewed;
  const toggleViewed = (file: CodeReviewFile) => {
    const on = !isViewed(file);
    setViewedNow(file.path, on);
    core.setReviewFileViewed(props.number, file.path, on).catch((err) => {
      setViewedNow(file.path, !on);
      fail(`Couldn't ${on ? "tick" : "untick"} ${file.path} Viewed`)(err);
    });
  };
  const viewedCount = () => files().filter(isViewed).length;

  // What goes under the diff's lines: the PR's threads and the Pending review's comments, by
  // `side:line`. The since-last-review diff's old side is the last review's, not the merge base's
  // the comments were made on, so only its new side has them.
  const shows = (side: DiffSide) => !since() || side === "right";
  const fileThreads = createMemo(() => threads().filter((t) => t.path === current()?.path));
  const fileComments = createMemo(() => pending.comments.filter((c) => c.path === current()?.path));
  const placed = createMemo(() => {
    const map = new Map<string, { threads: CodeThread[]; comments: PendingComment[] }>();
    const at = (side: DiffSide, line: number) => {
      const key = `${side}:${line}`;
      let here = map.get(key);
      if (!here) map.set(key, (here = { threads: [], comments: [] }));
      return here;
    };
    for (const t of fileThreads()) if (!t.outdated && t.line !== null && shows(t.side)) at(t.side, t.line).threads.push(t);
    for (const c of fileComments()) if (!c.outdated && shows(c.side)) at(c.side, c.line).comments.push(c);
    return map;
  });
  const outdatedComments = () => fileComments().filter((c) => c.outdated);
  const outdatedThreads = () => fileThreads().filter((t) => t.outdated || t.line === null);
  const commentsIn = (path: string) => pending.comments.filter((c) => c.path === path).length;

  // Picking lines: the menu beside them, then a comment box under them (one at a time).
  const [selection, setSelection] = createSignal<DiffSelection | null>(null);
  const [menuAt, setMenuAt] = createSignal<{ x: number; y: number } | null>(null);
  const [drafting, setDrafting] = createSignal<(DiffSelection & { body?: string; fromQuestion?: boolean }) | null>(null);
  const [editing, setEditing] = createSignal<number | null>(null);
  const [replyingTo, setReplyingTo] = createSignal<number | null>(null);
  /** An outdated comment waiting for the lines it's moved to. */
  const [moving, setMoving] = createSignal<PendingComment | null>(null);
  const closeMenu = () => {
    setMenuAt(null);
    if (!drafting()) setSelection(null);
  };
  const onPicked = (sel: DiffSelection & { rect: DOMRect }) => {
    const picked = { side: sel.side, startLine: sel.startLine, line: sel.line };
    if (!shows(sel.side)) {
      setNotice({ text: "Here the old side is your last review's: comment on the old side in the full diff." });
      return setSelection(null);
    }
    const m = moving();
    if (m) return void moveTo(m, picked);
    setDrafting(null);
    setSelection(picked);
    setMenuAt({ x: sel.rect.left + 100, y: sel.rect.bottom + 2 });
  };
  // A click anywhere but the menu closes it (a click on a line number picks again; a shift-click
  // extends what's picked, so that stays).
  const onDown = (e: MouseEvent) => {
    if (!menuAt() || (e.target instanceof Element && e.target.closest(".cr-menu"))) return;
    if (e.shiftKey) setMenuAt(null);
    else closeMenu();
  };
  window.addEventListener("mousedown", onDown, true);
  onCleanup(() => window.removeEventListener("mousedown", onDown, true));

  const startComment = () => {
    const sel = selection();
    setMenuAt(null);
    if (sel) setDrafting(sel);
  };
  const addComment = async (body: string) => {
    const d = drafting();
    const file = current();
    if (!d || !file) return;
    const added = await core.addReviewComment(props.number, { path: file.path, side: d.side, startLine: d.startLine, line: d.line, body, fromQuestion: d.fromQuestion });
    setPending("comments", (all) => [...all, added]);
    setDrafting(null);
    setSelection(null);
  };
  const editComment = async (c: PendingComment, body: string) => {
    await core.editReviewComment(props.number, c.id, body);
    setPending("comments", (x) => x.id === c.id, "body", body);
    setEditing(null);
  };
  const removeComment = (c: PendingComment) =>
    core
      .removeReviewComment(props.number, c.id)
      .then(() => setPending("comments", (all) => all.filter((x) => x.id !== c.id)))
      .catch(fail("Couldn't remove the comment"));
  /** Moves an outdated comment: the same words on the lines picked, then the old one goes. */
  const moveTo = async (c: PendingComment, to: DiffSelection) => {
    setMoving(null);
    setSelection(null);
    try {
      const added = await core.addReviewComment(props.number, { path: c.path, side: to.side, startLine: to.startLine, line: to.line, body: c.body, fromQuestion: c.fromQuestion });
      await core.removeReviewComment(props.number, c.id);
      setPending("comments", (all) => [...all.filter((x) => x.id !== c.id), added]);
      setNotice(null);
    } catch (err) {
      fail("Couldn't move the comment")(err);
    }
  };
  const reply = async (t: CodeThread, body: string) => {
    const first = t.comments[0]?.id;
    if (first == null) return;
    const posted = await core.replyToThread(props.number, first, body);
    setThreads((all) => all.map((x) => (x === t ? { ...x, comments: [...x.comments, posted] } : x)));
    setReplyingTo(null);
  };

  // Claude: the review session, in its column on the right.
  const session = (): SessionId | null => props.session?.id ?? null;
  const [dockOpen, setDockOpenNow] = createSignal(before?.dockOpen ?? dockPrefs().open);
  const setDockOpen = (open: boolean) => {
    setDockOpenNow(open);
    saveDockPrefs({ ...dockPrefs(), open });
  };
  const [asked, setAsked] = createSignal<Attachment[]>([]);
  const [shownQuestion, setShownQuestion] = createSignal<Question | null>(null, { equals: false });
  const [claudeBusy, setClaudeBusy] = createSignal(false);
  const claudeReview = async () => {
    setClaudeBusy(true);
    try {
      await core.claudeReview(props.number);
      setDockOpen(true);
    } catch (err) {
      fail("Couldn't start Claude's review")(err);
    } finally {
      setClaudeBusy(false);
    }
  };
  const askClaude = async () => {
    const sel = selection();
    const file = current();
    setMenuAt(null);
    setSelection(null);
    if (!sel || !file) return;
    try {
      const attachment = await core.askAttachment(props.number, file.path, sel.side, sel.startLine, sel.line);
      const question: Question = { path: file.path, side: sel.side, startLine: sel.startLine, line: sel.line, attachment: attachment.name };
      await core.addReviewQuestion(props.number, question);
      setPending("questions", (all) => [...all, question]);
      setAsked((all) => [...all, attachment]);
      // (Its session starts here if the setup is still running.)
      if (session() === null) await core.codeReviewSession(props.number);
      setDockOpen(true);
    } catch (err) {
      fail("Couldn't ask Claude")(err);
    }
  };
  /** A gutter marker: the dock opens on the message that asked it. */
  const showQuestion = async (q: Question) => {
    props.onQuestion?.(q);
    try {
      if (session() === null) await core.codeReviewSession(props.number);
      setDockOpen(true);
      setShownQuestion(q);
    } catch (err) {
      fail("Couldn't open the review session")(err);
    }
  };
  /** The question whose first line this is, for its gutter marker. */
  const questionAt = (side: DiffSide, line: number) => pending.questions.find((q) => q.path === current()?.path && q.side === side && (q.startLine ?? q.line) === line);

  // Submit.
  const [submitOpen, setSubmitOpen] = createSignal(false);
  const [verdict, setVerdict] = createSignal<Verdict>("comment");
  const [summary, setSummary] = createSignal("");
  const [submitting, setSubmitting] = createSignal(false);
  const [refused, setRefused] = createSignal<SubmitError | null>(null);
  /** The files GitHub's refusal named (or with outdated comments): their comments are marked. */
  const [failedPaths, setFailedPaths] = createSignal<string[]>([]);
  /** Submitted: then whether the Review Worktree can go (nothing in it would be lost). */
  const [submitted, setSubmitted] = createSignal<{ removable: boolean } | null>(null);
  const submit = async () => {
    const r = review();
    if (!r || submitting()) return;
    setSubmitting(true);
    setRefused(null);
    try {
      await core.submitReview(props.number, verdict(), summary());
      setPending(reconcile({ comments: [], questions: [] }));
      setFailedPaths([]);
      const check = await core.removalCheck(r.worktree).catch(() => null);
      const removable = !!check && !check.discardToRemove && check.sessions.every((id) => id === check.reviewSession);
      setSubmitted({ removable });
    } catch (err) {
      const e = submitError(err);
      setRefused(e);
      if (e.kind === "submitFailed") setFailedPaths(e.paths);
      else if (e.kind === "outdatedComments") setFailedPaths([...new Set(pending.comments.filter((c) => c.outdated).map((c) => c.path))]);
    } finally {
      setSubmitting(false);
    }
  };

  // Setup: the Review Worktree's runs once; the review itself doesn't wait for it.
  const setup = (): SetupView | undefined => {
    const r = review();
    return r ? (props.setupOf(r.worktree) ?? r.setup ?? undefined) : undefined;
  };
  const settingUp = () => {
    const s = setup();
    return s && !setupOver(s.status) ? s : undefined;
  };
  const [showSetup, setShowSetup] = createSignal(false);

  // Only while the file list or the diff has the focus (not Claude's composer, nor another Tab).
  let reviewPane: HTMLDivElement | undefined;
  useReviewKeys({
    next: () => go(at() + 1),
    previous: () => go(at() - 1),
    toggleViewed: () => current() && toggleViewed(current()!),
    active: () => props.active && !submitOpen(),
    scope: () => reviewPane,
  });
  // Shown, the review takes the keys (once it's there to take them).
  createEffect(() => {
    if (props.active && review() && !reviewPane?.contains(document.activeElement)) reviewPane?.focus({ preventScroll: true });
  });

  // Claude's column: dragged wider or narrower by its left edge (380 px to 60% of the view).
  let root: HTMLDivElement | undefined;
  const [dockWidth, setDockWidth] = createSignal(dockPrefs().width);
  const [dragging, setDragging] = createSignal(false);
  const [viewWidth, setViewWidth] = createSignal(window.innerWidth);
  onMount(() => {
    if (!root) return;
    const watch = new ResizeObserver(() => root && setViewWidth(root.clientWidth));
    watch.observe(root);
    onCleanup(() => watch.disconnect());
  });
  const clampWidth = (width: number) => Math.round(Math.max(DOCK_MIN, Math.min(width, viewWidth() * 0.6)));
  const resizeDock = (e: PointerEvent) => {
    const handle = e.currentTarget as HTMLElement;
    handle.setPointerCapture(e.pointerId);
    const start = e.clientX;
    const startWidth = clampWidth(dockWidth());
    setDragging(true);
    const move = (m: PointerEvent) => setDockWidth(clampWidth(startWidth + start - m.clientX));
    const up = () => {
      setDragging(false);
      saveDockPrefs({ ...dockPrefs(), width: dockWidth() });
      handle.removeEventListener("pointermove", move);
      handle.removeEventListener("pointerup", up);
    };
    handle.addEventListener("pointermove", move);
    handle.addEventListener("pointerup", up);
  };

  const onBodyClick = (e: MouseEvent) => void handleTranscriptClick(e, openUrl, () => {}, () => {}, fail("Couldn't open the link"));

  const Thread = (p: { thread: CodeThread }) => {
    const id = () => p.thread.comments[0]?.id ?? null;
    return (
      <div class="pr-thread cr-thread" classList={{ outdated: p.thread.outdated }}>
        <Show when={p.thread.outdated || p.thread.line === null}>
          <div class="pr-thread-head">
            <span class="mono muted">{lines({ side: p.thread.side, startLine: p.thread.startLine, line: p.thread.line ?? p.thread.originalLine ?? 0 })}</span>
            <span class="pr-outdated">Outdated</span>
          </div>
        </Show>
        <For each={p.thread.comments}>
          {(c) => (
            <div class="pr-thread-comment">
              <div class="who">
                <b>{c.author}</b>
                <span class="muted">{ago(c.createdAt)}</span>
              </div>
              <div class="markdown" innerHTML={markdown(c.body)} />
            </div>
          )}
        </For>
        <Show when={id() !== null}>
          <div class="cr-thread-foot">
            <Show
              when={replyingTo() === id()}
              fallback={
                <button class="link" onClick={() => setReplyingTo(id())}>
                  Reply
                </button>
              }
            >
              <CommentBox label="Reply" placeholder="Reply (Markdown): posted to GitHub straight away" onSave={(body) => reply(p.thread, body)} onCancel={() => setReplyingTo(null)} />
            </Show>
          </div>
        </Show>
      </div>
    );
  };

  const Pending = (p: { comment: PendingComment }) => (
    <div class="pr-thread cr-pending" classList={{ failed: failedPaths().includes(p.comment.path) }}>
      <div class="pr-thread-head">
        <span class="pr-outdated">Pending</span>
        <Show when={p.comment.outdated}>
          <span class="pr-outdated">Outdated</span>
        </Show>
        <span class="mono muted">{lines(p.comment)}</span>
        <span class="grow" />
        <Show when={p.comment.outdated}>
          <button class="ghost small" classList={{ on: moving()?.id === p.comment.id }} onClick={() => (setMoving(p.comment), setNotice({ text: "Pick the lines to move the comment to." }))} title="Move it to other lines">
            <MoveRight />
            Move
          </button>
        </Show>
        <Show when={!p.comment.outdated}>
          <button class="ghost icon" onClick={() => setEditing(p.comment.id)} title="Edit" aria-label="Edit">
            <Pencil />
          </button>
        </Show>
        <button class="ghost icon" onClick={() => void removeComment(p.comment)} title="Remove" aria-label="Remove">
          <Trash2 />
        </button>
      </div>
      <Show
        when={editing() === p.comment.id}
        fallback={
          <div class="pr-thread-comment">
            <div class="markdown" innerHTML={markdown(p.comment.body)} />
          </div>
        }
      >
        <div class="pr-thread-comment">
          <CommentBox initial={p.comment.body} label="Save" onSave={(body) => editComment(p.comment, body)} onCancel={() => setEditing(null)} />
        </div>
      </Show>
    </div>
  );

  // Called once per diff line (DiffView's `inline`): updates itself as threads and comments come and go.
  const under = (side: DiffSide, line: number) => {
    const here = () => placed().get(`${side}:${line}`);
    const box = () => {
      const d = drafting();
      return d && d.side === side && d.line === line ? d : null;
    };
    return (
      <>
        <For each={here()?.threads ?? []}>{(t) => <Thread thread={t} />}</For>
        <For each={here()?.comments ?? []}>{(c) => <Pending comment={c} />}</For>
        <Show when={box()}>
          {(d) => (
            <CommentBox
              initial={d().body}
              label="Add to review"
              placeholder={`Comment on ${lines(d())} (Markdown): sent with the review`}
              onSave={addComment}
              onCancel={() => (setDrafting(null), setSelection(null))}
            />
          )}
        </Show>
      </>
    );
  };
  const marker = (side: DiffSide, line: number) => (
    <Show when={questionAt(side, line)}>
      {(q) => (
        <span
          class="diff-marker"
          title={`Asked Claude about ${q().attachment}`}
          onMouseDown={(e) => e.stopPropagation()}
          onClick={(e) => {
            e.stopPropagation();
            void showQuestion(q());
          }}
        >
          <Sparkles />
        </span>
      )}
    </Show>
  );

  const colour = () => (review() ? props.colourOf(review()!.worktree) : "var(--fg-3)");
  const docked = () => {
    const id = session();
    return dockOpen() && id !== null && review() ? id : null;
  };

  return (
    <div class="code-review-view" ref={root} style={{ "--c": colour() }}>
      <div class="cr-pane" ref={reviewPane} tabIndex={-1} aria-label="Code review">
        <div class="modal-head review-head">
          <GitPullRequest class="review-mark" />
          <b>
            Code review
            <span class="review-branch muted">
              <span class="mono">#{props.number}</span>
              <span class="cr-title">{review()?.title}</span>
            </span>
          </b>
          <Show when={review()?.sinceLastReview}>
            {(s) => (
              <div class="segmented" title={`Your last review was on ${s().commit.slice(0, 7)}`}>
                <button classList={{ on: since() }} onClick={() => setScope(true)}>
                  Since last review
                </button>
                <button classList={{ on: !since() }} onClick={() => setScope(false)}>
                  Full diff
                </button>
              </div>
            )}
          </Show>
          <div class="segmented">
            <button classList={{ on: !sideBySide() }} onClick={() => setSideBySide(false)}>
              Unified
            </button>
            <button classList={{ on: sideBySide() }} onClick={() => setSideBySide(true)}>
              Side by side
            </button>
          </div>
          <Show when={newCommits() > 0}>
            <button class="outline" onClick={() => void refresh()} disabled={refreshing()} title="Move the Review Worktree to the PR's latest commits (pending comments follow their lines)">
              <Show when={refreshing()} fallback={<RefreshCw />}>
                <Loader class="spin" />
              </Show>
              {plural(newCommits(), "new commit")} — Refresh
            </button>
          </Show>
          <button onClick={() => void claudeReview()} disabled={!review() || claudeBusy()} title="Ask Claude to review the whole PR in the review session (Plan mode: it changes nothing)">
            <Show when={claudeBusy()} fallback={<Sparkles />}>
              <Loader class="spin" />
            </Show>
            Claude review
          </button>
          <button class="primary" onClick={() => (setSubmitted(null), setSubmitOpen(true))} disabled={!review()} title="Approve, request changes or comment, with the pending comments">
            <Send />
            Submit review
            <Show when={pending.comments.length}>
              <span class="badge">{pending.comments.length}</span>
            </Show>
          </button>
          <Show when={session() !== null}>
            <button
              class="ghost icon"
              classList={{ on: dockOpen() }}
              onClick={() => setDockOpen(!dockOpen())}
              title={dockOpen() ? "Hide Claude (the review session goes on)" : "Show Claude (the review session)"}
              aria-label={dockOpen() ? "Hide Claude" : "Show Claude"}
            >
              <PanelRight />
            </button>
          </Show>
        </div>
        <Show when={settingUp()}>
          {(s) => (
            <p class="git-notice cr-notice" classList={{ warn: s().status.kind === "failed" || s().status.kind === "sessionFailed" }}>
              <Show when={s().status.kind === "failed" || s().status.kind === "sessionFailed"} fallback={<Loader class="spin" />}>
                <TriangleAlert />
              </Show>
              <span class="grow">{s().status.kind === "running" ? "Setting up the Review Worktree…" : "The Review Worktree's setup failed."}</span>
              <button class="link" onClick={() => setShowSetup((on) => !on)}>
                {showSetup() ? "Back to the diff" : "Details"}
              </button>
            </p>
          )}
        </Show>
        <Show when={notice()}>
          {(n) => (
            <p class="git-notice cr-notice" classList={{ error: !!n().error }}>
              <Show when={n().error}>
                <TriangleAlert />
              </Show>
              <span class="grow">{n().text}</span>
              <Show when={n().action}>
                {(a) => (
                  <button class="link" onClick={() => a().run()}>
                    {a().label}
                  </button>
                )}
              </Show>
              <button class="ghost icon" onClick={() => setNotice(null)} aria-label="Dismiss">
                <X />
              </button>
            </p>
          )}
        </Show>
        <Switch>
          <Match when={loadError()}>
            <p class="error modal-error">{loadError()}</p>
          </Match>
          <Match when={!review()}>
            <p class="muted center review-loading">
              <Loader class="spin" /> Opening the Code review (the first time, this fetches the PR and makes its Review Worktree)…
            </p>
          </Match>
          <Match when={review()}>
            {(r) => (
              <>
                <div class="review-body">
                  <ReviewFiles
                    files={files()}
                    current={at()}
                    isViewed={isViewed}
                    onSelect={go}
                    onToggleViewed={toggleViewed}
                    progress={{ viewed: viewedCount(), total: files().length }}
                    empty={since() ? "Nothing changed since your last review." : `No changes against ${r().base}.`}
                    extra={(file) => (
                      <>
                        <Show when={failedPaths().includes(file.path)}>
                          <TriangleAlert class="cr-failed" />
                        </Show>
                        <Show when={commentsIn(file.path)}>
                          <span class="badge" title="Pending comments">
                            {commentsIn(file.path)}
                          </span>
                        </Show>
                      </>
                    )}
                  />
                  <section class="review-main">
                    <Show
                      when={!showSetup() || !settingUp()}
                      fallback={<Setup worktree={r().worktree} setup={settingUp()!} onError={fail("Setup")} onOpenSettings={props.onOpenSettings} />}
                    >
                      <Show when={current()} fallback={<div class="center muted editor-placeholder">{since() ? "Nothing changed since your last review." : "This PR changes no files."}</div>}>
                        {(file) => (
                          <>
                            <div class="review-file-head">
                              <span class={`change change-${file().change}`}>{LETTER[file().change]}</span>
                              <span class="mono grow review-path">
                                <Show when={file().renamedFrom}>{(from) => <span class="muted">{from()} → </span>}</Show>
                                {file().path}
                              </span>
                              <span class="review-stat mono">
                                <span class="added-key">+{file().additions}</span> <span class="removed-key">−{file().deletions}</span>
                              </span>
                              <label class="viewed-toggle" title="Viewed, as GitHub keeps it (V)">
                                <input type="checkbox" checked={isViewed(file())} onChange={() => toggleViewed(file())} />
                                Viewed
                              </label>
                            </div>
                            <Show when={outdatedComments().length || outdatedThreads().length}>
                              <div class="cr-outdated" onClick={onBodyClick}>
                                <For each={outdatedComments()}>{(c) => <Pending comment={c} />}</For>
                                <For each={outdatedThreads()}>{(t) => <Thread thread={t} />}</For>
                              </div>
                            </Show>
                            <div class="review-diff" ref={(el) => (diffBox = el, el.addEventListener("scroll", onDiffScroll, true))} onClick={onBodyClick}>
                              <Switch fallback={<p class="muted center review-loading"><Loader class="spin" /> Loading the diff…</p>}>
                                <Match when={diffOf(file()) && "error" in diffOf(file()) && (diffOf(file()) as { error: string })}>
                                  {(d) => <div class="center muted editor-placeholder">{d().error}</div>}
                                </Match>
                                <Match when={diffOf(file()) && "lines" in diffOf(file()) && (diffOf(file()) as { lines: DiffLine[] })}>
                                  {(d) => (
                                    <DiffView
                                      lines={d().lines}
                                      sideBySide={sideBySide()}
                                      language={languageOfPath(file().path)}
                                      empty={file().change === "renamed" ? "Renamed, with no changes." : "No changes to its text."}
                                      selectable
                                      selection={selection()}
                                      onSelect={onPicked}
                                      inline={under}
                                      gutter={marker}
                                    />
                                  )}
                                </Match>
                              </Switch>
                            </div>
                          </>
                        )}
                      </Show>
                    </Show>
                  </section>
                </div>
                <div class="modal-foot review-foot">
                  <span class="hint">
                    <kbd>J</kbd> / <kbd>K</kbd> next / previous file · <kbd>V</kbd> viewed · click a line number to comment, shift-click or drag for a range
                  </span>
                  <button class="ghost" disabled={at() === 0} onClick={() => go(at() - 1)}>
                    <ChevronLeft />
                    Previous
                  </button>
                  <button disabled={at() >= files().length - 1} onClick={() => go(at() + 1)} title="The next file (J)">
                    Next file
                    <ChevronRight />
                  </button>
                </div>
              </>
            )}
          </Match>
        </Switch>
        <Show when={menuAt()}>
          {(m) => (
            <div class="cr-menu" style={{ left: `${m().x}px`, top: `${m().y}px` }}>
              <button class="ghost" onClick={startComment}>
                <MessageSquare />
                Comment
              </button>
              <button class="ghost" onClick={() => void askClaude()} title="Attach these lines to a question for Claude in the review session">
                <Sparkles />
                Ask Claude
              </button>
            </div>
          )}
        </Show>
        <Show when={submitOpen()}>
          <div class="modal-backdrop" onClick={(e) => e.target === e.currentTarget && !submitting() && setSubmitOpen(false)}>
            <div class="modal cr-submit" role="dialog" aria-label="Submit review">
              <div class="modal-head">
                <b>{submitted() ? "Review submitted" : `Submit your review of #${props.number}`}</b>
                <button class="ghost icon" onClick={() => setSubmitOpen(false)} disabled={submitting()} aria-label="Close" title="Close (Esc)">
                  <X />
                </button>
              </div>
              <Show
                when={submitted()}
                fallback={
                  <>
                    <div class="modal-body">
                      <div class="segmented cr-verdicts">
                        <button classList={{ on: verdict() === "comment" }} onClick={() => setVerdict("comment")} title="Feedback without approving">
                          Comment
                        </button>
                        <button classList={{ on: verdict() === "approve" }} onClick={() => setVerdict("approve")} title="Approve the PR">
                          Approve
                        </button>
                        <button classList={{ on: verdict() === "requestChanges" }} onClick={() => setVerdict("requestChanges")} title="Changes are needed before it can merge">
                          Request changes
                        </button>
                      </div>
                      <label>
                        Summary
                        <textarea
                          class="review-body-text"
                          value={summary()}
                          onInput={(e) => setSummary(e.currentTarget.value)}
                          placeholder="What you think of it overall (Markdown)"
                          ref={(el) => queueMicrotask(() => el.focus())}
                        />
                      </label>
                      <p class="hint">
                        {pending.comments.length ? `${plural(pending.comments.length, "pending comment")} on its lines go with it.` : "No comments on its lines."}
                      </p>
                      <Show when={refused()}>
                        {(e) => (
                          <p class="error small review-pr-error">
                            {e().kind === "outdatedComments"
                              ? `${plural(e().count, "comment")} lost ${e().count === 1 ? "its line" : "their lines"} with the new commits: move or remove ${e().count === 1 ? "it" : "them"} (at the top of the marked files), then submit.`
                              : e().kind === "submitFailed"
                                ? `GitHub didn't take the review:\n${e().message}${e().paths.length ? `\n\nThe marked files' comments: ${e().paths.join(", ")}` : ""}`
                                : e().message}
                          </p>
                        )}
                      </Show>
                    </div>
                    <div class="modal-foot">
                      <button class="ghost" onClick={() => setSubmitOpen(false)} disabled={submitting()}>
                        Cancel
                      </button>
                      <button class="primary" onClick={() => void submit()} disabled={submitting()}>
                        <Show when={submitting()} fallback={<Send />}>
                          <Loader class="spin" />
                        </Show>
                        Submit review
                      </button>
                    </div>
                  </>
                }
              >
                {(done) => (
                  <>
                    <div class="modal-body review-done">
                      <CircleCheck class="done-mark" />
                      <p>
                        Your review is on GitHub.
                        <Show when={done().removable}> The Review Worktree isn't needed any more: nothing in it would be lost.</Show>
                      </p>
                    </div>
                    <div class="modal-foot">
                      <button class={done().removable ? "ghost" : "primary"} onClick={() => setSubmitOpen(false)}>
                        Close
                      </button>
                      <Show when={done().removable}>
                        <button
                          class="primary"
                          onClick={() => {
                            setSubmitOpen(false);
                            props.onRemoveWorktree(review()!.worktree);
                          }}
                        >
                          <Trash2 />
                          Remove Worktree
                        </button>
                      </Show>
                    </div>
                  </>
                )}
              </Show>
            </div>
          </div>
        </Show>
      </div>
      <Show when={review() && session() !== null}>
        <Show
          when={docked()}
          fallback={
            <div class="cr-dock-strip">
              <button class="ghost icon" onClick={() => setDockOpen(true)} title="Show Claude (the review session)" aria-label="Show Claude">
                <Sparkles />
              </button>
            </div>
          }
        >
          {(id) => (
            <>
              <div class="cr-dock-resize" classList={{ dragging: dragging() }} onPointerDown={resizeDock} title="Drag to resize">
                <span class="grip" />
              </div>
              <aside class="cr-dock" style={{ width: `${clampWidth(dockWidth())}px` }}>
                {props.chat({
                  number: props.number,
                  worktree: review()!.worktree,
                  session: id(),
                  asked,
                  takeAsked: () => {
                    const all = asked();
                    setAsked([]);
                    return all;
                  },
                  questions: () => pending.questions,
                  question: shownQuestion,
                  draftComment: (q, body) => {
                    const i = files().findIndex((f) => f.path === q.path);
                    if (i >= 0) setAt(i);
                    const sel = { side: q.side, startLine: q.startLine, line: q.line };
                    setSelection(sel);
                    setDrafting({ ...sel, body, fromQuestion: true });
                  },
                  onCollapse: () => setDockOpen(false),
                })}
              </aside>
            </>
          )}
        </Show>
      </Show>
    </div>
  );
}

/** A comment (or reply) being written: Ctrl+Enter saves, and a failure stays with the text. */
function CommentBox(props: { initial?: string; label: string; placeholder?: string; onSave: (body: string) => Promise<void>; onCancel: () => void }) {
  const [body, setBody] = createSignal(props.initial ?? "");
  const [busy, setBusy] = createSignal(false);
  const [error, setError] = createSignal("");
  const save = async () => {
    if (!body().trim() || busy()) return;
    setBusy(true);
    setError("");
    try {
      await props.onSave(body());
    } catch (err) {
      setError(String(err));
    } finally {
      setBusy(false);
    }
  };
  return (
    <div class="cr-box">
      <textarea
        value={body()}
        onInput={(e) => setBody(e.currentTarget.value)}
        placeholder={props.placeholder}
        ref={(el) => queueMicrotask(() => el.focus())}
        onKeyDown={(e) => {
          if (e.key === "Enter" && (e.ctrlKey || e.metaKey)) {
            e.preventDefault();
            void save();
          }
        }}
      />
      <Show when={error()}>
        <p class="error small">{error()}</p>
      </Show>
      <div class="cr-box-actions">
        <span class="hint">
          <kbd>Ctrl</kbd>+<kbd>Enter</kbd>
        </span>
        <span class="grow" />
        <button class="ghost small" onClick={() => props.onCancel()} disabled={busy()}>
          Cancel
        </button>
        <button class="primary small" onClick={() => void save()} disabled={busy() || !body().trim()}>
          <Show when={busy()}>
            <Loader class="spin" />
          </Show>
          {props.label}
        </button>
      </div>
    </div>
  );
}

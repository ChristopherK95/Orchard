// The Changes side panel of the Tabs view: everything the Worktree's branch changed (as Review
// lists it, committed or not) as a file tree, and the selected file's diff beside the session, kept
// up to date as the Agent works. Lines picked in the diff can be asked about: they go to the
// session's composer as an attachment, like "Ask Claude" in a Code review.
import { createEffect, createSignal, on, onCleanup, Show } from "solid-js";
import { addAttachment } from "./Chat";
import { core, type DiffLine, type Review, type ReviewFile, type SessionId } from "./core";
import { DiffView, type DiffSelection } from "./DiffView";
import { languageOfPath } from "./highlight";
import { GitBranch, Loader, Sparkles, X } from "./icons";
import { ReviewFiles, splitPath } from "./ReviewFiles";

/** A burst of git status events reads the changes once. */
const COALESCE_MS = 300;

export function ChangesPanel(props: {
  worktree: string;
  /** The session the asked-about lines go to (none: they can't be asked). */
  session: SessionId | null;
  label: string;
  colour: string;
  onClose: () => void;
}) {
  const [review, setReview] = createSignal<Review | null>(null);
  const [error, setError] = createSignal("");
  const [path, setPath] = createSignal<string | null>(null);
  const [lines, setLines] = createSignal<DiffLine[] | null>(null);
  const [sideBySide, setSideBySide] = createSignal(false);
  const [selection, setSelection] = createSignal<DiffSelection | null>(null);
  const [menuAt, setMenuAt] = createSignal<{ x: number; y: number } | null>(null);

  const files = () => review()?.files ?? [];
  const current = (): ReviewFile | undefined => files().find((f) => f.path === path()) ?? files()[0];

  // Only the latest request's answer is shown.
  let askedReview = 0;
  const readReview = async () => {
    const n = ++askedReview;
    const worktree = props.worktree;
    try {
      const next = await core.review(worktree);
      if (n !== askedReview || worktree !== props.worktree) return;
      setReview(next);
      setError("");
    } catch (err) {
      if (n === askedReview && worktree === props.worktree) {
        setReview(null);
        setError(String(err));
      }
    }
  };
  let askedDiff = 0;
  const readDiff = async () => {
    const n = ++askedDiff;
    const worktree = props.worktree;
    const file = current();
    const split = review()?.split;
    if (!file || !split) return setLines(null);
    try {
      const next = await core.reviewDiff(worktree, split, file);
      if (n === askedDiff && worktree === props.worktree) setLines(next);
    } catch (err) {
      if (n === askedDiff) setError(String(err));
    }
  };
  createEffect(
    on(
      () => props.worktree,
      () => {
        setReview(null);
        setPath(null);
        setLines(null);
        setSelection(null);
        setMenuAt(null);
        void readReview();
      },
    ),
  );
  // The diff is read again when the file changes or the list does (an Agent's edit).
  createEffect(
    on([() => current()?.path, review], ([p], prev) => {
      if (p !== prev?.[0]) {
        setLines(null);
        setSelection(null);
        setMenuAt(null);
      }
      void readDiff();
    }),
  );

  let due: ReturnType<typeof setTimeout> | undefined;
  let stop: (() => void) | undefined;
  let alive = true;
  onCleanup(() => {
    alive = false;
    clearTimeout(due);
    stop?.();
  });
  void core
    .onEvent((event) => {
      if (event.kind !== "gitStatusChanged" || event.worktree !== props.worktree || due !== undefined) return;
      due = setTimeout(() => {
        due = undefined;
        void readReview();
      }, COALESCE_MS);
    })
    .then((unlisten) => (alive ? (stop = unlisten) : unlisten()));

  const onPicked = (sel: DiffSelection & { rect: DOMRect }) => {
    setSelection({ side: sel.side, startLine: sel.startLine, line: sel.line });
    setMenuAt({ x: sel.rect.left + 100, y: sel.rect.bottom + 2 });
  };
  // A click anywhere but the menu or a line number closes the menu.
  const onDown = (e: MouseEvent) => {
    if (e.target instanceof Element && !e.target.closest(".cr-menu, .no")) setMenuAt(null);
  };
  window.addEventListener("mousedown", onDown, true);
  onCleanup(() => window.removeEventListener("mousedown", onDown, true));

  const ask = async () => {
    const sel = selection();
    const file = current();
    const split = review()?.split;
    const session = props.session;
    setMenuAt(null);
    setSelection(null);
    if (!sel || !file || !split || !session) return;
    try {
      addAttachment(session, await core.askAttachmentInWorktree(props.worktree, split, file, sel.side, sel.startLine, sel.line));
    } catch (err) {
      setError(`Couldn't ask Claude: ${err}`);
    }
  };

  return (
    <aside class="changes-panel">
      <div class="drawer-head" style={{ "--c": props.colour }}>
        <GitBranch />
        <span class="drawer-branch">{props.label}</span>
        <b class="changes-title">Changes</b>
        <button class="ghost icon" onClick={() => props.onClose()} title="Close the changes" aria-label="Close the changes">
          <X />
        </button>
      </div>
      <Show when={error()}>
        <p class="muted small changes-error">{error()}</p>
      </Show>
      <div class="changes-files">
        <ReviewFiles
          files={files()}
          current={Math.max(0, files().indexOf(current()!))}
          isViewed={() => false}
          hideViewed
          loading={review() === null && !error()}
          empty="No changes against the Base yet."
          onSelect={(i) => setPath(files()[i]?.path ?? null)}
          extra={(file) => (file.uncommitted ? <span class="uncommitted-dot" title="Not committed yet" /> : undefined)}
        />
      </div>
      <Show when={current()}>
        {(file) => (
          <>
            <div class="diff-head">
              <span class="grow muted ellipsis" title={file().path}>
                {splitPath(file().path).name}
              </span>
              <button class="ghost" classList={{ on: !sideBySide() }} onClick={() => setSideBySide(false)}>
                Unified
              </button>
              <button class="ghost" classList={{ on: sideBySide() }} onClick={() => setSideBySide(true)}>
                Side by side
              </button>
            </div>
            <div class="changes-diff">
              <Show
                when={lines()}
                fallback={
                  <p class="muted center review-loading">
                    <Loader class="spin" />
                  </p>
                }
              >
                {(l) => (
                  <DiffView
                    lines={l()}
                    sideBySide={sideBySide()}
                    language={languageOfPath(file().path)}
                    empty={file().change === "renamed" ? "Renamed, with no changes." : "No changes to its text."}
                    selectable
                    selection={selection()}
                    onSelect={onPicked}
                  />
                )}
              </Show>
            </div>
            <p class="muted small changes-hint">Click a line number, then shift-click (or drag) for a range, to ask Claude about it.</p>
          </>
        )}
      </Show>
      <Show when={menuAt()}>
        {(m) => (
          <div class="cr-menu" style={{ left: `${m().x}px`, top: `${m().y}px` }}>
            <button class="ghost" onClick={() => void ask()} disabled={!props.session} title="Attach these lines to your next message to the session">
              <Sparkles />
              Ask Claude
            </button>
          </div>
        )}
      </Show>
    </aside>
  );
}

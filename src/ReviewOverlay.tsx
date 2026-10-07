// Review and create PR (ticket 42): once the Agent's work looks done, every file its PR would
// change (committed or not, new files too) is gone through one at a time, GitHub-style: a file list
// with what's been viewed, each file's diff, Next / Previous. Then the PR step: the branch it goes
// into, title and description, and Create PR, which commits what's left, pushes the branch, and
// opens the PR assigned to the user.
import { openUrl } from "@tauri-apps/plugin-opener";
import { createEffect, createMemo, createSignal, For, Match, on, onCleanup, onMount, Show, Switch } from "solid-js";
import { createStore, reconcile } from "solid-js/store";
import { core, type DiffLine, type PullRequestOutcome, type Review, type ReviewFile } from "./core";
import { DiffView } from "./DiffView";
import { languageOfPath } from "./highlight";
import { Check, ChevronLeft, ChevronRight, CircleCheck, ExternalLink, GitBranch, GitPullRequest, Loader, RefreshCw, TriangleAlert, X } from "./icons";

const LETTER = { added: "A", modified: "M", deleted: "D", renamed: "R" } as const;
const plural = (n: number, thing: string) => `${n} ${thing}${n === 1 ? "" : "s"}`;

function splitPath(path: string) {
  const slash = path.lastIndexOf("/");
  return { name: path.slice(slash + 1), dir: slash < 0 ? "" : path.slice(0, slash + 1) };
}

/** A diff's identity: a file marked viewed stays so only while its diff is the same. */
const signature = (lines: DiffLine[]) => {
  let h = 0;
  for (const line of lines) for (let i = 0; i < line.text.length; i++) h = (h * 31 + line.text.charCodeAt(i) + line.kind.length) | 0;
  return `${lines.length}:${h}`;
};

/** What's been viewed, per Worktree and file (kept while the window is open, across reviews). */
const viewedBy = new Map<string, Map<string, string>>();

/** "agent/fix-login-form" → "Fix login form". */
function titleFromBranch(branch: string) {
  const words = (branch.split("/").pop() ?? branch).replace(/[-_]+/g, " ").trim();
  return words ? words[0].toUpperCase() + words.slice(1) : branch;
}

type Loaded = { lines: DiffLine[] } | { error: string };

export function ReviewOverlay(props: { worktree: string; label: string; colour: string; onClose: () => void }) {
  const [review, setReview] = createSignal<Review | null>(null);
  const [loadError, setLoadError] = createSignal("");
  const [step, setStep] = createSignal<"review" | "pr" | "done">("review");
  const [at, setAt] = createSignal(0);
  const [sideBySide, setSideBySide] = createSignal(false);
  const [diffs, setDiffs] = createStore<Record<string, Loaded>>({});
  const viewed = (() => {
    const existing = viewedBy.get(props.worktree);
    if (existing) return existing;
    const fresh = new Map<string, string>();
    viewedBy.set(props.worktree, fresh);
    return fresh;
  })();
  /** Bumped when `viewed` changes (it's a plain Map, kept outside the component). */
  const [viewedRev, setViewedRev] = createSignal(0);

  const load = async () => {
    setLoadError("");
    try {
      const next = await core.review(props.worktree);
      setDiffs(reconcile({}));
      setReview(next);
      setAt((i) => Math.min(i, Math.max(0, next.files.length - 1)));
      seedForm(next);
    } catch (err) {
      setLoadError(String(err));
    }
  };
  onMount(() => void load());

  const files = () => review()?.files ?? [];
  const current = () => files()[at()] as ReviewFile | undefined;
  const fetchDiff = (file: ReviewFile | undefined) => {
    const r = review();
    if (!file || !r || diffs[file.path]) return;
    core
      .reviewDiff(props.worktree, r.split, file)
      .then((lines) => setDiffs(file.path, { lines }))
      .catch((err) => setDiffs(file.path, { error: String(err) }));
  };
  // The file shown, and the next one ahead of time.
  createEffect(() => {
    fetchDiff(current());
    fetchDiff(files()[at() + 1]);
  });

  const isViewed = (file: ReviewFile) => {
    viewedRev();
    const seen = viewed.get(file.path);
    if (!seen) return false;
    const d = diffs[file.path];
    return !d || !("lines" in d) || signature(d.lines) === seen;
  };
  const setViewed = (file: ReviewFile, on: boolean) => {
    const d = diffs[file.path];
    if (on) viewed.set(file.path, d && "lines" in d ? signature(d.lines) : "?");
    else viewed.delete(file.path);
    setViewedRev((n) => n + 1);
  };
  const viewedCount = () => files().filter(isViewed).length;
  const stats = (file: ReviewFile) => {
    const d = diffs[file.path];
    if (!d || !("lines" in d)) return null;
    return {
      added: d.lines.filter((l) => l.kind === "added").length,
      removed: d.lines.filter((l) => l.kind === "removed").length,
    };
  };

  const go = (i: number) => setAt(Math.max(0, Math.min(files().length - 1, i)));
  /** Next file, marking this one viewed (as going on from it says it's been read). */
  const next = () => {
    const file = current();
    if (file) setViewed(file, true);
    if (at() < files().length - 1) go(at() + 1);
  };
  let diffBox: HTMLDivElement | undefined;
  createEffect(on(at, () => diffBox?.scrollTo({ top: 0 })));

  // The PR step.
  const [target, setTarget] = createSignal("");
  const [title, setTitle] = createSignal("");
  const [body, setBody] = createSignal("");
  const [commitMessage, setCommitMessage] = createSignal("");
  const [draft, setDraft] = createSignal(false);
  const [creating, setCreating] = createSignal(false);
  const [prError, setPrError] = createSignal("");
  const [working, setWorking] = createSignal<string[] | null>(null);
  const [outcome, setOutcome] = createSignal<Exclude<PullRequestOutcome, { kind: "sessionsWorking" }> | null>(null);
  let seeded = false;
  const seedForm = (r: Review) => {
    if (seeded) return;
    seeded = true;
    setTarget(r.target);
    setTitle(r.commits.length === 1 ? r.commits[0] : r.branch ? titleFromBranch(r.branch) : "");
    setBody(r.commits.length > 1 ? r.commits.map((c) => `- ${c}`).join("\n") : "");
  };
  const blocker = () => {
    const r = review();
    if (!r) return "";
    if (!r.branch) return "HEAD is detached: check out a branch to make a PR from.";
    if (r.operationInProgress) return "A merge or rebase is in progress here: finish or abort it first.";
    if (r.files.length === 0) return `Nothing to open a PR with: no changes against ${r.base}.`;
    if (target().trim() === r.branch) return `This Worktree is on ${r.branch} itself: choose another branch to merge into.`;
    return "";
  };
  const canCreate = () => !creating() && !blocker() && target().trim() !== "" && title().trim() !== "";
  const create = async (evenIfWorking = false) => {
    if (!evenIfWorking && !canCreate()) return;
    setCreating(true);
    setPrError("");
    setWorking(null);
    try {
      const result = await core.createPullRequest(props.worktree, {
        target: target().trim(),
        title: title().trim(),
        body: body(),
        commitMessage: commitMessage().trim(),
        draft: draft(),
        evenIfWorking,
      });
      if (result.kind === "sessionsWorking") setWorking(result.sessions);
      else {
        setOutcome(result);
        if (result.kind !== "pushRejected") {
          viewedBy.delete(props.worktree);
          setStep("done");
        }
      }
    } catch (err) {
      setPrError(String(err));
    } finally {
      setCreating(false);
    }
  };

  // Keys: j / → next file, k / ← previous, v viewed, Esc closes (or steps back from the PR form).
  const onKey = (e: KeyboardEvent) => {
    if (e.key === "Escape") {
      e.preventDefault();
      if (creating()) return;
      if (step() === "pr") setStep("review");
      else props.onClose();
      return;
    }
    if (step() !== "review" || e.ctrlKey || e.metaKey || e.altKey) return;
    const typing = e.target instanceof HTMLElement && e.target.closest("input, textarea, select");
    if (typing) return;
    if (e.key === "j" || e.key === "ArrowRight") (e.preventDefault(), next());
    else if (e.key === "k" || e.key === "ArrowLeft") (e.preventDefault(), go(at() - 1));
    else if (e.key === "v" && current()) (e.preventDefault(), setViewed(current()!, !isViewed(current()!)));
  };
  onMount(() => window.addEventListener("keydown", onKey, true));
  onCleanup(() => window.removeEventListener("keydown", onKey, true));

  const lineCount = createMemo(() => {
    let added = 0;
    let removed = 0;
    for (const f of files()) {
      const s = stats(f);
      if (s) (added += s.added), (removed += s.removed);
    }
    return { added, removed };
  });

  return (
    <div class="modal-backdrop review-backdrop" onClick={(e) => e.target === e.currentTarget && !creating() && props.onClose()}>
      <div class="modal review-overlay" role="dialog" aria-label="Review changes" style={{ "--c": props.colour }}>
        <div class="modal-head review-head">
          <GitPullRequest class="review-mark" />
          <b>
            {step() === "review" ? "Review changes" : step() === "pr" ? "Create pull request" : "Pull request"}
            <span class="review-branch muted">
              <GitBranch />
              <span class="mono">{review()?.branch ?? props.label}</span>
              <Show when={step() !== "review" && target()}>
                <ChevronRight />
                <span class="mono">{target()}</span>
              </Show>
            </span>
          </b>
          <ol class="review-steps">
            <li classList={{ on: step() === "review" }}>1 Review</li>
            <li classList={{ on: step() === "pr" }}>2 Pull request</li>
          </ol>
          <Show when={step() === "review"}>
            <button class="ghost icon" onClick={() => void load()} title="Read the changes again (the Agent may have changed files since)" aria-label="Refresh">
              <RefreshCw />
            </button>
          </Show>
          <button class="ghost icon" onClick={() => props.onClose()} disabled={creating()} aria-label="Close" title="Close (Esc)">
            <X />
          </button>
        </div>
        <Switch>
          <Match when={loadError()}>
            <p class="error modal-error">{loadError()}</p>
          </Match>
          <Match when={!review()}>
            <p class="muted center review-loading">
              <Loader class="spin" /> Reading the changes…
            </p>
          </Match>
          <Match when={step() === "review"}>
            <div class="review-body">
              <nav class="review-files">
                <div class="review-progress">
                  <span>
                    {viewedCount()} / {files().length} viewed
                  </span>
                  <span class="review-bar">
                    <span style={{ width: `${files().length ? (100 * viewedCount()) / files().length : 0}%` }} />
                  </span>
                </div>
                <For each={files()} fallback={<p class="muted small center">No changes against {review()!.base}.</p>}>
                  {(file, i) => (
                    <button class="review-file" classList={{ on: i() === at(), viewed: isViewed(file) }} onClick={() => go(i())} title={file.renamedFrom ? `${file.renamedFrom} → ${file.path}` : file.path}>
                      <span class={`change change-${file.change}`}>{LETTER[file.change]}</span>
                      <span class="review-file-name">
                        {splitPath(file.path).name}
                        <span class="dir">{splitPath(file.path).dir}</span>
                      </span>
                      <Show when={file.uncommitted}>
                        <span class="uncommitted-dot" title="Has changes that aren't committed yet: Create PR commits them" />
                      </Show>
                      <Show when={isViewed(file)}>
                        <Check class="viewed-check" />
                      </Show>
                    </button>
                  )}
                </For>
                <Show when={review()!.commits.length > 0}>
                  <div class="review-commits">
                    <div class="git-section">
                      Commits <span class="badge">{review()!.commits.length}</span>
                    </div>
                    <For each={review()!.commits}>{(c) => <div class="review-commit">{c}</div>}</For>
                  </div>
                </Show>
              </nav>
              <section class="review-main">
                <Show when={current()} fallback={<div class="center muted editor-placeholder">Nothing changed on this branch since it split from {review()!.base}.</div>}>
                  {(file) => (
                    <>
                      <div class="review-file-head">
                        <span class={`change change-${file().change}`}>{LETTER[file().change]}</span>
                        <span class="mono grow review-path">
                          <Show when={file().renamedFrom}>{(from) => <span class="muted">{from()} → </span>}</Show>
                          {file().path}
                        </span>
                        <Show when={stats(file())}>
                          {(s) => (
                            <span class="review-stat mono">
                              <span class="added-key">+{s().added}</span> <span class="removed-key">−{s().removed}</span>
                            </span>
                          )}
                        </Show>
                        <Show when={file().uncommitted}>
                          <span class="pill" title="Create PR commits these changes">uncommitted</span>
                        </Show>
                        <div class="segmented">
                          <button classList={{ on: !sideBySide() }} onClick={() => setSideBySide(false)}>
                            Unified
                          </button>
                          <button classList={{ on: sideBySide() }} onClick={() => setSideBySide(true)}>
                            Split
                          </button>
                        </div>
                        <label class="viewed-toggle" title="Mark as viewed (V)">
                          <input type="checkbox" checked={isViewed(file())} onChange={(e) => setViewed(file(), e.currentTarget.checked)} />
                          Viewed
                        </label>
                      </div>
                      <div class="review-diff" ref={diffBox}>
                        <Switch fallback={<p class="muted center review-loading"><Loader class="spin" /> Loading the diff…</p>}>
                          <Match when={diffs[file().path] && "error" in diffs[file().path] && (diffs[file().path] as { error: string })}>
                            {(d) => <div class="center muted editor-placeholder">{d().error}</div>}
                          </Match>
                          <Match when={diffs[file().path] && "lines" in diffs[file().path] && (diffs[file().path] as { lines: DiffLine[] })}>
                            {(d) => (
                              <DiffView
                                lines={d().lines}
                                sideBySide={sideBySide()}
                                language={languageOfPath(file().path)}
                                empty={file().change === "renamed" ? "Renamed, with no changes." : "No changes to its text."}
                              />
                            )}
                          </Match>
                        </Switch>
                      </div>
                    </>
                  )}
                </Show>
              </section>
            </div>
            <div class="modal-foot review-foot">
              <span class="hint">
                <kbd>J</kbd> / <kbd>K</kbd> next / previous file · <kbd>V</kbd> viewed
                <Show when={lineCount().added + lineCount().removed > 0}>
                  {" · "}
                  <span class="added-key">+{lineCount().added}</span> <span class="removed-key">−{lineCount().removed}</span> so far
                </Show>
              </span>
              <button class="ghost" disabled={at() === 0} onClick={() => go(at() - 1)}>
                <ChevronLeft />
                Previous
              </button>
              <button disabled={at() >= files().length - 1} onClick={next} title="Mark this file viewed and go to the next (J)">
                Next file
                <ChevronRight />
              </button>
              <button
                class="primary"
                disabled={files().length === 0}
                onClick={() => setStep("pr")}
                title={viewedCount() < files().length ? `${plural(files().length - viewedCount(), "file")} not marked viewed yet` : "On to the pull request"}
              >
                Next: pull request
                <ChevronRight />
              </button>
            </div>
          </Match>
          <Match when={step() === "pr"}>
            <div class="modal-body review-pr">
              <Show when={viewedCount() < files().length}>
                <p class="git-notice warn">
                  <TriangleAlert />
                  {plural(files().length - viewedCount(), "file")} of {files().length} not marked viewed.
                </p>
              </Show>
              <div class="review-pr-row">
                <label class="grow">
                  Merge into
                  <input class="mono" list="review-targets" value={target()} onInput={(e) => setTarget(e.currentTarget.value)} spellcheck={false} placeholder="main" />
                  <datalist id="review-targets">
                    <For each={review()!.targets}>{(t) => <option value={t} />}</For>
                  </datalist>
                </label>
                <label class="grow">
                  From
                  <input class="mono" value={review()!.branch ?? "(detached HEAD)"} disabled />
                </label>
              </div>
              <label>
                Title
                <input value={title()} onInput={(e) => setTitle(e.currentTarget.value)} ref={(el) => queueMicrotask(() => el.focus())} />
              </label>
              <label>
                Description
                <textarea class="review-body-text" value={body()} onInput={(e) => setBody(e.currentTarget.value)} placeholder="What it does and why (Markdown)" />
              </label>
              <Show when={review()!.uncommitted > 0}>
                <label>
                  Commit message for the {plural(review()!.uncommitted, "uncommitted file")}
                  <input value={commitMessage()} onInput={(e) => setCommitMessage(e.currentTarget.value)} placeholder={title() || "Commit message"} />
                </label>
              </Show>
              <label class="check-row">
                <input type="checkbox" checked={draft()} onChange={(e) => setDraft(e.currentTarget.checked)} />
                Open as a draft
              </label>
              <ul class="review-plan">
                <Show when={review()!.uncommitted > 0}>
                  <li>Commit the {plural(review()!.uncommitted, "uncommitted file")} (untracked ones too)</li>
                </Show>
                <li>
                  Push <span class="mono">{review()!.branch}</span> to <span class="mono">{review()!.remote}</span>
                </li>
                <li>
                  Open {draft() ? "a draft PR" : "a PR"} into <span class="mono">{target() || "…"}</span>, assigned to you
                </li>
              </ul>
              <Show when={blocker()}>
                <p class="error small">{blocker()}</p>
              </Show>
              <Show when={working()}>
                {(sessions) => (
                  <div class="git-ask">
                    <span>
                      {sessions().join(", ")} {sessions().length === 1 ? "is" : "are"} in the middle of a turn in this Worktree and may still change
                      files. Create the PR anyway?
                    </span>
                    <div class="actions">
                      <button class="primary" onClick={() => void create(true)} disabled={creating()}>
                        Create anyway
                      </button>
                      <button class="ghost" onClick={() => setWorking(null)}>
                        Cancel
                      </button>
                    </div>
                  </div>
                )}
              </Show>
              <Show when={outcome()?.kind === "pushRejected"}>
                <p class="git-notice warn">
                  <TriangleAlert />
                  {outcome()!.committed ? `Committed (${outcome()!.committed}), but the` : "The"} remote has commits this branch hasn't, so it turned the push down. Pull
                  first (or have an Agent merge or rebase), then try again.
                </p>
              </Show>
              <Show when={prError()}>
                <p class="error small review-pr-error">{prError()}</p>
              </Show>
            </div>
            <div class="modal-foot">
              <button class="ghost" onClick={() => setStep("review")} disabled={creating()}>
                <ChevronLeft />
                Back to review
              </button>
              <span class="grow" />
              <button class="primary" disabled={!canCreate()} onClick={() => void create()} title="Commit what's left, push, and open the PR">
                <Show when={creating()} fallback={<GitPullRequest />}>
                  <Loader class="spin" />
                </Show>
                {creating() ? "Creating…" : "Create PR"}
              </button>
            </div>
          </Match>
          <Match when={step() === "done" && outcome()}>
            {(o) => (
              <>
                <div class="modal-body review-done">
                  <CircleCheck class="done-mark" />
                  <p>
                    {o().kind === "alreadyOpen" ? "This branch already had a PR open; it now has the latest commits." : "Pull request opened, assigned to you."}
                    <Show when={o().committed}> The uncommitted changes went in as {o().committed}.</Show>
                  </p>
                  <Show when={"url" in o() && (o() as { url: string }).url}>
                    {(url) => <p class="mono small review-url">{url()}</p>}
                  </Show>
                </div>
                <div class="modal-foot">
                  <button class="ghost" onClick={() => props.onClose()}>
                    Close
                  </button>
                  <Show when={"url" in o() && (o() as { url: string }).url}>
                    {(url) => (
                      <button class="primary" onClick={() => void openUrl(url()).catch((err) => setPrError(String(err)))} ref={(el) => queueMicrotask(() => el.focus())}>
                        <ExternalLink />
                        Open in browser
                      </button>
                    )}
                  </Show>
                </div>
              </>
            )}
          </Match>
        </Switch>
      </div>
    </div>
  );
}

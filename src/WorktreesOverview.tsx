// The Worktrees overview: every Worktree of the repository, with whether its branch has been
// merged into its Base (however the PR landed), so the ones that are done can be removed. It shows
// what the last fetch knew at once, then fetches for news of merged PRs and looks again. Ticked
// Worktrees are removed together, after a confirmation of what each removal stops and deletes;
// one that would lose work is skipped there, and goes through its own Remove dialog instead.
import { createEffect, createMemo, createResource, createSignal, For, Match, onCleanup, onMount, Show, Switch } from "solid-js";
import { core, type MergeState, type RemovalCheck, type WorktreeMerge } from "./core";
import { Check, GitBranch, GitMerge, House, Loader, Trash2, TriangleAlert, X } from "./icons";
import { worktreeColour, type WorktreeTab } from "./Worktrees";

const plural = (n: number, thing: string) => `${n} ${thing}${n === 1 ? "" : "s"}`;

/** Its branch is in the Base, so removing it (and the branch) loses no committed work. */
const merged = (m: MergeState | null) => m?.kind === "merged" || m?.kind === "changesInBase";

/** Can go: merged, with nothing uncommitted. */
const ready = (row: WorktreeMerge) => !row.isMain && merged(row.merge) && row.changed === 0;

function status(row: WorktreeMerge): { text: string; tone: "ok" | "warn" | "muted" | "error" } {
  if (row.isMain) return { text: "Main checkout: stays", tone: "muted" };
  if (row.error) return { text: row.error, tone: "error" };
  const m = row.merge;
  switch (m?.kind) {
    case "merged":
      return { text: `Merged into ${m.into}`, tone: "ok" };
    case "changesInBase":
      return { text: `Merged into ${m.into} (squashed or rebased)`, tone: "ok" };
    case "nothingNew":
      return { text: "No commits of its own yet", tone: "muted" };
    case "remoteDeleted":
      return {
        text: `Remote branch deleted, but ${plural(m.commits, "commit")} ${m.commits === 1 ? "isn't" : "aren't"} in ${row.base}: the PR may have been closed without merging`,
        tone: "warn",
      };
    case "notMerged":
      return { text: `${plural(m.commits, "commit")} not in ${row.base}`, tone: "muted" };
    default:
      return { text: "", tone: "muted" };
  }
}

/** One ticked Worktree in the confirmation: what removing it does, or why it's skipped. */
interface Planned {
  path: string;
  check: RemovalCheck | null;
  /** Why it isn't removed with the others (it would lose work, or couldn't be checked). */
  skip: string | null;
  /** Its branch goes too: only a merged one, which loses nothing. */
  deleteBranch: boolean;
}

/** What removing a ticked Worktree would lose, which the bulk removal never discards. */
function lossOf(check: RemovalCheck): string | null {
  if (!check.discardToRemove) return null;
  const lost = [];
  if (check.changedCount > 0) lost.push(plural(check.changedCount, "uncommitted change"));
  if (!check.branch && check.unpushedCount > 0 && !check.merged) lost.push(`${plural(check.unpushedCount, "commit")} on a detached HEAD`);
  return `would lose ${lost.join(" and ")}: use its Remove… to review`;
}

interface Outcome {
  path: string;
  /** The error, or a warning (the Worktree went, its branch didn't). */
  message: string | null;
  removed: boolean;
}

export function WorktreesOverview(props: {
  root: string;
  /** The Worktree row's list: a change (one removed, files changed) looks again. */
  worktrees: WorktreeTab[];
  label: (path: string) => string;
  sessionCount: (path: string) => number;
  /** A dialog is open over this one (the removal dialog): keys are its. */
  covered: boolean;
  onRemove: (path: string) => void;
  /** A Worktree removed from here (with its warning, if any). */
  onRemoved: (path: string, warning: string | null) => void;
  onOpen: (path: string) => void;
  onClose: () => void;
}) {
  const [fetching, setFetching] = createSignal(true);
  const [fetchError, setFetchError] = createSignal("");
  const [fetched, setFetched] = createSignal(0);
  const listed = () => props.worktrees.map((w) => `${w.path} ${w.head} ${w.changed}`).join("\n");
  const [rows] = createResource(() => [listed(), fetched()] as const, () => core.mergeOverview());
  // Ready to remove first; the rest as the Worktree row orders them.
  const sorted = createMemo(() => {
    const all = rows.latest ?? [];
    return [...all.filter(ready), ...all.filter((r) => !ready(r))];
  });
  const readyCount = () => sorted().filter(ready).length;

  // Ticked for removal. Until the user ticks or unticks one, exactly the ready ones are (one that
  // stops being ready, say with new uncommitted changes, is unticked); Worktrees that are gone
  // drop out.
  const [checked, setChecked] = createSignal<string[]>([]);
  let chosen = false;
  createEffect(() => {
    const all = sorted();
    const live = (path: string) => all.some((r) => r.path === path && !r.isMain);
    setChecked((now) => {
      const kept = now.filter(live);
      return chosen ? kept : all.filter(ready).map((r) => r.path);
    });
  });
  const isChecked = (path: string) => checked().includes(path);
  const toggle = (path: string, on: boolean) => {
    chosen = true;
    setChecked((now) => (on ? [...now, path] : now.filter((p) => p !== path)));
  };

  // The bulk removal: null (the list), then the confirmation, then (once run) its outcomes.
  const [plan, setPlan] = createSignal<Planned[] | "checking" | null>(null);
  const [running, setRunning] = createSignal(false);
  const [outcomes, setOutcomes] = createSignal<Outcome[] | null>(null);
  const planned = () => {
    const p = plan();
    return Array.isArray(p) ? p : [];
  };
  const toRemove = () => planned().filter((p) => !p.skip);
  const stopping = () => toRemove().reduce((n, p) => n + (p.check?.sessions.length ?? 0), 0);

  const review = async () => {
    const paths = sorted().filter((r) => isChecked(r.path)).map((r) => r.path);
    setPlan("checking");
    const checks = await Promise.all(
      paths.map(async (path): Promise<Planned> => {
        try {
          const check = await core.removalCheck(path);
          return { path, check, skip: lossOf(check), deleteBranch: !!check.branch && check.merged };
        } catch (err) {
          return { path, check: null, skip: `couldn't be checked: ${String(err)}`, deleteBranch: false };
        }
      }),
    );
    // (Closed or cancelled meanwhile.)
    if (plan() === "checking") setPlan(checks);
  };

  const removeAll = async () => {
    if (running()) return;
    setRunning(true);
    const done: Outcome[] = [];
    // One at a time: each stops its sessions and setup first, and the core refreshes in between.
    for (const p of toRemove()) {
      try {
        // Never discards: work that appeared since the check is refused, not lost.
        const removed = await core.removeWorktree(p.path, { discard: null, deleteBranch: p.deleteBranch });
        done.push({ path: p.path, removed: true, message: removed.warning });
        props.onRemoved(p.path, removed.warning);
      } catch (err) {
        done.push({ path: p.path, removed: false, message: String(err) });
      }
    }
    setOutcomes(done);
    setPlan(null);
    setRunning(false);
  };

  const backToList = () => {
    setPlan(null);
    setOutcomes(null);
  };

  onMount(() => {
    core
      .gitFetch(props.root)
      .catch((err) => setFetchError(String(err)))
      .finally(() => {
        setFetching(false);
        setFetched((n) => n + 1);
      });
  });

  const onKey = (e: KeyboardEvent) => {
    if (e.key !== "Escape" || props.covered || running()) return;
    if (plan() !== null || outcomes() !== null) backToList();
    else props.onClose();
  };
  let dialog!: HTMLDivElement;
  onMount(() => dialog.focus());
  window.addEventListener("keydown", onKey);
  onCleanup(() => window.removeEventListener("keydown", onKey));

  return (
    <div class="modal-backdrop" onClick={(e) => e.target === e.currentTarget && !props.covered && !running() && props.onClose()}>
      <div class="modal worktrees-overview" role="dialog" aria-label="Worktrees" tabIndex={-1} ref={dialog}>
        <div class="modal-head">
          <b>Worktrees</b>
          <Show when={fetching()}>
            <span class="fetch-note">
              <Loader class="spin" />
              Fetching for merged PRs…
            </span>
          </Show>
          <button class="ghost icon" onClick={() => props.onClose()} disabled={running()} aria-label="Close" title="Close (Esc)">
            <X />
          </button>
        </div>
        <Switch>
          <Match when={outcomes()}>
            {(done) => (
              <>
                <div class="modal-body">
                  <ul class="plan-list">
                    <For each={done()}>
                      {(o) => (
                        <li classList={{ failed: !o.removed }}>
                          <Show when={o.removed} fallback={<TriangleAlert />}>
                            <Check />
                          </Show>
                          <span class="plan-text">
                            <span class="mono">{props.label(o.path)}</span>
                            <span class="small" classList={{ muted: o.removed, error: !o.removed }}>
                              {o.removed ? (o.message ?? "Removed") : `Not removed: ${o.message}`}
                            </span>
                          </span>
                        </li>
                      )}
                    </For>
                  </ul>
                </div>
                <div class="modal-foot">
                  <span class="hint">{plural(done().filter((o) => o.removed).length, "Worktree")} removed</span>
                  <button class="primary" onClick={backToList}>
                    Done
                  </button>
                </div>
              </>
            )}
          </Match>
          <Match when={plan() !== null}>
            <div class="modal-body">
              <Show when={plan() !== "checking"} fallback={<p class="muted">Checking what each removal would stop and delete…</p>}>
                <p class="small">
                  {toRemove().length === 0
                    ? "None of the ticked Worktrees can be removed without losing work."
                    : `Removes ${plural(toRemove().length, "Worktree")} and ${toRemove().length === 1 ? "its folder" : "their folders"}${stopping() ? `, stopping ${plural(stopping(), "running session")}` : ""}.`}
                </p>
                <ul class="plan-list">
                  <For each={planned()}>
                    {(p) => (
                      <li classList={{ skipped: !!p.skip }} style={{ "--c": worktreeColour(p.path) }}>
                        <Show when={!p.skip} fallback={<TriangleAlert />}>
                          <Trash2 />
                        </Show>
                        <span class="plan-text">
                          <span class="mono">{props.label(p.path)}</span>
                          <span class="small muted">
                            {p.skip
                              ? `Skipped: ${p.skip}`
                              : [
                                  p.deleteBranch ? `deletes branch ${p.check!.branch}` : p.check?.branch ? `keeps branch ${p.check.branch} (not merged)` : "detached HEAD",
                                  p.check?.sessions.length ? `stops ${plural(p.check.sessions.length, "session")}` : "",
                                  p.check?.ignoredCount ? `deletes ${plural(p.check.ignoredCount, "ignored file")}` : "",
                                ]
                                  .filter(Boolean)
                                  .join(" · ")}
                          </span>
                        </span>
                      </li>
                    )}
                  </For>
                </ul>
              </Show>
            </div>
            <div class="modal-foot">
              <button onClick={backToList} disabled={running()}>
                Back <kbd>Esc</kbd>
              </button>
              <button class="danger" onClick={() => void removeAll()} disabled={running() || plan() === "checking" || toRemove().length === 0}>
                {running() ? "Removing…" : `Remove ${plural(toRemove().length, "Worktree")}`}
              </button>
            </div>
          </Match>
          <Match when={plan() === null}>
            <div class="modal-body">
              <Show when={fetchError()}>
                <div class="callout warn">
                  <TriangleAlert />
                  <span>Couldn't fetch, so this is as of the last fetch: {fetchError()}</span>
                </div>
              </Show>
              <Show when={rows.error}>
                <p class="error">{String(rows.error)}</p>
              </Show>
              <Show when={rows.latest} fallback={<Show when={!rows.error}><p class="muted">Checking each branch…</p></Show>}>
                <p class="muted small">
                  {readyCount() === 0
                    ? "None of the Worktrees' branches are merged yet."
                    : `${plural(readyCount(), "Worktree")} can be removed: ${readyCount() === 1 ? "its branch is" : "their branches are"} merged, with nothing uncommitted.`}
                </p>
                <ul class="overview-list">
                  <For each={sorted()}>
                    {(row) => {
                      const s = () => status(row);
                      const sessions = () => props.sessionCount(row.path);
                      return (
                        <li classList={{ ready: ready(row) }} style={{ "--c": worktreeColour(row.path) }}>
                          <Show when={!row.isMain} fallback={<span class="overview-check" />}>
                            <input
                              type="checkbox"
                              class="overview-check"
                              checked={isChecked(row.path)}
                              onChange={(e) => toggle(row.path, e.currentTarget.checked)}
                              aria-label={`Remove ${props.label(row.path)}`}
                            />
                          </Show>
                          <button class="overview-open" onClick={() => props.onOpen(row.path)} title={`Go to ${row.path}`}>
                            <span class="overview-name">
                              <Show when={row.isMain} fallback={<GitBranch />}>
                                <House />
                              </Show>
                              <span class="mono">{props.label(row.path)}</span>
                            </span>
                            <span class={`overview-status ${s().tone}`}>
                              <Show when={merged(row.merge)}>
                                <GitMerge />
                              </Show>
                              {s().text}
                            </span>
                            <Show when={row.changed > 0 || sessions() > 0}>
                              <span class="overview-extra">
                                <Show when={row.changed > 0}>
                                  <span class="warn">{plural(row.changed, "uncommitted change")}</span>
                                </Show>
                                <Show when={sessions() > 0}>
                                  <span>{plural(sessions(), "session")} running</span>
                                </Show>
                              </span>
                            </Show>
                            <span class="mono muted small overview-path">{row.path}</span>
                          </button>
                          <Show when={!row.isMain}>
                            <button onClick={() => props.onRemove(row.path)} title="Remove just this Worktree (shows what it would lose first)">
                              <Trash2 />
                              Remove…
                            </button>
                          </Show>
                        </li>
                      );
                    }}
                  </For>
                </ul>
              </Show>
            </div>
            <div class="modal-foot">
              <span class="hint">Ticked Worktrees are removed together; any that would lose work are skipped.</span>
              <button class="primary" onClick={() => void review()} disabled={checked().length === 0}>
                <Trash2 />
                Remove checked ({checked().length})
              </button>
            </div>
          </Match>
        </Switch>
      </div>
    </div>
  );
}

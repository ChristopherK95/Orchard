// The Workspace picker (ticket 29): one input over the Recent Workspaces, plus Browse… for the
// native folder dialog. Typing filters the list (Ctrl+P's matching); a path opens as typed.
// ↑/↓ move, Enter opens, Delete removes a row from the list, Ctrl+O browses.
import { createEffect, createSignal, For, type JSX, onMount, Show } from "solid-js";
import { core, type RecentWorkspace, type WorkspaceInfo } from "./core";
import { Highlighted } from "./CommandPalette";
import { FolderOpen, Search, X } from "./icons";

/** Text that's a path rather than a filter: it has a separator, or starts with a drive or `~`. */
const looksLikePath = (text: string) => /[\\/]/.test(text) || /^[a-z]:/i.test(text) || text.startsWith("~");

/** "Just now", "5 min ago", "2 h ago", "Yesterday", "3 days ago", then the date. */
function ago(ms: number, now = Date.now()): string {
  const minutes = Math.floor((now - ms) / 60_000);
  if (minutes < 1) return "Just now";
  if (minutes < 60) return `${minutes} min ago`;
  if (minutes < 24 * 60) return `${Math.floor(minutes / 60)} h ago`;
  const days = Math.round((new Date(now).setHours(0, 0, 0, 0) - new Date(ms).setHours(0, 0, 0, 0)) / 86_400_000);
  if (days <= 1) return "Yesterday";
  if (days < 7) return `${days} days ago`;
  return new Date(ms).toLocaleDateString();
}

export function WorkspacePicker(props: {
  /** Above the heading (the app's mark, at startup). */
  header?: JSX.Element;
  onOpened: (w: WorkspaceInfo) => void;
  /** Shown as the picker opens (say, the CLI argument's repo failed to open). */
  error?: string;
  /** Put in the input as the picker opens. */
  initialText?: string;
}) {
  const [query, setQuery] = createSignal(props.initialText ?? "");
  const [rows, setRows] = createSignal<RecentWorkspace[]>([]);
  const [selected, setSelected] = createSignal(0);
  const [error, setError] = createSignal(props.error ?? "");
  const [opening, setOpening] = createSignal(false);
  let input!: HTMLInputElement;
  let asked = 0;

  const typedPath = () => (looksLikePath(query().trim()) ? query().trim() : null);

  const search = async (text: string) => {
    const mine = ++asked;
    const found = await core.recentWorkspaces(looksLikePath(text.trim()) ? "" : text).catch(() => []);
    if (mine !== asked) return; // (a later keystroke's answer wins)
    setRows(found);
    // Select the first one that can be opened: with no filter, the last Workspace opened.
    setSelected(Math.max(0, found.findIndex((w) => w.exists)));
  };

  const open = async (path: string) => {
    if (opening()) return;
    setError("");
    setOpening(true);
    try {
      props.onOpened(await core.openWorkspace(path));
    } catch (err) {
      setError(String(err));
    } finally {
      setOpening(false);
    }
  };

  const browse = async () => {
    const folder = await core.pickFolder().catch((err) => (setError(String(err)), null));
    if (folder) await open(folder);
    else input.focus();
  };

  const remove = async (row: RecentWorkspace) => {
    try {
      await core.removeRecentWorkspace(row.root);
    } catch (err) {
      setError(String(err));
    }
    await search(query());
    input.focus();
  };

  const pick = (row: RecentWorkspace | undefined) => row?.exists && void open(row.root);

  onMount(() => {
    input.focus();
    void search(query());
  });

  return (
    <div class="startup">
      <div class="startup-card">
        {props.header}
        <h1>Open a repository</h1>
        <div class="picker">
          <div class="palette-search">
            <Search />
            <input
              ref={input}
              class="palette-input"
              placeholder="Filter recent repositories, or type a path"
              value={query()}
              onInput={(e) => {
                setQuery(e.currentTarget.value);
                void search(e.currentTarget.value);
              }}
              onKeyDown={(e) => {
                const count = typedPath() ? 0 : rows().length;
                if (e.key === "ArrowDown") {
                  e.preventDefault();
                  setSelected((i) => (count ? (i + 1) % count : 0));
                } else if (e.key === "ArrowUp") {
                  e.preventDefault();
                  setSelected((i) => (count ? (i - 1 + count) % count : 0));
                } else if (e.key === "Enter") {
                  e.preventDefault();
                  const path = typedPath();
                  if (path) void open(path);
                  else pick(rows()[selected()]);
                } else if (e.key === "o" && e.ctrlKey && !e.shiftKey && !e.altKey) {
                  e.preventDefault();
                  void browse();
                } else if (e.key === "Delete" && count && input.selectionStart === input.value.length && input.selectionEnd === input.value.length) {
                  // (With the caret at the end, Delete has no text to delete: it removes the row.)
                  e.preventDefault();
                  const row = rows()[selected()];
                  if (row) void remove(row);
                }
              }}
              disabled={opening()}
              spellcheck={false}
            />
            <button type="button" class="ghost" onClick={() => void browse()} disabled={opening()} title="Choose a folder (Ctrl+O)">
              <FolderOpen />
              Browse…
            </button>
          </div>
          <div class="palette-list">
            <Show
              when={!typedPath()}
              fallback={
                <div class="palette-item on" onClick={() => void open(typedPath()!)}>
                  <FolderOpen />
                  <span class="file mono">{typedPath()}</span>
                  <span class="end">
                    Open <kbd>Enter</kbd>
                  </span>
                </div>
              }
            >
              <For
                each={rows()}
                fallback={
                  <p class="muted small palette-empty">
                    {query().trim() ? `No recent repository matches “${query().trim()}”.` : "No recent repositories yet. Type a path or browse for a folder."}
                  </p>
                }
              >
                {(row, i) => (
                  <>
                    <Show when={i() === 0}>
                      <div class="palette-group">Recent</div>
                    </Show>
                    <div
                      class="palette-item picker-row"
                      classList={{ on: i() === selected(), gone: !row.exists }}
                      title={row.exists ? row.root : `${row.root} isn't there any more`}
                      ref={(el) => createEffect(() => i() === selected() && el.scrollIntoView({ block: "nearest" }))}
                      onMouseEnter={() => setSelected(i())}
                      onClick={() => pick(row)}
                    >
                      <Highlighted text={row.root} indices={row.indices} />
                      <span class="end">
                        <Show when={row.exists} fallback={<span class="not-found">Not found</span>}>
                          <Show when={row.sessions}>
                            <span>{row.sessions === 1 ? "1 session" : `${row.sessions} sessions`}</span>
                          </Show>
                          <Show when={row.opened}>{(opened) => <span>{ago(opened())}</span>}</Show>
                        </Show>
                        <button
                          type="button"
                          class="ghost icon remove"
                          title="Remove from this list (Delete); its sessions are kept"
                          aria-label={`Remove ${row.name} from the list`}
                          onClick={(e) => {
                            e.stopPropagation();
                            void remove(row);
                          }}
                        >
                          <X />
                        </button>
                      </span>
                    </div>
                  </>
                )}
              </For>
            </Show>
          </div>
        </div>
        <Show when={error()}>
          <p class="error small">{error()}</p>
        </Show>
        <p class="muted small picker-keys">
          <kbd>↑</kbd> <kbd>↓</kbd> select · <kbd>Enter</kbd> open · <kbd>Del</kbd> remove from list · <kbd>Ctrl+O</kbd> browse
        </p>
      </div>
    </div>
  );
}

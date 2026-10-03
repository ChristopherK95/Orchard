// The composer's `/` menu: the slash commands (Claude Code's own, custom commands and skills) the
// session's Agent says it offers, filtered as you type the name. Picking one completes it; the
// prompt `/name args` then runs it like any message.
import { createEffect, createSignal, For, Show } from "solid-js";
import { createStore } from "solid-js/store";
import { core, type SessionId, type SessionInfo, type SlashCommand } from "./core";

/** Each session's list as the Agent last sent it, and the last list seen in each Worktree (what a
 *  Suspended session, whose Agent hasn't started yet, shows meanwhile). */
const [bySession, setBySession] = createStore<Record<SessionId, SlashCommand[]>>({});
const [byWorktree, setByWorktree] = createStore<Record<string, SlashCommand[]>>({});
let listening = false;

/** The commands to offer in a session (reactive). */
export function useSlashCommands(session: () => SessionInfo): () => SlashCommand[] {
  if (!listening) {
    listening = true;
    void core.onEvent((event) => {
      if (event.kind !== "availableCommandsChanged") return;
      setBySession(event.sessionId, event.commands);
    });
  }
  createEffect(() => {
    const { id } = session();
    if (bySession[id]) return;
    void core
      .availableCommands(id)
      .then((list) => !bySession[id] && list.length && setBySession(id, list))
      .catch(() => {});
  });
  // A session's list is also its Worktree's latest.
  createEffect(() => {
    const s = session();
    const list = bySession[s.id];
    if (list?.length) setByWorktree(s.worktree, list);
  });
  return () => {
    const s = session();
    return bySession[s.id]?.length ? bySession[s.id] : (byWorktree[s.worktree] ?? []);
  };
}

/** The typed command name, while the composer holds only `/name` (no space yet); else null. */
export const slashQuery = (text: string) => (/^\/\S*$/.test(text) ? text.slice(1).toLowerCase() : null);

/** Commands matching a query: names starting with it first, then ones containing it. */
export function matching(commands: SlashCommand[], query: string): SlashCommand[] {
  const starts = commands.filter((c) => c.name.toLowerCase().startsWith(query));
  const contains = commands.filter((c) => !c.name.toLowerCase().startsWith(query) && c.name.toLowerCase().includes(query));
  return [...starts, ...contains];
}

export function SlashMenu(props: {
  items: SlashCommand[];
  selected: number;
  /** The session's Agent hasn't said what it offers yet (it starts with the first message). */
  waiting: boolean;
  onHover: (index: number) => void;
  onPick: (command: SlashCommand) => void;
}) {
  return (
    <div class="slash-menu" role="listbox" aria-label="Commands and skills">
      <div class="slash-head">
        Commands and skills
        <span class="note">↑↓ choose · Enter or Tab complete · Esc close</span>
      </div>
      <div class="slash-list">
        <For
          each={props.items}
          fallback={
            <p class="slash-empty">{props.waiting ? "The Agent lists its commands once this session is running." : "No command matches."}</p>
          }
        >
          {(command, i) => (
            <button
              class="slash-item"
              classList={{ on: i() === props.selected }}
              role="option"
              aria-selected={i() === props.selected}
              ref={(row) => createEffect(() => i() === props.selected && row.scrollIntoView({ block: "nearest" }))}
              onMouseEnter={() => props.onHover(i())}
              // (mousedown, so the textarea keeps focus)
              onMouseDown={(e) => {
                e.preventDefault();
                props.onPick(command);
              }}
            >
              <span class="name">/{command.name}</span>
              <Show when={command.hint}>{(hint) => <span class="hint">{hint()}</span>}</Show>
              <span class="desc">{command.description}</span>
            </button>
          )}
        </For>
      </div>
    </div>
  );
}

/** The menu's state for a composer: open while the text is `/name`, unless dismissed with Esc. */
export function createSlashMenu(text: () => string, commands: () => SlashCommand[]) {
  const [selected, setSelected] = createSignal(0);
  const [dismissed, setDismissed] = createSignal<string | null>(null);
  const query = () => slashQuery(text());
  const items = () => {
    const q = query();
    return q === null ? [] : matching(commands(), q);
  };
  const open = () => query() !== null && dismissed() !== text();
  // A new query starts at the top.
  createEffect(() => {
    void query();
    setSelected(0);
  });
  return {
    open,
    items,
    selected,
    setSelected,
    /** Handles a key in the composer; true if the menu used it. */
    key(e: KeyboardEvent, complete: (command: SlashCommand) => void): boolean {
      if (!open()) return false;
      const count = items().length;
      if (e.key === "ArrowDown" && count) setSelected((i) => (i + 1) % count);
      else if (e.key === "ArrowUp" && count) setSelected((i) => (i - 1 + count) % count);
      else if ((e.key === "Enter" || e.key === "Tab") && !e.shiftKey && count) complete(items()[selected()]);
      else if (e.key === "Escape") setDismissed(text());
      else return false;
      e.preventDefault();
      return true;
    },
  };
}

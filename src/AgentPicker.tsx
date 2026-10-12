// Ctrl+K: Ctrl+P's look, for hopping between Agent sessions. Each row is a session with its
// Worktree and state; typing narrows by either name (letters in order, not necessarily adjacent).
// The ones that need the user come first. Arrow keys move, Enter shows the session, Esc closes.
import { createEffect, createSignal, For, onCleanup, onMount, Show } from "solid-js";
import type { SessionId, SessionInfo, SessionState } from "./core";
import { GitBranch, Search } from "./icons";
import { StateDot } from "./StateDot";
import { STATE_LABEL } from "./stateLabels";

/** Most urgent first; the Tabs' order within a state. */
const URGENCY: SessionState[] = ["needsYou", "working", "idle", "suspended", "exited"];

/** Where `query`'s characters fall, in order, in `text` (case-insensitive), or null if they don't all. */
function subsequence(text: string, query: string): number[] | null {
  // (By character, as `Marked` counts them.)
  const chars = [...text].map((ch) => ch.toLowerCase());
  const at: number[] = [];
  let from = 0;
  for (const ch of query) {
    const i = chars.indexOf(ch, from);
    if (i < 0) return null;
    at.push(i);
    from = i + 1;
  }
  return at;
}

/** Fewer gaps between the matched characters, and an earlier start, rank higher. */
const spread = (at: number[]) => (at.length ? at[at.length - 1] - at[0] - at.length + at[0] / 100 : 0);

type Row = { session: SessionInfo; worktree: string; colour: string; inName: number[]; inWorktree: number[]; rank: number };

export function AgentPicker(props: {
  /** Every session, in the Tabs' order. */
  sessions: SessionInfo[];
  /** The session on screen (the focused column's, in the Columns view). */
  current: SessionId | undefined;
  /** A Worktree's label (its branch) and colour, by path. */
  label: (worktree: string) => string;
  colour: (worktree: string) => string;
  onPick: (id: SessionId) => void;
  onClose: () => void;
}) {
  const [query, setQuery] = createSignal("");
  const [selected, setSelected] = createSignal(0);
  let input!: HTMLInputElement;

  const rows = (): Row[] => {
    const words = query().trim().toLowerCase().split(/\s+/).filter(Boolean);
    const found: Row[] = [];
    for (const session of props.sessions) {
      const worktree = props.label(session.worktree);
      const inName: number[] = [];
      const inWorktree: number[] = [];
      let rank = 0;
      // Each word matches the session's name or its Worktree's (the name first).
      const all = words.every((word) => {
        const name = subsequence(session.name, word);
        const where = name ? null : subsequence(worktree, word);
        if (!name && !where) return false;
        (name ? inName : inWorktree).push(...(name ?? where!));
        rank += spread(name ?? where!) + (name ? 0 : 0.5);
        return true;
      });
      if (all) found.push({ session, worktree, colour: props.colour(session.worktree), inName, inWorktree, rank });
    }
    const urgency = (r: Row) => URGENCY.indexOf(r.session.state);
    // Typed: best match first. Not: most urgent first.
    return found.sort((a, b) => (words.length ? a.rank - b.rank : 0) || urgency(a) - urgency(b));
  };

  /** The first row that isn't the session already on screen. */
  const firstOther = () => Math.max(0, rows().findIndex((r) => r.session.id !== props.current));

  const pick = (row: Row | undefined) => {
    if (!row) return;
    props.onClose();
    props.onPick(row.session.id);
  };

  onMount(() => {
    input.focus();
    setSelected(firstOther());
  });
  const onKey = (e: KeyboardEvent) => e.key === "Escape" && props.onClose();
  window.addEventListener("keydown", onKey);
  onCleanup(() => window.removeEventListener("keydown", onKey));

  return (
    <div class="modal-backdrop" onClick={(e) => e.target === e.currentTarget && props.onClose()}>
      <div class="modal palette agent-picker" role="dialog" aria-label="Go to Agent session">
        <div class="palette-search">
          <Search />
          <input
            ref={input}
            class="palette-input"
            placeholder="Go to an Agent session, by its name or worktree…"
            value={query()}
            onInput={(e) => {
              setQuery(e.currentTarget.value);
              setSelected(query().trim() ? 0 : firstOther());
            }}
            onKeyDown={(e) => {
              const count = rows().length;
              if (e.key === "ArrowDown") {
                e.preventDefault();
                setSelected((i) => (count ? (i + 1) % count : 0));
              } else if (e.key === "ArrowUp") {
                e.preventDefault();
                setSelected((i) => (count ? (i - 1 + count) % count : 0));
              } else if (e.key === "Enter") {
                e.preventDefault();
                pick(rows()[selected()]);
              }
            }}
            spellcheck={false}
          />
        </div>
        <div class="palette-list">
          <For
            each={rows()}
            fallback={
              <p class="muted small palette-empty">
                {props.sessions.length ? `No Agent sessions match “${query().trim()}”.` : "No Agent sessions are open."}
              </p>
            }
          >
            {(row, i) => (
              <button
                class="palette-item"
                classList={{ on: i() === selected() }}
                ref={(el) => createEffect(() => i() === selected() && el.scrollIntoView({ block: "nearest" }))}
                onMouseEnter={() => setSelected(i())}
                onClick={() => pick(row)}
              >
                <StateDot state={row.session.state} />
                <span class="file">
                  <Marked text={row.session.name} at={row.inName} />
                </span>
                <span class="agent-picker-worktree" style={{ "--c": row.colour }} title={row.session.worktree}>
                  <GitBranch />
                  <Marked text={row.worktree} at={row.inWorktree} />
                </span>
                <span class="end">
                  <Show when={row.session.id === props.current}>
                    <span>Current</span>
                  </Show>
                  <span class={`agent-picker-state ${row.session.state}`}>{STATE_LABEL[row.session.state]}</span>
                  <Show when={i() === selected()}>
                    <kbd>Enter</kbd>
                  </Show>
                </span>
              </button>
            )}
          </For>
        </div>
      </div>
    </div>
  );
}

/** `text` with the characters at `at` marked as hits. */
function Marked(props: { text: string; at: number[] }) {
  const parts = () => {
    const marked = new Set(props.at);
    const out: { text: string; hit: boolean }[] = [];
    [...props.text].forEach((ch, i) => {
      const hit = marked.has(i);
      const last = out.at(-1);
      if (last && last.hit === hit) last.text += ch;
      else out.push({ text: ch, hit });
    });
    return out;
  };
  return <For each={parts()}>{(p) => <span classList={{ hit: p.hit }}>{p.text}</span>}</For>;
}

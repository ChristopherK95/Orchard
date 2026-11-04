// Picking the skill an Agent session runs to create a PR: a button showing the chosen skill in
// full, opening a filterable list of the skills the Worktree's sessions offer. A name not listed
// can be used as typed (a new session's skills aren't known before it starts).
import { createEffect, createMemo, createSignal, For, onCleanup, Show } from "solid-js";
import type { SlashCommand } from "./core";
import { matching } from "./SlashCommands";
import { ChevronsUpDown, SquareSlash } from "./icons";

/** A row of the list: a skill, the one typed, or none (asking in plain words). */
type Pick = { kind: "none" } | { kind: "skill"; command: SlashCommand } | { kind: "typed"; name: string };

export function SkillPicker(props: {
  /** The chosen skill's name ("" for none). */
  value: string;
  skills: SlashCommand[];
  open: boolean;
  disabled?: boolean;
  onOpen: (open: boolean) => void;
  onChange: (name: string) => void;
}) {
  let trigger: HTMLButtonElement | undefined;
  const [filter, setFilter] = createSignal("");
  const [selected, setSelected] = createSignal(0);
  const typed = () => filter().trim().replace(/^\//, "");
  const picks = createMemo<Pick[]>(() => {
    const q = typed().toLowerCase();
    const found = matching(props.skills, q).map((command): Pick => ({ kind: "skill", command }));
    const exact = props.skills.some((c) => c.name.toLowerCase() === q);
    return [...(q ? [] : [{ kind: "none" } as Pick]), ...found, ...(q && !exact ? [{ kind: "typed", name: typed() } as Pick] : [])];
  });
  createEffect(() => {
    void filter();
    setSelected(0);
  });
  createEffect(() => {
    if (!props.open) return;
    setFilter("");
    const i = picks().findIndex((p) => p.kind === "skill" && p.command.name === props.value);
    setSelected(Math.max(0, i));
  });
  const pick = (p: Pick) => {
    props.onChange(p.kind === "none" ? "" : p.kind === "skill" ? p.command.name : p.name);
    props.onOpen(false);
    trigger?.focus();
  };
  const outside = (e: MouseEvent) => {
    const at = e.target as Element | null;
    if (props.open && !at?.closest?.(".skill-menu") && !trigger?.contains(at)) props.onOpen(false);
  };
  window.addEventListener("mousedown", outside);
  onCleanup(() => window.removeEventListener("mousedown", outside));
  /** Opens above the button (it sits low in the form), at least wide enough for descriptions. */
  const place = () => {
    const r = trigger!.getBoundingClientRect();
    const width = Math.min(Math.max(r.width, 460), window.innerWidth - 16);
    return { left: `${Math.max(8, Math.min(r.left, window.innerWidth - width - 8))}px`, bottom: `${window.innerHeight - r.top + 4}px`, width: `${width}px` };
  };
  return (
    <>
      <button
        ref={trigger}
        class="outline skill-trigger"
        classList={{ on: props.open }}
        disabled={props.disabled}
        onClick={() => props.onOpen(!props.open)}
        title={props.value ? `Runs /${props.value}` : "No skill: the session is asked in plain words"}
      >
        <SquareSlash />
        <Show when={props.value} fallback={<span class="none">No skill: ask in plain words</span>}>
          <span class="mono name">{props.value}</span>
        </Show>
        <ChevronsUpDown class="chevron" />
      </button>
      <Show when={props.open}>
        <div class="menu skill-menu" role="listbox" aria-label="Skill to create the PR with" style={place()}>
          <input
            class="mono"
            value={filter()}
            onInput={(e) => setFilter(e.currentTarget.value)}
            ref={(el) => queueMicrotask(() => el.focus())}
            placeholder="Filter skills, or type one's name"
            spellcheck={false}
            onKeyDown={(e) => {
              const count = picks().length;
              if (e.key === "ArrowDown" && count) setSelected((i) => (i + 1) % count);
              else if (e.key === "ArrowUp" && count) setSelected((i) => (i - 1 + count) % count);
              else if (e.key === "Enter" && count) pick(picks()[selected()]);
              else return;
              e.preventDefault();
            }}
          />
          <div class="slash-list">
            <For each={picks()}>
              {(p, i) => (
                <button
                  class="slash-item"
                  classList={{ on: i() === selected() }}
                  role="option"
                  aria-selected={i() === selected()}
                  ref={(row) => createEffect(() => i() === selected() && row.scrollIntoView({ block: "nearest" }))}
                  onMouseEnter={() => setSelected(i())}
                  onClick={() => pick(p)}
                >
                  <Show when={p.kind === "skill" && p.command} fallback={
                    <Show when={p.kind === "typed" && p.name} fallback={<span class="desc">No skill: ask in plain words</span>}>
                      {(name) => (
                        <>
                          <span class="name">/{name()}</span>
                          <span class="desc">Use as typed</span>
                        </>
                      )}
                    </Show>
                  }>
                    {(command) => (
                      <>
                        <span class="name">/{command().name}</span>
                        <span class="desc">{command().description}</span>
                      </>
                    )}
                  </Show>
                </button>
              )}
            </For>
            <Show when={props.skills.length === 0 && !typed()}>
              <p class="slash-empty">The Worktree's sessions haven't listed their skills yet: type one's name.</p>
            </Show>
          </div>
        </div>
      </Show>
    </>
  );
}

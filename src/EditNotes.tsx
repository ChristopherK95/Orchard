// Edit notes (ticket 17): files the user saved by hand that this session read or edited, as chips
// above the composer. Each expands to its diff and can be removed; the ones kept go with the next
// prompt. The core holds them (they wait while the session is Suspended).
import { createEffect, createSignal, For, onCleanup, Show } from "solid-js";
import { core, type EditNote, type SessionId } from "./core";
import { DiffView } from "./DiffView";
import { languageOfPath } from "./highlight";

export function EditNotes(props: { sessionId: SessionId }) {
  const [notes, setNotes] = createSignal<EditNote[]>([]);
  const [open, setOpen] = createSignal<string | null>(null);

  let stop: (() => void) | undefined;
  let alive = true;
  /** An event came for the session since the list was asked for: it's newer than the reply. */
  let heard = false;
  onCleanup(() => {
    alive = false;
    stop?.();
  });
  void core
    .onEvent((event) => {
      if (event.kind !== "editNotesChanged" || event.sessionId !== props.sessionId) return;
      heard = true;
      setNotes(event.notes);
    })
    .then((unlisten) => (alive ? (stop = unlisten) : unlisten()));
  createEffect(() => {
    const id = props.sessionId;
    setOpen(null);
    setNotes([]); // (not the last Tab's while this one's come)
    heard = false;
    void core
      .editNotes(id)
      .then((list) => id === props.sessionId && !heard && setNotes(list))
      .catch(() => {});
  });

  const remove = (note: EditNote) => void core.removeEditNote(props.sessionId, note.path).catch(() => {});
  const opened = () => notes().find((n) => n.path === open());

  return (
    <Show when={notes().length > 0}>
      <div class="edit-notes">
        <For each={notes()}>
          {(note) => (
            <span class="edit-note" classList={{ on: open() === note.path }}>
              <button
                class="edit-note-name"
                onClick={() => setOpen(open() === note.path ? null : note.path)}
                title="Sent with your next message. Click to see the change."
              >
                📝 You edited <span class="mono">{note.name}</span> (+{note.added} −{note.removed})
              </button>
              <button
                class="close-tab"
                aria-label={`Don't tell the Agent about ${note.name}`}
                title="Don't tell the Agent about these changes (a later save is noted from here)"
                onClick={() => remove(note)}
              >
                ×
              </button>
            </span>
          )}
        </For>
        <Show when={opened()}>
          {(note) => (
            <div class="edit-note-diff">
              <Show
                when={note().diff}
                fallback={<p class="muted">Changed substantially: the Agent is told to read the file again.</p>}
              >
                {(diff) => <DiffView lines={diff()} sideBySide={false} language={languageOfPath(note().path)} />}
              </Show>
            </div>
          )}
        </Show>
      </div>
    </Show>
  );
}

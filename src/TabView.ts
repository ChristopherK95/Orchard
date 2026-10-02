// One view slot's visible Tab (ticket 28): the Tabs view has one, and each column of the Columns
// view has its own. It holds the shown session's transcript from `start` on, streamed by the core
// for that slot, and loads earlier pages when scrolling back.
import { batch, createSignal } from "solid-js";
import { createStore, produce, reconcile } from "solid-js/store";
import { core, type SessionId, type TranscriptDelta, type TranscriptItem } from "./core";

export interface TabView {
  slot: string;
  /** The session shown in this slot, if any. */
  shown: () => SessionId | null;
  /** Its transcript from absolute index `start()` on (the core sends the latest page first). */
  items: TranscriptItem[];
  start: () => number;
  /** Shows a session here; the slot's previous stream ends. */
  show: (id: SessionId) => Promise<void>;
  /** Shows nothing here (and stops this slot's stream). */
  hide: () => void;
  /** Prepends the page before the loaded items; returns how many items were added. */
  loadEarlier: () => Promise<number>;
}

export function createTabView(slot: string): TabView {
  const [shown, setShown] = createSignal<SessionId | null>(null);
  const [items, setItems] = createStore<TranscriptItem[]>([]);
  const [start, setStart] = createSignal(0);

  // Each show() gets a token; batches from an earlier stream (even of the same session) are dropped.
  let current = 0;
  const applyFor = (token: number) => (deltas: TranscriptDelta[]) => {
    if (token !== current) return;
    for (const delta of deltas) {
      if (delta.kind === "reset") {
        setStart(delta.start);
        setItems(reconcile(delta.items));
        continue;
      }
      const at = delta.index - start();
      if (at < 0) continue; // a change to an item before the loaded page
      if (delta.kind === "itemAdded" || delta.kind === "itemUpdated") setItems(at, delta.item);
      else
        setItems(
          produce((all) => {
            const item = all[at];
            if (item?.kind === "agent") item.text += delta.text;
          }),
        );
    }
  };

  const show = async (id: SessionId) => {
    const token = ++current;
    setShown(id);
    setItems([]);
    setStart(0);
    await core.showSession(id, applyFor(token), slot);
  };

  const hide = () => {
    ++current; // nothing more should stream in here
    setShown(null);
    setItems([]);
    void core.hideTabs(slot);
  };

  let loadingEarlier = false;
  const loadEarlier = async (): Promise<number> => {
    const id = shown();
    const before = start();
    if (id === null || before === 0 || loadingEarlier) return 0;
    loadingEarlier = true;
    const token = current;
    try {
      const page = await core.transcriptPageBefore(id, before);
      // Only prepend if nothing moved underneath us (another Tab shown, or a Reset).
      if (token !== current || start() !== before) return 0;
      // Items and start change together, so row keys (start + index) never point at the wrong item.
      batch(() => {
        setItems((now) => [...page.items, ...now]);
        setStart(page.start);
      });
      return page.items.length;
    } finally {
      loadingEarlier = false;
    }
  };

  return { slot, shown, items, start, show, hide, loadEarlier };
}

// The visible Tab's transcript (ticket 02): a virtualised list, so only on-screen rows are in the
// DOM. It sticks to the bottom while output streams (unless you've scrolled up) and fetches
// earlier pages from the core when you scroll near the top, keeping your place. A run of tool
// calls folds into one row that sums it up (ticket 37).
import { openUrl } from "@tauri-apps/plugin-opener";
import { createVirtualizer } from "@tanstack/solid-virtual";
import { createEffect, createMemo, createSignal, For, type JSX, on, Show } from "solid-js";
import { Dynamic } from "solid-js/web";
import type { SessionId, TranscriptItem } from "./core";
import { BookOpen, Brain, Check, ChevronRight, Globe, Loader, MoveRight, Pencil, Play, TextSearch, Trash2, Wrench } from "./icons";
import { handleTranscriptClick, renderMarkdown } from "./markdown";
import { PermissionCard } from "./PermissionCard";

/** How close to the bottom (px) still counts as "following" new output. */
const FOLLOW_SLACK = 48;
/** How close to the top (px) triggers loading the previous page. */
const LOAD_EARLIER_AT = 400;

type ToolCallItem = Extract<TranscriptItem, { kind: "toolCall" }>;

const TOOL_ICON: Record<string, typeof Wrench> = {
  read: BookOpen, edit: Pencil, delete: Trash2, move: MoveRight, search: TextSearch, execute: Play, think: Brain, fetch: Globe,
};

export function Transcript(props: {
  sessionId: SessionId;
  /** The items from absolute index `start` on; `start > 0` means earlier pages exist. */
  items: TranscriptItem[];
  start: number;
  onLoadEarlier: () => Promise<number>;
  onError: (message: string) => void;
  /** "Open in editor" on a code block. */
  onOpenSnippet: (code: string, label: string) => void;
  /** Whether Y / N answer this transcript's open card (in the Columns view: only the focused column's). */
  answerKeys?: () => boolean;
  /** How much of the bottom the floating composer covers: the list ends that far up. */
  bottomInset?: () => number;
}) {
  let scroller!: HTMLDivElement;
  let following = true;
  let loading = false;

  // The rows: each item on its own, except that a run of two or more tool calls is one row. A row's
  // key is the absolute index of its first item; `groupEnds` maps a group's key to its last item's.
  const rows = createMemo(() => {
    const keys: number[] = [];
    const groupEnds = new Map<number, number>();
    const items = props.items;
    for (let i = 0; i < items.length; i++) {
      keys.push(props.start + i);
      let end = i;
      while (items[i].kind === "toolCall" && items[end + 1]?.kind === "toolCall") end++;
      if (end > i) groupEnds.set(props.start + i, props.start + end);
      i = end;
    }
    return { keys, groupEnds };
  });

  const virtualizer = createVirtualizer({
    get count() {
      return rows().keys.length;
    },
    getScrollElement: () => scroller,
    estimateSize: () => 64,
    overscan: 8,
    // Keys are absolute transcript indexes, so measurements survive prepending earlier pages.
    getItemKey: (i) => rows().keys[i],
    // Room under the last item for the composer floating over the bottom; "the bottom" is above it.
    get paddingEnd() {
      return props.bottomInset?.() ?? 0;
    },
    get scrollPaddingEnd() {
      return props.bottomInset?.() ?? 0;
    },
  });

  // Rows keyed by their (stable) key, so re-measuring doesn't rebuild what's on screen.
  const rowKeys = createMemo(() => virtualizer.getVirtualItems().map((row) => row.key as number));
  const rowFor = (key: number) => virtualizer.getVirtualItems().find((row) => row.key === key);

  const toBottom = () => requestAnimationFrame(() => virtualizer.scrollToIndex(rows().keys.length - 1, { align: "end" }));
  // The composer grew (Edit notes, an error): keep the newest item above it when following.
  createEffect(on(() => props.bottomInset?.(), () => following && props.items.length > 0 && toBottom(), { defer: true }));

  // Follow new or growing output while the user is at the bottom.
  createEffect(
    on(
      () => {
        const last = props.items[props.items.length - 1];
        return [props.items.length, last && ("text" in last ? last.text.length : last.kind)];
      },
      () => {
        if (following && props.items.length > 0) toBottom();
        // A first page that doesn't fill the view never scrolls, so fetch more right away.
        requestAnimationFrame(() => {
          if (props.start > 0 && scroller.scrollHeight <= scroller.clientHeight) void loadEarlier();
        });
      },
    ),
  );

  const loadEarlier = async () => {
    if (loading || props.start === 0) return;
    loading = true;
    // The row at the top of the view, by absolute index, to keep it there after the prepend.
    const topRow = virtualizer.getVirtualItems().find((row) => row.end > scroller.scrollTop);
    const topIndex = topRow ? (topRow.key as number) : props.start;
    const offsetIntoRow = topRow ? scroller.scrollTop - topRow.start : 0;
    try {
      const added = await props.onLoadEarlier();
      if (added > 0)
        requestAnimationFrame(() => {
          // The row holding that item: a group may now start earlier, in the new page.
          const keys = rows().keys;
          let row = keys.length - 1;
          while (row > 0 && keys[row] > topIndex) row--;
          virtualizer.scrollToIndex(row, { align: "start" });
          scroller.scrollTop += offsetIntoRow;
        });
    } catch (err) {
      props.onError(String(err));
    } finally {
      loading = false;
    }
  };

  const onScroll = () => {
    following = scroller.scrollHeight - scroller.scrollTop - scroller.clientHeight < FOLLOW_SLACK;
    if (scroller.scrollTop < LOAD_EARLIER_AT) void loadEarlier();
  };

  return (
    <div
      class="transcript"
      ref={scroller}
      onScroll={onScroll}
      onClick={(e) => void handleTranscriptClick(e, openUrl, props.onOpenSnippet, props.onError)}
    >
      <div class="transcript-inner" style={{ height: `${virtualizer.getTotalSize()}px` }}>
        <For each={rowKeys()}>
          {(key) => {
            const row = () => rowFor(key);
            const groupEnd = createMemo(() => rows().groupEnds.get(key));
            return (
              <div
                class="row"
                data-index={row()?.index}
                ref={(el) => queueMicrotask(() => el.isConnected && virtualizer.measureElement(el))}
                style={{ transform: `translateY(${row()?.start ?? 0}px)` }}
              >
                <Show
                  when={groupEnd()}
                  fallback={renderItem(props.sessionId, props.items[key - props.start], props.answerKeys ?? always)}
                >
                  {(end) => (
                    <ToolCallGroup
                      sessionId={props.sessionId}
                      calls={props.items.slice(key - props.start, end() - props.start + 1) as ToolCallItem[]}
                    />
                  )}
                </Show>
              </div>
            );
          }}
        </For>
      </div>
    </div>
  );
}

const always = () => true;

function renderItem(sessionId: SessionId, item: TranscriptItem | undefined, answerKeys: () => boolean): JSX.Element {
  if (!item) return null;
  switch (item.kind) {
    case "agent":
      return <div class="msg agent markdown" innerHTML={renderMarkdown(item.text)} />;
    case "permission":
      return <PermissionCard sessionId={sessionId} request={item.request} outcome={item.outcome} keys={answerKeys} />;
    case "toolCall":
      return <ToolCallRow item={item} />;
    case "user":
      return (
        <>
          <Show when={item.editNotes?.length}>
            <div class="sent-notes" title="Your hand edits that went with this message">
              <For each={item.editNotes}>
                {(tag) => (
                  <span class="sent-note">
                    <Pencil />
                    {tag}
                  </span>
                )}
              </For>
            </div>
          </Show>
          <div class="msg user">{item.text}</div>
        </>
      );
    default:
      return <div class={`msg ${item.kind}`}>{item.text}</div>;
  }
}

function ToolCallRow(props: { item: ToolCallItem }) {
  return (
    <div class={`tool ${props.item.status}`}>
      <Dynamic component={TOOL_ICON[props.item.toolKind ?? ""] ?? Wrench} />
      <span class="title">{props.item.title}</span>
      {props.item.target && !props.item.title.includes(props.item.target) && <span class="target">{props.item.target}</span>}
      <span class="tool-status">
        {props.item.status === "completed" ? (
          <Check />
        ) : props.item.status === "inProgress" ? (
          <>
            <Loader class="spin" />
            running
          </>
        ) : props.item.status === "failed" ? (
          "failed"
        ) : (
          "queued"
        )}
      </span>
    </div>
  );
}

/** Whether each group is expanded, by session and the group's first call: kept while the app runs,
 * so a group scrolled away (or a Tab switched away from) comes back as it was left. */
const groupsOpen = new Map<SessionId, Map<string, boolean>>();

/** The summary's parts in their fixed order: a tool kind and how to say n calls of it. */
const SUMMARY: [string, (n: number) => string][] = [
  ["read", (n) => (n === 1 ? "read 1 file" : `read ${n} files`)],
  ["edit", (n) => (n === 1 ? "edited 1 file" : `edited ${n} files`)],
  ["execute", (n) => (n === 1 ? "ran 1 command" : `ran ${n} commands`)],
  ["search", (n) => `searched ${times(n)}`],
  ["fetch", (n) => `fetched ${times(n)}`],
];

const times = (n: number) => (n === 1 ? "once" : n === 2 ? "twice" : `${n} times`);

/** "Read 2 files, searched once": what a run of calls did, by kind. */
function summarize(calls: ToolCallItem[]): string {
  const parts = SUMMARY.flatMap(([kind, say]) => {
    const n = calls.filter((call) => call.toolKind === kind).length;
    return n > 0 ? [say(n)] : [];
  });
  const other = calls.filter((call) => !SUMMARY.some(([kind]) => kind === call.toolKind)).length;
  if (other > 0) parts.push(other === 1 ? "used 1 other tool" : `used ${other} other tools`);
  const text = parts.join(", ");
  return text.charAt(0).toUpperCase() + text.slice(1);
}

/** A run of tool calls as one row: collapsed by default, open from the start if a call failed. While
 * collapsed, a running call still shows under it. */
function ToolCallGroup(props: { sessionId: SessionId; calls: ToolCallItem[] }) {
  const id = () => props.calls[0].toolCallId;
  const saved = () => groupsOpen.get(props.sessionId)?.get(id());
  const [chosen, setChosen] = createSignal(saved());
  // The Tab or the group's first call changed under this row: take that group's state.
  createEffect(on([() => props.sessionId, id], () => setChosen(saved()), { defer: true }));
  const failed = () => props.calls.filter((call) => call.status === "failed").length;
  const open = () => chosen() ?? failed() > 0;
  const toggle = () => {
    const next = !open();
    let session = groupsOpen.get(props.sessionId);
    if (!session) groupsOpen.set(props.sessionId, (session = new Map()));
    session.set(id(), next);
    setChosen(next);
  };
  return (
    <>
      <button type="button" class="tool-group" classList={{ open: open() }} aria-expanded={open()} onClick={toggle}>
        <ChevronRight class="chev" />
        <span class="title">{summarize(props.calls)}</span>
        <Show when={failed() > 0}>
          <span class="tool-group-failed">{failed()} failed</span>
        </Show>
      </button>
      <For each={open() ? props.calls : props.calls.filter((call) => call.status === "inProgress")}>
        {(call) => (
          <div class="tool-group-call">
            <ToolCallRow item={call} />
          </div>
        )}
      </For>
    </>
  );
}

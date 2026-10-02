// The visible Tab's transcript (ticket 02): a virtualised list, so only on-screen rows are in the
// DOM. It sticks to the bottom while output streams (unless you've scrolled up) and fetches
// earlier pages from the core when you scroll near the top, keeping your place.
import { openUrl } from "@tauri-apps/plugin-opener";
import { createVirtualizer } from "@tanstack/solid-virtual";
import { createEffect, createMemo, For, type JSX, on } from "solid-js";
import type { SessionId, TranscriptItem } from "./core";
import { handleTranscriptClick, renderMarkdown } from "./markdown";
import { PermissionCard } from "./PermissionCard";

/** How close to the bottom (px) still counts as "following" new output. */
const FOLLOW_SLACK = 48;
/** How close to the top (px) triggers loading the previous page. */
const LOAD_EARLIER_AT = 400;

type ToolCallItem = Extract<TranscriptItem, { kind: "toolCall" }>;

const TOOL_ICON: Record<string, string> = {
  read: "📖", edit: "✏️", delete: "🗑", move: "↔", search: "🔍", execute: "▶", think: "💭", fetch: "🌐",
};
const STATUS_LABEL: Record<ToolCallItem["status"], string> = {
  pending: "pending", inProgress: "running", completed: "done", failed: "failed",
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
}) {
  let scroller!: HTMLDivElement;
  let following = true;
  let loading = false;

  const virtualizer = createVirtualizer({
    get count() {
      return props.items.length;
    },
    getScrollElement: () => scroller,
    estimateSize: () => 64,
    overscan: 8,
    // Keys are absolute transcript indexes, so measurements survive prepending earlier pages.
    getItemKey: (i) => props.start + i,
  });

  // Rows keyed by their (stable) key, so re-measuring doesn't rebuild what's on screen.
  const rowKeys = createMemo(() => virtualizer.getVirtualItems().map((row) => row.key as number));
  const rowFor = (key: number) => virtualizer.getVirtualItems().find((row) => row.key === key);

  const toBottom = () => requestAnimationFrame(() => virtualizer.scrollToIndex(props.items.length - 1, { align: "end" }));

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
    const topIndex = topRow ? props.start + topRow.index : props.start;
    const offsetIntoRow = topRow ? scroller.scrollTop - topRow.start : 0;
    try {
      const added = await props.onLoadEarlier();
      if (added > 0)
        requestAnimationFrame(() => {
          virtualizer.scrollToIndex(topIndex - props.start, { align: "start" });
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
            return (
              <div
                class="row"
                data-index={row()?.index}
                ref={(el) => queueMicrotask(() => el.isConnected && virtualizer.measureElement(el))}
                style={{ transform: `translateY(${row()?.start ?? 0}px)` }}
              >
                {renderItem(props.sessionId, props.items[key - props.start])}
              </div>
            );
          }}
        </For>
      </div>
    </div>
  );
}

function renderItem(sessionId: SessionId, item: TranscriptItem | undefined): JSX.Element {
  if (!item) return null;
  switch (item.kind) {
    case "agent":
      return <div class="msg agent markdown" innerHTML={renderMarkdown(item.text)} />;
    case "permission":
      return <PermissionCard sessionId={sessionId} request={item.request} outcome={item.outcome} />;
    case "toolCall":
      return <ToolCallRow item={item} />;
    default:
      return <div class={`msg ${item.kind}`}>{item.text}</div>;
  }
}

function ToolCallRow(props: { item: ToolCallItem }) {
  return (
    <div class={`tool ${props.item.status}`}>
      <span class="tool-icon">{TOOL_ICON[props.item.toolKind ?? ""] ?? "⚙"}</span>
      <b>{props.item.title}</b>
      {props.item.target && !props.item.title.includes(props.item.target) && <span class="mono muted">{props.item.target}</span>}
      <span class="grow" />
      <span class="tool-status">{STATUS_LABEL[props.item.status]}</span>
    </div>
  );
}

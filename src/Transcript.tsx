// The visible Tab's transcript (ticket 02): a virtualised list, so only on-screen rows are in the
// DOM. It sticks to the bottom while output streams (unless you've scrolled up) and fetches
// earlier pages from the core when you scroll near the top.
import { openUrl } from "@tauri-apps/plugin-opener";
import { createVirtualizer } from "@tanstack/solid-virtual";
import { createEffect, For, Match, on, Switch } from "solid-js";
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
  /** The items from absolute index `start` on. */
  items: TranscriptItem[];
  start: number;
  hasEarlier: boolean;
  onLoadEarlier: () => Promise<void>;
}) {
  let scroller!: HTMLDivElement;
  let following = true;

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

  const toBottom = () => requestAnimationFrame(() => virtualizer.scrollToIndex(props.items.length - 1, { align: "end" }));

  // Follow new or growing output while the user is at the bottom.
  createEffect(
    on(
      () => {
        const last = props.items[props.items.length - 1];
        return [props.items.length, last && ("text" in last ? last.text.length : last.kind)];
      },
      () => following && props.items.length > 0 && toBottom(),
    ),
  );

  let loading = false;
  const onScroll = async () => {
    following = scroller.scrollHeight - scroller.scrollTop - scroller.clientHeight < FOLLOW_SLACK;
    if (loading || !props.hasEarlier || scroller.scrollTop > LOAD_EARLIER_AT) return;
    loading = true;
    const heightBefore = virtualizer.getTotalSize();
    const offsetBefore = scroller.scrollTop;
    try {
      await props.onLoadEarlier();
      // Keep the rows you were looking at in place once the earlier page is above them.
      requestAnimationFrame(() => (scroller.scrollTop = offsetBefore + (virtualizer.getTotalSize() - heightBefore)));
    } finally {
      loading = false;
    }
  };

  return (
    <div
      class="transcript"
      ref={scroller}
      onScroll={() => void onScroll()}
      onClick={(e) => void handleTranscriptClick(e, openUrl)}
    >
      <div class="transcript-inner" style={{ height: `${virtualizer.getTotalSize()}px` }}>
        <For each={virtualizer.getVirtualItems()}>
          {(row) => (
            <div
              class="row"
              data-index={row.index}
              ref={(el) => queueMicrotask(() => virtualizer.measureElement(el))}
              style={{ transform: `translateY(${row.start}px)` }}
            >
              <Item sessionId={props.sessionId} item={props.items[row.index]} />
            </div>
          )}
        </For>
      </div>
    </div>
  );
}

function Item(props: { sessionId: SessionId; item: TranscriptItem | undefined }) {
  return (
    <Switch>
      <Match when={props.item?.kind === "agent" && props.item}>
        {(item) => <div class="msg agent markdown" innerHTML={renderMarkdown((item() as { text: string }).text)} />}
      </Match>
      <Match when={props.item?.kind === "permission" && (props.item as Extract<TranscriptItem, { kind: "permission" }>)}>
        {(item) => <PermissionCard sessionId={props.sessionId} request={item().request} outcome={item().outcome} />}
      </Match>
      <Match when={props.item?.kind === "toolCall" && (props.item as ToolCallItem)}>{(item) => <ToolCallRow item={item()} />}</Match>
      <Match when={props.item && "text" in props.item && props.item}>
        {(item) => <div class={`msg ${item().kind}`}>{(item() as { text: string }).text}</div>}
      </Match>
    </Switch>
  );
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

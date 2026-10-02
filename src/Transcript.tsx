// The visible Tab's transcript (ticket 02): a virtualised list, so only on-screen rows are in the
// DOM. It sticks to the bottom while output streams (unless you've scrolled up) and fetches
// earlier pages from the core when you scroll near the top, keeping your place.
import { openUrl } from "@tauri-apps/plugin-opener";
import { createVirtualizer } from "@tanstack/solid-virtual";
import { createEffect, createMemo, For, type JSX, on, Show } from "solid-js";
import { Dynamic } from "solid-js/web";
import type { SessionId, TranscriptItem } from "./core";
import { BookOpen, Brain, Check, Globe, Loader, MoveRight, Pencil, Play, TextSearch, Trash2, Wrench } from "./icons";
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
                {renderItem(props.sessionId, props.items[key - props.start], props.answerKeys ?? always)}
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

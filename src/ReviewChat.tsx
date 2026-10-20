// Claude in the Code review (issue 44): the review session's transcript and composer in a column on
// the Code review's right, in a view slot of its own. "Ask Claude"'s lines arrive in the composer as
// attachments; an answer to one can become a draft comment on those lines.
import { createEffect, createSignal, onCleanup, Show, untrack } from "solid-js";
import { addAttachment, Composer } from "./Chat";
import type { ReviewDock } from "./CodeReviewView";
import type { PermissionRequest, Question, SessionId, SessionInfo, TranscriptItem } from "./core";
import { Loader, MessageSquare, PanelRight, Sparkles, X } from "./icons";
import { StateDot } from "./StateDot";
import { STATE_LABEL } from "./stateLabels";
import { createTabView, type TabView } from "./TabView";
import { Transcript, type TranscriptHandle } from "./Transcript";

/** The user message asking `q`: it carries the question's attachment. */
const asks = (q: Question) => (item: TranscriptItem) => item.kind === "user" && !!item.attachments?.includes(q.attachment);

export function ReviewChat(props: {
  dock: ReviewDock;
  /** The review session as the app knows it (undefined until its Tab is listed). */
  session: SessionInfo | undefined;
  onOpenSnippet: (code: string, label: string) => void;
  onOpenProposal: (sessionId: SessionId, request: PermissionRequest) => void;
  /** The view slot already showing the session (the Tabs view's, when the review is its Tab), so
   *  it isn't streamed twice; without it, one of its own. */
  view?: TabView;
  /** `/clear` gave the review a fresh session. */
  onCleared?: (id: SessionId) => void;
}) {
  /** Said under the dock's head: a failure, or a question not found in the transcript. */
  const [note, setNote] = createSignal<{ text: string; error?: boolean } | null>(null);
  const onError = (message: string) => setNote({ text: message, error: true });
  const own = !props.view;
  const view = props.view ?? createTabView(`review:${props.dock.number}`);
  if (own) {
    createEffect(() => {
      const id = props.dock.session;
      if (id !== view.shown()) view.show(id).catch((err) => onError(String(err)));
    });
    onCleanup(() => view.hide());
  }
  const [composerHeight, setComposerHeight] = createSignal(0);

  // "Ask Claude": each attachment goes into the composer (or its draft, until it shows).
  createEffect(() => {
    if (props.dock.asked().length) for (const a of untrack(props.dock.takeAsked)) addAttachment(props.dock.session, a);
  });

  // A gutter marker: the message that asked about those lines, once the transcript has loaded.
  const [transcript, setTranscript] = createSignal<TranscriptHandle>();
  createEffect(() => {
    const q = props.dock.question();
    const handle = transcript();
    if (!q || !handle || view.items.length === 0) return;
    setNote(null);
    void handle.scrollTo(asks(q)).then((found) => !found && setNote({ text: `${q.attachment} hasn't been sent to Claude yet.` }), (err) => onError(String(err)));
  });

  /** The question an Agent message answers: only the last one before the user's next message (the
   *  end of the answer), after a message carrying a question's attachment. */
  const answering = (index: number): Question | undefined => {
    const items = view.items;
    const at = index - view.start();
    for (let i = at + 1; i < items.length; i++) {
      if (items[i].kind === "agent") return undefined;
      if (items[i].kind === "user") break;
    }
    for (let i = at - 1; i >= 0; i--) {
      const item = items[i];
      if (item.kind !== "user") continue;
      const all = props.dock.questions();
      for (let q = all.length - 1; q >= 0; q--) if (asks(all[q])(item)) return all[q];
      return undefined;
    }
    return undefined;
  };

  return (
    <>
      <div class="cr-dock-head">
        <Sparkles />
        <b>Claude</b>
        <Show when={props.session}>
          {(s) => (
            <>
              <span class="muted ellipsis">{s().name}</span>
              <StateDot state={s().state} title={STATE_LABEL[s().state]} />
            </>
          )}
        </Show>
        <span class="grow" />
        <button class="ghost icon" onClick={() => props.dock.onCollapse()} title="Hide Claude (the review session goes on)" aria-label="Hide Claude">
          <PanelRight />
        </button>
      </div>
      <Show when={note()}>
        {(n) => (
          <p class="cr-dock-note" classList={{ error: !!n().error }}>
            <span class="grow">{n().text}</span>
            <button class="ghost icon" onClick={() => setNote(null)} aria-label="Dismiss">
              <X />
            </button>
          </p>
        )}
      </Show>
      <div class="cr-dock-body">
        <Show
          when={props.session}
          keyed
          fallback={
            <p class="muted center review-loading">
              <Loader class="spin" /> Starting the review session…
            </p>
          }
        >
          {(s) => (
            <>
              <Transcript
                sessionId={s.id}
                items={view.items}
                start={view.start()}
                onLoadEarlier={view.loadEarlier}
                onError={onError}
                onOpenSnippet={props.onOpenSnippet}
                onOpenProposal={props.onOpenProposal}
                bottomInset={composerHeight}
                handle={setTranscript}
                underAgent={(index, text) => (
                  <Show when={answering(index)}>
                    {(q) => (
                      <div class="cr-use">
                        <button class="link" onClick={() => props.dock.draftComment(q(), text)} title={`A draft comment on ${q().attachment}, to edit before adding it`}>
                          <MessageSquare />
                          Use as comment
                        </button>
                      </div>
                    )}
                  </Show>
                )}
              />
              {/* (It doesn't take the keyboard by itself: J / K / V are the review's.) */}
              <Composer session={s} onHeight={setComposerHeight} takeFocus={false} onCleared={props.onCleared} />
            </>
          )}
        </Show>
      </div>
    </>
  );
}

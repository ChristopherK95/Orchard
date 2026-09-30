// An inline permission card (ticket 03): what the Agent wants to do, the diff for edits, and the
// Agent's own options by name. Y/N are handled for the whole Tab by `answerByKey`.
import { createSignal, For, onMount, Show } from "solid-js";
import {
  core,
  type PermissionOption,
  type PermissionOutcome,
  type PermissionRequest,
  type SessionId,
  type TranscriptItem,
} from "./core";

const DIFF_PREFIX = { hunk: "", context: " ", added: "+", removed: "−" } as const;

/** The option `Y` picks: allow once. */
export const yesOption = (request: PermissionRequest) => request.options.find((o) => o.kind === "allowOnce");
/** The option `N` picks: reject (once, else always). */
export const noOption = (request: PermissionRequest) =>
  request.options.find((o) => o.kind === "rejectOnce") ?? request.options.find((o) => o.kind === "rejectAlways");

/**
 * Answers the oldest open card on `Y`/`N`, unless a text field has focus. Returns true if the key
 * was used.
 */
export function answerByKey(event: KeyboardEvent, sessionId: SessionId, items: TranscriptItem[]): boolean {
  const key = event.key.toLowerCase();
  if ((key !== "y" && key !== "n") || event.ctrlKey || event.metaKey || event.altKey) return false;
  const focused = document.activeElement;
  if (focused instanceof HTMLInputElement || focused instanceof HTMLTextAreaElement || focused instanceof HTMLSelectElement)
    return false;
  if (focused instanceof HTMLElement && focused.isContentEditable) return false;
  const open = items.find((i) => i.kind === "permission" && i.outcome === null);
  if (open?.kind !== "permission") return false;
  const option = key === "y" ? yesOption(open.request) : noOption(open.request);
  if (!option) return false;
  void core.answerPermission(sessionId, open.request.toolCallId, option.id);
  return true;
}

export function PermissionCard(props: {
  sessionId: SessionId;
  request: PermissionRequest;
  outcome: PermissionOutcome | null;
}) {
  const [error, setError] = createSignal("");
  const [sending, setSending] = createSignal(false);
  const pending = () => props.outcome === null;
  let card!: HTMLDivElement;

  const answer = async (option: PermissionOption) => {
    if (!pending() || sending()) return;
    setSending(true);
    setError("");
    try {
      await core.answerPermission(props.sessionId, props.request.toolCallId, option.id);
    } catch (err) {
      setError(String(err));
    } finally {
      setSending(false);
    }
  };

  // Bring the question into view (and move focus off the disabled composer).
  onMount(() => pending() && card.focus());

  const chosen = () => {
    const outcome = props.outcome;
    if (!outcome) return null;
    if (outcome.kind === "cancelled") return "Cancelled: the turn ended before an answer";
    return `You chose: ${props.request.options.find((o) => o.id === outcome.optionId)?.name ?? outcome.optionId}`;
  };

  return (
    <div ref={card} class={`permission ${pending() ? "pending" : "answered"}`} tabindex={-1}>
      <div class="permission-head">
        <span class={`dot ${pending() ? "needsYou" : "idle"}`} />
        <b>{props.request.title}</b>
        <Show when={props.request.target && !props.request.title.includes(props.request.target)}>
          <span class="mono muted">{props.request.target}</span>
        </Show>
        <span class="grow" />
        <span class="muted">{pending() ? "Permission requested" : chosen()}</span>
      </div>
      <Show when={props.request.diff}>
        {(diff) => (
          <pre class="diff">
            <For each={diff()}>
              {(line) => (
                <span class={`line ${line.kind}`}>
                  {DIFF_PREFIX[line.kind]}
                  {line.text}
                </span>
              )}
            </For>
          </pre>
        )}
      </Show>
      <Show when={pending()}>
        <div class="permission-actions">
          <For each={props.request.options}>
            {(option) => (
              <button class={option.kind === "allowOnce" ? "primary" : ""} disabled={sending()} onClick={() => void answer(option)}>
                {option.name}
                <Show when={option === yesOption(props.request)}>
                  {" "}
                  <kbd>Y</kbd>
                </Show>
                <Show when={option === noOption(props.request)}>
                  {" "}
                  <kbd>N</kbd>
                </Show>
              </button>
            )}
          </For>
        </div>
      </Show>
      <Show when={error()}>
        <p class="error">{error()}</p>
      </Show>
    </div>
  );
}

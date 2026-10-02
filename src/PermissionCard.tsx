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
import { hasUnsavedChanges } from "./documents";
import { highlightCode, languageOfPath } from "./highlight";
import { Ban, Check, TriangleAlert } from "./icons";
import { StateDot } from "./StateDot";

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
  /** Whether Y / N answer this card's transcript (in the Columns view: only the focused column's). */
  keys?: () => boolean;
}) {
  const keys = () => props.keys?.() ?? true;
  const [error, setError] = createSignal("");
  const [sending, setSending] = createSignal(false);
  const pending = () => props.outcome === null;
  const language = () => languageOfPath(props.request.target);
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
  onMount(() => pending() && keys() && card.focus());

  const chosen = () => {
    const outcome = props.outcome;
    if (!outcome) return null;
    if (outcome.kind === "cancelled") return "Cancelled: the turn ended before an answer";
    return `You chose: ${props.request.options.find((o) => o.id === outcome.optionId)?.name ?? outcome.optionId}`;
  };
  const cancelled = () => props.outcome?.kind === "cancelled";
  const counts = () => {
    const diff = props.request.diff ?? [];
    return { added: diff.filter((l) => l.kind === "added").length, removed: diff.filter((l) => l.kind === "removed").length };
  };
  /** A command to run is shown itself (it is the thing being authorised). */
  const command = () => (!props.request.diff && props.request.kind === "execute" ? props.request.target : null);

  return (
    <div ref={card} class={`permission ${pending() ? "pending" : "answered"}`} classList={{ cancelled: cancelled() }} tabindex={-1}>
      <div class="permission-head">
        {pending() ? <StateDot state="needsYou" /> : cancelled() ? <Ban /> : <Check />}
        <span class="title">{props.request.title}</span>
        <Show when={props.request.target && !props.request.title.includes(props.request.target)}>
          <span class="target">{props.request.target}</span>
        </Show>
        <span class="outcome">{pending() ? "Permission requested" : chosen()}</span>
      </div>
      <Show when={pending() && props.request.file && hasUnsavedChanges(props.request.file)}>
        <p class="permission-warning">
          <TriangleAlert />
          You have unsaved changes in this file. If you allow this, you'll be asked which version to keep.
        </p>
      </Show>
      <Show when={props.request.diff || command()}>
        <div class="permission-body">
          <Show when={props.request.diff}>
            {(diff) => (
              <>
                <div class="permission-stat">
                  <span class="add">+{counts().added}</span>
                  <span class="del">−{counts().removed}</span>
                  <span>{props.request.target}</span>
                </div>
                <pre class="diff">
                  <For each={diff()}>
                    {(line) =>
                      line.kind === "hunk" ? (
                        <span class="line hunk">{line.text}</span>
                      ) : (
                        // Each line is highlighted on its own: cheap, and good enough for short snippets.
                        <span class={`line ${line.kind}`}>
                          {DIFF_PREFIX[line.kind]}
                          <span innerHTML={highlightCode(line.text, language())} />
                        </span>
                      )
                    }
                  </For>
                </pre>
              </>
            )}
          </Show>
          <Show when={command()}>
            {(cmd) => (
              <pre class="diff command">
                <span class="line">
                  <span class="dim">$ </span>
                  <span class="cmd">{cmd()}</span>
                </span>
              </pre>
            )}
          </Show>
        </div>
      </Show>
      <Show when={pending()}>
        <div class="permission-actions">
          <For each={props.request.options}>
            {(option) => (
              <button class={option.kind === "allowOnce" ? "primary" : ""} disabled={sending()} onClick={() => void answer(option)}>
                {option.name}
                <Show when={keys() && option === yesOption(props.request)}>
                  <kbd>Y</kbd>
                </Show>
                <Show when={keys() && option === noOption(props.request)}>
                  <kbd>N</kbd>
                </Show>
              </button>
            )}
          </For>
          <span class="hint">{keys() ? "Y / N answer the oldest open card" : "focus this column to answer with Y / N"}</span>
        </div>
      </Show>
      <Show when={error()}>
        <p class="error">{error()}</p>
      </Show>
    </div>
  );
}

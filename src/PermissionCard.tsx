// An inline permission card (ticket 03): what the Agent wants to do, the diff for edits, and the
// Agent's own options. Y answers with the allow-once option, N with the reject option.
import { createSignal, For, onMount, Show } from "solid-js";
import { core, type PermissionOption, type PermissionOutcome, type PermissionRequest, type SessionId } from "./core";

const DIFF_PREFIX = { hunk: "", context: " ", added: "+", removed: "−" } as const;

export function PermissionCard(props: {
  sessionId: SessionId;
  request: PermissionRequest;
  outcome: PermissionOutcome | null;
}) {
  const [error, setError] = createSignal("");
  const [sending, setSending] = createSignal(false);
  const pending = () => props.outcome === null;
  let card!: HTMLDivElement;

  const answer = async (option: PermissionOption | undefined) => {
    if (!option || !pending() || sending()) return;
    setSending(true);
    setError("");
    try {
      await core.answerPermission(props.sessionId, option.id);
    } catch (err) {
      setError(String(err));
    } finally {
      setSending(false);
    }
  };
  const optionOf = (kind: PermissionOption["kind"]) => props.request.options.find((o) => o.kind === kind);

  // Take focus while waiting, so Y/N work straight away (the composer is disabled meanwhile).
  onMount(() => pending() && card.focus());

  const chosen = () => {
    const outcome = props.outcome;
    if (!outcome) return null;
    if (outcome.kind === "cancelled") return "Cancelled: the turn ended before an answer";
    return `You chose: ${props.request.options.find((o) => o.id === outcome.optionId)?.name ?? outcome.optionId}`;
  };

  return (
    <div
      ref={card}
      class={`permission ${pending() ? "pending" : "answered"}`}
      tabindex={pending() ? 0 : -1}
      onKeyDown={(e) => {
        if (e.target !== card) return;
        if (e.key === "y" || e.key === "Y") void answer(optionOf("allowOnce"));
        if (e.key === "n" || e.key === "N") void answer(optionOf("rejectOnce") ?? optionOf("rejectAlways"));
      }}
    >
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
              <button
                class={option.kind === "allowOnce" ? "primary" : ""}
                disabled={sending()}
                onClick={() => void answer(option)}
              >
                {option.name}
                <Show when={option.kind === "allowOnce"}>
                  {" "}
                  <kbd>Y</kbd>
                </Show>
                <Show when={option === (optionOf("rejectOnce") ?? optionOf("rejectAlways"))}>
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

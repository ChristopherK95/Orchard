// The composer's context meter: how full the session's context window is, as a ring and a
// percentage, and on hover the tokens used and left, what the session has spent, and its cost.
import { createEffect, Show } from "solid-js";
import { createStore } from "solid-js/store";
import { core, type SessionId, type SessionUsage } from "./core";

/** Each session's usage as the core last announced it. */
const [bySession, setBySession] = createStore<Record<SessionId, SessionUsage>>({});
let listening = false;

const RING = 2 * Math.PI * 6; // the ring's circumference (radius 6)

/** "1,234", "12.3k", "1.2M". */
function tokens(n: number): string {
  if (n < 10_000) return n.toLocaleString("en-US");
  if (n < 1_000_000) return `${(n / 1000).toFixed(n < 100_000 ? 1 : 0)}k`;
  return `${(n / 1_000_000).toFixed(1)}M`;
}

export function ContextMeter(props: { session: SessionId }) {
  if (!listening) {
    listening = true;
    void core.onEvent((event) => event.kind === "usageChanged" && setBySession(event.sessionId, event.usage));
  }
  createEffect(() => {
    const id = props.session;
    if (bySession[id]) return;
    void core
      .sessionUsage(id)
      .then((usage) => !bySession[id] && setBySession(id, usage))
      .catch(() => {});
  });
  const usage = () => bySession[props.session];
  const context = () => {
    const u = usage();
    if (u?.contextUsed == null || !u.contextSize) return null;
    const used = Math.min(u.contextUsed, u.contextSize);
    return { used, size: u.contextSize, left: u.contextSize - used, percent: Math.round((used / u.contextSize) * 100) };
  };
  const spent = () => {
    const u = usage();
    return u ? u.inputTokens + u.outputTokens + u.cacheReadTokens + u.cacheWriteTokens : 0;
  };

  return (
    <Show when={context() || spent()}>
      <div
        class="context-meter"
        classList={{ high: (context()?.percent ?? 0) >= 70, full: (context()?.percent ?? 0) >= 90 }}
        tabIndex={0}
        aria-label={context() ? `Context ${context()!.percent}% full` : "Tokens used"}
      >
        <Show when={context()} fallback={<span class="value">{tokens(spent())} tokens</span>}>
          {(c) => (
            <>
              <svg viewBox="0 0 16 16" aria-hidden="true">
                <circle class="track" cx="8" cy="8" r="6" />
                <circle class="fill" cx="8" cy="8" r="6" stroke-dasharray={`${(RING * c().percent) / 100} ${RING}`} />
              </svg>
              <span class="value">{c().percent}%</span>
            </>
          )}
        </Show>
        <div class="context-popover" role="tooltip">
          <Show when={context()}>
            {(c) => (
              <>
                <div class="section-label">Context window</div>
                <div class="bar">
                  <span style={{ width: `${c().percent}%` }} />
                </div>
                <dl>
                  <dt>Used</dt>
                  <dd>
                    {tokens(c().used)} <span class="muted">/ {tokens(c().size)}</span>
                  </dd>
                  <dt>Left</dt>
                  <dd>{tokens(c().left)}</dd>
                </dl>
              </>
            )}
          </Show>
          <Show when={spent()}>
            <div class="section-label">Tokens this session</div>
            <dl>
              <dt>Input</dt>
              <dd>{tokens(usage()!.inputTokens)}</dd>
              <dt>Output</dt>
              <dd>{tokens(usage()!.outputTokens)}</dd>
              <dt>Cache read</dt>
              <dd>{tokens(usage()!.cacheReadTokens)}</dd>
              <dt>Cache write</dt>
              <dd>{tokens(usage()!.cacheWriteTokens)}</dd>
              <dt>Total</dt>
              <dd>{tokens(spent())}</dd>
            </dl>
          </Show>
          <Show when={usage()?.costUsd != null}>
            <dl>
              <dt>Cost</dt>
              <dd>${usage()!.costUsd!.toFixed(2)}</dd>
            </dl>
          </Show>
          <p class="muted">Not kept when the editor restarts: a restored session counts from its next turn.</p>
        </div>
      </div>
    </Show>
  );
}

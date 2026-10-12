// What a session card says besides its name and state, for the sidebar's cards and the Board's:
// how long since it was active, the model, how full its context is and what it has cost.
import { type Accessor, createSignal, Show } from "solid-js";
import { ago } from "./Chat";
import { contextOf, useUsage } from "./ContextMeter";
import type { SessionInfo } from "./core";
import { STATE_LABEL } from "./stateLabels";

/** The time, a minute at a time: so "5 min ago" moves on. */
const [minute, setMinute] = createSignal(Date.now());
setInterval(() => setMinute(Date.now()), 60_000);

/** How long ago, briefly: "now", "4m", "3h", "2d". */
function since(ms: number): string {
  const min = Math.floor((Date.now() - ms) / 60_000);
  if (min < 1) return "now";
  if (min < 60) return `${min}m`;
  if (min < 60 * 24) return `${Math.floor(min / 60)}h`;
  return `${Math.floor(min / (60 * 24))}d`;
}

/** "$0.42"; under a cent, "<$0.01". */
function dollars(usd: number): string {
  return usd > 0 && usd < 0.01 ? "<$0.01" : `$${usd.toFixed(2)}`;
}

export function useSessionFacts(session: Accessor<SessionInfo>) {
  const usage = useUsage(() => session().id);
  const context = () => contextOf(usage());
  const costUsd = () => usage()?.costUsd ?? null;
  const cost = () => (costUsd() === null ? null : dollars(costUsd()!));
  /** "3h", or "" if it never was. */
  const active = () => (minute(), session().lastActive === null ? "" : since(session().lastActive!));
  /** The model without its details ("Opus 4.8 (1M context)" is "Opus 4.8"); the title has them. */
  const model = () => session().model?.replace(/\s*\(.*\)\s*$/, "") || null;
  /** All of it, spelled out, for a tooltip. */
  const title = () => {
    const s = session();
    const last = (minute(), s.lastActive === null ? "" : ago(new Date(s.lastActive).toISOString()));
    return [s.name, STATE_LABEL[s.state], s.model, last && `Last active ${last}`, context() && `Context ${context()!.percent}% full`, cost() && `Cost ${cost()}`]
      .filter(Boolean)
      .join("\n");
  };
  return { context, cost, active, model, title };
}

/** How full the context window is: a short bar and the percentage, amber from 70 %, red from 90 %. */
export function ContextBar(props: { context: ReturnType<typeof contextOf> }) {
  return (
    <Show when={props.context}>
      {(c) => (
        <span class="context-bar" classList={{ high: c().percent >= 70, full: c().percent >= 90 }}>
          <span class="track">
            <span style={{ width: `${c().percent}%` }} />
          </span>
          {c().percent}%
        </span>
      )}
    </Show>
  );
}

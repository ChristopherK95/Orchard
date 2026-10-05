// The Agent session state dot: one silhouette per state, so it reads without colour (Foundations ·
// Agent session states). Drawn on a 14 px box; the Working ring breathes unless motion is reduced.
import { Match, Switch } from "solid-js";
import type { SessionState } from "./core";
import { STATE_LABEL } from "./stateLabels";

export function StateDot(props: { state: SessionState; title?: string }) {
  return (
    <svg class={`state-dot ${props.state}`} viewBox="0 0 14 14" width="14" height="14" aria-hidden={props.title ? undefined : "true"}>
      {props.title && <title>{props.title}</title>}
      <Switch>
        <Match when={props.state === "working"}>
          <circle class="pulse" cx="7" cy="7" r="6" fill="none" stroke="currentColor" stroke-width="1" />
          <circle cx="7" cy="7" r="3" fill="currentColor" />
        </Match>
        <Match when={props.state === "needsYou"}>
          <circle cx="7" cy="7" r="6.5" fill="none" stroke="currentColor" stroke-width="1" opacity="0.32" />
          <circle cx="7" cy="7" r="4.3" fill="none" stroke="currentColor" stroke-width="1.4" />
          <circle cx="7" cy="7" r="2.25" fill="currentColor" />
        </Match>
        <Match when={props.state === "idle"}>
          <circle cx="7" cy="7" r="3.5" fill="currentColor" />
        </Match>
        <Match when={props.state === "suspended"}>
          <circle cx="7" cy="7" r="3.2" fill="none" stroke="currentColor" stroke-width="1.3" />
        </Match>
        <Match when={props.state === "exited"}>
          <path d="M7 3.6 L11.2 11 H2.8 Z" fill="currentColor" stroke="currentColor" stroke-width="0.8" stroke-linejoin="round" />
        </Match>
      </Switch>
    </svg>
  );
}

/** A session's state as a pill: a dot and its name, tinted by state (Badge · session state). */
export function StateBadge(props: { state: SessionState; label?: string }) {
  return (
    <span class={`state-badge ${props.state}`}>
      <span class="dot" />
      {props.label ?? STATE_LABEL[props.state]}
    </span>
  );
}

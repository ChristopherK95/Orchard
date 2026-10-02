// Worktree setup (ticket 08), shown where the Worktree's first Tab will be: each command and how it
// went, the output as it streams, and Retry / Start anyway once a command fails.
import { createEffect, createSignal, For, Show } from "solid-js";
import { core, type SetupStatus } from "./core";

/** What the view knows of a setup; built from events, so nothing printed before it opened is lost. */
export interface SetupView {
  commands: string[];
  status: SetupStatus;
  output: string;
}

/** The frontend keeps as much output as the core does. */
export const SETUP_OUTPUT_LIMIT = 256 * 1024;

export function keepOutput(output: string, text: string) {
  const all = output + text;
  return all.length > SETUP_OUTPUT_LIMIT ? all.slice(all.length - SETUP_OUTPUT_LIMIT) : all;
}

type StepState = "done" | "running" | "failed" | "pending";

function stepState(status: SetupStatus, step: number): StepState {
  switch (status.kind) {
    case "running":
      return step < status.step ? "done" : step === status.step ? "running" : "pending";
    case "failed":
      return step < status.step ? "done" : step === status.step ? "failed" : "pending";
    default:
      return "done"; // (or skipped by Start anyway)
  }
}

const STEP_ICON: Record<StepState, string> = { done: "✓", running: "…", failed: "✗", pending: "·" };

export function Setup(props: {
  worktree: string;
  setup: SetupView;
  onError: (message: string) => void;
  /** Opens the repo's settings (to fix its setup commands). */
  onOpenSettings: () => void;
}) {
  let output!: HTMLPreElement;
  // Follow the output while it streams, unless the user scrolled up to read.
  let following = true;
  createEffect(() => {
    void props.setup.output;
    if (following) output.scrollTop = output.scrollHeight;
  });

  // One Retry / Start anyway at a time (the core takes only the first anyway).
  const [busy, setBusy] = createSignal(false);
  const act = (action: () => Promise<void>) => () => {
    if (busy()) return;
    setBusy(true);
    action()
      .catch((err) => props.onError(String(err)))
      .finally(() => setBusy(false));
  };
  const failed = () => {
    const status = props.setup.status;
    return status.kind === "failed" || status.kind === "sessionFailed" ? status : null;
  };
  const heading = () => {
    switch (props.setup.status.kind) {
      case "running":
        return "Setting up this Worktree…";
      case "startingSession":
        return "Starting the Agent session…";
      case "failed":
        return "Worktree setup failed";
      case "sessionFailed":
        return "The Agent session didn't start";
      case "done":
        return "Worktree set up";
    }
  };

  return (
    <div class="setup">
      <div class="setup-head">
        <b classList={{ error: !!failed() }}>{heading()}</b>
        <span class="grow" />
        <button class="ghost" onClick={() => props.onOpenSettings()} title="Edit this repo's setup commands">
          Repo settings
        </button>
      </div>
      <ol class="setup-steps">
        <For each={props.setup.commands}>
          {(command, i) => (
            <li class={stepState(props.setup.status, i())}>
              <span class="setup-icon">{STEP_ICON[stepState(props.setup.status, i())]}</span>
              <span class="mono">{command}</span>
            </li>
          )}
        </For>
      </ol>
      <pre
        ref={output}
        class="setup-output mono"
        onScroll={() => (following = output.scrollTop + output.clientHeight >= output.scrollHeight - 4)}
      >
        {props.setup.output}
      </pre>
      <Show when={failed()}>
        {(f) => (
          <div class="setup-actions">
            <span class="error grow">{f().message}</span>
            <Show when={f().kind === "failed"}>
              <button onClick={act(() => core.startAnyway(props.worktree))} disabled={busy()}>
                Start anyway
              </button>
            </Show>
            <button class="primary" onClick={act(() => core.retrySetup(props.worktree))} disabled={busy()}>
              {f().kind === "failed" ? "Retry from this command" : "Retry"}
            </button>
          </div>
        )}
      </Show>
    </div>
  );
}

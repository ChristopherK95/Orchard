// The windows draw their own title bar (no native decorations), so it fits the app: the app's bar
// is the drag region (double-click maximises) and these are its minimise / maximise / close
// buttons, at the right edge. Close asks the window to close, so a pop-out with unsaved changes
// can still ask first.
import { getCurrentWindow } from "@tauri-apps/api/window";
import { createSignal, type JSX, onCleanup, onMount } from "solid-js";

export function WindowControls() {
  const window = getCurrentWindow();
  const [maximized, setMaximized] = createSignal(false);
  // (Registered before the awaits, so Solid ties it to this component; a switched-away
  // Workspace's title bar goes with it.)
  let stop: (() => void) | undefined;
  let gone = false;
  onCleanup(() => {
    gone = true;
    stop?.();
  });
  onMount(async () => {
    setMaximized(await window.isMaximized());
    const unlisten = await window.onResized(async () => setMaximized(await window.isMaximized()));
    if (gone) unlisten();
    else stop = unlisten;
  });
  const glyph = (d: string) => (
    <svg viewBox="0 0 10 10" width="10" height="10" aria-hidden="true">
      <path d={d} fill="none" stroke="currentColor" stroke-width="1" shape-rendering="crispEdges" />
    </svg>
  );
  return (
    <div class="window-controls">
      <button tabIndex={-1} onClick={() => void window.minimize()} title="Minimise" aria-label="Minimise">
        {glyph("M0 5.5h10")}
      </button>
      <button tabIndex={-1} onClick={() => void window.toggleMaximize()} title={maximized() ? "Restore" : "Maximise"} aria-label={maximized() ? "Restore" : "Maximise"}>
        {maximized() ? glyph("M2.5 2.5V0.5h7v7h-2M0.5 2.5h7v7h-7z") : glyph("M0.5 0.5h9v9h-9z")}
      </button>
      <button class="close" tabIndex={-1} onClick={() => void window.close()} title="Close" aria-label="Close">
        {glyph("M0 0l10 10M10 0L0 10")}
      </button>
    </div>
  );
}

/** A bare title bar for windows without the app's (startup, pop-outs): a title, drag, controls. */
export function BareTitlebar(props: { children?: JSX.Element }) {
  return (
    <header class="titlebar bare" data-tauri-drag-region>
      <span class="name" data-tauri-drag-region>
        {props.children ?? "Agent Editor"}
      </span>
      <span class="grow" data-tauri-drag-region />
      <WindowControls />
    </header>
  );
}

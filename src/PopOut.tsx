// A popped-out Manual editor (ticket 15): its own OS window holding a file moved out of the pane,
// with the same save and conflict rules. The core holds the window's file until the window is
// gone, so a reload of this window collects it again. Closing it with unsaved changes asks first.
import { getCurrentWindow } from "@tauri-apps/api/window";
import { createEffect, createSignal, lazy, onCleanup, onMount, Show } from "solid-js";
import { core, type LoadedSettings } from "./core";
import type { OpenRequest } from "./ManualEditor";
import { BareTitlebar } from "./WindowControls";
import { applyAppearance } from "./appearance";

const ManualEditor = lazy(() => import("./ManualEditor").then((m) => ({ default: m.ManualEditor })));

export function PopOut() {
  const [requests, setRequests] = createSignal<{ open: OpenRequest; n: number }[]>([]);
  const [settings, setSettings] = createSignal<LoadedSettings | null>(null);
  createEffect(() => settings() && applyAppearance(settings()!.settings.appearance));
  const [error, setError] = createSignal("");
  const [dirty, setDirty] = createSignal(false);
  const [confirmClose, setConfirmClose] = createSignal(false);
  let saveAll: (() => Promise<boolean>) | undefined;
  const window = getCurrentWindow();

  const closeNow = () => void window.destroy();
  const saveAndClose = async () => {
    if (saveAll && (await saveAll())) closeNow();
    else setConfirmClose(false); // (a save was refused or failed: its banner says why)
  };

  // Registered before anything is awaited, so Solid ties them to this component.
  const stops: (() => void)[] = [];
  onCleanup(() => stops.forEach((stop) => stop()));

  onMount(async () => {
    stops.push(
      await window.onCloseRequested((event) => {
        if (!dirty()) return;
        event.preventDefault();
        setConfirmClose(true);
      }),
    );
    stops.push(await core.onEvent((event) => event.kind === "settingsChanged" && setSettings(event.settings)));
    setSettings(await core.settings());
    try {
      const file = await core.collectPopOut();
      setRequests([{ open: { kind: "poppedOut", file }, n: 1 }]);
    } catch (err) {
      setError(`Couldn't show the file: ${err}`);
    }
  });

  const [title, setTitle] = createSignal("");
  onMount(async () => setTitle(await window.title()));

  return (
    <div class="workspace popout">
      <BareTitlebar>{title()}</BareTitlebar>
      <Show when={confirmClose()}>
        <div class="editor-banner warning">
          <span class="grow">This window has unsaved changes. Close it anyway?</span>
          <button class="primary" onClick={() => void saveAndClose()}>
            Save and close
          </button>
          <button onClick={closeNow}>Close without saving</button>
          <button class="ghost" onClick={() => setConfirmClose(false)}>
            Cancel
          </button>
        </div>
      </Show>
      <Show when={error()}>
        <p class="error banner">{error()}</p>
      </Show>
      <div class="main-row">
        <ManualEditor
          requests={requests()}
          vim={settings()?.settings.editor?.vim ?? false}
          poppedOut
          onEmpty={closeNow}
          onError={setError}
          onDirtyChange={setDirty}
          controls={(c) => (saveAll = c.saveAll)}
        />
      </div>
    </div>
  );
}

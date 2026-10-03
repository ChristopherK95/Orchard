// The settings page (ticket 31): an overlay over the Workspace with the Application settings
// (General, Agents) and the open repo's settings. It edits settings.toml, keeping its comments and
// layout: toggles apply at once, fields when they lose focus or on Enter. While the file doesn't
// parse, the controls are locked (nothing is written over it) and the error says why.
import { createEffect, createSignal, For, type JSX, on, onCleanup, onMount, Show } from "solid-js";
import { core, type FontFamily, type LoadedSettings, type RepoSettings, type SettingChange } from "./core";
import { CODE_FONT_SIZES } from "./appearance";
import { ArrowDown, ArrowUp, ChevronDown, Plus, TriangleAlert, X } from "./icons";

export type SettingsSection = "general" | "appearance" | "agents" | "repo";

export function SettingsPage(props: {
  settings: LoadedSettings | null;
  /** The open repo's name. */
  repoName: string;
  section: SettingsSection;
  /** Opens settings.toml in the Manual editor. */
  onOpenFile: () => void;
  onClose: () => void;
}) {
  const [section, setSection] = createSignal(props.section);
  const [repo, setRepo] = createSignal<RepoSettings | null>(null);
  const [error, setError] = createSignal("");
  /** "Different commands on Windows / Linux" is open (it starts open if the file has either). */
  const [perOs, setPerOs] = createSignal<boolean | null>(null);
  const broken = () => !!props.settings?.error;
  /** The installed fonts, once the core has found them. */
  const [fonts, setFonts] = createSignal<FontFamily[] | null>(null);
  onMount(() => void core.installedFonts().then(setFonts, () => setFonts([])));
  const app = () => props.settings?.settings;

  const loadRepo = async () => {
    try {
      const now = await core.repoSettings();
      setRepo(now);
      if (perOs() === null) setPerOs(now.setupWindows !== null || now.setupLinux !== null);
    } catch (err) {
      setError(String(err));
    }
  };
  // (And again whenever the file changes, by this page or by hand.)
  createEffect(on(() => props.settings, () => void loadRepo()));

  const change = async (...changes: SettingChange[]) => {
    setError("");
    try {
      for (const c of changes) await core.changeSetting(c);
    } catch (err) {
      setError(String(err));
    }
    await loadRepo();
  };

  onMount(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== "Escape") return;
      e.preventDefault();
      // A field being edited is applied first (its change fires on blur).
      if (document.activeElement instanceof HTMLInputElement) document.activeElement.blur();
      props.onClose();
    };
    window.addEventListener("keydown", onKey);
    onCleanup(() => window.removeEventListener("keydown", onKey));
  });

  const nav = (id: SettingsSection, label: JSX.Element) => (
    <button class="settings-nav-item" classList={{ on: section() === id }} onClick={() => setSection(id)}>
      {label}
    </button>
  );

  return (
    <div class="modal-backdrop" onClick={(e) => e.target === e.currentTarget && props.onClose()}>
      <div class="modal settings-page" role="dialog" aria-label="Settings">
        <div class="modal-head">
          <b>Settings</b>
          <button class="ghost icon" onClick={() => props.onClose()} title="Close (Esc)" aria-label="Close">
            <X />
          </button>
        </div>
        <div class="settings-main">
          <nav class="settings-nav">
            <div class="palette-group">Application</div>
            {nav("general", "General")}
            {nav("appearance", "Appearance")}
            {nav("agents", "Agents")}
            <div class="palette-group">Repository</div>
            {nav("repo", <span class="mono">{props.repoName}</span>)}
          </nav>
          <div class="settings-content">
            <Show when={props.settings?.error}>
              {(message) => (
                <div class="settings-broken">
                  <TriangleAlert />
                  <div>
                    <p>The settings file has an error, so these can't be changed here until it's fixed:</p>
                    <p class="mono small">{message()}</p>
                    <button onClick={() => props.onOpenFile()}>Open settings file</button>
                  </div>
                </div>
              )}
            </Show>
            <Show when={error()}>
              <p class="error small">{error()}</p>
            </Show>
            <fieldset class="settings-fields" disabled={broken() || !app()}>
              <Show when={section() === "general"}>
                <h2>General</h2>
                <Toggle
                  label="Vim keybindings in the Manual editor"
                  hint=":w saves, :q closes the file tab."
                  on={app()?.editor.vim ?? false}
                  onChange={(on) => void change({ kind: "vim", on })}
                />
                <Toggle
                  label="Notify when a session finishes its turn"
                  hint="For a Tab you're not looking at. A session that needs you always notifies."
                  on={app()?.notifications.turnFinished ?? false}
                  onChange={(on) => void change({ kind: "turnFinished", on })}
                />
              </Show>
              <Show when={section() === "appearance"}>
                <h2>Appearance</h2>
                <Row label="Interface font" hint="Menus, the chat, and everything else that isn't code.">
                  <FontPicker
                    value={app()?.appearance.uiFont ?? null}
                    defaultName="Inter"
                    fonts={fonts()}
                    onChange={(family) => void change({ kind: "uiFont", family })}
                  />
                </Row>
                <Row label="Code font" hint="The Manual editor, code in the chat, paths and branch names.">
                  <FontPicker
                    value={app()?.appearance.codeFont ?? null}
                    defaultName="JetBrains Mono"
                    fonts={fonts()}
                    monospace
                    onChange={(family) => void change({ kind: "codeFont", family })}
                  />
                </Row>
                <Row label="Code font size" hint={`In the Manual editor (${CODE_FONT_SIZES.min}–${CODE_FONT_SIZES.max}).`}>
                  <NumberField
                    value={app()?.appearance.codeFontSize ?? CODE_FONT_SIZES.default}
                    unit="px"
                    min={CODE_FONT_SIZES.min}
                    max={CODE_FONT_SIZES.max}
                    onCommit={(px) => void change({ kind: "codeFontSize", px })}
                  />
                </Row>
                <Toggle
                  label="Font ligatures"
                  hint="Draw => and != as single symbols where code is shown, if the code font has them."
                  on={app()?.appearance.ligatures ?? true}
                  onChange={(on) => void change({ kind: "ligatures", on })}
                />
              </Show>
              <Show when={section() === "agents"}>
                <h2>Agents</h2>
                <Row label="Memory limit" hint="When the Agents together use more than this, the longest-Idle sessions are Suspended until they're under it. 0: no limit.">
                  <NumberField value={app()?.agents.memoryLimitMb ?? 0} unit="MB" onCommit={(mb) => void change({ kind: "memoryLimitMb", mb })} />
                </Row>
                <Toggle
                  label="Suspend Idle sessions"
                  hint="Stops the Agent of a session left Idle; sending it a message brings it back."
                  on={app()?.agents.idleSuspend ?? false}
                  onChange={(on) => void change({ kind: "idleSuspend", on })}
                />
                <Row label="After" hint="How long a session may be Idle first. 0: never." indent>
                  <NumberField
                    value={app()?.agents.idleSuspendMinutes ?? 0}
                    unit="minutes"
                    disabled={!app()?.agents.idleSuspend}
                    onCommit={(minutes) => void change({ kind: "idleSuspendMinutes", minutes })}
                  />
                </Row>
              </Show>
              <Show when={section() === "repo"}>
                <h2>
                  Repository: <span class="mono">{props.repoName}</span>
                </h2>
                <Show when={repo()} fallback={<p class="muted small">Loading…</p>}>
                  {(r) => (
                    <>
                      <Row label="Worktree setup" hint="Run in order in each new Worktree before its first Agent session, stopping at the first failure." block>
                        <CommandList commands={r().setup} onCommit={(commands) => void change({ kind: "setup", commands })} />
                      </Row>
                      <Row label="Windows shell" hint="What setup runs in on Windows (Linux always uses bash).">
                        <div class="segmented">
                          <button classList={{ on: r().windowsShell === "powershell" }} onClick={() => void change({ kind: "windowsShell", shell: "powershell" })}>
                            PowerShell
                          </button>
                          <button classList={{ on: r().windowsShell === "git-bash" }} onClick={() => void change({ kind: "windowsShell", shell: "git-bash" })}>
                            Git Bash
                          </button>
                        </div>
                      </Row>
                      <Toggle
                        label="Different commands on Windows / Linux"
                        hint="Replace the list above on one OS. Turning this off removes both."
                        on={perOs() ?? false}
                        onChange={(on) => {
                          setPerOs(on);
                          if (!on && (r().setupWindows !== null || r().setupLinux !== null))
                            void change({ kind: "setupWindows", commands: null }, { kind: "setupLinux", commands: null });
                        }}
                      />
                      <Show when={perOs()}>
                        <Row label="On Windows" hint="Empty: the list above." block indent>
                          <CommandList commands={r().setupWindows ?? []} onCommit={(c) => void change({ kind: "setupWindows", commands: c.length ? c : null })} />
                        </Row>
                        <Row label="On Linux" hint="Empty: the list above." block indent>
                          <CommandList commands={r().setupLinux ?? []} onCommit={(c) => void change({ kind: "setupLinux", commands: c.length ? c : null })} />
                        </Row>
                      </Show>
                    </>
                  )}
                </Show>
              </Show>
            </fieldset>
          </div>
        </div>
        <div class="modal-foot">
          <span class="hint">Saved to settings.toml as you change them.</span>
          <button class="ghost" onClick={() => props.onOpenFile()}>
            Open settings file
          </button>
        </div>
      </div>
    </div>
  );
}

function Row(props: { label: string; hint?: string; block?: boolean; indent?: boolean; children: JSX.Element }) {
  return (
    <div class="settings-row" classList={{ block: props.block, indent: props.indent }}>
      <div class="settings-label">
        <div>{props.label}</div>
        <Show when={props.hint}>
          <div class="hint">{props.hint}</div>
        </Show>
      </div>
      <div class="settings-control">{props.children}</div>
    </div>
  );
}

function Toggle(props: { label: string; hint?: string; on: boolean; onChange: (on: boolean) => void }) {
  return (
    <label class="settings-row toggle">
      <input type="checkbox" checked={props.on} onChange={(e) => props.onChange(e.currentTarget.checked)} />
      <div class="settings-label">
        <div>{props.label}</div>
        <Show when={props.hint}>
          <div class="hint">{props.hint}</div>
        </Show>
      </div>
    </label>
  );
}

/** A whole number in range (≥ 0 by default), applied when it loses focus or on Enter (anything
 *  else is put back). */
function NumberField(props: { value: number; unit: string; min?: number; max?: number; disabled?: boolean; onCommit: (n: number) => void }) {
  return (
    <span class="settings-number">
      <input
        type="number"
        min={props.min ?? 0}
        max={props.max}
        step="1"
        value={props.value}
        disabled={props.disabled}
        onChange={(e) => {
          const n = Number(e.currentTarget.value);
          if (e.currentTarget.value.trim() === "" || !Number.isInteger(n) || n < (props.min ?? 0) || (props.max !== undefined && n > props.max)) {
            e.currentTarget.value = String(props.value);
            return;
          }
          if (n !== props.value) props.onCommit(n);
        }}
        onKeyDown={(e) => e.key === "Enter" && e.currentTarget.blur()}
      />
      <span class="muted small">{props.unit}</span>
    </span>
  );
}

/** Commands to edit, add, remove and reorder. Edits apply when a field loses focus or on Enter;
 *  the rest at once. Empty commands are dropped. */
function CommandList(props: { commands: string[]; onCommit: (commands: string[]) => void }) {
  const [rows, setRows] = createSignal<string[]>([]);
  createEffect(on(() => props.commands, (commands) => setRows([...commands])));
  let list!: HTMLDivElement;

  const commit = (next: string[]) => {
    setRows(next);
    const kept = next.map((c) => c.trim()).filter(Boolean);
    if (kept.join("\n") !== props.commands.join("\n")) props.onCommit(kept);
  };
  const move = (at: number, by: number) => {
    const next = [...rows()];
    [next[at], next[at + by]] = [next[at + by], next[at]];
    commit(next);
  };

  return (
    <div class="settings-commands" ref={list}>
      <For each={rows()} fallback={<p class="muted small">No commands.</p>}>
        {(command, i) => (
          <div class="settings-command">
            <input
              class="mono"
              value={command}
              placeholder="e.g. pnpm install"
              spellcheck={false}
              onChange={(e) => commit(rows().map((c, j) => (j === i() ? e.currentTarget.value : c)))}
              onKeyDown={(e) => e.key === "Enter" && e.currentTarget.blur()}
            />
            <button class="ghost icon" disabled={i() === 0} onClick={() => move(i(), -1)} title="Move up" aria-label="Move up">
              <ArrowUp />
            </button>
            <button class="ghost icon" disabled={i() === rows().length - 1} onClick={() => move(i(), 1)} title="Move down" aria-label="Move down">
              <ArrowDown />
            </button>
            <button class="ghost icon" onClick={() => commit(rows().filter((_, j) => j !== i()))} title="Remove" aria-label="Remove">
              <X />
            </button>
          </div>
        )}
      </For>
      <button
        class="ghost add-command"
        onClick={() => {
          setRows([...rows(), ""]);
          list.querySelectorAll("input").item(rows().length - 1)?.focus();
        }}
      >
        <Plus />
        Add command
      </button>
    </div>
  );
}

/** An installed font, or the bundled default (`null`), picked from a list filtered by typing. Each
 *  name is shown in its own font. A font named in the file but not installed says so. */
function FontPicker(props: {
  value: string | null;
  defaultName: string;
  fonts: FontFamily[] | null;
  /** List monospace fonts first, marked (for the code font). */
  monospace?: boolean;
  onChange: (family: string | null) => void;
}) {
  const [open, setOpen] = createSignal(false);
  const [filter, setFilter] = createSignal("");
  const [selected, setSelected] = createSignal(0);
  let box!: HTMLDivElement;

  const missing = () => !!props.value && !!props.fonts && !props.fonts.some((f) => f.name.toLowerCase() === props.value!.toLowerCase());
  /** null: the default. */
  const options = (): (FontFamily | null)[] => {
    const words = filter().trim().toLowerCase();
    let list = (props.fonts ?? []).filter((f) => !words || f.name.toLowerCase().includes(words));
    if (props.monospace) list = [...list.filter((f) => f.monospace), ...list.filter((f) => !f.monospace)];
    const showDefault = !words || `default ${props.defaultName}`.toLowerCase().includes(words);
    return showDefault ? [null, ...list] : list;
  };
  const pick = (family: FontFamily | null | undefined) => {
    if (family === undefined) return;
    setOpen(false);
    if ((family?.name ?? null) !== props.value) props.onChange(family?.name ?? null);
  };
  const css = (name: string) => `"${name.replace(/["\\]/g, "\\$&")}", ${props.monospace ? "monospace" : "sans-serif"}`;

  onMount(() => {
    // A click anywhere else closes the list.
    const onDown = (e: MouseEvent) => open() && !box.contains(e.target as Node) && setOpen(false);
    window.addEventListener("mousedown", onDown);
    onCleanup(() => window.removeEventListener("mousedown", onDown));
  });

  return (
    <div class="font-picker" ref={box}>
      <button
        class="font-picker-button"
        onClick={() => {
          setFilter("");
          setSelected(0);
          setOpen((now) => !now);
        }}
        style={{ "font-family": props.value ? css(props.value) : undefined }}
      >
        <span class="grow">{props.value ?? `Default (${props.defaultName})`}</span>
        <Show when={missing()}>
          <span class="not-installed">Not installed</span>
        </Show>
        <ChevronDown />
      </button>
      <Show when={open()}>
        <div class="font-picker-menu">
          <input
            ref={(el) => setTimeout(() => el.focus())}
            placeholder="Filter fonts"
            value={filter()}
            spellcheck={false}
            onInput={(e) => {
              setFilter(e.currentTarget.value);
              setSelected(0);
            }}
            onKeyDown={(e) => {
              const count = options().length;
              if (e.key === "ArrowDown") {
                e.preventDefault();
                setSelected((i) => (count ? (i + 1) % count : 0));
              } else if (e.key === "ArrowUp") {
                e.preventDefault();
                setSelected((i) => (count ? (i - 1 + count) % count : 0));
              } else if (e.key === "Enter") {
                e.preventDefault();
                pick(options()[selected()]);
              } else if (e.key === "Escape") {
                // (Closes the list, not the settings page.)
                e.preventDefault();
                e.stopPropagation();
                setOpen(false);
              }
            }}
          />
          <div class="font-picker-list">
            <Show when={props.fonts} fallback={<p class="muted small palette-empty">Looking for installed fonts…</p>}>
              <For each={options()} fallback={<p class="muted small palette-empty">No installed font matches.</p>}>
                {(family, i) => (
                  <button
                    class="palette-item"
                    classList={{ on: i() === selected(), current: (family?.name ?? null) === props.value }}
                    ref={(el) => createEffect(() => i() === selected() && el.scrollIntoView({ block: "nearest" }))}
                    onMouseEnter={() => setSelected(i())}
                    onClick={() => pick(family)}
                    style={{ "font-family": family ? css(family.name) : undefined }}
                  >
                    <span class="file">{family?.name ?? `Default (${props.defaultName})`}</span>
                    <Show when={props.monospace && family?.monospace}>
                      <span class="end">monospace</span>
                    </Show>
                  </button>
                )}
              </For>
            </Show>
          </div>
        </div>
      </Show>
    </div>
  );
}

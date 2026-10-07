// The Terminal panel: a Worktree's shells, docked beside the transcript in the Tabs view (following
// the active Worktree) or filling a column in the Columns view, as its Terminal tab. A Worktree can
// have several shells (ticket 35), one tab each; the panel shows one at a time. They keep running
// out of sight, and a panel comes back to what a shell printed. Each panel is a view slot of its
// own, so the columns' terminals stream side by side. xterm.js draws.
import { FitAddon } from "@xterm/addon-fit";
import { Terminal } from "@xterm/xterm";
import "@xterm/xterm/css/xterm.css";
import { createEffect, createSignal, For, getOwner, on, onCleanup, onMount, runWithOwner, Show } from "solid-js";
import { core, type TerminalInfo, type TerminalOutput } from "./core";
import { Eraser, Folder, GitBranch, Maximize2, Minimize2, Play, Plus, RefreshCw, SquareTerminal, Trash2, X } from "./icons";
import { refreshShells, shellsIn, wantedShell } from "./shells";

/** The panel's smallest width, and the least it leaves the transcript beside it. */
const MIN_WIDTH = 280;
const MIN_LEFT = 420;

/** The shell each panel last showed, by view slot and Worktree (so switching back returns to it). */
const lastShown = new Map<string, number>();
/** The last `wantedShell` request each view slot took (one opening later doesn't take it again). */
const wantedTaken = new Map<string, number>();

export function TerminalPanel(props: {
  /** Its view slot: the Tabs view's, or its column's. */
  slot: string;
  worktree: string;
  /** The Worktree's branch (or label) and colour. */
  label: string;
  colour: string;
  /** Beside the transcript (its width set, resizable), or filling a column in its place. */
  placement: "side" | "fill";
  size?: number;
  onSize?: (px: number) => void;
  /** Beside the transcript: whether it takes the transcript's room too. */
  maximized?: boolean;
  onMaximize?: () => void;
  /** Whether showing a Worktree's shell takes the keyboard (not an unfocused column's). */
  takeFocus: boolean;
  onClose?: () => void;
}) {
  let body!: HTMLDivElement;
  let clear = () => {};
  /** Shows one of the Worktree's shells (null: the one it last showed, else its oldest, else a new
   *  one), or a new one. */
  let showShell = (_which: number | null | "new") => {};
  /** The shell on screen. */
  const [current, setCurrent] = createSignal<number | null>(null);
  const [exited, setExited] = createSignal(false);
  const [error, setError] = createSignal("");
  const [dragging, setDragging] = createSignal(false);

  onMount(() => {
    const css = getComputedStyle(document.documentElement);
    const token = (name: string) => css.getPropertyValue(name).trim();
    const fontFamily = token("--mono");
    const fontSize = parseInt(token("--code-size"), 10) || 13;
    const owner = getOwner();
    let disposed = false;
    onCleanup(() => (disposed = true));
    // Measured as it opens, so the font has to be there first.
    void document.fonts
      .load(`${fontSize}px ${fontFamily}`)
      .catch(() => [])
      .then(() => !disposed && runWithOwner(owner, () => start(fontFamily, fontSize, token)));
  });

  const start = (fontFamily: string, fontSize: number, token: (name: string) => string) => {
    const term = new Terminal({
      fontFamily,
      fontSize,
      cursorBlink: true,
      scrollback: 5000,
      theme: {
        background: token("--code-bg"),
        foreground: token("--fg-1"),
        cursor: token("--accent"),
        cursorAccent: token("--code-bg"),
        selectionBackground: token("--accent-dim"),
        red: token("--danger"),
        green: token("--ok"),
        yellow: token("--warn"),
        blue: token("--accent"),
        magenta: token("--hunk"),
        cyan: token("--info"),
      },
    });
    const fit = new FitAddon();
    term.loadAddon(fit);
    term.open(body);
    clear = () => term.clear();
    onCleanup(() => {
      void core.hideTerminal(props.slot).catch(() => {});
      term.dispose();
    });

    /** The Worktree shown, and which showing of it (output from an earlier one is dropped). */
    let shown: string | null = null;
    let showing = 0;
    /** Replayed output being written: what xterm answers to queries in it isn't typed. */
    let replaying = 0;
    const show = (path: string, which: number | null | "new") => {
      const mine = ++showing;
      shown = path;
      setCurrent(null);
      setExited(false);
      setError("");
      term.reset();
      fit.fit();
      const onOutput = (batch: TerminalOutput[]) => {
        if (mine !== showing) return;
        for (const out of batch) {
          if (out.kind === "replay") {
            replaying++;
            term.write(out.text, () => replaying--);
          } else if (out.kind === "output") term.write(out.text);
          else {
            setExited(true);
            const code = out.code === null ? "" : ` with code ${out.code}`;
            term.write(`\r\n\x1b[2m[The shell exited${code}. Press Enter for a new one.]\x1b[0m\r\n`);
          }
        }
      };
      const key = `${props.slot}\n${path}`;
      const opening =
        which === "new"
          ? core.newTerminal(props.slot, path, term.cols, term.rows, onOutput)
          : core.openTerminal(props.slot, path, which ?? lastShown.get(key) ?? null, term.cols, term.rows, onOutput);
      opening
        .then((info: TerminalInfo) => {
          if (mine !== showing) return;
          setCurrent(info.id);
          lastShown.set(key, info.id);
        })
        .catch((err) => mine === showing && setError(String(err)));
      if (props.takeFocus) term.focus();
    };
    showShell = (which) => shown !== null && show(shown, which);
    /** A shell asked for on this Worktree (an Action's, just started), unless already taken. */
    const take = (path: string) => {
      const wanted = wantedShell();
      if (!wanted || wanted.worktree !== path || (wantedTaken.get(props.slot) ?? 0) >= wanted.n) return null;
      wantedTaken.set(props.slot, wanted.n);
      return wanted.id;
    };
    createEffect(on(() => props.worktree, (path) => show(path, take(path))));
    createEffect(
      on(
        wantedShell,
        () => {
          const id = shown !== null ? take(shown) : null;
          if (id !== null && shown !== null) show(shown, id);
        },
        { defer: true },
      ),
    );

    /** After Enter, what runs in the foreground may have changed (the tabs' names). */
    let renamed = 0;
    const type = (data: string) => {
      const id = current();
      if (replaying > 0 || shown === null) return;
      if (exited()) {
        if (data === "\r") show(shown, "new");
        return;
      }
      if (id === null) return;
      void core.terminalInput(id, data).catch(() => {});
      if (data.includes("\r") || data === "\x03") {
        clearTimeout(renamed);
        renamed = window.setTimeout(refreshShells, 600);
      }
    };
    onCleanup(() => clearTimeout(renamed));
    term.onData(type);
    term.onBinary(type);
    term.onResize(({ cols, rows }) => {
      const id = current();
      if (id !== null && !exited()) void core.resizeTerminal(id, cols, rows).catch(() => {});
    });
    let frame = 0;
    const observer = new ResizeObserver(() => {
      cancelAnimationFrame(frame);
      frame = requestAnimationFrame(() => fit.fit());
    });
    observer.observe(body);
    onCleanup(() => {
      observer.disconnect();
      cancelAnimationFrame(frame);
    });

    term.attachCustomKeyEventHandler((e) => {
      if (e.type !== "keydown") return true;
      const key = e.key.toLowerCase();
      // Ctrl+C copies a selection (else it interrupts); Ctrl+Shift+C always copies.
      if (e.ctrlKey && key === "c" && (e.shiftKey || term.hasSelection())) {
        e.preventDefault();
        if (term.hasSelection()) void navigator.clipboard.writeText(term.getSelection()).catch(() => {});
        term.clearSelection();
        return false;
      }
      // Ctrl+V and Ctrl+Shift+V paste: left to the browser, whose paste event xterm takes.
      if (e.ctrlKey && key === "v") return false;
      // The app's own shortcuts go on to it, not the shell.
      if (e.ctrlKey && !e.altKey && (e.code === "Backquote" || (!e.shiftKey && (key === "p" || key === ",")) || (e.shiftKey && "egto".includes(key))))
        return false;
      return true;
    });
  };

  /** Dragging the left edge sets the width. */
  const resize = (e: PointerEvent) => {
    const handle = e.currentTarget as HTMLElement;
    handle.setPointerCapture(e.pointerId);
    const start = e.clientX;
    const startSize = props.size ?? MIN_WIDTH;
    const max = Math.max(MIN_WIDTH, window.innerWidth - MIN_LEFT);
    setDragging(true);
    const move = (m: PointerEvent) => props.onSize?.(Math.min(max, Math.max(MIN_WIDTH, startSize + start - m.clientX)));
    const up = () => {
      setDragging(false);
      handle.removeEventListener("pointermove", move);
      handle.removeEventListener("pointerup", up);
    };
    handle.addEventListener("pointermove", move);
    handle.addEventListener("pointerup", up);
  };

  const shells = () => shellsIn(props.worktree);
  /** Stops a shell, asking first if something other than the shell runs in it. */
  const stop = (shell: TerminalInfo | undefined) => {
    if (!shell) return;
    if (shell.busy && !window.confirm(`Stop ${shell.name}? It's still running in this shell.`)) return;
    // The one on screen: on to another of the Worktree's, if it has one.
    const next = shell.id === current() ? shells().find((t) => t.id !== shell.id) : undefined;
    void core
      .closeTerminal(shell.id)
      .then(() => next && showShell(next.id))
      .catch((err) => setError(String(err)));
  };
  /** Runs an Action's shell's command again in a new shell, and shows that. */
  const restart = (id: number | null) => {
    if (id === null) return;
    void core
      .restartTerminal(id)
      .then((fresh) => showShell(fresh.id))
      .catch((err) => setError(String(err)));
  };
  /** The shell tabs and +: in the panel's header beside the transcript, in the cwd strip in a column. */
  const tabs = () => (
    <>
      <div class="tabs-list">
        <For each={shells()}>
          {(shell) => (
            <span class="trigger-wrap" classList={{ on: shell.id === current() }}>
              <button
                class="trigger shell-trigger"
                onClick={() => shell.id !== current() && showShell(shell.id)}
                onAuxClick={(e) => e.button === 1 && stop(shell)}
                title={`${shell.name} (middle-click stops it)`}
              >
                {shell.action ? <Play /> : <SquareTerminal />}
                {shell.name}
              </button>
              <Show when={shells().length > 1}>
                <button class="close-tab" aria-label={`Stop ${shell.name}`} title="Stop this shell and everything it started" onClick={() => stop(shell)}>
                  <X />
                </button>
              </Show>
            </span>
          )}
        </For>
      </div>
      <button class="ghost icon" onClick={() => showShell("new")} title="Another shell in this Worktree" aria-label="New shell">
        <Plus />
      </button>
    </>
  );
  const side = () => props.placement === "side";

  return (
    <aside
      class={`terminal-panel ${props.placement}`}
      classList={{ maximized: side() && props.maximized }}
      style={{ width: side() && !props.maximized ? `${props.size}px` : undefined, "--c": props.colour }}
    >
      <Show when={side() && !props.maximized}>
        <div class="terminal-resize" classList={{ dragging: dragging() }} onPointerDown={resize}>
          <span class="grip" />
        </div>
      </Show>
      <Show when={side()}>
        <div class="terminal-head">
          {tabs()}
          <span class="grow" />
          <button
            class="ghost icon"
            onClick={() => props.onMaximize?.()}
            title={props.maximized ? "Back beside the conversation" : "Take the conversation's room too"}
            aria-label={props.maximized ? "Restore the terminal" : "Maximise the terminal"}
          >
            {props.maximized ? <Minimize2 /> : <Maximize2 />}
          </button>
          <button class="ghost icon" onClick={() => props.onClose?.()} title="Hide the terminal; its shell keeps running (Ctrl+`)" aria-label="Hide the terminal">
            <X />
          </button>
        </div>
      </Show>
      <div class="terminal-cwd">
        <Show when={!side() && shells().length > 1}>{tabs()}</Show>
        <Show when={side()} fallback={<Folder />}>
          <GitBranch class="wt-glyph" />
          <span class="branch">{props.label}</span>
        </Show>
        <span class="path" title={props.worktree}>
          {props.worktree}
        </span>
        <span class="grow" />
        <Show when={!side() && shells().length <= 1}>
          <button class="ghost icon" onClick={() => showShell("new")} title="Another shell in this Worktree" aria-label="New shell">
            <Plus />
          </button>
        </Show>
        <Show when={shells().find((t) => t.id === current())?.action}>
          {(action) => (
            <button class="ghost icon" onClick={() => restart(current())} title={`Stop this shell and run its command again (Action: ${action()})`} aria-label="Restart">
              <RefreshCw />
            </button>
          )}
        </Show>
        <button class="ghost icon" onClick={() => clear()} title="Clear the screen" aria-label="Clear the screen">
          <Eraser />
        </button>
        <button
          class="ghost icon"
          onClick={() => stop(shells().find((t) => t.id === current()))}
          disabled={exited() || current() === null}
          title="Stop this shell and everything it started"
          aria-label="Stop the shell"
        >
          <Trash2 />
        </button>
      </div>
      <Show when={error()}>
        <div class="terminal-error">{error()}</div>
      </Show>
      <div class="terminal-body" ref={body} />
    </aside>
  );
}

// The Terminal panel: a Worktree's shell, docked beside the transcript in the Tabs view (following
// the active Worktree) or under a column's in the Columns view. The core runs one shell per
// Worktree, which keeps running out of sight; a panel comes back to what its shell printed. Each
// panel is a view slot of its own, so the columns' terminals stream side by side. xterm.js draws.
import { FitAddon } from "@xterm/addon-fit";
import { Terminal } from "@xterm/xterm";
import "@xterm/xterm/css/xterm.css";
import { createEffect, createSignal, getOwner, on, onCleanup, onMount, runWithOwner, Show } from "solid-js";
import { core, type TerminalOutput } from "./core";
import { GitBranch, SquareTerminal, Trash2, X } from "./icons";

/** The panel's smallest size, and the least it leaves the transcript (beside it, or above it). */
const MIN_WIDTH = 280;
const MIN_LEFT = 420;
const MIN_HEIGHT = 120;
const MIN_ABOVE = 260;

export function TerminalPanel(props: {
  /** Its view slot: the Tabs view's, or its column's. */
  slot: string;
  worktree: string;
  /** The Worktree's branch (or label) and colour. */
  label: string;
  colour: string;
  /** Beside the transcript (its width set), or under it (its height). */
  placement: "side" | "bottom";
  size: number;
  onSize: (px: number) => void;
  /** Whether showing a Worktree's shell takes the keyboard (not an unfocused column's). */
  takeFocus: boolean;
  onClose: () => void;
}) {
  let body!: HTMLDivElement;
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
        background: token("--bg-1"),
        foreground: token("--fg-1"),
        cursor: token("--accent"),
        cursorAccent: token("--bg-1"),
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
    onCleanup(() => {
      void core.hideTerminal(props.slot).catch(() => {});
      term.dispose();
    });

    /** The Worktree shown, and which showing of it (output from an earlier one is dropped). */
    let shown: string | null = null;
    let showing = 0;
    /** Replayed output being written: what xterm answers to queries in it isn't typed. */
    let replaying = 0;
    const show = (path: string) => {
      const mine = ++showing;
      shown = path;
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
      core.openTerminal(props.slot, path, term.cols, term.rows, onOutput).catch((err) => mine === showing && setError(String(err)));
      if (props.takeFocus) term.focus();
    };
    createEffect(on(() => props.worktree, show));

    const type = (data: string) => {
      if (replaying > 0 || shown === null) return;
      if (!exited()) void core.terminalInput(shown, data).catch(() => {});
      else if (data === "\r") show(shown);
    };
    term.onData(type);
    term.onBinary(type);
    term.onResize(({ cols, rows }) => shown !== null && !exited() && void core.resizeTerminal(shown, cols, rows).catch(() => {}));
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

  /** Dragging the inner edge (left beside, top under) sets the size. */
  const resize = (e: PointerEvent) => {
    const handle = e.currentTarget as HTMLElement;
    handle.setPointerCapture(e.pointerId);
    const side = props.placement === "side";
    const start = side ? e.clientX : e.clientY;
    const startSize = props.size;
    const [min, max] = side
      ? [MIN_WIDTH, Math.max(MIN_WIDTH, window.innerWidth - MIN_LEFT)]
      : [MIN_HEIGHT, Math.max(MIN_HEIGHT, window.innerHeight - MIN_ABOVE)];
    setDragging(true);
    const move = (m: PointerEvent) => props.onSize(Math.min(max, Math.max(min, startSize + start - (side ? m.clientX : m.clientY))));
    const up = () => {
      setDragging(false);
      handle.removeEventListener("pointermove", move);
      handle.removeEventListener("pointerup", up);
    };
    handle.addEventListener("pointermove", move);
    handle.addEventListener("pointerup", up);
  };

  return (
    <aside
      class={`terminal-panel ${props.placement}`}
      style={{ [props.placement === "side" ? "width" : "height"]: `${props.size}px`, "--c": props.colour }}
    >
      <div class="terminal-resize" classList={{ dragging: dragging() }} onPointerDown={resize} />
      <div class="drawer-head">
        <SquareTerminal />
        <span class="terminal-title">Terminal</span>
        <GitBranch />
        <span class="drawer-branch">{props.label}</span>
        <span class="grow" />
        <button
          class="ghost icon"
          onClick={() => void core.closeTerminal(props.worktree).catch((err) => setError(String(err)))}
          disabled={exited()}
          title="Stop this shell and everything it started"
          aria-label="Stop the shell"
        >
          <Trash2 />
        </button>
        <button class="ghost icon" onClick={() => props.onClose()} title="Hide the terminal; its shell keeps running (Ctrl+`)" aria-label="Hide the terminal">
          <X />
        </button>
      </div>
      <Show when={error()}>
        <div class="terminal-error">{error()}</div>
      </Show>
      <div class="terminal-body" ref={body} />
    </aside>
  );
}

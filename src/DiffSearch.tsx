// Search in the file shown in a code review (issue 60): a box in the file's head that finds text in
// the diff's code, steps through the matches with Enter / Shift+Enter, and takes Ctrl+F. The matches
// are painted with the CSS highlight API rather than by wrapping them in elements, so the diff's own
// DOM (and the line picking built on it) is left alone.
import { createEffect, createSignal, on, onCleanup, onMount } from "solid-js";
import { ChevronLeft, ChevronRight, Search, X } from "./icons";

type Highlights = { set(name: string, h: unknown): void; delete(name: string): void };
const highlights = (): Highlights | undefined => (CSS as unknown as { highlights?: Highlights }).highlights;
const HighlightOf = () => (window as unknown as { Highlight?: new (...r: Range[]) => unknown }).Highlight;

/** `box` is the element holding the diff; `file` changes when another file is shown. */
export function DiffSearch(props: { box: () => HTMLElement | undefined; file: string }) {
  const [query, setQuery] = createSignal("");
  const [matches, setMatches] = createSignal<Range[]>([]);
  const [at, setAt] = createSignal(0);
  let input: HTMLInputElement | undefined;

  const paint = () => {
    const h = highlights();
    const Highlight = HighlightOf();
    if (!h || !Highlight) return;
    const all = matches();
    if (all.length) {
      h.set("diff-search", new Highlight(...all));
      h.set("diff-search-current", new Highlight(all[at()]));
    } else {
      h.delete("diff-search");
      h.delete("diff-search-current");
    }
  };

  const find = () => {
    const box = props.box();
    const q = query().toLowerCase();
    const found: Range[] = [];
    if (box && q) {
      for (const code of box.querySelectorAll(".line .code")) {
        // A line is searched as a whole, so a match split across highlighted tokens is still found.
        const walker = document.createTreeWalker(code, NodeFilter.SHOW_TEXT);
        const nodes: Text[] = [];
        let text = "";
        for (let n = walker.nextNode(); n; n = walker.nextNode()) {
          nodes.push(n as Text);
          text += n.nodeValue ?? "";
        }
        const lower = text.toLowerCase();
        const place = (offset: number, end: boolean) => {
          let rest = offset;
          for (const node of nodes) {
            if (rest < node.length || (end && rest === node.length)) return { node, offset: rest };
            rest -= node.length;
          }
          return null;
        };
        for (let i = lower.indexOf(q); i >= 0; i = lower.indexOf(q, i + q.length)) {
          const from = place(i, false);
          const to = place(i + q.length, true);
          if (!from || !to) continue;
          const range = document.createRange();
          range.setStart(from.node, from.offset);
          range.setEnd(to.node, to.offset);
          found.push(range);
        }
      }
    }
    setMatches(found);
    setAt((a) => (a < found.length ? a : 0));
    paint();
  };

  const show = () => {
    matches()[at()]?.startContainer.parentElement?.scrollIntoView({ block: "center", inline: "nearest" });
  };
  const step = (by: number) => {
    const n = matches().length;
    if (!n) return;
    setAt((at() + by + n) % n);
    paint();
    show();
  };
  const clear = () => setQuery("");

  // A new query starts from the first match; a new file, or a diff loading in, is searched again.
  createEffect(
    on(
      query,
      () => {
        setAt(0);
        find();
        show();
      },
      { defer: true },
    ),
  );
  createEffect(on(() => props.file, () => requestAnimationFrame(find), { defer: true }));
  let watching: MutationObserver | undefined;
  let pending = 0;
  createEffect(() => {
    const box = props.box();
    watching?.disconnect();
    if (!box) return;
    watching = new MutationObserver(() => {
      if (!query()) return;
      cancelAnimationFrame(pending);
      pending = requestAnimationFrame(find);
    });
    watching.observe(box, { childList: true, subtree: true });
  });
  const onKey = (e: KeyboardEvent) => {
    if ((e.ctrlKey || e.metaKey) && !e.altKey && e.key.toLowerCase() === "f") {
      e.preventDefault();
      input?.focus();
      input?.select();
    }
  };
  onMount(() => window.addEventListener("keydown", onKey));
  onCleanup(() => {
    window.removeEventListener("keydown", onKey);
    watching?.disconnect();
    cancelAnimationFrame(pending);
    highlights()?.delete("diff-search");
    highlights()?.delete("diff-search-current");
  });

  return (
    <div class="diff-search" title="Search in this file (Ctrl+F)">
      <Search />
      <input
        ref={input}
        type="text"
        placeholder="Search in file"
        spellcheck={false}
        value={query()}
        onInput={(e) => setQuery(e.currentTarget.value)}
        onKeyDown={(e) => {
          if (e.key === "Enter") (e.preventDefault(), step(e.shiftKey ? -1 : 1));
        }}
      />
      <span class="diff-search-count muted">{query() ? (matches().length ? `${at() + 1} / ${matches().length}` : "No matches") : ""}</span>
      <button class="ghost" disabled={!matches().length} onClick={() => step(-1)} title="Previous match (Shift+Enter)">
        <ChevronLeft />
      </button>
      <button class="ghost" disabled={!matches().length} onClick={() => step(1)} title="Next match (Enter)">
        <ChevronRight />
      </button>
      <button class="ghost" disabled={!query()} onClick={clear} title="Clear the search">
        <X />
      </button>
    </div>
  );
}

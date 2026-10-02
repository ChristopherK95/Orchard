// Agent messages as markdown (ticket 02). Raw HTML in messages is shown as text, never rendered,
// and images are off: an Agent-written image URL would load (and leak data) without any click.
// Code fences become static highlighted blocks with line numbers, a language label, Copy and Open
// in editor (wired up by delegation in the transcript; see `handleTranscriptClick`).
import MarkdownIt from "markdown-it";
import { highlightCode, languageOf } from "./highlight";
import { escapeHtml } from "./html";

const md = new MarkdownIt({ html: false, linkify: true, breaks: false });
md.disable("image");

md.renderer.rules.fence = (tokens, index) => {
  const token = tokens[index];
  const label = token.info.trim().split(/\s+/)[0] ?? "";
  return codeBlock(token.content.replace(/\n$/, ""), label);
};

/** Lucide's external-link and copy, as markup (code blocks are HTML strings, not components). */
const svg = (paths: string) =>
  `<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">${paths}</svg>`;
const OPEN_ICON = svg('<path d="M15 3h6v6"/><path d="M10 14 21 3"/><path d="M18 13v6a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2h6"/>');
const COPY_ICON = svg('<rect width="14" height="14" x="8" y="8" rx="2" ry="2"/><path d="M4 16c-1.1 0-2-.9-2-2V4c0-1.1.9-2 2-2h10c1.1 0 2 .9 2 2"/>');

function codeBlock(code: string, label: string): string {
  const lines = highlightCode(code, languageOf(label))
    .split("\n")
    .map((line) => `<span class="l">${line || " "}</span>`)
    .join("");
  return (
    `<div class="cb" data-code="${escapeHtml(code)}" data-label="${escapeHtml(label)}"><div class="cb-head"><span>${escapeHtml(label || "text")}</span>` +
    `<span class="grow"></span><button type="button" class="ghost" data-open>${OPEN_ICON}Open in editor</button>` +
    `<button type="button" class="ghost" data-copy>${COPY_ICON}<span>Copy</span></button></div>` +
    `<pre class="numbered"><code>${lines}</code></pre></div>`
  );
}

/** Markdown to HTML. Reactive through the highlighter, so blocks re-render when their grammar loads. */
export function renderMarkdown(text: string): string {
  return md.render(text);
}

const SAFE_LINK = /^(https?:|mailto:)/i;

/**
 * Handles clicks inside rendered chat: Copy buttons copy their block, Open in editor opens it in
 * the Manual editor; links open in the system browser (never inside the app). Failures are
 * reported to `onError`.
 */
export async function handleTranscriptClick(
  event: MouseEvent,
  openUrl: (url: string) => Promise<void>,
  openSnippet: (code: string, label: string) => void,
  onError: (message: string) => void,
) {
  const target = event.target as HTMLElement | null;
  try {
    const open = target?.closest<HTMLButtonElement>("[data-open]");
    if (open) {
      const block = open.closest<HTMLElement>(".cb");
      return openSnippet(block?.dataset.code ?? "", block?.dataset.label ?? "");
    }
    const copy = target?.closest<HTMLButtonElement>("[data-copy]");
    if (copy) {
      await navigator.clipboard.writeText(copy.closest<HTMLElement>(".cb")?.dataset.code ?? "");
      const text = copy.lastElementChild!;
      text.textContent = "Copied";
      setTimeout(() => (text.textContent = "Copy"), 1200);
      return;
    }
    const link = target?.closest<HTMLAnchorElement>("a[href]");
    if (link) {
      event.preventDefault();
      const href = link.getAttribute("href") ?? "";
      if (SAFE_LINK.test(href)) await openUrl(href);
    }
  } catch (err) {
    onError(String(err));
  }
}

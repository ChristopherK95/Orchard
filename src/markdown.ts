// Agent messages as markdown (ticket 02). Raw HTML in messages is shown as text, never rendered.
// Code fences become static highlighted blocks with line numbers, a language label and Copy
// (wired up by delegation in the transcript; see `handleTranscriptClick`).
import MarkdownIt from "markdown-it";
import { escapeHtml, highlightCode, languageOf } from "./highlight";

const md = new MarkdownIt({ html: false, linkify: true, breaks: false });

md.renderer.rules.fence = (tokens, index) => {
  const token = tokens[index];
  const label = token.info.trim().split(/\s+/)[0] ?? "";
  const code = token.content.replace(/\n$/, "");
  return codeBlock(code, label);
};

/** A static highlighted code block (also used outside markdown). */
export function codeBlock(code: string, label: string): string {
  const lines = highlightCode(code, languageOf(label))
    .split("\n")
    .map((line) => `<span class="l">${line || " "}</span>`)
    .join("");
  return (
    `<div class="cb" data-code="${escapeHtml(code)}"><div class="cb-head"><span>${escapeHtml(label || "text")}</span>` +
    `<span class="grow"></span><button type="button" class="ghost" data-copy>Copy</button></div>` +
    `<pre class="numbered"><code>${lines}</code></pre></div>`
  );
}

/** Markdown to HTML. Reactive through the highlighter, so blocks re-render when a grammar loads. */
export function renderMarkdown(text: string): string {
  return md.render(text);
}

const SAFE_LINK = /^(https?:|mailto:)/i;

/**
 * Handles clicks inside rendered chat: Copy buttons copy their block, links open in the system
 * browser (never inside the app). Returns true if the click was handled.
 */
export async function handleTranscriptClick(event: MouseEvent, openUrl: (url: string) => Promise<void>): Promise<boolean> {
  const target = event.target as HTMLElement | null;
  const copy = target?.closest<HTMLButtonElement>("[data-copy]");
  if (copy) {
    const block = copy.closest<HTMLElement>(".cb");
    await navigator.clipboard.writeText(block?.dataset.code ?? "");
    copy.textContent = "Copied";
    setTimeout(() => (copy.textContent = "Copy"), 1200);
    return true;
  }
  const link = target?.closest<HTMLAnchorElement>("a[href]");
  if (link) {
    event.preventDefault();
    const href = link.getAttribute("href") ?? "";
    if (SAFE_LINK.test(href)) await openUrl(href);
    return true;
  }
  return false;
}

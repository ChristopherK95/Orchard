// Agent messages as markdown (ticket 02). Raw HTML in messages is shown as text, never rendered,
// and images are off: an Agent-written image URL would load (and leak data) without any click.
// Code fences become static highlighted blocks with line numbers, a language label and Copy
// (wired up by delegation in the transcript; see `handleTranscriptClick`).
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

function codeBlock(code: string, label: string): string {
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

/** Markdown to HTML. Reactive through the highlighter, so blocks re-render when their grammar loads. */
export function renderMarkdown(text: string): string {
  return md.render(text);
}

const SAFE_LINK = /^(https?:|mailto:)/i;

/**
 * Handles clicks inside rendered chat: Copy buttons copy their block; links open in the system
 * browser (never inside the app). Failures are reported to `onError`.
 */
export async function handleTranscriptClick(
  event: MouseEvent,
  openUrl: (url: string) => Promise<void>,
  onError: (message: string) => void,
) {
  const target = event.target as HTMLElement | null;
  try {
    const copy = target?.closest<HTMLButtonElement>("[data-copy]");
    if (copy) {
      await navigator.clipboard.writeText(copy.closest<HTMLElement>(".cb")?.dataset.code ?? "");
      copy.textContent = "Copied";
      setTimeout(() => (copy.textContent = "Copy"), 1200);
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

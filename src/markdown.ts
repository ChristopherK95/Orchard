// Agent messages as markdown (ticket 02). Raw HTML in messages is shown as text, never rendered,
// and images are off: an Agent-written image URL would load (and leak data) without any click.
// Code fences become static highlighted blocks with line numbers, a language label, Copy and Open
// in editor (wired up by delegation in the transcript; see `handleTranscriptClick`).
import MarkdownIt, { type StateCore } from "markdown-it";
import { highlightCode, languageOf } from "./highlight";
import { escapeHtml } from "./html";

const md = new MarkdownIt({ html: false, linkify: true, breaks: false });
md.disable("image");

// Your own messages: the same markdown, but a line break stays one (you typed it), and the
// Agent-reply cards below don't apply.
const userMd = new MarkdownIt({ html: false, linkify: true, breaks: true });
userMd.disable("image");

for (const instance of [md, userMd]) {
  instance.renderer.rules.fence = (tokens, index) => {
    const token = tokens[index];
    const label = token.info.trim().split(/\s+/)[0] ?? "";
    return codeBlock(token.content.replace(/\n$/, ""), label);
  };
}

// A question the Agent asks you becomes a card: a top-level paragraph that starts with a bold
// "**1. Title**", then an italic or bold "Recommendation: …" (in that paragraph or a later one),
// a list of "(a) …" options, or both. What comes between them (text, lists, code) is the card's
// body. Options (and the recommendation, when there are none) answer it into the composer
// (`data-answer`, see `handleTranscriptClick`). The marked form works too: "❓ **Q5 – Title?**"
// and a "➡️ (a). Why" recommendation. Anything else stays plain markdown.
const QUESTION = /^\*\*(\d+)[.)]\s+([^*]+?)\*\*\s*([\s\S]*)$/;
// The marked form: "❓ **Q5 – Title?** …", with the title bold or ending at its first "?".
const QUESTION_MARKED = /^❓️?\s*(?:\*\*)?Q(\d+)\s*[-–—:]\s*([^*\n]+?)(?:\*\*|(?<=\?)(?!\*))\s*([\s\S]*)$/;
// The marked recommendation: "➡️ (a). Why", the pick (if any) leading its text.
const ARROW = /^➡️?\s*([\s\S]+)$/;
const OPTION =/^(?:\*\*)?\(([a-z])\)(?:\*\*)?\s+([\s\S]+)$/;
const RECOMMENDATION = /^(\*\*|\*|__|_)Recommendation:?\s*([\s\S]*?)\1\s*([\s\S]*)$/i;
const RECOMMENDATION_IN = /(\*\*|\*|__|_)Recommendation\b/i;
/** A line like "Round 2": short, with no closing punctuation. It heads what follows; no card takes it in. */
const TITLE_LIKE = /^[^\n.:!?]{1,40}$/;
/** How many blocks a question's body may hold before the recommendation (more: not a question). */
const MAX_BODY = 8;

type Tokens = StateCore["tokens"];
type Options = { options: { letter: string; text: string }[]; end: number };

/** The number, title and rest of a question paragraph, in either form. */
function questionIn(text: string | null): RegExpExecArray | null {
  return text === null ? null : (QUESTION.exec(text) ?? QUESTION_MARKED.exec(text));
}

/** A recommendation paragraph in either form, shaped as `RECOMMENDATION` matches it. */
function recommendationIn(text: string): RegExpExecArray | null {
  const arrow = ARROW.exec(text);
  return arrow ? (Object.assign(["", "", "", arrow[1]], { index: 0, input: text }) as RegExpExecArray) : RECOMMENDATION.exec(text);
}

/** The text of the top-level paragraph that starts at `i`. */
function paragraphAt(tokens: Tokens, i: number): string | null {
  return tokens[i]?.type === "paragraph_open" && tokens[i].level === 0 && tokens[i + 2]?.type === "paragraph_close"
    ? tokens[i + 1].content
    : null;
}

/** Where the block that starts at `i` ends (the index after its closing token). */
function blockEnd(tokens: Tokens, i: number): number {
  if (tokens[i].nesting !== 1) return i + 1;
  let depth = 0;
  do depth += tokens[i++].nesting;
  while (depth > 0 && i < tokens.length);
  return i;
}

/** The "(a) …" items of the top-level list that starts at `i`, if every item is one. */
function optionsAt(tokens: Tokens, i: number): Options | null {
  if (tokens[i]?.type !== "bullet_list_open" || tokens[i].level !== 0) return null;
  const options = [];
  let j = i + 1;
  while (tokens[j]?.type === "list_item_open") {
    if (tokens[j + 1]?.type !== "paragraph_open" || tokens[j + 3]?.type !== "paragraph_close" || tokens[j + 4]?.type !== "list_item_close") return null;
    const match = OPTION.exec(tokens[j + 2].content);
    if (!match) return null;
    options.push({ letter: match[1], text: match[2] });
    j += 5;
  }
  return tokens[j]?.type === "bullet_list_close" && options.length ? { options, end: j + 1 } : null;
}

md.core.ruler.before("inline", "question_cards", (state) => {
  const tokens = state.tokens;
  const html = (content: string) => Object.assign(new state.Token("html_block", "", 0), { content });
  const inline = (content: string) => Object.assign(new state.Token("inline", "", 0), { content, children: [] });
  for (let i = 0; i < tokens.length; i++) {
    const head = questionIn(paragraphAt(tokens, i));
    if (!head) continue;
    const n = head[1];
    let lead = head[3];
    // "**8. Title** *Recommendation:* …": the recommendation shares the title's paragraph.
    const at = lead.search(RECOMMENDATION_IN);
    let rec = at >= 0 ? RECOMMENDATION.exec(lead.slice(at)) : null;
    if (rec) lead = lead.slice(0, at).trim();
    // The body: blocks up to the recommendation, the options list among them.
    let end = i + 3;
    const parts: (Tokens | Options)[] = [];
    let list: Options | null = null;
    for (let blocks = 0; !rec && blocks < MAX_BODY && end < tokens.length; blocks++) {
      const text = paragraphAt(tokens, end);
      if (questionIn(text) || tokens[end].type === "heading_open" || tokens[end].type === "hr") break;
      rec = text !== null ? recommendationIn(text) : null;
      if (rec) {
        end += 3;
        break;
      }
      const options: Options | null = list ? null : optionsAt(tokens, end);
      const next = options?.end ?? blockEnd(tokens, end);
      parts.push(options ?? tokens.slice(end, next));
      list ??= options;
      end = next;
    }
    // Options with no recommendation after them: the card ends with the options.
    if (!rec && list) {
      parts.length = parts.indexOf(list) + 1;
      end = list.end;
    } else if (!rec) continue;
    // What follows the recommendation is still its text ("top to bottom:" and a list, then a
    // closing line) when the next question comes after it. Before none, only a list it introduces
    // with a colon: the rest may be the message's own closing words.
    const after: Tokens = [];
    if (rec) {
      const ends: number[] = [];
      let next = end;
      let questionNext = false;
      while (ends.length < MAX_BODY && next < tokens.length) {
        const text = paragraphAt(tokens, next);
        if (questionIn(text)) questionNext = true;
        if (questionNext || (text !== null && TITLE_LIKE.test(text)) || tokens[next].type === "heading_open" || tokens[next].type === "hr") break;
        ends.push((next = blockEnd(tokens, next)));
      }
      const take = questionNext ? ends.length : /:\s*$/.test(rec[3] || rec[2]) ? Math.min(1, ends.length) : 0;
      if (take) {
        after.push(...tokens.slice(end, ends[take - 1]));
        end = ends[take - 1];
      }
    }

    // The pick is the italic part ("(c), with …"); a bold label leaves it in the body instead.
    const pick = rec?.[2].replace(/[.:]\s*$/, "") ?? "";
    const recommended = /^(?:\*\*)?\(([a-z])\)/.exec(pick || (rec?.[3] ?? ""))?.[1];
    const card = [
      html(`<section class="qcard"><div class="qcard-head"><span class="qcard-n">${n}</span><span class="qcard-title">`),
      inline(head[2]),
      html(`</span></div>`),
    ];
    if (lead) card.push(html(`<p class="qcard-lead">`), inline(lead), html(`</p>`));
    for (const part of parts) {
      if (Array.isArray(part)) {
        card.push(html(`<div class="qcard-body">`), ...part, html(`</div>`));
        continue;
      }
      card.push(html(`<div class="qcard-options">`));
      for (const { letter, text } of part.options) {
        const chosen = letter === recommended;
        card.push(
          html(`<button type="button" class="qcard-option${chosen ? " recommended" : ""}" data-answer="${n}: (${letter})" title="Answer (${letter}) in the composer"><span class="qcard-letter">${letter}</span><span class="qcard-text">`),
          inline(text),
          html(`</span>${chosen ? `<span class="qcard-badge">Recommended</span>` : ""}</button>`),
        );
      }
      card.push(html(`</div>`));
    }
    if (rec) {
      card.push(html(`<div class="qcard-rec"><div class="qcard-rec-text"><span class="qcard-rec-label">Recommendation</span>`));
      if (pick) card.push(html(`<strong>`), inline(pick), html(rec[3] ? `</strong> — ` : `</strong>`));
      if (rec[3]) card.push(inline(rec[3]));
      card.push(...after, html(`</div>`));
      if (!list) card.push(html(`<button type="button" class="outline" data-answer="${n}: as recommended">Go with this</button>`));
      card.push(html(`</div>`));
    }
    card.push(html(`</section>`));
    tokens.splice(i, end - i, ...card);
    i += card.length - 1;
  }
});

// A list whose every item starts with a bold lead ("**Core: the setting.** What it does") reads as
// rows: the lead as a title (its "Core:" prefix as a tag), the rest under it, and in an ordered
// list a number badge. Other lists stay as they are.
const LEAD = /^\*\*([^*]+?)\*\*\s*([\s\S]*)$/;
const TAG = /^([A-Z][\w /-]{0,20}):\s+([\s\S]+)$/;
/** How many tag colours styles.css has (`.step-tag.c0` …). */
const TAG_COLOURS = 5;

/** The same tag always gets the same colour. */
function tagColour(tag: string): number {
  let hash = 0;
  for (const c of tag.toLowerCase()) hash = (hash * 31 + c.charCodeAt(0)) | 0;
  return Math.abs(hash) % TAG_COLOURS;
}

md.core.ruler.before("inline", "lead_lists", (state) => {
  const tokens = state.tokens;
  const html = (content: string) => Object.assign(new state.Token("html_block", "", 0), { content });
  const inline = (content: string) => Object.assign(new state.Token("inline", "", 0), { content, children: [] });
  // Backwards, so a nested list is done before its parent and splicing never moves one still to do.
  for (let i = tokens.length - 1; i >= 0; i--) {
    const list = tokens[i];
    if (list.type !== "bullet_list_open" && list.type !== "ordered_list_open") continue;
    const items: number[] = [];
    let j = i + 1;
    while (tokens[j]?.type === "list_item_open" && tokens[j + 1]?.type === "paragraph_open" && LEAD.test(tokens[j + 2].content)) {
      items.push(j);
      j = blockEnd(tokens, j);
    }
    if (tokens[j]?.type === "list_item_open" || items.length < 2) continue;
    list.attrJoin("class", "steps");
    const first = Number(list.attrGet("start") ?? 1);
    for (let k = items.length - 1; k >= 0; k--) {
      const at = items[k];
      const [, lead, rest] = LEAD.exec(tokens[at + 2].content)!;
      const tagged = TAG.exec(lead);
      const title = (tagged?.[2] ?? lead).replace(/[.:]\s*$/, "");
      const tag = tagged ? `<span class="step-tag c${tagColour(tagged[1])}">${escapeHtml(tagged[1])}</span>` : "";
      // The item's paragraph becomes the title and text blocks (a <p> can't hold them).
      tokens[at + 1].hidden = tokens[at + 3].hidden = true;
      tokens.splice(
        at + 2,
        1,
        html(`<div class="step-body"><div class="step-title">${tag}`),
        inline(title),
        html(`</div>`),
        ...(rest ? [html(`<div class="step-text">`), inline(rest), html(`</div>`)] : []),
        html(`</div>`),
      );
      if (list.type === "ordered_list_open") tokens.splice(at + 1, 0, html(`<span class="step-n">${first + k}</span>`));
    }
  }
});

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

/** Markdown for a message you sent. */
export function renderUserMarkdown(text: string): string {
  return userMd.render(text);
}

const SAFE_LINK = /^(https?:|mailto:)/i;

/**
 * Handles clicks inside rendered chat: Copy buttons copy their block, Open in editor opens it in
 * the Manual editor; a question card's option goes to `answer` ("1: (a)"); links open in the
 * system browser (never inside the app). Failures are reported to `onError`.
 */
export async function handleTranscriptClick(
  event: MouseEvent,
  openUrl: (url: string) => Promise<void>,
  openSnippet: (code: string, label: string) => void,
  answer: (line: string) => void,
  onError: (message: string) => void,
) {
  const target = event.target as HTMLElement | null;
  try {
    const option = target?.closest<HTMLButtonElement>("[data-answer]");
    if (option) return answer(option.dataset.answer ?? "");
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

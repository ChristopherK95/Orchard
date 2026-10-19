// The composer's markdown formatting (its toolbar and shortcuts), like GitHub's comment box: each
// format is an edit to the draft around the selection, so the composer can make it through the
// input's own editing and Ctrl+Z undoes it. Applying a format to text that already has it removes it.

export type Format = "heading" | "bold" | "italic" | "quote" | "code" | "link" | "numbered" | "bulleted" | "task";

/** Replace `from`..`to` with `insert`, then select `selStart`..`selEnd`. */
export interface Edit {
  from: number;
  to: number;
  insert: string;
  selStart: number;
  selEnd: number;
}

const URL = /^https?:\/\/\S+$/;

/** The edit that applies `format` to the selection `start`..`end` of `text`. */
export function formatEdit(text: string, start: number, end: number, format: Format): Edit {
  switch (format) {
    case "bold":
      return wrap(text, start, end, "**");
    case "italic":
      return wrap(text, start, end, "_");
    case "code":
      return text.slice(start, end).includes("\n") ? fence(text, start, end) : wrap(text, start, end, "`");
    case "link":
      return link(text.slice(start, end), start, end);
    default:
      return prefixLines(text, start, end, format);
  }
}

/** Wraps the selection in `mark`, or unwraps it if it's wrapped already (inside or just outside it). */
function wrap(text: string, start: number, end: number, mark: string): Edit {
  const m = mark.length;
  const selected = text.slice(start, end);
  if (text.slice(start - m, start) === mark && text.slice(end, end + m) === mark && start >= m) {
    return { from: start - m, to: end + m, insert: selected, selStart: start - m, selEnd: end - m };
  }
  if (selected.length >= 2 * m && selected.startsWith(mark) && selected.endsWith(mark)) {
    return { from: start, to: end, insert: selected.slice(m, -m), selStart: start, selEnd: end - 2 * m };
  }
  return { from: start, to: end, insert: mark + selected + mark, selStart: start + m, selEnd: end + m };
}

/** A selection of several lines goes in a fenced code block, the fences on lines of their own. */
function fence(text: string, start: number, end: number): Edit {
  const selected = text.slice(start, end).replace(/\n$/, "");
  const before = start > 0 && text[start - 1] !== "\n" ? "\n" : "";
  const after = end < text.length && text[end] !== "\n" ? "\n" : "";
  const open = `${before}\`\`\`\n`;
  const insert = `${open}${selected}\n\`\`\`${after}`;
  return { from: start, to: end, insert, selStart: start + open.length, selEnd: start + open.length + selected.length };
}

/** `[text](url)` with `url` selected to type over; a selected URL becomes the link, the caret in its text. */
function link(selected: string, start: number, end: number): Edit {
  if (URL.test(selected)) return { from: start, to: end, insert: `[](${selected})`, selStart: start + 1, selEnd: start + 1 };
  if (!selected) return { from: start, to: end, insert: "[](url)", selStart: start + 1, selEnd: start + 1 };
  const url = start + selected.length + 3;
  return { from: start, to: end, insert: `[${selected}](url)`, selStart: url, selEnd: url + 3 };
}

/** A pasted URL over selected text links it, as on GitHub; else null (an ordinary paste). */
export function pasteLinkEdit(text: string, start: number, end: number, pasted: string): Edit | null {
  const selected = text.slice(start, end);
  const url = pasted.trim();
  if (!selected || selected.includes("\n") || URL.test(selected) || !URL.test(url)) return null;
  const insert = `[${selected}](${url})`;
  return { from: start, to: end, insert, selStart: start + insert.length, selEnd: start + insert.length };
}

const MARKS: Record<Exclude<Format, "bold" | "italic" | "code" | "link">, { has: RegExp; add: (n: number) => string }> = {
  heading: { has: /^#{1,6} /, add: () => "### " },
  quote: { has: /^> ?/, add: () => "> " },
  bulleted: { has: /^[-*+] (?!\[[ xX]\] )/, add: () => "- " },
  numbered: { has: /^\d+[.)] /, add: (n) => `${n}. ` },
  task: { has: /^[-*+] \[[ xX]\] /, add: () => "- [ ] " },
};
/** Any list's marker: a list format replaces another's rather than stacking on it. */
const LIST_MARK = /^(?:[-*+] \[[ xX]\] |[-*+] |\d+[.)] )/;

/** Starts each selected line with the format's marker, or takes it off if every line has it. */
function prefixLines(text: string, start: number, end: number, format: keyof typeof MARKS): Edit {
  const lineStart = text.lastIndexOf("\n", start - 1) + 1;
  // (A selection ending at the start of a line doesn't take that line.)
  const last = end > start && text[end - 1] === "\n" ? end - 1 : end;
  const lineEnd = text.indexOf("\n", last) < 0 ? text.length : text.indexOf("\n", last);
  const lines = text.slice(lineStart, lineEnd).split("\n");
  const { has, add } = MARKS[format];
  // Blank lines between the selected ones are left blank.
  const marked = (line: string) => lines.length === 1 || line.trim() !== "";
  const remove = lines.filter(marked).every((line) => has.test(line));
  let n = 0;
  const changed = lines.map((line) => {
    if (!marked(line)) return line;
    if (remove) return line.replace(has, "");
    const bare = format === "heading" ? line.replace(MARKS.heading.has, "") : format === "quote" ? line : line.replace(LIST_MARK, "");
    return add(++n) + bare;
  });
  const insert = changed.join("\n");
  if (start === end && lines.length === 1) {
    const caret = Math.max(lineStart, start + insert.length - lines[0].length);
    return { from: lineStart, to: lineEnd, insert, selStart: caret, selEnd: caret };
  }
  return { from: lineStart, to: lineEnd, insert, selStart: lineStart, selEnd: lineStart + insert.length };
}

/** A newline in a list or quote starts its next item (the next number, an unticked task); on an item
 *  left empty it ends the list instead. Null when the caret's line isn't one. */
export function continueListEdit(text: string, start: number, end: number): Edit | null {
  if (start !== end) return null;
  const lineStart = text.lastIndexOf("\n", start - 1) + 1;
  const found = /^(\s*)([-*+] \[[ xX]\] |[-*+] |(\d+)([.)]) |> ?)/.exec(text.slice(lineStart, start));
  if (!found) return null;
  const [marker, indent, mark, number, dot] = found;
  const lineEnd = text.indexOf("\n", start) < 0 ? text.length : text.indexOf("\n", start);
  if (text.slice(lineStart, lineEnd).trim() === marker.trim()) {
    return { from: lineStart, to: lineEnd, insert: "", selStart: lineStart, selEnd: lineStart };
  }
  const next = number ? `${Number(number) + 1}${dot} ` : /\[[ xX]\]/.test(mark) ? `${mark[0]} [ ] ` : mark;
  const insert = `\n${indent}${next}`;
  return { from: start, to: end, insert, selStart: start + insert.length, selEnd: start + insert.length };
}

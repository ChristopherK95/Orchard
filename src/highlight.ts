// Static syntax highlighting with Lezer grammars (ticket 02): highlighted HTML, no editor instance.
// Grammars load on first use; anything rendered before its grammar arrives shows as plain text,
// then re-renders because `highlightCode` reads the `grammarsLoaded` signal.
import type { Parser } from "@lezer/common";
import { classHighlighter, highlightCode as lezerHighlight } from "@lezer/highlight";
import { createSignal } from "solid-js";

type Loader = () => Promise<Parser>;

const LOADERS: Record<string, Loader> = {
  javascript: () => import("@lezer/javascript").then((m) => m.parser),
  jsx: () => import("@lezer/javascript").then((m) => m.parser.configure({ dialect: "jsx" })),
  typescript: () => import("@lezer/javascript").then((m) => m.parser.configure({ dialect: "ts" })),
  tsx: () => import("@lezer/javascript").then((m) => m.parser.configure({ dialect: "ts jsx" })),
  rust: () => import("@lezer/rust").then((m) => m.parser),
  python: () => import("@lezer/python").then((m) => m.parser),
  json: () => import("@lezer/json").then((m) => m.parser),
  css: () => import("@lezer/css").then((m) => m.parser),
  html: () => import("@lezer/html").then((m) => m.parser),
  go: () => import("@lezer/go").then((m) => m.parser),
  java: () => import("@lezer/java").then((m) => m.parser),
  cpp: () => import("@lezer/cpp").then((m) => m.parser),
  yaml: () => import("@lezer/yaml").then((m) => m.parser),
  markdown: () => import("@lezer/markdown").then((m) => m.parser),
  xml: () => import("@lezer/xml").then((m) => m.parser),
  php: () => import("@lezer/php").then((m) => m.parser),
};

/** Fence labels and file extensions, mapped to the grammar names above. */
const ALIASES: Record<string, string> = {
  js: "javascript", mjs: "javascript", cjs: "javascript", node: "javascript",
  ts: "typescript", mts: "typescript", cts: "typescript",
  rs: "rust", py: "python", pyi: "python", jsonc: "json", json5: "json",
  htm: "html", svg: "xml", xaml: "xml", csproj: "xml", yml: "yaml", md: "markdown",
  c: "cpp", h: "cpp", cc: "cpp", cxx: "cpp", hpp: "cpp", hh: "cpp", "c++": "cpp",
  scss: "css", less: "css", golang: "go",
};

const parsers = new Map<string, Parser>();
const loading = new Set<string>();
const [grammarsLoaded, setGrammarsLoaded] = createSignal(0);

/** The grammar for a fence label (`ts`, `rust`, …), or `undefined` if none is known. */
export function languageOf(label: string | null | undefined): string | undefined {
  const name = label?.trim().toLowerCase();
  if (!name) return undefined;
  const resolved = ALIASES[name] ?? name;
  return resolved in LOADERS ? resolved : undefined;
}

/** The grammar for a file path, by its extension. */
export function languageOfPath(path: string | null | undefined): string | undefined {
  const ext = path?.split(/[\\/]/).pop()?.split(".").pop();
  return ext && ext !== path ? languageOf(ext) : undefined;
}

export function escapeHtml(text: string): string {
  return text.replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[c]!);
}

/**
 * `code` as highlighted HTML (lines separated by "\n"), or escaped plain text while the grammar
 * loads or if the language is unknown. Reactive: re-runs once a grammar arrives.
 */
export function highlightCode(code: string, language: string | undefined): string {
  grammarsLoaded(); // re-render when a grammar finishes loading
  const parser = language ? parsers.get(language) : undefined;
  if (!parser) {
    if (language) void load(language);
    return escapeHtml(code);
  }
  let html = "";
  lezerHighlight(
    code,
    parser.parse(code),
    classHighlighter,
    (text, classes) => (html += classes ? `<span class="${classes}">${escapeHtml(text)}</span>` : escapeHtml(text)),
    () => (html += "\n"),
  );
  return html;
}

async function load(language: string) {
  if (loading.has(language) || parsers.has(language)) return;
  loading.add(language);
  try {
    parsers.set(language, await LOADERS[language]());
    setGrammarsLoaded((n) => n + 1);
  } catch {
    // Leave it plain: an unloadable grammar shouldn't break the chat.
  } finally {
    loading.delete(language);
  }
}

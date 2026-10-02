// Static syntax highlighting with Lezer grammars (ticket 02): highlighted HTML, no editor instance.
// Grammars load on first use; anything rendered before its grammar arrives shows as plain text,
// then re-renders, because `highlightCode` reads that language's "loaded" signal (and only that
// language's, so loading one grammar doesn't re-render code in others).
import type { Parser } from "@lezer/common";
import { classHighlighter, highlightCode as lezerHighlight } from "@lezer/highlight";
import { type Accessor, createSignal, type Setter } from "solid-js";
import { escapeHtml } from "./html";

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

interface Grammar {
  parser: Parser | undefined;
  loaded: Accessor<boolean>;
  setLoaded: Setter<boolean>;
  /** The load in flight (or done), shared by everyone waiting for this grammar. */
  loading: Promise<void> | undefined;
}

const grammars = new Map<string, Grammar>();

function grammar(language: string): Grammar {
  let g = grammars.get(language);
  if (!g) {
    const [loaded, setLoaded] = createSignal(false);
    g = { parser: undefined, loaded, setLoaded, loading: undefined };
    grammars.set(language, g);
  }
  return g;
}

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

/**
 * `code` as highlighted HTML (lines separated by "\n"), or escaped plain text while the grammar
 * loads or if the language is unknown. Reactive: re-runs once that language's grammar arrives.
 */
export function highlightCode(code: string, language: string | undefined): string {
  if (!language) return escapeHtml(code);
  const g = grammar(language);
  if (!g.loaded()) {
    void load(language, g);
    return escapeHtml(code);
  }
  let html = "";
  lezerHighlight(
    code,
    g.parser!.parse(code),
    classHighlighter,
    (text, classes) => (html += classes ? `<span class="${classes}">${escapeHtml(text)}</span>` : escapeHtml(text)),
    () => (html += "\n"),
  );
  return html;
}

function load(language: string, g: Grammar): Promise<void> {
  g.loading ??= LOADERS[language]()
    .then((parser) => {
      g.parser = parser;
      g.setLoaded(true);
    })
    .catch(() => {
      // Leave it plain: an unloadable grammar shouldn't break the chat (a later use retries).
      g.loading = undefined;
    });
  return g.loading;
}

/** The Lezer parser for `language`, loading it on first use (shared with the chat's highlighting). */
export async function parserFor(language: string): Promise<Parser | undefined> {
  const g = grammar(language);
  await load(language, g);
  return g.parser;
}
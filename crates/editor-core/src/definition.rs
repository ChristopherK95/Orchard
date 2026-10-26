//! Go to definition in the Manual editor: no language server, just text. The import line that
//! brings a name into the file says which file defines it, so that file is found first (relative
//! and `crate::` / package paths, re-exports followed a few hops) and the name's definition is
//! looked for in it. A name with no import, or one that can't be followed, is looked for in the
//! file itself and then, as the imprecise last resort, in every Worktree file of the same
//! language. TypeScript / JavaScript, Rust and Python.

use std::collections::HashSet;

use serde::Serialize;

/// How many re-exports (`export * from`, `pub use`) are followed from the imported file.
const HOPS: usize = 3;
/// The most definitions the last-resort search over the Worktree reports.
const MAX_FALLBACK: usize = 20;
/// Files bigger than this aren't searched (generated or minified code).
pub(crate) const MAX_FILE_BYTES: u64 = 2 * 1024 * 1024;

/// Where a name is defined: a file (absolute) and the 1-based line.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Definition {
    pub path: String,
    pub line: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Lang {
    Ts,
    Rust,
    Python,
}

impl Lang {
    fn of(path: &str) -> Option<Lang> {
        let ext = path.rsplit_once('.')?.1;
        match ext {
            "ts" | "tsx" | "js" | "jsx" | "mjs" | "cjs" | "mts" | "cts" => Some(Lang::Ts),
            "rs" => Some(Lang::Rust),
            "py" | "pyi" => Some(Lang::Python),
            _ => None,
        }
    }
}

/// A definition found: a file relative to the Worktree, and the 1-based line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Found {
    pub path: String,
    pub line: u32,
}

/// What to search: the Worktree's files, and the file the symbol was clicked in (its text may have
/// unsaved changes, so it's given rather than read).
pub(crate) struct Lookup<'a> {
    pub files: &'a [String],
    pub current: &'a str,
    pub text: &'a str,
    /// A Worktree file's text (relative path), if it can be read.
    pub read: &'a dyn Fn(&str) -> Option<String>,
}

pub(crate) fn find(lookup: &Lookup, symbol: &str) -> Vec<Found> {
    let Some(lang) = Lang::of(lookup.current) else {
        return Vec::new();
    };
    if symbol.is_empty() || !symbol.chars().all(is_ident) || is_keyword(lang, symbol) {
        return Vec::new();
    }
    let ctx = Ctx {
        lookup,
        lang,
        existing: lookup.files.iter().map(String::as_str).collect(),
    };

    if let Some(import) = import_of(lang, lookup.text, symbol) {
        let mut seen = HashSet::new();
        let found: Vec<Found> = ctx
            .resolve(&import.module, lookup.current)
            .iter()
            .flat_map(|file| ctx.search_file(file, &import.name, HOPS, &mut seen))
            .collect();
        if !found.is_empty() {
            return found;
        }
    }
    let here = lines_defining(lang, lookup.text, symbol);
    if !here.is_empty() {
        return here
            .into_iter()
            .map(|line| Found {
                path: lookup.current.to_owned(),
                line,
            })
            .collect();
    }
    ctx.search_everywhere(symbol)
}

struct Ctx<'a> {
    lookup: &'a Lookup<'a>,
    lang: Lang,
    existing: HashSet<&'a str>,
}

impl Ctx<'_> {
    /// Definitions of `symbol` in `file`, following its re-exports of the name.
    fn search_file(
        &self,
        file: &str,
        symbol: &str,
        hops: usize,
        seen: &mut HashSet<String>,
    ) -> Vec<Found> {
        if !seen.insert(format!("{file}\0{symbol}")) {
            return Vec::new();
        }
        let Some(text) = (self.lookup.read)(file) else {
            return Vec::new();
        };
        let here = lines_defining(self.lang, &text, symbol);
        if !here.is_empty() {
            return here
                .into_iter()
                .map(|line| Found {
                    path: file.to_owned(),
                    line,
                })
                .collect();
        }
        if hops == 0 {
            return Vec::new();
        }
        // (module, the name in it) pairs the file passes the name on from.
        let mut onward: Vec<(String, String)> = Vec::new();
        if let Some(import) = import_of(self.lang, &text, symbol) {
            onward.push((import.module, import.name));
        }
        if self.lang == Lang::Ts {
            onward.extend(star_exports(&text).into_iter().map(|m| (m, symbol.to_owned())));
        }
        let mut found = Vec::new();
        for (module, name) in onward {
            for target in self.resolve(&module, file) {
                found.extend(self.search_file(&target, &name, hops - 1, seen));
            }
        }
        found
    }

    /// The Worktree files a module path names, as written in `from` (a file relative to the
    /// Worktree).
    fn resolve(&self, module: &str, from: &str) -> Vec<String> {
        match self.lang {
            Lang::Ts => self.resolve_ts(module, from),
            Lang::Rust => self.resolve_rust(module),
            Lang::Python => self.resolve_python(module, from),
        }
    }

    fn resolve_ts(&self, spec: &str, from: &str) -> Vec<String> {
        if !spec.starts_with('.') {
            return Vec::new(); // (a package, or a path alias: not followed)
        }
        let base = join_relative(dir_of(from), spec);
        // `./a.js` in TypeScript names `a.ts`.
        let stem = ["js", "jsx", "mjs", "cjs"]
            .iter()
            .find_map(|ext| base.strip_suffix(&format!(".{ext}")))
            .unwrap_or(&base)
            .to_owned();
        let mut tries = vec![base.clone()];
        for ext in ["ts", "tsx", "d.ts", "js", "jsx", "mjs", "cjs", "mts", "cts"] {
            tries.push(format!("{stem}.{ext}"));
            tries.push(format!("{base}.{ext}"));
        }
        for ext in ["ts", "tsx", "js", "jsx"] {
            tries.push(format!("{base}/index.{ext}"));
        }
        tries
            .into_iter()
            .find(|candidate| self.existing.contains(candidate.as_str()))
            .into_iter()
            .collect()
    }

    fn resolve_rust(&self, module: &str) -> Vec<String> {
        // `crate::a::b` and `a::b` both name the module path `a/b` somewhere in the Worktree (a
        // crate's `src` folder, a workspace member, …).
        let segments: Vec<&str> = module
            .split("::")
            .map(str::trim)
            .filter(|s| !matches!(*s, "crate" | "self" | "super" | ""))
            .collect();
        if segments.is_empty() {
            return Vec::new();
        }
        let tail = segments.join("/");
        self.ending_with(&[format!("{tail}.rs"), format!("{tail}/mod.rs")])
    }

    fn resolve_python(&self, module: &str, from: &str) -> Vec<String> {
        let dots = module.chars().take_while(|c| *c == '.').count();
        let rest = module[dots..].replace('.', "/");
        if rest.is_empty() {
            return Vec::new();
        }
        let wanted = [format!("{rest}.py"), format!("{rest}/__init__.py")];
        if dots == 0 {
            return self.ending_with(&wanted);
        }
        let mut dir = dir_of(from);
        for _ in 1..dots {
            dir = dir_of(dir);
        }
        wanted
            .iter()
            .map(|w| if dir.is_empty() { w.clone() } else { format!("{dir}/{w}") })
            .filter(|c| self.existing.contains(c.as_str()))
            .collect()
    }

    /// Files whose path is one of `wanted` or ends in one of them after a `/`.
    fn ending_with(&self, wanted: &[String]) -> Vec<String> {
        self.lookup
            .files
            .iter()
            .filter(|f| {
                wanted
                    .iter()
                    .any(|w| *f == w || f.ends_with(&format!("/{w}")))
            })
            .cloned()
            .collect()
    }

    /// The last resort: every definition of the name in the Worktree's files of this language.
    fn search_everywhere(&self, symbol: &str) -> Vec<Found> {
        let mut found = Vec::new();
        for file in self.lookup.files {
            if Lang::of(file) != Some(self.lang) || file == self.lookup.current {
                continue;
            }
            let Some(text) = (self.lookup.read)(file) else {
                continue;
            };
            for line in lines_defining(self.lang, &text, symbol) {
                found.push(Found {
                    path: file.clone(),
                    line,
                });
            }
            if found.len() >= MAX_FALLBACK {
                found.truncate(MAX_FALLBACK);
                break;
            }
        }
        found
    }
}

fn is_ident(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '$'
}

fn is_keyword(lang: Lang, word: &str) -> bool {
    let common = ["if", "else", "for", "while", "return", "match", "in", "as"];
    let own: &[&str] = match lang {
        Lang::Ts => &["function", "class", "const", "let", "var", "switch", "catch", "new"],
        Lang::Rust => &["fn", "let", "mut", "struct", "enum", "impl", "use", "pub", "loop"],
        Lang::Python => &["def", "class", "import", "from", "lambda", "with", "try"],
    };
    common.contains(&word) || own.contains(&word)
}

fn dir_of(path: &str) -> &str {
    path.rsplit_once('/').map_or("", |(dir, _)| dir)
}

/// `spec` (`./a`, `../b/c`) resolved against folder `dir`, `/`-separated.
fn join_relative(dir: &str, spec: &str) -> String {
    let mut parts: Vec<&str> = dir.split('/').filter(|p| !p.is_empty()).collect();
    for part in spec.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            other => parts.push(other),
        }
    }
    parts.join("/")
}

struct Import {
    /// The module path as written (`./config`, `crate::config`, `.config`).
    module: String,
    /// The name it has in that module (differs from the local one with `as`).
    name: String,
}

/// The import that brings `symbol` into `text`, if there is one.
fn import_of(lang: Lang, text: &str, symbol: &str) -> Option<Import> {
    match lang {
        Lang::Ts => ts_import(text, symbol),
        Lang::Rust => rust_import(text, symbol),
        Lang::Python => python_import(text, symbol),
    }
}

/// One `a`, `a as b` or `type a` item of an import list: (name there, name here).
fn item_names(item: &str) -> Option<(String, String)> {
    let item = item.trim();
    let item = item.strip_prefix("type ").unwrap_or(item).trim();
    let mut words = item.split_whitespace();
    let name = words.next()?.to_owned();
    match (words.next(), words.next()) {
        (Some("as"), Some(local)) => Some((name, local.to_owned())),
        _ => Some((name.clone(), name)),
    }
}

/// A TS statement split at its last `from`: the clause before it, and the quoted module path.
fn ts_from(statement: &str) -> Option<(&str, &str)> {
    let at = statement.rfind("from ")?;
    let (clause, rest) = statement.split_at(at);
    if !clause.is_empty() && !clause.ends_with([' ', '}', '*']) {
        return None;
    }
    let rest = rest.strip_prefix("from")?.trim();
    let quote = rest.chars().next().filter(|c| matches!(c, '"' | '\''))?;
    let inner = &rest[1..];
    let end = inner.find(quote)?;
    Some((clause, &inner[..end]))
}

fn ts_import(text: &str, symbol: &str) -> Option<Import> {
    let mut lines = text.lines();
    while let Some(line) = lines.next() {
        let start = line.trim_start();
        let is_import = start.starts_with("import ") || start.starts_with("import{");
        let is_reexport = start.starts_with("export {") || start.starts_with("export type {");
        if !(is_import || is_reexport) {
            continue;
        }
        let mut statement = start.to_owned();
        let mut taken = 0;
        while ts_from(&statement).is_none() && !statement.trim_end().ends_with(';') && taken < 60 {
            let Some(next) = lines.next() else { break };
            statement.push(' ');
            statement.push_str(next.trim());
            taken += 1;
        }
        let Some((clause, module)) = ts_from(&statement) else {
            continue;
        };
        let clause = clause
            .trim_start_matches("import")
            .trim_start_matches("export")
            .trim();
        let clause = clause.strip_prefix("type ").unwrap_or(clause);
        let (default, named) = match clause.split_once('{') {
            Some((default, rest)) => (default, rest.split('}').next().unwrap_or("")),
            None => (clause, ""),
        };
        let default = default.trim().trim_end_matches(',').trim();
        if is_import && default == symbol {
            return Some(Import {
                module: module.to_owned(),
                name: symbol.to_owned(),
            });
        }
        for item in named.split(',') {
            if let Some((name, local)) = item_names(item) {
                if local == symbol {
                    return Some(Import {
                        module: module.to_owned(),
                        name,
                    });
                }
            }
        }
    }
    None
}

/// The modules a TS file re-exports everything of (`export * from "./a"`).
fn star_exports(text: &str) -> Vec<String> {
    text.lines()
        .filter(|l| l.trim_start().starts_with("export *"))
        .filter_map(|l| ts_from(l).map(|(_, module)| module.to_owned()))
        .collect()
}

fn rust_import(text: &str, symbol: &str) -> Option<Import> {
    let mut lines = text.lines();
    while let Some(line) = lines.next() {
        let start = line.trim_start();
        let rest = ["pub use ", "use "]
            .iter()
            .find_map(|kw| start.strip_prefix(kw))
            .or_else(|| {
                // `pub(crate) use …`
                start
                    .strip_prefix("pub(")
                    .and_then(|r| r.split_once(") use "))
                    .map(|(_, r)| r)
            });
        let Some(rest) = rest else { continue };
        let mut statement = rest.to_owned();
        let mut taken = 0;
        while !statement.contains(';') && taken < 60 {
            let Some(next) = lines.next() else { break };
            statement.push(' ');
            statement.push_str(next.trim());
            taken += 1;
        }
        let body = statement.split(';').next().unwrap_or("");
        let (prefix, items) = match body.split_once('{') {
            Some((prefix, inner)) => (prefix, inner.to_owned()),
            None => match body.rsplit_once("::") {
                Some((prefix, last)) => (prefix, last.to_owned()),
                None => continue,
            },
        };
        let prefix = prefix.trim().trim_end_matches("::").trim();
        for item in items.split([',', '}']) {
            // (`a::b` inside a group names a deeper module: its own tail is the name.)
            let (module, item) = match item.trim().rsplit_once("::") {
                Some((inner, last)) => (format!("{prefix}::{inner}"), last),
                None => (prefix.to_owned(), item.trim()),
            };
            if let Some((name, local)) = item_names(item) {
                if local == symbol {
                    return Some(Import { module, name });
                }
            }
        }
    }
    None
}

fn python_import(text: &str, symbol: &str) -> Option<Import> {
    let mut lines = text.lines();
    while let Some(line) = lines.next() {
        let Some(rest) = line.trim_start().strip_prefix("from ") else {
            continue;
        };
        let Some((module, names)) = rest.split_once(" import ") else {
            continue;
        };
        let mut names = names.to_owned();
        let mut taken = 0;
        while names.contains('(') && !names.contains(')') && taken < 60 {
            let Some(next) = lines.next() else { break };
            names.push(' ');
            names.push_str(next.trim());
            taken += 1;
        }
        let names = names.split('#').next().unwrap_or("");
        let names = names.trim().trim_start_matches('(').trim_end_matches(')');
        for item in names.split(',') {
            if let Some((name, local)) = item_names(item) {
                if local == symbol {
                    return Some(Import {
                        module: module.trim().to_owned(),
                        name,
                    });
                }
            }
        }
    }
    None
}

/// The 1-based lines of `text` that define `symbol`.
fn lines_defining(lang: Lang, text: &str, symbol: &str) -> Vec<u32> {
    text.lines()
        .enumerate()
        .filter(|(_, line)| line.contains(symbol) && defines(lang, line, symbol))
        .map(|(i, _)| i as u32 + 1)
        .collect()
}

/// Strips leading words in `words` (each followed by a space) from `s`.
fn strip_words<'a>(mut s: &'a str, words: &[&str]) -> &'a str {
    'again: loop {
        for word in words {
            if let Some(rest) = s.strip_prefix(word).and_then(|r| r.strip_prefix(' ')) {
                s = rest.trim_start();
                continue 'again;
            }
        }
        return s;
    }
}

fn ident_at(s: &str) -> &str {
    let end = s.find(|c: char| !is_ident(c)).unwrap_or(s.len());
    &s[..end]
}

/// Whether `s` is `keyword name …` for any of the keywords.
fn keyword_then_name(s: &str, keywords: &[&str], symbol: &str) -> bool {
    keywords.iter().any(|kw| {
        s.strip_prefix(kw)
            .and_then(|r| r.strip_prefix([' ', '*']))
            .is_some_and(|r| ident_at(r.trim_start_matches(['*', ' '])) == symbol)
    })
}

fn defines(lang: Lang, line: &str, symbol: &str) -> bool {
    let s = line.trim_start();
    let indent = line.len() - s.len();
    match lang {
        Lang::Ts => {
            let s = strip_words(s, &["export", "default", "declare", "abstract", "async"]);
            let keywords = [
                "function",
                "class",
                "interface",
                "type",
                "enum",
                "const",
                "let",
                "var",
                "namespace",
                "module",
            ];
            if keyword_then_name(s, &keywords, symbol) {
                return true;
            }
            // A method: `async name(` / `private static name<T>(`, its signature ending in `{`.
            let s = strip_words(
                s,
                &["public", "private", "protected", "static", "readonly", "override", "get", "set", "async"],
            );
            let name = ident_at(s);
            indent > 0
                && name == symbol
                && s[name.len()..].starts_with(['(', '<'])
                && s.trim_end().ends_with('{')
        }
        Lang::Rust => {
            let mut s = s;
            // `pub`, `pub(crate)`, `pub(in path)`.
            if let Some(rest) = s.strip_prefix("pub") {
                s = match rest.strip_prefix('(').and_then(|r| r.split_once(')')) {
                    Some((_, after)) => after.trim_start(),
                    None => rest.trim_start(),
                };
            }
            let s = strip_words(s, &["default", "async", "unsafe", "const", "extern", "safe"]);
            if let Some(rest) = s.strip_prefix("macro_rules!") {
                return ident_at(rest.trim_start()) == symbol;
            }
            let keywords = ["fn", "struct", "enum", "trait", "type", "const", "static", "mod", "union"];
            keywords.iter().any(|kw| {
                s.strip_prefix(kw)
                    .and_then(|r| r.strip_prefix(' '))
                    .is_some_and(|r| ident_at(strip_words(r.trim_start(), &["mut"])) == symbol)
            })
        }
        Lang::Python => {
            if keyword_then_name(strip_words(s, &["async"]), &["def", "class"], symbol) {
                return true;
            }
            // A module-level `NAME = …` or `NAME: type = …`.
            let after = s.strip_prefix(symbol).map(str::trim_start);
            indent == 0
                && after.is_some_and(|a| {
                    (a.starts_with('=') && !a.starts_with("==")) || a.starts_with(':')
                })
                && ident_at(s) == symbol
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn lookup_in(files: &[(&str, &str)], current: &str, symbol: &str) -> Vec<(String, u32)> {
        let map: HashMap<String, String> = files
            .iter()
            .map(|(p, t)| ((*p).to_owned(), (*t).to_owned()))
            .collect();
        let names: Vec<String> = files.iter().map(|(p, _)| (*p).to_owned()).collect();
        let read = |p: &str| map.get(p).cloned();
        let lookup = Lookup {
            files: &names,
            current,
            text: &map[current],
            read: &read,
        };
        find(&lookup, symbol)
            .into_iter()
            .map(|f| (f.path, f.line))
            .collect()
    }

    fn at(path: &str, line: u32) -> Vec<(String, u32)> {
        vec![(path.to_owned(), line)]
    }

    #[test]
    fn a_typescript_import_leads_to_its_file_and_line() {
        let files = [
            ("src/app.ts", "import { parseConfig as parse } from \"./config\";\nparse();"),
            ("src/config.ts", "// config\nexport function parseConfig() {}\n"),
            ("src/other.ts", "export function parseConfig() {}\n"),
        ];
        assert_eq!(lookup_in(&files, "src/app.ts", "parse"), at("src/config.ts", 2));
    }

    #[test]
    fn an_index_file_and_its_re_exports_are_followed() {
        let files = [
            ("a/main.ts", "import {\n  widget,\n} from \"./lib\";\n"),
            ("a/lib/index.ts", "export * from \"./widget\";\n"),
            ("a/lib/widget.ts", "\nexport const widget = 1;\n"),
        ];
        assert_eq!(lookup_in(&files, "a/main.ts", "widget"), at("a/lib/widget.ts", 2));
    }

    #[test]
    fn rust_use_paths_lead_to_module_files() {
        let files = [
            ("src/main.rs", "use crate::config::{load, Other};\nfn main() { load() }"),
            ("src/config.rs", "pub fn load() {}\n"),
            ("src/other/mod.rs", "pub fn load() {}\n"),
        ];
        assert_eq!(lookup_in(&files, "src/main.rs", "load"), at("src/config.rs", 1));
    }

    #[test]
    fn python_relative_and_package_imports() {
        let files = [
            ("pkg/app.py", "from .config import (\n    parse_config,\n)\n"),
            ("pkg/config.py", "X = 1\n\ndef parse_config():\n    pass\n"),
            ("main.py", "from pkg.config import parse_config\n"),
        ];
        assert_eq!(lookup_in(&files, "pkg/app.py", "parse_config"), at("pkg/config.py", 3));
        assert_eq!(lookup_in(&files, "main.py", "parse_config"), at("pkg/config.py", 3));
    }

    #[test]
    fn a_name_defined_in_the_same_file_is_found_there() {
        let files = [("a.ts", "const x = 1;\nfunction helper() {}\nhelper();")];
        assert_eq!(lookup_in(&files, "a.ts", "helper"), at("a.ts", 2));
    }

    #[test]
    fn a_method_falls_back_to_a_search_of_the_worktree() {
        let files = [
            ("src/a.rs", "fn main() { thing.run(); }"),
            ("src/b.rs", "impl Thing {\n    pub async fn run(&self) {}\n}\n"),
            ("src/c.ts", "function run() {}"),
        ];
        assert_eq!(lookup_in(&files, "src/a.rs", "run"), at("src/b.rs", 2));
    }

    #[test]
    fn calls_and_keywords_are_not_definitions() {
        let files = [("a.ts", "if (ready) {\n  run(1);\n}\nreturn run;")];
        assert!(lookup_in(&files, "a.ts", "run").is_empty());
        assert!(lookup_in(&files, "a.ts", "if").is_empty());
    }
}

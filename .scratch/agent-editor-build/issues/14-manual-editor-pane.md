# 14: Manual editor pane

**What to build:** Opening a file (from `Ctrl+P`, the Files tree, or "Open in editor" on a chat code block) shows it in a CodeMirror 6 **Manual editor** pane to the right of the chat, with its own file tabs. Lezer highlighting loads per language. Editing features: find/replace, go to line, soft-wrap toggle, bracket matching, auto-indent, multiple cursors. Vim is available through `editor.vim` (off by default), with `:w` saving and `:q` closing the file tab. Saving sends the expected on-disk version, and the core asks before overwriting a newer file. Files over ~5 MB open read-only and unhighlighted, binaries show a placeholder, and minified single-line files soft-wrap.

**Blocked by:** 13 (File watching and search), 08 (Settings file).

**Status:** done (Windows; Arch pending)

- [x] Files open in the pane with file tabs; highlighting loads on first use of a language.
- [x] Save writes the file; saving over a newer on-disk version asks first.
- [x] Toggling `editor.vim` in settings enables or disables vim live; `:w` and `:q` behave as specified.
- [x] Big, binary and minified files follow the spec's rules.
- [x] The memory benchmark scenario now includes 1 open Manual editor and still passes.
- [x] Core tests: save with a stale expected version is refused.

Notes: auto-indent means Enter keeps the line's indentation (the bare Lezer grammars carry no indent rules). Open in editor on a chat code block opens an unsaved snippet. Benchmark (Windows, with a file open): 116.4 MB total, 2.1 MB per extra Tab, 0.39% idle CPU; see docs/benchmarks/memory.md.

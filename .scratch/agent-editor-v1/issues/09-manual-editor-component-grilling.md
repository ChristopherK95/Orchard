# Manual editor component and vim mode

Type: grilling
Status: resolved
Blocked by: 04
Part of: [map](../map.md)
Prototype: branch prototype/code-blocks, .scratch/agent-editor-v1/prototypes/code-blocks-prototype.html (commit 0b2ef8c)

## Question

Which editor component powers the manual editor under the chosen stack (e.g. CodeMirror 6 vs Monaco for Tauri, or a custom widget for Odin), and how is syntax highlighting done? What level of vim support is "enough" for v1 (motions, text objects, `:w`, registers, visual mode)? Weigh the memory cost of having several editor windows open.

Context: the stack is Tauri 2 + SolidJS (ADR 0002), so the realistic candidates are CodeMirror 6 (+ `@replit/codemirror-vim`, Lezer or tree-sitter highlighting) and Monaco (+ `monaco-vim`). Each must fit the memory budget in ADR 0002 on both WebKitGTK and WebView2. Layout: the editor is a pane beside the chat with its own file tabs, and can pop out into a window (move semantics: text and cursor move, undo history is lost; ADR 0003). The manual-edit decision also needs: a "file changed on disk" banner with **Show diff / Reload / Keep mine**, silent reload of clean buffers, and a check against the on-disk version before saving. That means the component must show a diff (e.g. CodeMirror's merge view, or Monaco's diff editor).

## Answer

Resolved 2026-09-30.

- **Component: CodeMirror 6**, not Monaco. It's modular and light, embeds in SolidJS directly, and has the stronger vim mode. Monaco is heavier per instance and brings LSP-shaped machinery that's out of scope.
- **Highlighting: Lezer grammars, lazy-loaded per language** the first time a file needs one. Unknown languages open as plain text. tree-sitter/WASM was rejected.
- **Vim**: `@replit/codemirror-vim` as-is, with no custom vim work. `:w` maps to Save, which triggers the Edit note flow, and `:q` closes the file tab. It's a setting, `editor.vim`, **off by default** (the user described vim as optional).
- **Diff view** (`@codemirror/merge`), used by the conflict banner, "Changes vs base" and "Show diff":
  - **unified by default**, with a side-by-side toggle;
  - **read-only**, with **Edit file** (and Reload / Keep mine in the conflict banner).
- **Chat code blocks and permission-card diffs are static highlighted HTML**, rendered once with the same Lezer grammars. There's no editor instance per block. They have line numbers, **Copy** and **Open in editor**. Chosen over a CodeMirror instance per block and over "static, becoming live on click" after comparing both in the prototype, to keep the per-Tab budget.
- **Big and binary files**:
  - over ~5 MB opens read-only, without highlighting, with a notice;
  - binary files show a placeholder;
  - minified single-line files force soft-wrap on.
- **Editor features**: line numbers, find/replace (`Ctrl+F`), go to line, soft-wrap toggle, bracket matching, auto-indent, multiple cursors. **Not included**: minimap, folding, autocomplete.

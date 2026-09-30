# Manual editor component and vim mode

Type: grilling
Status: open
Blocked by: 04
Part of: [map](../map.md)

## Question

Which editor component powers the manual editor under the chosen stack (e.g. CodeMirror 6 vs Monaco for Tauri, or a custom widget for Odin), and how is syntax highlighting done? What level of vim support is "enough" for v1 (motions, text objects, `:w`, registers, visual mode)? Weigh the memory cost of having several editor windows open.

Context: the stack is Tauri 2 + SolidJS (ADR 0002), so the realistic candidates are CodeMirror 6 (+ `@replit/codemirror-vim`, Lezer or tree-sitter highlighting) and Monaco (+ `monaco-vim`). Each must fit the memory budget in ADR 0002 on both WebKitGTK and WebView2. Layout: the editor is a pane beside the chat with its own file tabs, and can pop out into a window (move semantics: text and cursor move, undo history is lost; ADR 0003). The manual-edit decision also needs: a "file changed on disk" banner with **Show diff / Reload / Keep mine**, silent reload of clean buffers, and a check against the on-disk version before saving. That means the component must show a diff (e.g. CodeMirror's merge view, or Monaco's diff editor).

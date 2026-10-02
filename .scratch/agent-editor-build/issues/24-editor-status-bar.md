# 24: Manual editor breadcrumb and status bar

**What to build:** The Manual editor pane gets the two thin bars from the design. Under the file tabs, a **breadcrumb**: the Worktree's branch with its colour as a 2 px left edge, then the file's path inside the Worktree (`agent/fix-login › src/ui/board.rs`). At the bottom, a **status bar**: language, `Ln 8, Col 12`, `Wrap: on/off`, indentation (`Spaces: 4` or `Tabs`), and the line ending (`LF` / `CRLF`). Read-only files (big files, review diffs) show a lock and "Read-only".

**Blocked by:** 14 (Manual editor pane).

**Status:** ready-for-agent

- [ ] The breadcrumb shows branch › relative path for a Worktree file, the settings file's path for the settings file, and "Unsaved snippet" for a chat snippet; a diff tab shows what it compares.
- [ ] Ln/Col follow the main cursor live; with several cursors it shows the main one and "(+N)".
- [ ] Clicking `Wrap` toggles soft-wrap (the same toggle as the tab bar's button).
- [ ] Indentation is detected from the file when it opens (spaces with their width, or tabs), falling back to 4 spaces.
- [ ] The line ending comes from the file as opened (`OpenedFile.lineEnding`).
- [ ] The pop-out window shows both bars too.
- [ ] Updating Ln/Col on every cursor move doesn't add noticeable CPU: idle CPU in the memory benchmark stays ≤ 1%.

Notes: the bars are 22–24 px, `$bg-2` on a `$line` border, 11.5 px text in `$fg-3` (mono for Ln/Col). Design reference: `pen_design.pen`, Screens · "Files drawer + Manual editor pane" and "Row — editor pane detail states".

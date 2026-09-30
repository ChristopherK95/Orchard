# 15: Pop out the Manual editor

**What to build:** A pop-out button moves a file from the pane into its own OS window, carrying its text and cursor (undo history is lost). The popped-out window follows the same save and conflict rules.

**Blocked by:** 14 (Manual editor pane).

**Status:** ready-for-agent

- [ ] Pop-out opens a new OS window with the file's current text and cursor, and removes it from the pane.
- [ ] Unsaved changes survive the move.
- [ ] The core still tracks the file as open and dirty.
- [ ] Closing the window with unsaved changes asks first.

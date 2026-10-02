# 15: Pop out the Manual editor

**What to build:** A pop-out button moves a file from the pane into its own OS window, carrying its text and cursor (undo history is lost). The popped-out window follows the same save and conflict rules.

**Blocked by:** 14 (Manual editor pane).

**Status:** done

- [x] Pop-out opens a new OS window with the file's current text and cursor, and removes it from the pane.
- [x] Unsaved changes survive the move.
- [x] The core still tracks the file as open and dirty.
- [x] Closing the window with unsaved changes asks first.

**Notes:** The core keeps a `DocumentTracker` (`documents.rs`): which files are open in which window and whether they're dirty. `pop_out` registers the new window's file before the window even loads, and `collect_pop_out` hands it over (again after a reload). A window that closes before it shows its file gives the text back to the pane (`PopOutReturned`). Tracking and hand-off are covered by `tests/documents.rs`. The window itself and the close prompt ("Save and close" / "Close without saving" / Cancel) need the running app to check.

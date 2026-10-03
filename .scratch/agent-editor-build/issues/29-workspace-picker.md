# 29: Workspace picker

**What to build:** The startup screen's lone path box becomes the **Workspace picker**: one input over a list of **Recent Workspaces**, plus a **Browse…** button (the native folder dialog). It still reads "Open a repository". The last Workspace is preselected, so Enter opens it; a CLI argument (or `AGENT_EDITOR_WORKSPACE`) skips the picker and opens its repo straight away.

**Blocked by:** 11 (close, reopen and restore: the app state the recents come from).

**Status:** done (Windows, type-checked and core-tested; not run in the app; Arch pending)

- [x] Opening a Workspace records when it was opened in its app state entry; the picker lists Recent Workspaces newest first, at most 20.
- [x] Workspaces in the app state from before this ticket (no time) are listed after the timed ones, alphabetically.
- [x] A row shows the name, the path (muted, truncated in the middle), when it was last opened ("2 h ago", "Yesterday") and how many sessions it will restore ("3 sessions"). No git is run for the list.
- [x] With no CLI argument, the most recently opened Workspace is selected, so Enter opens it.
- [x] With a CLI argument or `AGENT_EDITOR_WORKSPACE`, that repo opens without showing the picker (the benchmark relies on this); if it fails to open, the picker shows with the error.
- [x] Typing filters the rows by name and path, with the same fuzzy matching and highlighting as Ctrl+P. ↑/↓ move the selection; Enter opens it.
- [x] Text that looks like a path (contains `/` or `\`, or starts with a drive letter or `~`) opens as a path on Enter instead of filtering.
- [x] **Browse…** (and Ctrl+O) opens the native folder dialog; choosing a folder opens it.
- [x] A folder that isn't in a git repository shows an error naming it; nothing offers `git init`.
- [x] × on a row's hover (or Delete on the selected row) removes it from the list only: its saved Tabs and Recent sessions stay, and opening that repo again brings the row back.
- [x] A recent whose folder no longer exists stays listed, dimmed, with "Not found"; it can't be opened, can be removed, and is never pruned automatically.

Notes:
- The time and the "removed from the list" flag live on `WorkspaceState` in the app state (`#[serde(default)]`, so older files load). The core gives the view a `recent_workspaces()` list with the root, name, last opened, saved Tab count and whether the folder exists.
- Browse needs `tauri-plugin-dialog` (Rust crate, JS package, capability).
- Styling: the existing startup card and the Ctrl+P list. No design pass in `pen_design.pen`.
- Decided (2026-10-03): no scanning of folders for repos never opened, no pinned favourites, no `git init`. Switching Workspaces from inside one is ticket 30.

**Notes (done):**
- Core: `WorkspaceState` gained `opened` (ms since the epoch) and `hidden`; opening a Workspace stamps `opened`, clears `hidden` and saves at once (so a Workspace with no Tabs is listed too). `Core::recent_workspaces(query)` and `remove_recent_workspace(root)`; the filter is Ctrl+P's `files::find` over the roots. Tests: `tests/workspace_picker.rs`, plus app_state unit tests.
- The core now reads the state file afresh when listing and restoring, rather than the copy loaded at startup; ticket 30 needs that to switch back to a Workspace saved during this run.
- **Deviation:** Browse… calls the dialog plugin from Rust (`pick_folder`), so no JS package or capability was needed.
- **Deviation:** the path is cut off at its end, not in the middle: the name is shown in full before it, so the end of the path adds little.
- Delete removes the selected row only when the caret is at the end of the input, so it never stops deleting typed text.
- `~` is expanded by the `open_workspace` command.
- Any CLI argument now opens its repo straight away (it used to only in bench mode).

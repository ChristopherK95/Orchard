# 30: Switch Workspace in place

**What to build:** Open the Workspace picker from inside a Workspace and switch the window to another repo. The picker opens as an overlay; nothing closes until a different repo is picked. The old Workspace is saved exactly as on quit, so opening it again later restores its Tabs.

**Blocked by:** 29 (Workspace picker).

**Status:** done except the benchmark scene (Windows, type-checked and core-tested; not run in the app; Arch pending)

- [x] The picker opens from the repo name in the title bar, a "Switch repository…" command in Ctrl+P's commands section, and Ctrl+Shift+O. Escape (or a click outside) cancels, leaving everything as it was.
- [x] Picking the Workspace that's already open just closes the picker.
- [x] If any Agent session is Working or Needs you, switching first asks: "N Agent sessions are working. Switching stops them; they can be resumed when you come back." Cancel keeps the current Workspace.
- [x] Unsaved Manual editor changes (in the pane or a popped-out window) get the existing Save / Discard / Cancel question; Cancel keeps the current Workspace.
- [x] Popped-out editor windows close.
- [x] The old Workspace's open Tabs, active Tab, Bases and pins are saved as on quit; its Agent processes stop, its watchers and Worktree actors go, and running Worktree setups stop.
- [x] The new Workspace opens in the same window with its own restored Tabs; no state (sessions, Recent sessions, drawer, Manual editor tabs, Board, Columns) leaks across.
- [ ] Switching back and forth a few times doesn't grow memory: the memory benchmark gets a switch scene and stays within ADR 0002's budget.

Notes:
- `open_workspace` already drops the previous Workspace's setups; this ticket makes the rest of the teardown explicit (a core `close_workspace`, tested in `tests/`), then reuses the open path.
- The frontend can remount `WorkspaceView` for the new Workspace rather than resetting each signal.
- Decided (2026-10-03): switching is in place, in one window. Several Workspaces in separate windows is out of scope.

**Notes (done):**
- Core: `open_workspace` with another Workspace open closes it first (`close_workspace`): it saves the Tabs, then takes the Workspace away so nothing saves over them, stops every Agent (in parallel), resets the per-Workspace state (session ids keep counting), and drops the Worktree watch, actors, visible Tabs and open-file tracking (`DocumentTracker::close_all`). Opening the Workspace already open changes nothing. A path that isn't a repository fails before anything closes. `workspace_for(path)` resolves a path without opening it. Tests: `tests/switch_workspace.rs`.
- Frontend: `App` shows `WorkspaceView` keyed by Workspace, so a switch builds a fresh view. The picker is the same component as at startup, as an overlay (the open one is labelled "Open now", and the one before it is selected). The view's `switchTo` resolves the path, asks if needed, closes popped-out windows (`close_pop_outs`), then opens.
- **Deviation:** "Save and switch" is only offered when every unsaved file is in the main window. The main window can't save a popped-out window's buffer, so the dialog says to save those in their own window first, or discard them.
- **Not done:** the memory benchmark switch scene. Benchmarks aren't run unless asked; the core tests check that no session or Worktree comes across, that the old Agents get `session/close`, and that switching back restores the Tabs. Nothing checks that the actors and watch really free their memory.

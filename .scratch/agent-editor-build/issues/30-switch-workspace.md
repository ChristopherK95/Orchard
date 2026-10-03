# 30: Switch Workspace in place

**What to build:** Open the Workspace picker from inside a Workspace and switch the window to another repo. The picker opens as an overlay; nothing closes until a different repo is picked. The old Workspace is saved exactly as on quit, so opening it again later restores its Tabs.

**Blocked by:** 29 (Workspace picker).

**Status:** ready-for-agent

- [ ] The picker opens from the repo name in the title bar, a "Switch repository…" command in Ctrl+P's commands section, and Ctrl+Shift+O. Escape (or a click outside) cancels, leaving everything as it was.
- [ ] Picking the Workspace that's already open just closes the picker.
- [ ] If any Agent session is Working or Needs you, switching first asks: "N Agent sessions are working. Switching stops them; they can be resumed when you come back." Cancel keeps the current Workspace.
- [ ] Unsaved Manual editor changes (in the pane or a popped-out window) get the existing Save / Discard / Cancel question; Cancel keeps the current Workspace.
- [ ] Popped-out editor windows close.
- [ ] The old Workspace's open Tabs, active Tab, Bases and pins are saved as on quit; its Agent processes stop, its watchers and Worktree actors go, and running Worktree setups stop.
- [ ] The new Workspace opens in the same window with its own restored Tabs; no state (sessions, Recent sessions, drawer, Manual editor tabs, Board, Columns) leaks across.
- [ ] Switching back and forth a few times doesn't grow memory: the memory benchmark gets a switch scene and stays within ADR 0002's budget.

Notes:
- `open_workspace` already drops the previous Workspace's setups; this ticket makes the rest of the teardown explicit (a core `close_workspace`, tested in `tests/`), then reuses the open path.
- The frontend can remount `WorkspaceView` for the new Workspace rather than resetting each signal.
- Decided (2026-10-03): switching is in place, in one window. Several Workspaces in separate windows is out of scope.

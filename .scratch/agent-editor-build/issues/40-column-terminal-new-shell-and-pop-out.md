# 40: New shell and pop-out on a column's Terminal tab

**What to build:** Two buttons the design puts in the cwd strip of a column's Terminal tab, which the redesign left out: + (a new shell) and pop-out (the terminal in its own window).

**Blocked by:** none (35 is done).

**Status:** not started

- [x] + starts another shell in the Worktree and shows it in the column, using 35's way of switching between shells. (Done with 35.)
- [ ] Pop-out moves the Worktree's terminal into its own window, like a popped-out Manual editor. The column's Terminal tab then says it's popped out and offers a button to bring it back. Closing the window brings it back too.
- [ ] The popped-out window passes the app's shortcuts (Ctrl+P, Ctrl+`, …) on to the main window, as the panel does.
- [ ] The Tabs view's Terminal panel gets the same pop-out.
- [ ] Switching Workspace closes popped-out terminals, as it does popped-out editors, and their shells stop with the Workspace.

Notes:
- Pop-outs already exist for the Manual editor (`pop_out`, `PopOut.tsx`). A terminal window would be a second kind, with its own view slot.
- Clear and Stop are already in the cwd strip (a26221e).

# 23: Typed confirmation when removing a Worktree would lose work

**What to build:** When removing a Worktree would lose work (the Remove dialog needs "Discard and remove"), the dialog adds a "Type the branch name to confirm" field, and "Discard and remove" stays disabled until the field matches the Worktree's branch. A clean Worktree still removes with one Enter. From the design (Notes · "Typed confirmation only when work would be lost"): friction proportional to the damage, so the clean case stays trivial and the dirty case is slow on purpose.

**Blocked by:** 09 (Remove a Worktree safely).

**Status:** ready-for-agent

- [ ] With uncommitted changes, or commits that would be lost (branch deleted too, or HEAD detached), the dialog shows the field with "Required only when work would be lost." under it.
- [ ] "Discard and remove" is disabled until the typed text equals the branch name exactly; Enter in the field runs it once it matches.
- [ ] Detached HEAD (no branch): the field asks for the Worktree's folder name instead, and says so.
- [ ] Ticking or unticking "Delete branch too" re-evaluates whether the field is needed (it can come and go), keeping what was typed.
- [ ] A clean, pushed Worktree shows no field and removes with one Enter, as now.
- [ ] Esc still cancels from the field.

Notes: frontend only. The core already refuses a discard whose work changed since the dialog's check (fingerprint), so this adds friction, not safety logic. Design reference: `pen_design.pen`, Screens · "Remove dirty".

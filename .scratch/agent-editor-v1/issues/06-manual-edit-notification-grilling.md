# How manual edits reach the Agent session

Type: grilling
Status: resolved
Blocked by: 02
Part of: [map](../map.md)

## Question

When the user edits a file by hand, how and when does the relevant Agent session learn about it? Settle:

- Which sessions count as "relevant": every session on that Worktree, or only the one that last touched the file?
- The trigger: on save, on an explicit "send to agent", or batched into the next prompt.
- The payload: a diff or a short note.
- What happens while the agent is mid-turn.
- Conflicts when the agent writes a file the user has open with unsaved changes.

Context: the core tracks which files are open and unsaved in any Manual editor, and Worktree actors watch the files (ADR 0003). Sessions are hosted through ACP (ADR 0001), so IDE integration is out. The mechanisms available are Claude Code's built-in changed-file diffs (undocumented; covers files the Agent fully read), hook `additionalContext` (mid-turn via PostToolUse, at prompt time via UserPromptSubmit), and sending a message through ACP (it lands after the current tool calls).

## Answer

Resolved 2026-09-30.

- **Who hears about an edit**: only live Agent sessions **on the same Worktree that have read or edited that file** in their conversation. The core tracks this from the tool calls it sees over ACP. Other sessions are never told. Sessions in other Worktrees have their own copy of the file. A Suspended session's notes queue until it resumes.
- **Trigger**: **on save** only. Unsaved text doesn't exist for the Agent, which reads from disk. Several saves before delivery combine into one diff per file.
- **Delivery**:
  - The guaranteed path is an **Edit note attached to the next prompt** the user sends in that session, as normal ACP prompt content.
  - Claude Code's own undocumented changed-file detection (files it fully read) is a free bonus, not relied on.
  - Idle sessions are not woken.
  - Mid-turn delivery via hooks is **not in v1**. A Working Agent that touches an edited file is already covered by Claude Code's "modified since read, re-read it" safety.
- **What the note contains**:
  - A unified diff per file of the user's changes since the last delivered note, capped at ~200 lines per file. Beyond the cap it becomes "`<file>` changed substantially, re-read it".
  - Shown as a visible, expandable **chip above the composer** (*"📝 You edited `status.rs` (+5 −1)"*). It can be removed before sending and is sent with the message. It's never silent.
- **Conflicts**:
  - When the Agent writes a file open in a Manual editor, a **clean** buffer reloads silently. A **dirty** buffer shows a banner, *"Agent changed this file"*, with **Show diff / Reload (drop mine) / Keep mine**, and never auto-reloads.
  - An Edit permission card on a file with unsaved changes warns *"You have unsaved changes in this file"*.
  - Saving when the file changed on disk since it was loaded asks before overwriting.
- **No manual "send to agent"** (selection/file → chat) in v1.

# What an Agent session Tab shows: terminal or native UI

Type: grilling
Status: resolved
Blocked by: 02
Part of: [map](../map.md)

## Question

Does an Agent session Tab render the **Claude Code TUI in an embedded terminal**, or a **native chat / tool-call UI** built from a structured stream (stream-json, SDK, or ACP), or a hybrid of the two? Decide based on what the hosting research shows is possible on a subscription, which features each option unlocks (inline diffs, permission prompts, notifying about manual edits, slash commands), and memory cost.

## Answer

Resolved 2026-09-30. Recorded as [ADR 0001](../../../docs/adr/0001-native-chat-via-acp-on-subscription.md); session states are in `CONTEXT.md`.

- **Native chat UI** (not an embedded TUI), hosted through the **ACP adapter** (`claude-agent-acp`): one shared Node process for all sessions, plus one `claude` process per Agent session, on the user's **Claude subscription**. The terms grey zone is accepted knowingly; if it breaks, it gets fixed then. ACP doubles as the boundary for other Agents later.
- **No terminal escape hatch** in v1.
- **Permission prompts**: inline approve/deny cards in the chat, with the diff shown *before* an edit is approved, and "always allow" rules. The permission mode is switchable per Tab, defaulting to "ask for edits". The Tab shows a badge, and there's an OS notification when the Tab isn't focused.
- **Tab states**: Working, Needs you, Idle, Suspended, Exited. Needs you always notifies; a finished turn in a background Tab optionally notifies.
- **Suspend**: manual per Tab (stop the `claude` process, keep the Tab, resume the conversation on return), plus optional auto-suspend after N idle minutes, off by default. The detailed rules belong to the Worktree and Agent session lifecycle ticket.

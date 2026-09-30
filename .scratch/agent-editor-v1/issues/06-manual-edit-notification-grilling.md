# How manual edits reach the Agent session

Type: grilling
Status: open
Blocked by: 02
Part of: [map](../map.md)

## Question

When the user edits a file by hand, how and when does the relevant Agent session learn about it? Settle:

- Which sessions count as "relevant": every session on that Worktree, or only the one that last touched the file?
- The trigger: on save, on an explicit "send to agent", or batched into the next prompt.
- The payload: a diff or a short note.
- What happens while the agent is mid-turn.
- Conflicts when the agent writes a file the user has open with unsaved changes.

# Window and layout prototype

Type: prototype
Status: resolved
Blocked by: none
Part of: [map](../map.md)
Prototype: branch prototype/layout, .scratch/agent-editor-v1/prototypes/layout-prototype.html (round 1 = commit 382416e, round 2 = 43b2505)

## Question

How should the agent-first layout look and behave? Build a rough, stack-independent clickable mock in plain HTML to react to. It should show:

- Tabs for Agent sessions and how they're grouped by Worktree.
- The Worktree switcher.
- The fuzzy file-search palette.
- The file tree.
- The git panel.
- Whether the manual editor opens as a **separate OS window** or as a pane/overlay.

The Agent session stays the primary focus.

## Answer

Resolved 2026-09-30 over two prototype rounds. Round 1 compared A (browser tabs), B (session sidebar) and C (board + focus). Round 2 compared three tab-strip treatments of A.

- **Primary view: A1, "two rows".**
  - Row 1 holds the **Worktrees**: branch with the `⎇` glyph, the Worktree's colour, ahead count, a mini state dot per Agent session, and a "Needs you" badge for background Worktrees. There's a **＋ worktree** button (new session in a fresh worktree).
  - Row 2 holds only the **active Worktree's Agent sessions** (`✦` + state dot + name), plus **＋ session**.
  - The active Worktree tab is raised and joins the session row in its colour.
  - Switching Worktree returns to the session last used in it.
- **Context bar** under the tabs: `⎇ branch · path · ↑↓ · N changed › ✦ session · state`, with suspend on the right. It answers "which Worktree and session am I in?" at a glance.
- **Board as a second view**, toggled with a **Tabs | Board** switch in the title bar (`Ctrl+B`; `Esc` returns to Tabs).
  - It shows columns by Agent session state (Needs you / Working / Idle / Suspended+Exited), filterable by Worktree.
  - Permissions can be approved or denied from a card, and clicking a card opens that session's Tab.
  - A title-bar "N need you" button jumps to the Board.
- **Files and Git** live in a right-edge drawer toggled from the title bar, scoped to the active Worktree.
- **Manual editor opens as a pane** inside the main window, split to the right of the chat (with its own file tabs), between the chat and the Files/Git drawer. A **pop-out button** moves a file into a separate OS window on demand. This keeps the default to one window, and one WebKit process on Linux.
- `Ctrl+P` fuzzy file search, scoped to the active Worktree, also offers "New Agent session in a fresh worktree / in this worktree".

# 03: Permission cards and permission modes

**What to build:** When the Agent requests permission, the session becomes **Needs you** and the chat shows an inline card with Allow / Always allow / Deny, also answerable with `Y`/`N`. Edit requests show a static highlighted diff *before* approval. Each Tab has a permission-mode selector (Ask for edits / Accept edits / Plan), defaulting to Ask for edits.

**Blocked by:** 01 (Walking skeleton).

**Status:** ready-for-agent

- [ ] A permission request moves the session to Needs you and renders a card with tool, target and (for edits) a diff.
- [ ] Allow, Always allow and Deny are sent back through ACP, and the session continues or stops accordingly.
- [ ] `Y`/`N` answer the pending card when the Tab is focused and no text input has focus.
- [ ] The permission mode can be changed per Tab and is applied to the session.
- [ ] Core tests: fake agent requests permission → state is Needs you → answer → state returns to Working; "always allow" suppresses the next matching request.

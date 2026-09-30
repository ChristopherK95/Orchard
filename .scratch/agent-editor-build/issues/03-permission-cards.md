# 03: Permission cards and permission modes

**What to build:** When the Agent requests permission, the session becomes **Needs you** and the chat shows an inline card with the Agent's own options by name. The real adapter's options vary per tool, e.g. "Yes" / "No", or several "Yes, and don't ask again for …". `Y` picks allow-once and `N` picks reject. Edit requests show a static diff *before* approval: +/− coloured here, with syntax highlighting arriving with ticket 02's highlighter. Several open questions are queued and each is answered by its own card. Each Tab has a permission-mode selector (Ask for edits / Accept edits / Plan), defaulting to Ask for edits.

**Blocked by:** 01 (Walking skeleton).

**Status:** ready-for-agent

- [ ] A permission request moves the session to Needs you and renders a card with tool, target and (for edits) a diff.
- [ ] The chosen option (allow once, an allow-always option, or reject) is sent back through ACP, and the session continues or stops accordingly. "Always allow" is the adapter's `allow_always` option; remembering the rule is the Agent's (Claude Code's) job.
- [ ] `Y`/`N` answer the oldest open card when the Tab is focused and no text input has focus.
- [ ] The permission mode can be changed per Tab and is applied to the session.
- [ ] Core tests: fake agent requests permission → state is Needs you → answer → state returns to Working; each option kind sends its own optionId; two questions in one turn are answered separately; a question open when the turn ends, or when the Agent dies, is cancelled and the state stays correct.

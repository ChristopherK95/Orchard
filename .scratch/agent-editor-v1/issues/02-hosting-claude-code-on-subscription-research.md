# Hosting Claude Code sessions on a subscription

Type: research
Status: open
Blocked by: none
Part of: [map](../map.md)

## Question

What are the viable ways for a third-party editor to host many concurrent **Claude Code** Agent sessions **authenticated with a Claude Pro/Max subscription** (not an API key), and what does each allow? Cover:

- **Options**: the interactive `claude` CLI inside a PTY; headless `claude -p` with `--input-format` / `--output-format stream-json`; the Claude Agent SDK (TS/Python), and whether subscription auth is technically possible *and permitted* by Anthropic's terms for this use; ACP (Agent Client Protocol) adapters as used by Zed; Claude Code's IDE integration (the protocol the VS Code/JetBrains extensions use via `/ide`, lockfiles under `~/.claude/ide/`, MCP tools such as openDiff / selection / diagnostics).
- **Telling an Agent session about manual edits**: which mechanisms exist (IDE-integration notifications, hooks such as UserPromptSubmit / PostToolUse, injecting a message, Claude Code's own file-change detection), and whether they work mid-turn.
- **Per-session cost**: process count and memory per concurrent session.
- **Session resume**: `--resume` / `--continue`, session ids, where transcripts live.
- **Permission prompts and tool approvals**: how each hosting option surfaces them to a host UI.
- **Cross-platform**: any differences between Windows 11 and Linux.

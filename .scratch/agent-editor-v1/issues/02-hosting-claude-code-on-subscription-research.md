# Hosting Claude Code sessions on a subscription

Type: research
Status: resolved
Blocked by: none
Part of: [map](../map.md)
Research: branch research/hosting-claude-code, .scratch/agent-editor-v1/research/hosting-claude-code.md

## Question

What are the viable ways for a third-party editor to host many concurrent **Claude Code** Agent sessions **authenticated with a Claude Pro/Max subscription** (not an API key), and what does each allow? Cover:

- **Options**: the interactive `claude` CLI inside a PTY; headless `claude -p` with `--input-format` / `--output-format stream-json`; the Claude Agent SDK (TS/Python), and whether subscription auth is technically possible *and permitted* by Anthropic's terms for this use; ACP (Agent Client Protocol) adapters as used by Zed; Claude Code's IDE integration (the protocol the VS Code/JetBrains extensions use via `/ide`, lockfiles under `~/.claude/ide/`, MCP tools such as openDiff / selection / diagnostics).
- **Telling an Agent session about manual edits**: which mechanisms exist (IDE-integration notifications, hooks such as UserPromptSubmit / PostToolUse, injecting a message, Claude Code's own file-change detection), and whether they work mid-turn.
- **Per-session cost**: process count and memory per concurrent session.
- **Session resume**: `--resume` / `--continue`, session ids, where transcripts live.
- **Permission prompts and tool approvals**: how each hosting option surfaces them to a host UI.
- **Cross-platform**: any differences between Windows 11 and Linux.

## Answer

- **Permitted**: the only option the terms name explicitly is the **unmodified interactive `claude` binary, signed in with the user's own `/login`**. The legal page says it does not prevent "an end user from signing in to the unmodified Claude Code binary with their own Claude subscription, including where a platform hosts Claude Code". Calling the API directly with subscription tokens is prohibited.
- **Agent SDK, `claude -p` stream-json, and ACP (which is built on the SDK)** all work technically on a subscription, and the Help Center says they "still draw from your subscription's usage limits". The paused 2026-06-15 plan would have moved them to a separate credit. But the SDK docs say third parties may not "offer claude.ai login … for their products … unless previously approved". No source says whether that covers a single-user personal tool.
- **Structured events**: `-p`/SDK/ACP give NDJSON or typed events plus host-handled permission prompts (`canUseTool`, `--permission-prompt-tool`, ACP `request_permission`). A PTY gives only a TUI, plus side channels: hooks (HTTP), the IDE-integration WebSocket (`~/.claude/ide/<port>.lock`, `openDiff` blocks for accept or reject), transcript JSONL, and `claude agents --json`.
- **Manual edits**: Claude Code already attaches a diff for files it has fully read that changed on disk (undocumented `changed_files`). Hook `additionalContext` reaches Claude mid-turn through PostToolUse and at prompt time through UserPromptSubmit. Queued or streamed messages land after the current tool calls. Channels (research preview, dev flag) push text in.
- **Cost, resume, platform**: one `claude` process per session in every option (observed about 215-340 MB working set; docs say "1 GiB… floor" under load). Resume uses `--session-id`/`--resume`, with JSONL in `~/.claude/projects/`. Windows needs ConPTY and uses Git Bash or PowerShell, and native Windows has no Bash sandbox.


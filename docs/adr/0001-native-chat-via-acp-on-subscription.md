# Native chat UI via ACP on a Claude subscription, accepting the terms risk

Status: accepted (2026-09-30)

Agent session Tabs render a native chat UI, driven through the ACP adapter (`@agentclientprotocol/claude-agent-acp`: one shared Node process, one `claude` process per session), authenticated with the user's own Claude Pro/Max subscription rather than an API key. Anthropic's terms explicitly permit only the unmodified interactive `claude` TUI on a subscription; the Agent SDK docs say third parties may not offer claude.ai login "unless previously approved", and nothing addresses a personal, undistributed tool. We knowingly accept that grey zone for a personal daily driver: if Anthropic enforces or it breaks, we fix it then (e.g. switch to an API key). ACP was chosen over the raw Agent SDK and over speaking `claude -p` stream-json directly because it is a published protocol, needs the least integration code, tracks Claude Code changes for us, and is also the boundary for adding other Agents (Gemini CLI, Codex) later.

## Considered Options

- **Interactive `claude` TUI in an embedded terminal, with hooks and IDE integration as side channels.** Clearly permitted, but gives up the native chat UI.
- **Agent SDK (TS) directly.** Official, but it's another Node sidecar and is Claude-specific.
- **Raw `claude -p` stream-json from the core.** No sidecar, but means re-implementing an undocumented control protocol and tracking the planned `--bare` default (which drops subscription auth).
- **API key.** Fully permitted, but pay-per-token instead of the subscription.

## Consequences

- The UI stack must render rich chat (markdown, code, diff cards) and speak JSON-RPC to a Node process.
- The plan depends on a third-party adapter keeping pace with Claude Code.
- There is no terminal escape hatch in v1. If the route breaks, sessions are unusable until fixed.

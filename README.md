# Agent Editor

An agent-first source-code editor: Claude Code sessions are the primary surface, across git
Worktrees. Planning lives in `.scratch/` (the v1 spec is `.scratch/agent-editor-v1/spec.md`),
decisions in `docs/adr/`, and vocabulary in `CONTEXT.md`.

## Prerequisites (Arch Linux and Windows 11)

- Rust stable (`rustup`)
- Node.js ≥ 22.12 and pnpm
- git ≥ 2.55
- Tauri 2 system dependencies: `webkit2gtk-4.1` and friends on Arch; the MSVC build tools on
  Windows (WebView2 ships with Windows 11)
- Claude Code, logged in with your own subscription (`claude /login`)

The editor checks git and Node at startup and says what to install if either is missing or too old.

## Run

```sh
pnpm install
pnpm dev                 # opens the editor
```

Set `AGENT_EDITOR_WORKSPACE` to a repository path to pre-fill the "Open a repository" field.

Set `AGENT_EDITOR_ACP_ADAPTER` to another ACP agent executable to use it instead of the pinned
`claude-agent-acp`. For example, point it at `target/debug/fake-acp-agent` to click around without
spending subscription usage.

## Test

```sh
cargo test --workspace                                            # core tests against the fake ACP agent
cargo test -p editor-core --test real_adapter -- --ignored        # handshake with the real adapter
pnpm typecheck
```

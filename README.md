# Orchard

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

Set `ORCHARD_WORKSPACE` to a repository path to pre-fill the "Open a repository" field.

Set `ORCHARD_ACP_ADAPTER` to another ACP agent executable to use it instead of the pinned
`claude-agent-acp`. For example, point it at `target/debug/fake-acp-agent` to click around without
spending subscription usage.

## Install

Installers come from the **Installers** GitHub workflow: push a `v*` tag (e.g. `v0.1.0`) to get a
draft release with all of them, or run the workflow by hand to get them as workflow artifacts.

- **Windows:** `Orchard_<version>_x64-setup.exe` (installs for the current user, no admin) or the
  `.msi`.
- **Linux:** the `.deb` (Debian/Ubuntu), the `.rpm` (Fedora), or the `.AppImage` (anything, Arch
  included: `chmod +x` and run it).

The installed app still needs git ≥ 2.55, Node.js ≥ 22.12 (with npm) and a logged-in Claude Code.
On its first run it fetches the pinned `claude-agent-acp` with npm into its data folder.

To build the installers for the machine you're on: `pnpm tauri build` (output in
`target/release/bundle/`).

## Test

```sh
cargo test --workspace                                            # core tests against the fake ACP agent
cargo test -p editor-core --test real_adapter -- --ignored        # handshake with the real adapter
pnpm typecheck
```

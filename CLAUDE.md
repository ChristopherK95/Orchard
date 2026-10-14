# Orchard

An agent-first source-code editor: Claude Code sessions (driven over ACP) are the primary
surface, across git Worktrees. Tauri 2 desktop app, Rust core + SolidJS frontend, targeting Arch
Linux and Windows 11.

- `crates/editor-core/` — the Rust core. Owns all state and processes: Agent sessions, the ACP
  adapter, git, Worktrees, settings, terminals (ADR 0003). Most logic and all tests live here.
- `src-tauri/` — thin Tauri shell: `#[tauri::command]` wrappers over editor-core in `main.rs`,
  registered in `generate_handler!`.
- `src/` — SolidJS frontend. It only renders. `src/core.ts` is the typed IPC layer; its types
  mirror editor-core's serde types.
- `CONTEXT.md` — the domain vocabulary (Workspace, Worktree, Agent session, Tab, Base, …).
- `docs/adr/` — architecture decisions. `.scratch/` — specs and tickets.

## Setup and run

Needs Rust stable, Node ≥ 22.12 + pnpm, git ≥ 2.55, and (to run the app) the Tauri 2 system
deps (`webkit2gtk-4.1` on Linux).

```sh
pnpm install
pnpm dev          # runs the app (tauri dev)
```

`ORCHARD_ACP_ADAPTER=target/debug/fake-acp-agent` runs the app against the scripted fake agent
instead of real Claude Code.

## Test and check

```sh
cargo test -p editor-core    # core tests, against the fake ACP agent (no Claude needed)
cargo test --workspace       # also builds src-tauri; needs the Tauri system deps
pnpm typecheck               # tsc --noEmit for the frontend
```

`tests/real_adapter.rs` is `#[ignore]`d — it needs a logged-in Claude; don't rely on it.
Run `cargo test -p editor-core` and `pnpm typecheck` before every commit.

## Conventions

- **Logic goes in editor-core, not the frontend.** A feature usually means: core code + an
  integration test in `crates/editor-core/tests/` (shared helpers in `tests/support/`), a
  command in `src-tauri/src/main.rs`, the matching types/calls in `src/core.ts`, then the UI.
  Keep `core.ts` types in sync with the Rust serde types by hand.
- **Git only via the `git` CLI** (ADR 0004), through editor-core's git interface. No libgit2 or
  gitoxide.
- **Use the `CONTEXT.md` vocabulary** in code, comments, UI text and commit messages, and avoid
  the listed `_Avoid_` synonyms. A new domain concept gets a `CONTEXT.md` entry.
- **Cross-platform:** everything must work on Linux and Windows; gate platform code with
  `cfg(unix)` / `cfg(windows)`.
- **Memory matters** (ADR 0002): don't add heavy dependencies or keep large state in the webview.
- **Comments** are sparse, full sentences, and say *why*; new dependencies get a one-line
  comment in `Cargo.toml` saying what they're for.
- **Commits:** subject is `Area: what the user now sees, in plain words` (e.g. `Composer: tokens
  used and cost shown beside the context meter`), optional wrapped body explaining the change.
  One feature per PR.
- Don't bump versions (`Cargo.toml`, `package.json`, `src-tauri/tauri.conf.json`); releases do
  that separately.

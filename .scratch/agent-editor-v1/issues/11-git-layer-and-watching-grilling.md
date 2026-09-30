# Git layer and file watching under Tauri

Type: grilling
Status: resolved
Blocked by: none
Part of: [map](../map.md)

## Question

Given the git-layer research and a Rust core (ADR 0002), where each Worktree's watcher, git status cache and file index live in a per-Worktree actor that starts when the Worktree is shown and stops when it's dimmed and idle (ADR 0003), how does v1 do git, watching, and file search? Settle:

- `git` CLI vs git2 vs gix, or a mix: e.g. the CLI for worktree operations, push and status (fsmonitor, auth parity), and a library for hot read paths?
- The minimum git version to require.
- `GIT_OPTIONAL_LOCKS=0` for background refreshes.
- The watching strategy across many Worktrees: `notify` per Worktree, ignore filtering, inotify limit handling, and overflow rescans on Windows.
- Operations the lifecycle decision needs:
  - live discovery of worktrees created outside the editor;
  - `fetch` before creating a Worktree from `origin/<default>`;
  - the pre-removal checks (dirty, unpushed/unmerged commits, is the branch merged);
  - reading the `origin` URL to key per-repo settings.
- The file listing and fuzzy matcher for search (`git ls-files` or the `ignore` crate; `nucleo` vs `frizbee`).

## Answer

Resolved 2026-09-30. Recorded as [ADR 0004](../../../docs/adr/0004-git-cli-only.md).

- **Backend: the `git` command only.** Every operation runs `git` as a subprocess using machine-readable output (`--porcelain=v2 -z`, `worktree list --porcelain -z`, `diff --name-status -z`), behind one internal interface so a library could replace hot paths later if measurements demand it.
  - Background calls set `GIT_OPTIONAL_LOCKS=0`. Every call sets `GIT_TERMINAL_PROMPT=0`.
  - Nothing git-related stays in memory between calls.
- **Minimum git: 2.55**, checked at startup with a clear message.
- **Repo git config is left alone.** The editor doesn't enable `core.fsmonitor` or `core.untrackedCache`; it uses whatever the user has set.
- **Watching** (per Worktree actor, only while the Worktree is shown):
  - **Linux**: the `ignore` walker adds non-recursive inotify watches for **non-ignored directories only**, and adds new directories as they appear.
  - **Windows**: one recursive watch per Worktree root. Ignored-path events are dropped, and a full rescan runs on overflow.
  - Both also watch git metadata (HEAD, index, refs) and refresh status 200 ms after the last event.
  - **Worktree discovery**: watch `.git/worktrees/`, then re-run `git worktree list`; also refresh when the window regains focus.
  - **Watch-limit exhaustion**: that Worktree falls back to 5 s status polling, with a toast giving the `sysctl` fix.
  - `notify` on its current stable 8.x, moving to 9 when it's stable.
- **Auth**: rely on the user's existing credential helpers (Git Credential Manager on Windows), SSH agent and `~/.ssh/config`. If git needs input, the operation fails with the error plus "run `git push` in a terminal once". No built-in prompt in v1.
- **File search**: an in-memory file index per Worktree, built with the `ignore` walker and kept current by watcher events, matched with **frizbee** (typo tolerance on, ranked below exact matches).
- **Lifecycle operations**:
  - `git fetch` before creating from `origin/<default>`;
  - removal checks via `status` (dirty) and `rev-list`/`branch --merged` (unpushed/unmerged, merged);
  - per-repo settings keyed by `git remote get-url origin`.
- **Branch-diff view is in v1**: a **"Changes vs base"** mode in the Git drawer lists `git diff --name-status <base>...HEAD`, with the base defaulting to `origin/<default>` and changeable. Clicking a file opens its diff in the Manual editor pane, reusing the conflict banner's diff view. With "check out existing branch → new Worktree", this is the manual review flow (PR list still out of scope).

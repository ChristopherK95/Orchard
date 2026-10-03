# 31: Settings page

**What to build:** A settings page instead of only the hand-edited file. It's an overlay over the Workspace with sections down the left: **Application** settings (General, Agents) that apply everywhere, and **Repository: \<name\>**, the open repo's settings only. It still edits `settings.toml`, keeping the file's comments and layout, so the file still works and "Open settings file" stays.

**Blocked by:** 08 (settings file and Worktree setup).

**Status:** done (Windows, type-checked and core-tested; not run in the app; Arch pending)

- [x] The ⚙ button and Ctrl+, open the page; the setup screen's "Repo settings" link opens it at the repository section. Esc (or a click outside) closes it.
- [x] General: Vim keybindings in the Manual editor; notify when a Tab you're not looking at finishes its turn.
- [x] Agents: the memory limit in MB (0: no limit); suspend Idle sessions, and after how many minutes (the minutes are enabled only while it's on).
- [x] Repository: the setup commands (add, remove, reorder) and the Windows shell (PowerShell / Git Bash). "Different commands on Windows / Linux" shows those two lists, and starts open when the file has either.
- [x] Only the open repo's section is shown. It's the section "Repo settings" finds (by `origin` URL, else by path); if there's none, the first change adds it.
- [x] Toggles apply at once; text and number fields apply when they lose focus or on Enter. Each change is written to `settings.toml` straight away, keeping the file's comments and layout.
- [x] A change to the file by hand shows on the page.
- [x] While the file doesn't parse, the page shows the error and an "Open settings file" button, and its controls are disabled (nothing is written over the broken file).
- [x] "Open settings file" opens the file in the Manual editor, as "Repo settings" does today.

Notes:
- Decided (2026-10-03): this replaces the design brief's "no settings UI in v1". The file stays the storage; edits go through `toml_edit`. A change that wouldn't leave the file valid is refused rather than written.
- Styling: existing modal, form and segmented-control styles. No design pass.

**Notes (done):**
- Core: `SettingChange` (one typed change per control) and `Core::change_setting`, which edits the file with `toml_edit` and returns the settings now in force. A change that would leave the file invalid is refused, as is any change while it doesn't parse (`SettingsInvalid`). A repo change goes in the section "Repo settings" finds (`Settings::repo_entry`); if there's none, it's added the same way, comments and all. `Core::repo_settings()` is the open repo's section, or its defaults. An existing key keeps its place and its trailing comment. Tests: `settings.rs` unit tests and `tests/settings.rs`.
- Frontend: `SettingsPage.tsx`. A broken file disables the controls through a disabled `<fieldset>`. The repo section reloads whenever the settings change, whether the page changed them or they were changed by hand.
- An empty Windows or Linux list removes that override, so the shared list applies there again. An explicitly empty override (no setup on that OS) can still be written in the file.
- The error banner above the Tabs ("Settings not applied…") still opens the file itself, since that's where the fix is.
- The design brief's §6.22 has a note that this supersedes "no settings UI".

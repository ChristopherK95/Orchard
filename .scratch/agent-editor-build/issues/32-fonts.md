# 32: Fonts

**What to build:** An **Appearance** section in the settings page's Application settings: the interface font, the code font (the Manual editor, chat code blocks, paths and every other monospace text), the Manual editor's font size, and font ligatures. Stored as `[appearance]` in `settings.toml`.

**Blocked by:** 31 (settings page).

**Status:** done (Windows, type-checked and core-tested; not run in the app; Arch pending)

- [x] "Interface font" and "Code font" are each picked from the fonts installed on the machine, in a list you can filter by typing. "Default (Inter)" / "Default (JetBrains Mono)" comes first; the code font's list puts monospace fonts first and marks them.
- [x] A choice applies at once to the main window and popped-out editor windows, and to the startup screen on the next launch.
- [x] A font named in the file that isn't installed shows as "Not installed", and the default is used.
- [x] "Code font size": the Manual editor's font size in px, 9–28, 13 by default. Other monospace text keeps the size the layout gives it.
- [x] "Font ligatures" (on by default) turns ligatures on or off wherever code is shown: the Manual editor, diffs and chat code.
- [x] The installed fonts are found by the core (a font-scanning library, off the UI thread), once per run.

Notes:
- Decided (2026-10-03): both fonts are changeable; only the editor's size; no whole-UI zoom yet (a later setting); the section is Appearance, where a theme would go later.

**Notes (done):**
- Core: `[appearance]` (`Appearance`: `ui_font`, `code_font`, `code_font_size`, `ligatures`), four more `SettingChange`s (the size is clamped to 9–28 on write), and `Core::installed_fonts()`. It uses `fontdb` on a blocking thread, once per run (a `OnceCell`). A new settings file starts with a commented `[appearance]` section.
- Frontend: `appearance.ts` sets `--user-ui`, `--user-mono`, `--code-size` and `--code-ligatures` on the root. styles.css puts the chosen font in front of the bundled one, which still stands in for a font that isn't installed. It's applied in the main window (from startup, then on every settings change) and in popped-out windows.
- The font picker shows each name in its own font. Esc closes the list, not the settings page.
- A size outside 9–28 typed into the file is shown as written in the field; the editor uses the nearest size in range, and the page only writes sizes in range.

//! The hand-edited TOML settings file (ticket 08), in the app's config folder. It's reloaded when
//! saved; a file that doesn't parse is reported and the last good settings stay in force. Unknown
//! keys are errors too, so a typo (say `setup_windwos`) doesn't silently do nothing.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::Duration;

use notify::{RecursiveMode, Watcher};
use serde::{Deserialize, Serialize};

use crate::worktrees;

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields, rename_all(serialize = "camelCase"))]
pub struct Settings {
    pub appearance: Appearance,
    pub editor: EditorSettings,
    pub notifications: Notifications,
    pub agents: AgentSettings,
    /// Per-repo settings, keyed by the repo's `origin` URL or its main checkout's path.
    pub repos: BTreeMap<String, RepoSettings>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields, rename_all(serialize = "camelCase"))]
pub struct Notifications {
    /// Notify when a Tab you're not looking at finishes its turn (off by default).
    pub turn_finished: bool,
}

/// Fonts (ticket 32).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields, rename_all(serialize = "camelCase"))]
pub struct Appearance {
    /// The interface font's family (none: Geist, which comes with the editor).
    pub ui_font: Option<String>,
    /// The code font's family, for all monospace text (none: Geist Mono, which comes with it).
    pub code_font: Option<String>,
    /// The Manual editor's font size in px (`CODE_FONT_SIZES`).
    pub code_font_size: u32,
    /// Ligatures (`=>`, `!=` drawn as one symbol) where code is shown.
    pub ligatures: bool,
}

/// The code font sizes allowed (others in the file are taken as the nearest).
pub const CODE_FONT_SIZES: std::ops::RangeInclusive<u32> = 9..=28;

impl Default for Appearance {
    fn default() -> Self {
        Self {
            ui_font: None,
            code_font: None,
            code_font_size: 13,
            ligatures: true,
        }
    }
}

/// The Manual editor (ticket 14).
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields, rename_all(serialize = "camelCase"))]
pub struct EditorSettings {
    /// Vim keybindings (`:w` saves, `:q` closes the file tab); off by default.
    pub vim: bool,
}

/// Limits on the Agents' processes (ticket 12).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields, rename_all(serialize = "camelCase"))]
pub struct AgentSettings {
    /// When the `claude` processes together use more than this many MiB, the longest-Idle
    /// sessions are Suspended until they're under it (0: no limit).
    pub memory_limit_mb: u64,
    /// Suspend sessions Idle longer than `idle_suspend_minutes` (off by default).
    pub idle_suspend: bool,
    /// How long a session may be Idle before idle auto-suspend takes it (0: never).
    pub idle_suspend_minutes: u64,
}

impl Default for AgentSettings {
    fn default() -> Self {
        Self {
            memory_limit_mb: 4096,
            idle_suspend: false,
            idle_suspend_minutes: 30,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields, rename_all(serialize = "camelCase"))]
pub struct RepoSettings {
    /// Worktree setup: commands run in order in each new Worktree before its first Agent session.
    pub setup: Vec<String>,
    /// Replaces `setup` on Windows.
    pub setup_windows: Option<Vec<String>>,
    /// Replaces `setup` on Linux.
    pub setup_linux: Option<Vec<String>>,
    /// The shell setup and the Terminal panel run on Windows (on Linux, setup uses `bash` and the
    /// terminal the user's `$SHELL`).
    pub windows_shell: WindowsShell,
    /// Actions: named commands the user runs on demand, each in a shell of its own.
    pub actions: Vec<Action>,
}

/// A named set of commands run on demand in a Worktree (a dev server, say), each command typed
/// into a new shell of the Terminal panel, so its output stays in view and it can be stopped,
/// rerun or restarted.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields, rename_all(serialize = "camelCase"))]
pub struct Action {
    pub name: String,
    /// One shell per command, in the Worktree's folder.
    pub run: Vec<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum WindowsShell {
    /// `pwsh` if it's installed, else Windows PowerShell.
    #[default]
    Powershell,
    GitBash,
}

impl RepoSettings {
    /// The setup commands for this OS: its override if there is one, else the shared list.
    pub fn setup_commands(&self) -> &[String] {
        let override_ = if cfg!(windows) {
            &self.setup_windows
        } else {
            &self.setup_linux
        };
        override_.as_deref().unwrap_or(&self.setup)
    }
}

impl Settings {
    /// The settings for a repo: the section keyed by its `origin` URL, else by its main checkout.
    pub fn repo(&self, origin: Option<&str>, root: &Path) -> Option<&RepoSettings> {
        self.repo_entry(origin, root).map(|(_, repo)| repo)
    }

    /// The repo's section (`repo`), with the key it's under in the file.
    pub(crate) fn repo_entry(
        &self,
        origin: Option<&str>,
        root: &Path,
    ) -> Option<(&String, &RepoSettings)> {
        let by_origin =
            origin.and_then(|origin| self.repos.iter().find(|(key, _)| same_url(key, origin)));
        by_origin.or_else(|| self.repos.iter().find(|(key, _)| same_path(key, root)))
    }
}

/// One change the settings page makes. Repo changes are to the open Workspace's section.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum SettingChange {
    /// `None`: the default font.
    UiFont {
        family: Option<String>,
    },
    CodeFont {
        family: Option<String>,
    },
    CodeFontSize {
        px: u32,
    },
    Ligatures {
        on: bool,
    },
    Vim {
        on: bool,
    },
    TurnFinished {
        on: bool,
    },
    MemoryLimitMb {
        mb: u64,
    },
    IdleSuspend {
        on: bool,
    },
    IdleSuspendMinutes {
        minutes: u64,
    },
    Setup {
        commands: Vec<String>,
    },
    /// `None` takes the Windows override away (`setup` applies there again).
    SetupWindows {
        commands: Option<Vec<String>>,
    },
    SetupLinux {
        commands: Option<Vec<String>>,
    },
    WindowsShell {
        shell: WindowsShell,
    },
    /// The whole list (an empty one removes the key).
    Actions {
        actions: Vec<Action>,
    },
}

impl SettingChange {
    pub(crate) fn is_repo(&self) -> bool {
        matches!(
            self,
            Self::Setup { .. }
                | Self::SetupWindows { .. }
                | Self::SetupLinux { .. }
                | Self::WindowsShell { .. }
                | Self::Actions { .. }
        )
    }

    /// The table it's in (`repo`: the repo's key) and the key it sets; `None` removes the key.
    fn target(&self, repo: Option<&str>) -> (Vec<String>, &'static str, Option<toml_edit::Item>) {
        use toml_edit::value;
        let list = |commands: &[String]| value(commands.iter().collect::<toml_edit::Array>());
        let app = |table: &str| vec![table.to_owned()];
        let repo = || vec!["repos".to_owned(), repo.unwrap_or_default().to_owned()];
        let font = |family: &Option<String>| {
            family
                .as_deref()
                .map(str::trim)
                .filter(|f| !f.is_empty())
                .map(value)
        };
        match self {
            Self::UiFont { family } => (app("appearance"), "ui_font", font(family)),
            Self::CodeFont { family } => (app("appearance"), "code_font", font(family)),
            Self::CodeFontSize { px } => (
                app("appearance"),
                "code_font_size",
                Some(value(
                    (*px).clamp(*CODE_FONT_SIZES.start(), *CODE_FONT_SIZES.end()) as i64,
                )),
            ),
            Self::Ligatures { on } => (app("appearance"), "ligatures", Some(value(*on))),
            Self::Vim { on } => (app("editor"), "vim", Some(value(*on))),
            Self::TurnFinished { on } => (app("notifications"), "turn_finished", Some(value(*on))),
            Self::MemoryLimitMb { mb } => {
                (app("agents"), "memory_limit_mb", Some(value(*mb as i64)))
            }
            Self::IdleSuspend { on } => (app("agents"), "idle_suspend", Some(value(*on))),
            Self::IdleSuspendMinutes { minutes } => (
                app("agents"),
                "idle_suspend_minutes",
                Some(value(*minutes as i64)),
            ),
            Self::Setup { commands } => (repo(), "setup", Some(list(commands))),
            Self::SetupWindows { commands } => {
                (repo(), "setup_windows", commands.as_deref().map(list))
            }
            Self::SetupLinux { commands } => (repo(), "setup_linux", commands.as_deref().map(list)),
            Self::WindowsShell { shell } => (
                repo(),
                "windows_shell",
                Some(value(match shell {
                    WindowsShell::Powershell => "powershell",
                    WindowsShell::GitBash => "git-bash",
                })),
            ),
            Self::Actions { actions } => {
                let mut actions: toml_edit::Array = actions
                    .iter()
                    .filter(|a| !a.name.trim().is_empty())
                    .map(|a| {
                        let mut table = toml_edit::InlineTable::new();
                        table.insert("name", a.name.trim().into());
                        let run: toml_edit::Array = a
                            .run
                            .iter()
                            .map(|c| c.trim())
                            .filter(|c| !c.is_empty())
                            .collect();
                        table.insert("run", run.into());
                        table
                    })
                    .collect();
                // (One per line.)
                for action in actions.iter_mut() {
                    action.decor_mut().set_prefix("\n  ");
                }
                actions.set_trailing("\n");
                actions.set_trailing_comma(true);
                (
                    repo(),
                    "actions",
                    (!actions.is_empty()).then(|| value(actions)),
                )
            }
        }
    }
}

/// Makes `change` in the file (creating it if need be), keeping its comments and layout. Refused,
/// and nothing written, if the file doesn't parse or the change wouldn't leave valid settings.
/// `repo` is the key of the repo's section, for repo changes.
pub(crate) fn change(
    path: &Path,
    repo: Option<&str>,
    change: &SettingChange,
) -> Result<(), String> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => FILE_HEADER.to_owned(),
        Err(err) => return Err(format!("couldn't read {}: {err}", path.display())),
    };
    let mut doc: toml_edit::DocumentMut = text
        .parse()
        .map_err(|err| format!("{} isn't valid: {err}", path.display()))?;
    let (tables, key, item) = change.target(repo);
    let mut table: &mut dyn toml_edit::TableLike = doc.as_table_mut();
    for (depth, name) in tables.iter().enumerate() {
        if table.get(name).is_none() {
            let mut new = toml_edit::Table::new();
            // (`[repos."…"]` alone, with no bare `[repos]` header above it.)
            new.set_implicit(depth + 1 < tables.len());
            table.insert(name, toml_edit::Item::Table(new));
        }
        table = table
            .get_mut(name)
            .and_then(|item| item.as_table_like_mut())
            .ok_or_else(|| format!("`{name}` in {} isn't a table", path.display()))?;
    }
    match item {
        // (Kept where it is, with its comments, if it's there already.)
        Some(item) => match table.get_mut(key) {
            Some(existing) if existing.is_value() && item.is_value() => {
                let decor = existing.as_value().map(|v| v.decor().clone());
                *existing = item;
                if let (Some(decor), Some(value)) = (decor, existing.as_value_mut()) {
                    *value.decor_mut() = decor;
                }
            }
            _ => {
                table.insert(key, item);
            }
        },
        None => {
            table.remove(key);
        }
    }
    let text = doc.to_string();
    toml::from_str::<Settings>(&text)
        .map_err(|err| format!("the change would leave {} invalid: {err}", path.display()))?;
    if let Some(folder) = path.parent() {
        std::fs::create_dir_all(folder).map_err(|e| e.to_string())?;
    }
    std::fs::write(path, text).map_err(|e| format!("couldn't write {}: {e}", path.display()))
}

/// The settings in force, and why the file on disk isn't them (if it doesn't parse).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LoadedSettings {
    pub settings: Settings,
    pub error: Option<String>,
}

/// The same repo, whichever way its URL is written: `https://github.com/me/app`,
/// `git@github.com:me/app.git` and `ssh://git@GitHub.com/me/app/` all match.
fn same_url(a: &str, b: &str) -> bool {
    url_key(a) == url_key(b)
}

/// `host/path` with the host in lower case, without scheme, user, `.git` or a trailing slash.
fn url_key(url: &str) -> String {
    let url = url.trim().trim_end_matches('/');
    let url = url.strip_suffix(".git").unwrap_or(url);
    let (rest, scp_like) = match url.split_once("://") {
        Some((_, rest)) => (rest, false),
        None => (url, true),
    };
    // A user (`git@`) comes before the host, ahead of any `:` or `/`.
    let rest = match rest.split_once('@') {
        Some((user, host)) if !user.contains(['/', ':']) => host,
        _ => rest,
    };
    // scp-like `host:path` (but not a Windows drive like `C:\…`).
    let rest = match rest.split_once(':') {
        Some((host, path)) if scp_like && host.len() > 1 && !host.contains('/') => {
            format!("{host}/{path}")
        }
        _ => rest.to_owned(),
    };
    match rest.split_once('/') {
        Some((host, path)) => format!("{}/{path}", host.to_lowercase()),
        None => rest.to_lowercase(),
    }
}

fn same_path(key: &str, root: &Path) -> bool {
    let key = key.trim();
    !key.is_empty() && worktrees::normalize(PathBuf::from(key)) == root
}

/// What reading the file found: settings, a parse error, or no file.
pub(crate) enum Read {
    Parsed(Settings),
    Invalid(String),
    Missing,
}

pub(crate) fn read(path: &Path) -> Read {
    match std::fs::read_to_string(path) {
        Ok(text) => match toml::from_str(&text) {
            Ok(settings) => Read::Parsed(settings),
            Err(err) => Read::Invalid(format!("{} isn't valid: {err}", path.display())),
        },
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Read::Missing,
        Err(err) => Read::Invalid(format!("couldn't read {}: {err}", path.display())),
    }
}

/// Applies what was read on top of `current`: an invalid file keeps the settings already in force,
/// and so does a missing one (an editor saving by rename leaves a moment with no file).
pub(crate) fn apply(current: &LoadedSettings, read: Read) -> LoadedSettings {
    match read {
        Read::Parsed(settings) => LoadedSettings {
            settings,
            error: None,
        },
        Read::Invalid(error) => LoadedSettings {
            settings: current.settings.clone(),
            error: Some(error),
        },
        Read::Missing => LoadedSettings {
            settings: current.settings.clone(),
            error: None,
        },
    }
}

/// How long saves are gathered before the file is re-read (editors often write in several steps).
const SETTLE: Duration = Duration::from_millis(100);

/// Watches the settings file's folder and calls `reload` after each burst of changes to the file.
/// Runs on its own thread (no async runtime needed); dropping the returned watcher stops it.
pub(crate) fn watch(
    path: &Path,
    mut reload: impl FnMut() -> bool + Send + 'static,
) -> Option<notify::RecommendedWatcher> {
    let folder = path.parent()?.to_owned();
    let name = path.file_name()?.to_owned();
    std::fs::create_dir_all(&folder).ok()?;
    let (tx, rx) = mpsc::channel::<()>();
    let mut watcher = notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
        // An error may mean changes were lost (e.g. the OS's event buffer overflowed): re-read.
        // (Reading it, as a reload does, is no change.)
        let touches_file = event.map_or(true, |e| {
            crate::files::is_change(&e)
                && e.paths
                    .iter()
                    .any(|p| p.file_name() == Some(name.as_os_str()))
        });
        if touches_file {
            let _ = tx.send(());
        }
    })
    .ok()?;
    watcher.watch(&folder, RecursiveMode::NonRecursive).ok()?;
    std::thread::spawn(move || {
        // Ends when the watcher (and so the sender) is dropped, or `reload` says the core is gone.
        while rx.recv().is_ok() {
            while rx.recv_timeout(SETTLE).is_ok() {}
            if !reload() {
                return;
            }
        }
    });
    Some(watcher)
}

/// A new file starts with this, so the settings there are to be found.
const FILE_HEADER: &str = "\
# Orchard settings. Saved changes apply straight away.

[appearance]
# ui_font = \"Segoe UI\"     # the interface font (default: Geist)
# code_font = \"Consolas\"   # the code font, for all monospace text (default: Geist Mono)
# The Manual editor's font size in px (9-28).
code_font_size = 13
# Ligatures (=> and != drawn as one symbol) where code is shown.
ligatures = true

[editor]
# Vim keybindings in the Manual editor (:w saves, :q closes the file tab).
vim = false

[notifications]
# Notify when a Tab you're not looking at finishes its turn.
turn_finished = false

[agents]
# When the claude processes together use more than this (in MB; 0 = no limit), the
# longest-Idle sessions are Suspended until they're under it.
memory_limit_mb = 4096
# Suspend sessions Idle longer than idle_suspend_minutes (off by default).
idle_suspend = false
idle_suspend_minutes = 30
";

/// Adds a section for the repo keyed `key` to the file (creating it if need be). Appends text, so the
/// user's comments and layout are kept.
pub(crate) fn add_repo_section(path: &Path, key: &str, name: &str) -> std::io::Result<()> {
    let existing = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            if let Some(folder) = path.parent() {
                std::fs::create_dir_all(folder)?;
            }
            FILE_HEADER.to_owned()
        }
        Err(err) => return Err(err),
    };
    let separator = if existing.ends_with('\n') || existing.is_empty() {
        ""
    } else {
        "\n"
    };
    let quoted_key = toml::Value::String(key.to_owned()).to_string();
    let section = format!(
        "\n# {name}. Worktree setup runs in each new Worktree before its first Agent session, in\n\
         # order, stopping at the first failure. Anything a command leaves running in the\n\
         # background is stopped when it finishes.\n\
         [repos.{quoted_key}]\n\
         setup = []\n\
         # setup_windows = []         # replaces `setup` on Windows\n\
         # setup_linux = []           # replaces `setup` on Linux\n\
         # windows_shell = \"git-bash\" # instead of PowerShell\n"
    );
    std::fs::write(path, format!("{existing}{separator}{section}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_change_keeps_the_files_comments_and_layout() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.toml");
        let text = "# mine\n[editor]\nvim = false # keep me\n\n[agents]\nmemory_limit_mb = 4096\n";
        std::fs::write(&path, text).unwrap();
        change(&path, None, &SettingChange::Vim { on: true }).unwrap();
        change(&path, None, &SettingChange::IdleSuspend { on: true }).unwrap();
        let now = std::fs::read_to_string(&path).unwrap();
        assert!(
            now.starts_with("# mine\n[editor]\nvim = true # keep me\n"),
            "{now}"
        );
        assert!(
            now.contains("memory_limit_mb = 4096\nidle_suspend = true"),
            "{now}"
        );
        let Read::Parsed(settings) = read(&path) else {
            panic!("{now}")
        };
        assert!(settings.editor.vim && settings.agents.idle_suspend);
    }

    #[test]
    fn a_repo_change_goes_in_its_section_and_none_removes_the_key() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.toml");
        let key = "https://github.com/me/app";
        change(
            &path,
            Some(key),
            &SettingChange::Setup {
                commands: vec!["pnpm install".into()],
            },
        )
        .unwrap();
        change(
            &path,
            Some(key),
            &SettingChange::SetupWindows {
                commands: Some(vec!["pnpm i".into()]),
            },
        )
        .unwrap();
        let now = std::fs::read_to_string(&path).unwrap();
        assert!(
            now.contains("[repos.\"https://github.com/me/app\"]"),
            "{now}"
        );
        assert!(!now.contains("[repos]\n"), "{now}");
        let Read::Parsed(settings) = read(&path) else {
            panic!("{now}")
        };
        let repo = &settings.repos[key];
        assert_eq!(repo.setup, ["pnpm install"]);
        assert_eq!(
            repo.setup_windows.as_deref(),
            Some(&["pnpm i".to_owned()][..])
        );

        change(
            &path,
            Some(key),
            &SettingChange::SetupWindows { commands: None },
        )
        .unwrap();
        let Read::Parsed(settings) = read(&path) else {
            panic!()
        };
        assert_eq!(settings.repos[key].setup_windows, None);
    }

    #[test]
    fn fonts_are_set_and_unset_and_the_size_kept_in_range() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.toml");
        let family = |f: &str| Some(f.to_owned());
        change(
            &path,
            None,
            &SettingChange::CodeFont {
                family: family("Consolas"),
            },
        )
        .unwrap();
        change(&path, None, &SettingChange::CodeFontSize { px: 99 }).unwrap();
        let Read::Parsed(settings) = read(&path) else {
            panic!()
        };
        assert_eq!(settings.appearance.code_font.as_deref(), Some("Consolas"));
        assert_eq!(settings.appearance.code_font_size, 28);
        assert!(settings.appearance.ligatures, "on by default");

        change(&path, None, &SettingChange::CodeFont { family: None }).unwrap();
        let Read::Parsed(settings) = read(&path) else {
            panic!()
        };
        assert_eq!(settings.appearance.code_font, None);
    }

    #[test]
    fn a_file_that_doesnt_parse_is_left_alone() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.toml");
        std::fs::write(&path, "[editor\nvim = ").unwrap();
        assert!(change(&path, None, &SettingChange::Vim { on: true }).is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "[editor\nvim = ");
    }

    #[test]
    fn ssh_and_https_urls_of_one_repo_match() {
        assert!(same_url(
            "git@github.com:me/app.git",
            "https://github.com/me/app"
        ));
        assert!(same_url(
            "ssh://git@GitHub.com/me/app/",
            "https://github.com/me/app.git"
        ));
        assert!(same_url(r"C:\repos\origin.git", r"C:\repos\origin"));
        assert!(!same_url(
            "https://github.com/me/App",
            "https://github.com/me/app"
        ));
    }

    #[test]
    fn unknown_keys_are_errors() {
        assert!(toml::from_str::<Settings>("[repos.x]\nsetup_windwos = []").is_err());
        assert!(toml::from_str::<Settings>("[notifications]\nturn_finshed = true").is_err());
    }

    #[test]
    fn urls_match_with_or_without_dot_git_or_a_trailing_slash() {
        assert!(same_url(
            "https://github.com/me/app.git",
            "https://github.com/me/app"
        ));
        assert!(same_url(
            "https://github.com/me/app/",
            "https://github.com/me/app.git"
        ));
        assert!(!same_url(
            "https://github.com/me/app",
            "https://github.com/me/apps"
        ));
    }

    #[test]
    fn the_os_override_replaces_the_shared_list() {
        let repo: RepoSettings = toml::from_str(
            "setup = [\"shared\"]\nsetup_windows = [\"win\"]\nsetup_linux = [\"linux\"]",
        )
        .unwrap();
        let expected = if cfg!(windows) { "win" } else { "linux" };
        assert_eq!(repo.setup_commands(), [expected]);

        let shared_only: RepoSettings = toml::from_str("setup = [\"shared\"]").unwrap();
        assert_eq!(shared_only.setup_commands(), ["shared"]);
    }

    #[test]
    fn the_added_section_parses_and_keeps_what_was_there() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.toml");
        std::fs::write(&path, "# mine\n[notifications]\nturn_finished = true").unwrap();
        add_repo_section(&path, r"C:\work\app", "app").unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.starts_with("# mine\n[notifications]\nturn_finished = true\n"));
        let Read::Parsed(settings) = read(&path) else {
            panic!("doesn't parse: {text}")
        };
        assert!(settings.notifications.turn_finished);
        assert_eq!(settings.repos[r"C:\work\app"].setup, Vec::<String>::new());
    }
}

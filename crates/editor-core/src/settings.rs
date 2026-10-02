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
    /// The shell setup runs in on Windows (Linux always uses `bash`).
    pub windows_shell: WindowsShell,
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
        let by_origin = origin.and_then(|origin| {
            self.repos
                .iter()
                .find(|(key, _)| same_url(key, origin))
                .map(|(_, repo)| repo)
        });
        by_origin.or_else(|| {
            self.repos
                .iter()
                .find(|(key, _)| same_path(key, root))
                .map(|(_, repo)| repo)
        })
    }
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
        let touches_file = event.map_or(true, |e| {
            e.paths
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
# Agent editor settings. Saved changes apply straight away.

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

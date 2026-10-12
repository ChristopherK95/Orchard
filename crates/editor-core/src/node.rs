//! Which Node.js what Orchard starts runs on (the ACP adapter, and so the Agent's commands; setup
//! commands; git hooks; the Terminal panel). Started from the desktop, Orchard's own `PATH` often
//! leads to the system's `node` rather than the version manager's the user set up in their shell,
//! and an Agent then fails on a project's `engines`. So: automatically the `node` on the login
//! shell's `PATH`; or one the user picks in the settings (`[agents] node`), put first on `PATH`.

use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::sync::RwLock;

use serde::Serialize;

/// The Node picked in the settings (its executable), `None` for automatic. Kept here, not passed
/// along, as git and setup reach it from deep down; the settings file is the app's, one per run.
static CHOSEN: RwLock<Option<PathBuf>> = RwLock::new(None);

/// Called whenever the settings in force change.
pub(crate) fn choose(node: Option<&str>) {
    let node = node
        .map(str::trim)
        .filter(|n| !n.is_empty())
        .map(PathBuf::from);
    *CHOSEN.write().expect("node lock") = node;
}

pub(crate) fn chosen() -> Option<PathBuf> {
    CHOSEN.read().expect("node lock").clone()
}

/// The `PATH` for something Orchard starts: on Unix the login shell's (falling back to Orchard's
/// own), with the chosen Node's folder first. `None`: leave it as it is.
pub(crate) async fn path() -> Option<OsString> {
    with_first(
        chosen().as_deref().and_then(Path::parent),
        login_path().await,
    )
}

/// The login shell's `PATH` on Unix, else (or if it couldn't be read) Orchard's own.
async fn login_path() -> Option<OsString> {
    #[cfg(unix)]
    let login = crate::setup::shell_env()
        .await
        .iter()
        .find(|(key, _)| key == "PATH")
        .map(|(_, value)| value.clone());
    #[cfg(not(unix))]
    let login = None;
    login.or_else(own_path)
}

/// Orchard's own `PATH`, less an AppImage's folders (`process::host_env`).
fn own_path() -> Option<OsString> {
    match crate::process::host_env()
        .iter()
        .find(|(key, _)| key == "PATH")
    {
        Some((_, fixed)) => fixed.clone(),
        None => std::env::var_os("PATH"),
    }
}

/// The `PATH` for a Terminal panel shell: Orchard's own with the chosen Node's folder first (the
/// shell's profile does the rest, and may put a version manager's ahead of it). `None`: as it is.
pub(crate) fn terminal_path() -> Option<OsString> {
    let node = chosen()?;
    with_first(node.parent(), own_path())
}

/// `path` with `first` put ahead of it (and not again further on). `None` if there's no change.
fn with_first(first: Option<&Path>, path: Option<OsString>) -> Option<OsString> {
    let Some(first) = first else { return path };
    let rest = path
        .as_deref()
        .map(|path| {
            std::env::split_paths(path)
                .filter(|dir| dir != first)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    std::env::join_paths(std::iter::once(first.to_owned()).chain(rest)).ok()
}

/// A Node.js found on this machine, for the settings page.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NodeInstall {
    /// As `node --version` says it, without the `v` (`24.1.0`).
    pub version: String,
    /// The executable.
    pub path: PathBuf,
    /// Where it comes from: `nvm`, `fnm`, `volta`, `asdf`, `mise`, `n`, or `system`.
    pub source: String,
    /// It's the one used when none is picked (first on the login shell's `PATH`).
    pub automatic: bool,
}

/// Every Node.js found: the ones on `PATH` and the ones the common version managers installed,
/// newest first.
pub async fn installed() -> Vec<NodeInstall> {
    let automatic = login_path()
        .await
        .and_then(|path| on_path(&path).into_iter().next());
    let mut candidates: Vec<PathBuf> = automatic.iter().cloned().collect();
    candidates.extend(own_path().map(|p| on_path(&p)).unwrap_or_default());
    candidates.extend(managed());

    // The same executable can be reached several ways (`/usr/bin/node` and `/bin/node`).
    let mut seen = std::collections::HashSet::new();
    candidates.retain(|path| seen.insert(std::fs::canonicalize(path).unwrap_or(path.clone())));

    let mut checks = tokio::task::JoinSet::new();
    for (order, path) in candidates.into_iter().enumerate() {
        checks.spawn(async move { (order, version(&path).await, path) });
    }
    let mut found = vec![];
    while let Some(Ok((order, Some(version), path))) = checks.join_next().await {
        found.push((order, version, path));
    }
    found.sort_by(|a, b| numbers(&b.1).cmp(&numbers(&a.1)).then(a.0.cmp(&b.0)));
    found
        .into_iter()
        .map(|(_, version, path)| NodeInstall {
            version,
            source: source(&path).into(),
            automatic: automatic.as_ref() == Some(&path),
            path,
        })
        .collect()
}

const EXE: &str = if cfg!(windows) { "node.exe" } else { "node" };

/// Every `node` on `path`, in order.
fn on_path(path: &OsStr) -> Vec<PathBuf> {
    std::env::split_paths(path)
        .map(|dir| dir.join(EXE))
        .filter(|node| node.is_file())
        .collect()
}

/// The `node`s the version managers keep, each in a folder per version.
fn managed() -> Vec<PathBuf> {
    let var = |name: &str| {
        std::env::var_os(name)
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
    };
    let home = var(if cfg!(windows) { "USERPROFILE" } else { "HOME" });
    let mut roots: Vec<(PathBuf, &str)> = vec![];
    let mut add = |root: Option<PathBuf>, inside: &'static str| {
        if let Some(root) = root {
            roots.push((root, inside));
        }
    };
    let bin = if cfg!(windows) { "" } else { "bin" };
    let fnm = if cfg!(windows) {
        "installation"
    } else {
        "installation/bin"
    };
    let home_join = |p: &str| home.as_ref().map(|h| h.join(p));
    // nvm (`$NVM_DIR`); fnm; Volta; asdf; mise; n.
    add(
        var("NVM_DIR")
            .or_else(|| home_join(".nvm"))
            .map(|d| d.join("versions/node")),
        bin,
    );
    add(var("FNM_DIR").map(|d| d.join("node-versions")), fnm);
    add(home_join(".fnm/node-versions"), fnm);
    add(
        var("VOLTA_HOME")
            .or_else(|| home_join(".volta"))
            .map(|d| d.join("tools/image/node")),
        bin,
    );
    add(
        var("ASDF_DATA_DIR")
            .or_else(|| home_join(".asdf"))
            .map(|d| d.join("installs/nodejs")),
        bin,
    );
    add(var("N_PREFIX").map(|d| d.join("n/versions/node")), bin);
    if cfg!(windows) {
        // nvm-windows keeps `v24.1.0\node.exe` in `%NVM_HOME%`.
        add(var("NVM_HOME"), "");
        add(var("APPDATA").map(|d| d.join("fnm/node-versions")), fnm);
        add(
            var("LOCALAPPDATA").map(|d| d.join("Volta/tools/image/node")),
            "",
        );
    } else {
        let data = var("XDG_DATA_HOME").or_else(|| home_join(".local/share"));
        // nvm.fish keeps `v24.1.0/bin/node` in `$XDG_DATA_HOME/nvm`.
        add(data.as_ref().map(|d| d.join("nvm")), bin);
        add(data.as_ref().map(|d| d.join("fnm/node-versions")), fnm);
        add(data.as_ref().map(|d| d.join("mise/installs/node")), bin);
        add(Some("/usr/local/n/versions/node".into()), bin);
    }
    let mut found = vec![];
    for (root, inside) in roots {
        let Ok(entries) = std::fs::read_dir(&root) else {
            continue;
        };
        for entry in entries.flatten() {
            let node = entry.path().join(inside).join(EXE);
            if node.is_file() {
                found.push(node);
            }
        }
    }
    found
}

/// What `node --version` says, without the `v`. `None` if it doesn't run.
async fn version(node: &Path) -> Option<String> {
    let run = crate::process::command(node)
        .arg("--version")
        .stdin(std::process::Stdio::null())
        .kill_on_drop(true)
        .output();
    let output = tokio::time::timeout(std::time::Duration::from_secs(5), run)
        .await
        .ok()?
        .ok()?;
    let text = String::from_utf8_lossy(&output.stdout);
    let version = text.trim().trim_start_matches('v');
    (output.status.success() && !version.is_empty()).then(|| version.to_owned())
}

/// `24.1.0` as numbers, to sort by.
fn numbers(version: &str) -> Vec<u64> {
    version
        .split('.')
        .map(|part| part.parse().unwrap_or(0))
        .collect()
}

/// Which version manager a `node` belongs to, from where it is.
fn source(node: &Path) -> &'static str {
    let path = node.to_string_lossy().replace('\\', "/").to_lowercase();
    const MANAGERS: &[(&str, &str)] = &[
        ("/fnm/", "fnm"),
        ("/.fnm/", "fnm"),
        ("/volta/", "volta"),
        ("/.volta/", "volta"),
        ("/.asdf/", "asdf"),
        ("/mise/", "mise"),
        ("/n/versions/", "n"),
        ("/nvm/", "nvm"),
        ("/.nvm/", "nvm"),
    ];
    MANAGERS
        .iter()
        .find(|(part, _)| path.contains(part))
        .map_or("system", |(_, name)| name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn the_chosen_nodes_folder_goes_first_on_path_once() {
        let path = with_first(
            Some(Path::new("/nvm/v24/bin")),
            Some("/usr/bin:/nvm/v24/bin:/bin".into()),
        );
        assert_eq!(path, Some("/nvm/v24/bin:/usr/bin:/bin".into()));
        assert_eq!(
            with_first(None, Some("/usr/bin".into())),
            Some("/usr/bin".into())
        );
        assert_eq!(
            with_first(Some(Path::new("/n/bin")), None),
            Some("/n/bin".into())
        );
    }

    #[test]
    fn versions_sort_by_number_not_text() {
        assert!(numbers("24.1.0") > numbers("22.13.1"));
        assert!(numbers("9.0.0") < numbers("10.0.0"));
    }

    #[test]
    fn a_nodes_version_manager_is_told_from_its_path() {
        assert_eq!(
            source(Path::new("/home/me/.local/share/nvm/v24.1.0/bin/node")),
            "nvm"
        );
        assert_eq!(
            source(Path::new("/home/me/.nvm/versions/node/v20/bin/node")),
            "nvm"
        );
        assert_eq!(
            source(Path::new(
                "/home/me/.local/share/fnm/node-versions/v22/installation/bin/node"
            )),
            "fnm"
        );
        assert_eq!(
            source(Path::new(
                "C:\\Users\\me\\AppData\\Local\\Volta\\tools\\image\\node\\22\\node.exe"
            )),
            "volta"
        );
        assert_eq!(source(Path::new("/usr/bin/node")), "system");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_node_is_listed_with_its_version() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let node = dir.path().join("node");
        std::fs::write(&node, "#!/bin/sh\necho v24.1.0\n").unwrap();
        std::fs::set_permissions(&node, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(version(&node).await.as_deref(), Some("24.1.0"));
        assert_eq!(on_path(dir.path().as_os_str()), [node]);
    }
}

//! Worktree actors (ticket 13): for each Worktree being shown (or with sessions), an in-memory index
//! of its files kept live by watching it, the files it has changed, and fuzzy matching for Ctrl+P.
//!
//! The index is what git doesn't ignore and that's on disk (`git ls-files`: ignore-aware, fast,
//! and through the git CLI per ADR 0004; it stands in for the spec's "ignore-aware walker").
//! Watching skips ignored folders: on Linux one non-recursive watch per indexed folder (inotify
//! watches are a limited resource), on Windows one recursive watch with events in ignored paths
//! dropped before anything else. If watching fails at the watch limit the Worktree is polled
//! instead. The Worktree's git folder and the shared refs are watched too, so its branch status
//! (ahead/behind, changed) is refreshed when a terminal commits, pushes or fetches.

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

use notify::event::{EventKind, ModifyKind};
use notify::{RecursiveMode, Watcher};
use serde::Serialize;
use tokio::sync::mpsc;

use crate::git;

/// How file watching behaves.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileWatchConfig {
    /// Test override: more watches than this count as the OS's watch limit being reached (`None`:
    /// only the OS's own limit).
    pub watch_limit: Option<usize>,
    /// How often a Worktree is re-read once watching it has failed.
    pub poll_interval: Duration,
}

impl Default for FileWatchConfig {
    fn default() -> Self {
        Self {
            watch_limit: None,
            poll_interval: Duration::from_secs(5),
        }
    }
}

/// How a Worktree's files are being kept up to date.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum WatchStatus {
    /// Watched: `watched` lists what (on Windows the Worktree recursively, plus git's folders).
    Watching { watched: Vec<PathBuf> },
    /// The watch limit was reached: re-read every few seconds.
    Polling,
}

/// The index's size and how often it's been re-read in full (a re-read is the expensive path,
/// which ignored files must never cause).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IndexStats {
    pub files: usize,
    pub full_reindexes: u64,
}

/// A Ctrl+P result: a file, and which of its characters matched (for highlighting).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileMatch {
    /// `/`-separated, relative to the Worktree.
    pub path: String,
    /// Byte offsets into `path` of the matched characters.
    pub indices: Vec<u32>,
}

/// One entry of a folder in the Files drawer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DirEntry {
    pub name: String,
    /// `/`-separated, relative to the Worktree.
    pub path: String,
    pub is_dir: bool,
    /// The file's `git status` code (`M`, `A`, `D`, `??`, …), if it's changed.
    pub change: Option<String>,
    /// A folder with changed files somewhere inside.
    pub has_changes: bool,
}

/// Something worth telling the core about.
pub(crate) enum ActorNews {
    /// The index or the changed files moved on (or the first index is ready).
    Files,
    /// The branch status may have changed (a commit, a checkout, staging, a fetch, a new file).
    Status,
    /// Watching failed; polling instead, and why.
    Fallback(String),
    /// These paths (absolute, ignored ones too) may have been written to; `None`: any of them may
    /// have (a rescan, or a poll).
    Touched(Option<Vec<PathBuf>>),
}

pub(crate) type NewsSink = Arc<dyn Fn(&Path, ActorNews) + Send + Sync>;

/// A Worktree's files, kept up to date while the actor runs; dropping it stops everything.
pub(crate) struct WorktreeActor {
    root: PathBuf,
    /// The Worktree's own git folder, and the repository's shared one (the same for the main
    /// checkout).
    git_dir: Option<PathBuf>,
    common_dir: Option<PathBuf>,
    state: Mutex<Index>,
    watcher: Mutex<Option<notify::RecommendedWatcher>>,
    status: Mutex<WatchStatus>,
    task: Mutex<Option<tokio::task::JoinHandle<()>>>,
    full_reindexes: AtomicU64,
}

#[derive(Default)]
struct Index {
    files: BTreeSet<String>,
    /// `files` as a list for matching, rebuilt only after it changes.
    snapshot: Option<Arc<Vec<String>>>,
    /// Ignored folders (ending in `/`) and files, as git reports them: events there are dropped.
    ignored_dirs: Vec<String>,
    ignored_files: HashSet<String>,
    /// Changed files and their `git status` codes.
    changed: BTreeMap<String, String>,
}

impl Index {
    fn insert(&mut self, file: String) -> bool {
        let added = self.files.insert(file);
        if added {
            self.snapshot = None;
        }
        added
    }

    /// Removes a file, or a folder and everything in it; whether anything went.
    fn remove(&mut self, rel: &str) -> bool {
        let prefix = format!("{rel}/");
        let before = self.files.len();
        self.files.remove(rel);
        self.files.retain(|f| !f.starts_with(&prefix));
        let removed = self.files.len() != before;
        if removed {
            self.snapshot = None;
        }
        removed
    }

    fn is_ignored(&self, rel: &str) -> bool {
        self.ignored_files.contains(rel)
            || self
                .ignored_dirs
                .iter()
                .any(|dir| rel.starts_with(dir.as_str()) || rel == dir.trim_end_matches('/'))
    }
}

/// Events are gathered until this long after the last one (but no longer than `SETTLE_MAX`).
const SETTLE: Duration = Duration::from_millis(200);
const SETTLE_MAX: Duration = Duration::from_secs(1);
/// More changed (non-ignored) paths than this at once is a full re-read, not path by path.
const BURST: usize = 500;

impl Drop for WorktreeActor {
    fn drop(&mut self) {
        if let Some(task) = self.task.lock().expect("task lock").take() {
            task.abort();
        }
    }
}

/// What a path in an event is to the index.
enum Kind {
    /// A file or folder of the Worktree that git doesn't (known to) ignore.
    Candidate(String),
    /// Something git keeps branch state in (HEAD, the index, refs, logs).
    Git,
    /// `.gitignore` or `info/exclude`: what's ignored changed.
    IgnoreRules,
    /// Ignored, or git's internals that don't affect status (`objects/`, lock files).
    Noise,
}

/// Whether a path inside a git folder (relative to it) holds branch state.
fn git_relevant(sub: &str) -> bool {
    matches!(
        sub,
        "HEAD" | "index" | "ORIG_HEAD" | "FETCH_HEAD" | "MERGE_HEAD" | "packed-refs"
    ) || sub.starts_with("logs/")
        || sub.starts_with("refs/")
}

/// The outcome of a burst of events.
#[derive(Default)]
struct Applied {
    files: bool,
    status: bool,
    /// A new folder couldn't be watched: the watch limit was reached.
    exhausted: bool,
}

impl WorktreeActor {
    /// Indexes the Worktree and starts keeping it up to date.
    pub(crate) async fn start(root: PathBuf, config: FileWatchConfig, news: NewsSink) -> Arc<Self> {
        let git_dir = git::git_dir(&root).await;
        let common_dir = git::common_dir(&root).await;
        let actor = Arc::new(Self {
            root,
            git_dir,
            common_dir,
            state: Mutex::default(),
            watcher: Mutex::new(None),
            status: Mutex::new(WatchStatus::Polling),
            task: Mutex::new(None),
            full_reindexes: AtomicU64::new(0),
        });
        actor.reindex().await;
        let (tx, rx) = mpsc::unbounded_channel();
        let weak = Arc::downgrade(&actor);
        let task = match actor.watch(config, tx) {
            Ok(()) => tokio::spawn(follow(weak, rx, config, news.clone())),
            Err(why) => {
                news(&actor.root, ActorNews::Fallback(why));
                tokio::spawn(poll(weak, config.poll_interval, news.clone()))
            }
        };
        *actor.task.lock().expect("task lock") = Some(task);
        news(&actor.root, ActorNews::Files); // the first index is ready
        news(&actor.root, ActorNews::Touched(None)); // (open files may have changed unwatched)
        actor
    }

    pub(crate) fn status(&self) -> WatchStatus {
        self.status.lock().expect("status lock").clone()
    }

    pub(crate) fn stats(&self) -> IndexStats {
        IndexStats {
            files: self.state.lock().expect("index lock").files.len(),
            full_reindexes: self.full_reindexes.load(Ordering::Relaxed),
        }
    }

    /// Re-reads the whole index, the ignored paths and the changed files; whether anything moved.
    async fn reindex(&self) -> bool {
        self.full_reindexes.fetch_add(1, Ordering::Relaxed);
        let files = git::files(&self.root, None).await.unwrap_or_default();
        let ignored = git::ignored(&self.root).await.unwrap_or_default();
        let changed = changed_files(&self.root).await;
        let mut state = self.state.lock().expect("index lock");
        let files: BTreeSet<String> = files.into_iter().collect();
        let moved = state.files != files || state.changed != changed;
        if state.files != files {
            state.files = files;
            state.snapshot = None;
        }
        state.ignored_dirs = ignored
            .iter()
            .filter(|p| p.ends_with('/'))
            .cloned()
            .collect();
        state.ignored_files = ignored.into_iter().filter(|p| !p.ends_with('/')).collect();
        state.changed = changed;
        moved
    }

    /// Watches the Worktree (its indexed folders, or on Windows its root) and git's folders.
    fn watch(
        &self,
        config: FileWatchConfig,
        events: mpsc::UnboundedSender<notify::Result<notify::Event>>,
    ) -> Result<(), String> {
        let mut targets: Vec<(PathBuf, RecursiveMode)> = vec![];
        if cfg!(windows) {
            targets.push((self.root.clone(), RecursiveMode::Recursive));
        } else {
            for dir in self.folders() {
                targets.push((self.root.join(dir), RecursiveMode::NonRecursive));
            }
        }
        // Branch state: the Worktree's git folder (its top level and reflog: `objects/` churns for
        // nothing) and the shared refs. On Windows the main checkout's `.git` is already in the
        // recursive watch.
        let inside = |path: &Path| cfg!(windows) && path.starts_with(&self.root);
        let mut git_targets = vec![];
        if let Some(git_dir) = &self.git_dir {
            git_targets.push((git_dir.clone(), RecursiveMode::NonRecursive));
            git_targets.push((git_dir.join("logs"), RecursiveMode::NonRecursive));
        }
        if let Some(common) = &self.common_dir {
            if Some(common) != self.git_dir.as_ref() {
                git_targets.push((common.clone(), RecursiveMode::NonRecursive));
            }
            git_targets.push((common.join("refs"), RecursiveMode::Recursive));
        }
        for (path, mode) in git_targets {
            if path.is_dir() && !inside(&path) && !targets.iter().any(|(p, _)| *p == path) {
                targets.push((path, mode));
            }
        }
        if config
            .watch_limit
            .is_some_and(|limit| targets.len() > limit)
        {
            return Err(watch_limit_message(targets.len()));
        }
        let mut watcher = notify::recommended_watcher(move |event| {
            let _ = events.send(event);
        })
        .map_err(|e| format!("couldn't watch files: {e}"))?;
        for (path, mode) in &targets {
            if let Err(err) = watcher.watch(path, *mode) {
                return Err(match err.kind {
                    notify::ErrorKind::MaxFilesWatch => watch_limit_message(targets.len()),
                    _ => format!("couldn't watch {}: {err}", path.display()),
                });
            }
        }
        *self.status.lock().expect("status lock") = WatchStatus::Watching {
            watched: targets.into_iter().map(|(path, _)| path).collect(),
        };
        *self.watcher.lock().expect("watcher lock") = Some(watcher);
        Ok(())
    }

    /// Every folder holding an indexed file (and the root), relative, `/`-separated, `""` = root.
    fn folders(&self) -> BTreeSet<String> {
        let state = self.state.lock().expect("index lock");
        let mut folders = BTreeSet::from([String::new()]);
        for file in &state.files {
            let mut at = file.as_str();
            while let Some((parent, _)) = at.rsplit_once('/') {
                if !folders.insert(parent.to_owned()) {
                    break;
                }
                at = parent;
            }
        }
        folders
    }

    fn classify(&self, path: &Path) -> Kind {
        let in_git = |dir: &Option<PathBuf>| {
            dir.as_ref().and_then(|dir| relative(dir, path)).map(|sub| {
                if git_relevant(&sub) {
                    Kind::Git
                } else {
                    Kind::Noise
                }
            })
        };
        let Some(rel) = relative(&self.root, path) else {
            // Outside the Worktree: its git folder or the shared one (`info/exclude` lives there).
            if let Some(common) = &self.common_dir {
                if relative(common, path).as_deref() == Some("info/exclude") {
                    return Kind::IgnoreRules;
                }
            }
            return in_git(&self.git_dir)
                .or_else(|| in_git(&self.common_dir))
                .unwrap_or(Kind::Noise);
        };
        if rel.is_empty() || rel == ".git" {
            return Kind::Noise; // the Worktree's folder itself, or its `.git` file or folder
        }
        if let Some(sub) = rel.strip_prefix(".git/") {
            return match sub {
                "info/exclude" => Kind::IgnoreRules,
                _ if git_relevant(sub) => Kind::Git,
                _ => Kind::Noise,
            };
        }
        if rel == ".gitignore" || rel.ends_with("/.gitignore") {
            return Kind::IgnoreRules;
        }
        if self.state.lock().expect("index lock").is_ignored(&rel) {
            return Kind::Noise;
        }
        Kind::Candidate(rel)
    }

    /// Brings the index up to date with a burst of changed paths (and which were renames).
    async fn apply(&self, paths: Vec<(PathBuf, bool)>, rescan: bool) -> Applied {
        let mut applied = Applied::default();
        let mut candidates: Vec<String> = vec![];
        let mut renamed_in: BTreeSet<String> = BTreeSet::new();
        let mut full = rescan;
        for (path, renamed) in &paths {
            match self.classify(path) {
                Kind::Candidate(rel) => {
                    if *renamed {
                        // A rename (even a case-only one, which a lookup on Windows can't see): the
                        // folder it's in is re-read.
                        let parent = rel.rsplit_once('/').map_or("", |(parent, _)| parent);
                        renamed_in.insert(parent.to_owned());
                    }
                    candidates.push(rel);
                }
                Kind::Git => applied.status = true,
                Kind::IgnoreRules => full = true,
                Kind::Noise => {}
            }
        }
        candidates.sort();
        candidates.dedup();
        // (Counted after dropping ignored paths: a build in `target/` is never a re-read.)
        if full || candidates.len() > BURST {
            return self.reindex_fully().await;
        }
        if candidates.is_empty() {
            if applied.status {
                applied.files = self.refresh_changed().await;
            }
            return applied;
        }
        // New paths git ignores (say, a first `target/`) are dropped, and remembered as ignored.
        let Ok(ignored) = git::check_ignored(&self.root, &candidates).await else {
            return self.reindex_fully().await;
        };
        let ignored: HashSet<String> = ignored.into_iter().collect();
        let mut new_dirs = vec![];
        {
            let mut state = self.state.lock().expect("index lock");
            for rel in &candidates {
                let meta = std::fs::symlink_metadata(self.root.join(rel));
                if ignored.contains(rel) {
                    if meta.as_ref().is_ok_and(|m| m.is_dir()) {
                        state.ignored_dirs.push(format!("{rel}/"));
                    } else {
                        state.ignored_files.insert(rel.clone());
                    }
                    continue;
                }
                match meta {
                    Ok(meta) if meta.is_dir() => new_dirs.push(rel.clone()),
                    // A file, or a symlink (git tracks those as files, whatever they point at).
                    Ok(_) => applied.files |= state.insert(rel.clone()),
                    // Gone: the file, or a folder and everything in it.
                    Err(_) => applied.files |= state.remove(rel),
                }
            }
        }
        for dir in renamed_in.iter().chain(&new_dirs) {
            applied.files |= self.reread(dir).await;
        }
        if !cfg!(windows) {
            for dir in &new_dirs {
                applied.exhausted |= self.watch_new_folders(dir).await;
            }
        }
        let changed = self.refresh_changed().await;
        applied.files |= changed;
        applied.status |= changed;
        applied
    }

    async fn reindex_fully(&self) -> Applied {
        Applied {
            files: self.reindex().await,
            status: true,
            exhausted: self.sync_watches(),
        }
    }

    /// Replaces what's indexed under folder `dir` (`""`: everything) with what git lists there.
    async fn reread(&self, dir: &str) -> bool {
        let under = (!dir.is_empty()).then_some(dir);
        let Ok(listed) = git::files(&self.root, under).await else {
            return false;
        };
        let mut state = self.state.lock().expect("index lock");
        let prefix = if dir.is_empty() {
            String::new()
        } else {
            format!("{dir}/")
        };
        let before: BTreeSet<String> = state
            .files
            .iter()
            .filter(|f| f.starts_with(&prefix))
            .cloned()
            .collect();
        let after: BTreeSet<String> = listed.into_iter().collect();
        if before == after {
            return false;
        }
        state.files.retain(|f| !f.starts_with(&prefix));
        state.files.extend(after);
        state.snapshot = None;
        true
    }

    /// On Linux, a folder that appeared (even empty) needs watches of its own, as do its
    /// non-ignored subfolders; whether the watch limit was hit.
    async fn watch_new_folders(&self, dir: &str) -> bool {
        let mut found = vec![dir.to_owned()];
        let mut at = 0;
        while at < found.len() {
            if let Ok(entries) = std::fs::read_dir(self.root.join(&found[at])) {
                for entry in entries.flatten() {
                    if entry.file_type().is_ok_and(|t| t.is_dir()) {
                        let name = entry.file_name().to_string_lossy().into_owned();
                        found.push(format!("{}/{name}", found[at]));
                    }
                }
            }
            at += 1;
        }
        let ignored: Vec<String> = git::check_ignored(&self.root, &found[1..])
            .await
            .unwrap_or_default();
        let folders: Vec<PathBuf> = found
            .iter()
            .filter(|f| {
                !ignored
                    .iter()
                    .any(|i| *f == i || f.starts_with(&format!("{i}/")))
            })
            .map(|f| self.root.join(f))
            .collect();
        self.add_watches(folders)
    }

    /// After a full re-read on Linux, folders that are new (or no longer ignored) get watches.
    fn sync_watches(&self) -> bool {
        if cfg!(windows) {
            return false;
        }
        let folders = self
            .folders()
            .into_iter()
            .map(|f| self.root.join(f))
            .collect();
        self.add_watches(folders)
    }

    /// Adds non-recursive watches for `folders` not yet watched; whether the limit was hit.
    fn add_watches(&self, folders: Vec<PathBuf>) -> bool {
        let mut watcher = self.watcher.lock().expect("watcher lock");
        let Some(watcher) = watcher.as_mut() else {
            return false;
        };
        let mut status = self.status.lock().expect("status lock");
        let WatchStatus::Watching { watched } = &mut *status else {
            return false;
        };
        for folder in folders {
            if watched.contains(&folder) {
                continue;
            }
            match watcher.watch(&folder, RecursiveMode::NonRecursive) {
                Ok(()) => watched.push(folder),
                Err(err) if matches!(err.kind, notify::ErrorKind::MaxFilesWatch) => return true,
                Err(_) => {} // (gone again already)
            }
        }
        false
    }

    async fn refresh_changed(&self) -> bool {
        let changed = changed_files(&self.root).await;
        let mut state = self.state.lock().expect("index lock");
        let moved = state.changed != changed;
        state.changed = changed;
        moved
    }

    /// Stops watching and polls instead (the watch limit was reached).
    fn give_up_watching(&self) {
        self.watcher.lock().expect("watcher lock").take();
        *self.status.lock().expect("status lock") = WatchStatus::Polling;
    }

    /// The best `limit` matches for `query`, best first (with no query: changed files first).
    pub(crate) fn find(&self, query: &str, limit: usize) -> Vec<FileMatch> {
        let empty = query.trim().is_empty();
        let (files, changed) = {
            let mut state = self.state.lock().expect("index lock");
            let snapshot = match &state.snapshot {
                Some(snapshot) => snapshot.clone(),
                None => {
                    let snapshot = Arc::new(state.files.iter().cloned().collect::<Vec<_>>());
                    state.snapshot = Some(snapshot.clone());
                    snapshot
                }
            };
            let changed: Vec<String> = if empty {
                state
                    .changed
                    .keys()
                    .filter(|path| state.files.contains(*path))
                    .take(limit)
                    .cloned()
                    .collect()
            } else {
                vec![]
            };
            (snapshot, changed)
        };
        if empty {
            let rest = files.iter().filter(|f| !changed.contains(f));
            return changed
                .iter()
                .chain(rest)
                .take(limit)
                .map(|path| FileMatch {
                    path: path.clone(),
                    indices: vec![],
                })
                .collect();
        }
        find(&files, query, limit)
    }

    /// The entries of folder `dir` (relative, `""` for the root): folders first, then files.
    pub(crate) fn list(&self, dir: &str) -> Vec<DirEntry> {
        let state = self.state.lock().expect("index lock");
        let dir = dir.trim_end_matches('/');
        let prefix = if dir.is_empty() {
            String::new()
        } else {
            format!("{dir}/")
        };
        let mut folders: BTreeMap<String, bool> = BTreeMap::new();
        let mut files: BTreeMap<String, DirEntry> = BTreeMap::new();
        let mut add = |path: &str, change: Option<&String>| {
            let Some(rest) = path.strip_prefix(&prefix) else {
                return;
            };
            match rest.split_once('/') {
                Some((folder, _)) => {
                    *folders.entry(folder.to_owned()).or_default() |= change.is_some();
                }
                None if !rest.is_empty() => {
                    files.entry(rest.to_owned()).or_insert_with(|| DirEntry {
                        name: rest.to_owned(),
                        path: path.to_owned(),
                        is_dir: false,
                        change: change.cloned(),
                        has_changes: false,
                    });
                }
                None => {}
            }
        };
        // Only the folder's part of the (sorted) index.
        for file in state
            .files
            .range(prefix.clone()..)
            .take_while(|f| f.starts_with(&prefix))
        {
            add(file, state.changed.get(file));
        }
        // Deleted files are changes too, though they're no longer on disk.
        for (path, change) in state.changed.range(prefix.clone()..) {
            if !path.starts_with(&prefix) {
                break;
            }
            add(path, Some(change));
        }
        let mut entries: Vec<DirEntry> = folders
            .into_iter()
            .map(|(name, has_changes)| DirEntry {
                path: format!("{prefix}{name}"),
                name,
                is_dir: true,
                change: None,
                has_changes,
            })
            .collect();
        entries.extend(files.into_values());
        entries
    }
}

fn watch_limit_message(needed: usize) -> String {
    if cfg!(target_os = "linux") {
        format!(
            "This Worktree needs {needed} file watches, more than the system allows, so it's checked \
             for changes every few seconds instead. To raise the limit: \
             sudo sysctl fs.inotify.max_user_watches=524288 (add it to /etc/sysctl.d/ to keep it)."
        )
    } else {
        format!(
            "This Worktree needs {needed} file watches, more than allowed, so it's checked for \
             changes every few seconds instead."
        )
    }
}

/// The path relative to `root`, `/`-separated; `None` if it isn't inside.
fn relative(root: &Path, path: &Path) -> Option<String> {
    let rel = path.strip_prefix(root).ok()?;
    let parts: Vec<String> = rel
        .components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect();
    Some(parts.join("/"))
}

async fn changed_files(root: &Path) -> BTreeMap<String, String> {
    git::changes(root)
        .await
        .unwrap_or_default()
        .into_iter()
        .filter_map(|line| {
            let (code, path) = (line.get(..2)?, line.get(3..)?);
            Some((
                path.trim_end_matches('/').to_owned(),
                code.trim().to_owned(),
            ))
        })
        .collect()
}

/// Follows the watcher's events, a burst at a time, until the actor is dropped; at the watch
/// limit it switches to polling.
async fn follow(
    weak: Weak<WorktreeActor>,
    mut events: mpsc::UnboundedReceiver<notify::Result<notify::Event>>,
    config: FileWatchConfig,
    news: NewsSink,
) {
    while let Some(first) = events.recv().await {
        let mut burst = vec![first];
        let started = tokio::time::Instant::now();
        loop {
            let deadline = (tokio::time::Instant::now() + SETTLE).min(started + SETTLE_MAX);
            match tokio::time::timeout_at(deadline, events.recv()).await {
                Ok(Some(event)) => burst.push(event),
                _ => break,
            }
        }
        let Some(actor) = weak.upgrade() else {
            return;
        };
        let mut paths: Vec<(PathBuf, bool)> = vec![];
        let mut rescan = false;
        for event in burst {
            match event {
                Ok(event) => {
                    rescan |= event.need_rescan();
                    let renamed = matches!(event.kind, EventKind::Modify(ModifyKind::Name(_)));
                    paths.extend(event.paths.into_iter().map(|p| (p, renamed)));
                }
                Err(_) => rescan = true, // (e.g. the OS's event buffer overflowed)
            }
        }
        let touched = (!rescan).then(|| paths.iter().map(|(p, _)| p.clone()).collect());
        let applied = actor.apply(paths, rescan).await;
        news(&actor.root, ActorNews::Touched(touched));
        if applied.files {
            news(&actor.root, ActorNews::Files);
        }
        if applied.status {
            news(&actor.root, ActorNews::Status);
        }
        if applied.exhausted {
            actor.give_up_watching();
            let why = watch_limit_message(actor.folders().len());
            news(&actor.root, ActorNews::Fallback(why));
            drop(actor);
            return poll(weak, config.poll_interval, news).await;
        }
    }
}

/// Re-reads the Worktree every `interval` until the actor is dropped (watching failed).
async fn poll(weak: Weak<WorktreeActor>, interval: Duration, news: NewsSink) {
    let mut ticks = tokio::time::interval(interval);
    ticks.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    ticks.tick().await;
    loop {
        ticks.tick().await;
        let Some(actor) = weak.upgrade() else {
            return;
        };
        if actor.reindex().await {
            news(&actor.root, ActorNews::Files);
            news(&actor.root, ActorNews::Status);
        }
        news(&actor.root, ActorNews::Touched(None)); // (a file's content changing moves nothing)
    }
}

/// The best `limit` of `files` for `query`, best first. Typos are allowed in longer queries (one
/// per four characters, up to two); a typo costs score, so exact matches rank above typo ones.
pub(crate) fn find(files: &[String], query: &str, limit: usize) -> Vec<FileMatch> {
    let query = query.trim();
    let typos = (query.chars().count() / 4).min(2) as u16;
    let config = frizbee::Config {
        max_typos: Some(typos),
        ..frizbee::Config::default()
    };
    let mut matcher = frizbee::Matcher::new(query, &config);
    let top: Vec<&String> = matcher
        .match_list(files)
        .iter()
        .take(limit)
        .map(|m| &files[m.index as usize])
        .collect();
    let mut highlighter = frizbee::Matcher::new(query, &config);
    let mut indexed = highlighter.match_list_indices(&top);
    indexed.sort_by_key(|m| m.index);
    top.iter()
        .enumerate()
        .map(|(i, path)| FileMatch {
            path: (*path).clone(),
            indices: indexed
                .iter()
                .find(|m| m.index as usize == i)
                .map(|m| m.indices.clone())
                .unwrap_or_default(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn files(paths: &[&str]) -> Vec<String> {
        paths.iter().map(|p| (*p).to_owned()).collect()
    }

    #[test]
    fn exact_matches_rank_above_typos_and_typos_still_match() {
        let all = files(&["src/praser.rs", "src/parser.rs", "docs/readme.md"]);
        let found = find(&all, "parser", 10);
        assert_eq!(found[0].path, "src/parser.rs", "{found:?}");
        let typo = find(&all, "parsr", 10);
        assert!(typo.iter().any(|m| m.path == "src/parser.rs"), "{typo:?}");
        assert!(find(&all, "zzzzzz", 10).is_empty());
    }

    #[test]
    fn matched_characters_are_marked() {
        let found = find(&files(&["src/core.rs"]), "core", 10);
        assert_eq!(found[0].indices.len(), 4);
    }

    #[test]
    fn a_large_repo_is_matched_quickly() {
        let all: Vec<String> = (0..50_000)
            .map(|i| format!("crates/module_{}/src/file_{i}.rs", i % 300))
            .collect();
        let started = std::time::Instant::now();
        let found = find(&all, "module_42 file_4242", 50);
        assert!(!found.is_empty());
        // Generous for a debug build; release is far faster.
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "{:?}",
            started.elapsed()
        );
    }
}

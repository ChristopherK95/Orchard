//! App state persisted between runs (ticket 11): JSON in the app data folder, per Workspace. It
//! records which Tabs were open and each Worktree's Recent sessions, never a conversation (those
//! live in Claude Code's own transcripts and come back through ACP).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::session::PermissionMode;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AppState {
    pub(crate) version: u32,
    /// By the Workspace's main checkout path.
    pub(crate) workspaces: BTreeMap<PathBuf, WorkspaceState>,
}

const VERSION: u32 = 1;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WorkspaceState {
    /// The open Tabs, in order.
    pub(crate) tabs: Vec<SavedSession>,
    /// Closed sessions, most recently closed first.
    pub(crate) recent: Vec<SavedSession>,
    /// The ACP id of the Tab last shown, to show again.
    #[serde(default)]
    pub(crate) active: Option<String>,
}

/// An Agent session as it's remembered: enough to show its Tab and resume it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SavedSession {
    /// The ACP session id, which Claude Code resumes it by.
    pub(crate) acp_id: String,
    pub(crate) name: String,
    pub(crate) worktree: PathBuf,
    pub(crate) permission_mode: PermissionMode,
    /// The files it read or edited (for Edit notes, before its conversation is loaded again).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) files: Vec<PathBuf>,
}

/// How many Recent sessions are kept per Worktree.
pub(crate) const RECENT_PER_WORKTREE: usize = 20;

/// The state saved at `path`; a missing file is a first run. A file that isn't state this editor
/// understands is set aside (as `<name>.bad`) and the editor starts afresh. `Err` if the file is
/// there but can't be read (say, another program has it open): then nothing must be saved over it.
pub(crate) fn load(path: &Path) -> Result<AppState, String> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(AppState::default()),
        Err(err) => return Err(format!("couldn't read {}: {err}", path.display())),
    };
    match serde_json::from_str::<AppState>(&text) {
        Ok(state) if state.version <= VERSION => Ok(state),
        Ok(_) | Err(_) => {
            eprintln!(
                "{} isn't app state this editor understands; set aside",
                path.display()
            );
            let _ = std::fs::rename(path, path.with_extension("json.bad"));
            Ok(AppState::default())
        }
    }
}

/// Saves one Workspace's state into the file, keeping the other Workspaces' entries as the file
/// has them now, and replacing the file in one step so a crash mid-write can't leave half of it.
pub(crate) fn save_workspace(
    path: &Path,
    root: &Path,
    saved: &WorkspaceState,
) -> Result<(), String> {
    let mut state = load(path)?;
    state.version = VERSION;
    state.workspaces.insert(root.to_owned(), saved.clone());
    if let Some(folder) = path.parent() {
        std::fs::create_dir_all(folder).map_err(|e| e.to_string())?;
    }
    let text = serde_json::to_string_pretty(&state).map_err(|e| e.to_string())?;
    // Per process, so two editors saving at once don't write the same half-done file.
    let partial = path.with_extension(format!("json.{}.partial", std::process::id()));
    std::fs::write(&partial, text).map_err(|e| e.to_string())?;
    std::fs::rename(&partial, path).map_err(|e| {
        let _ = std::fs::remove_file(&partial);
        format!("couldn't replace {}: {e}", path.display())
    })
}
/// Remembers `closed` as its Worktree's most recent session (once), keeping the list short.
pub(crate) fn remember_closed(recent: &mut Vec<SavedSession>, closed: SavedSession) {
    recent.retain(|s| s.acp_id != closed.acp_id);
    recent.insert(0, closed);
    let mut per_worktree: BTreeMap<PathBuf, usize> = BTreeMap::new();
    recent.retain(|s| {
        let kept = per_worktree.entry(s.worktree.clone()).or_default();
        *kept += 1;
        *kept <= RECENT_PER_WORKTREE
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn saved(acp_id: &str, worktree: &str) -> SavedSession {
        SavedSession {
            acp_id: acp_id.into(),
            name: format!("Session {acp_id}"),
            worktree: worktree.into(),
            permission_mode: PermissionMode::Plan,
            files: vec![],
        }
    }

    #[test]
    fn the_state_round_trips_and_other_workspaces_are_kept() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");
        let repo = WorkspaceState {
            tabs: vec![saved("a", "C:/repo"), saved("b", "C:/repo.worktrees/x")],
            recent: vec![saved("c", "C:/repo")],
            active: Some("b".into()),
        };
        let other = WorkspaceState {
            tabs: vec![saved("d", "C:/other")],
            ..WorkspaceState::default()
        };
        save_workspace(&path, Path::new("C:/repo"), &repo).unwrap();
        save_workspace(&path, Path::new("C:/other"), &other).unwrap();
        let loaded = load(&path).unwrap();
        assert_eq!(loaded.workspaces[Path::new("C:/repo")], repo);
        assert_eq!(loaded.workspaces[Path::new("C:/other")], other);
        assert_eq!(loaded.version, VERSION);
    }

    #[test]
    fn state_that_isnt_ours_is_set_aside_not_lost() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");
        std::fs::write(&path, "{ not json").unwrap();
        assert_eq!(load(&path), Ok(AppState::default()));
        assert!(dir.path().join("state.json.bad").exists());
    }

    #[test]
    fn a_file_that_cant_be_read_isnt_saved_over() {
        let dir = tempfile::tempdir().unwrap();
        // A folder where the file should be: reading it fails with something other than NotFound.
        let path = dir.path().join("state.json");
        std::fs::create_dir(&path).unwrap();
        assert!(load(&path).is_err());
        assert!(save_workspace(&path, Path::new("C:/repo"), &WorkspaceState::default()).is_err());
        assert!(path.is_dir(), "left alone");
    }

    #[test]
    fn recent_sessions_are_newest_first_without_repeats_and_capped() {
        let mut recent = vec![];
        for i in 0..RECENT_PER_WORKTREE + 5 {
            remember_closed(&mut recent, saved(&i.to_string(), "w"));
        }
        remember_closed(&mut recent, saved("other", "v"));
        remember_closed(&mut recent, saved("3", "w"));
        assert_eq!(recent[0].acp_id, "3");
        assert_eq!(recent.iter().filter(|s| s.acp_id == "3").count(), 1);
        assert_eq!(
            recent
                .iter()
                .filter(|s| s.worktree == Path::new("w"))
                .count(),
            RECENT_PER_WORKTREE
        );
        assert!(recent.iter().any(|s| s.acp_id == "other"));
    }
}

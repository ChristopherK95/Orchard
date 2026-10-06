//! Tauri shell: forwards commands to `editor-core` and relays its events. No state lives here
//! (ADR 0003).
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::path::{Path, PathBuf};

use editor_core::{
    check_prerequisites, AdapterCommand, Attachment, BranchList, Core, CoreConfig, CoreError,
    CreatedWorktree, DirEntry, FileMatch, FontFamily, LoadedSettings, MissingPrerequisite,
    NewWorktree, OpenedFile, OtherConversation, PermissionMode, PoppedOutFile, QueuedPrompt,
    RecentSession, RecentWorkspace, RemovalCheck, RemoveWorktree, RemovedWorktree, RepoSettings,
    SaveOver, SessionId, SessionInfo, SettingChange, SetupInfo, SlashCommand, TerminalId,
    TerminalInfo, TerminalOutput, TerminalStream, Tools, TranscriptDelta, TranscriptPage,
    WorkspaceInfo, WorktreeInfo, WorktreeMerge,
};
use tauri::ipc::Channel;
use tauri::{AppHandle, Emitter, Manager, State};

mod notifications;

/// The pinned `claude-agent-acp` (its version comes from the root `package.json`, see `build.rs`).
const ADAPTER_PACKAGE: &str = "@agentclientprotocol/claude-agent-acp";
const ADAPTER_VERSION: &str = env!("ORCHARD_ACP_VERSION");

/// Where an installed app keeps the adapter `install_adapter` fetched.
fn installed_adapter_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("acp-adapter").join(ADAPTER_VERSION)
}

fn adapter_script(node_modules_parent: &Path) -> PathBuf {
    node_modules_parent
        .join("node_modules")
        .join(ADAPTER_PACKAGE)
        .join("dist/index.js")
}

/// Dev builds run the copy `pnpm install` put in the repo; installed (release) builds the one
/// `install_adapter` fetched into the app's data folder.
/// `ORCHARD_ACP_ADAPTER` swaps in another executable, e.g. the fake agent for demos.
fn adapter_command(data_dir: Option<&Path>) -> AdapterCommand {
    if let Some(program) = std::env::var_os("ORCHARD_ACP_ADAPTER") {
        return AdapterCommand {
            program: program.into(),
            args: vec![],
            env: vec![],
        };
    }
    let script = match data_dir {
        Some(data_dir) if !cfg!(debug_assertions) => {
            adapter_script(&installed_adapter_dir(data_dir))
        }
        _ => adapter_script(&PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..")),
    };
    AdapterCommand {
        program: "node".into(),
        args: vec![script.display().to_string()],
        env: vec![],
    }
}

/// Whether the adapter still has to be fetched before a session can start.
#[tauri::command]
fn adapter_missing(app: AppHandle) -> bool {
    if cfg!(debug_assertions) || std::env::var_os("ORCHARD_ACP_ADAPTER").is_some() {
        return false;
    }
    app.path().app_data_dir().map_or(true, |dir| {
        !adapter_script(&installed_adapter_dir(&dir)).is_file()
    })
}

/// Fetches the pinned adapter with npm into the app's data folder (an installed app's first run),
/// replacing any other version there. Staged in a side folder, so a failed install leaves nothing
/// that looks installed.
#[tauri::command]
async fn install_adapter(app: AppHandle) -> CommandResult<()> {
    static INSTALLING: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
    let _one_at_a_time = INSTALLING.lock().await;
    if !adapter_missing(app.clone()) {
        return Ok(());
    }
    let dir = installed_adapter_dir(&app.path().app_data_dir().map_err(|e| e.to_string())?);
    let root = dir.parent().expect("the version folder has a parent");
    let staging = root.join(format!("{ADAPTER_VERSION}.partial"));
    let _ = std::fs::remove_dir_all(&staging);
    std::fs::create_dir_all(&staging).map_err(|e| e.to_string())?;

    let mut npm = tokio::process::Command::new(if cfg!(windows) { "npm.cmd" } else { "npm" });
    npm.arg("install")
        .arg("--prefix")
        .arg(&staging)
        .args(["--omit=dev", "--no-audit", "--no-fund", "--loglevel=error"])
        .arg(format!("{ADAPTER_PACKAGE}@{ADAPTER_VERSION}"))
        .stdin(std::process::Stdio::null());
    for (key, value) in editor_core::host_env() {
        match value {
            Some(value) => npm.env(key, value),
            None => npm.env_remove(key),
        };
    }
    #[cfg(windows)]
    npm.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    let output = npm
        .output()
        .await
        .map_err(|e| format!("Couldn't run npm (it comes with Node.js): {e}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!(
            "npm install failed:
{}",
            stderr.trim()
        ));
    }

    // Older versions are no longer used.
    if let Ok(entries) = std::fs::read_dir(root) {
        for entry in entries.flatten() {
            if entry.path() != staging {
                let _ = std::fs::remove_dir_all(entry.path());
            }
        }
    }
    std::fs::rename(&staging, &dir).map_err(|e| e.to_string())
}

type CommandResult<T> = Result<T, String>;

#[tauri::command]
async fn prerequisites() -> Vec<MissingPrerequisite> {
    check_prerequisites(&Tools::default()).await
}

/// The repo to offer on startup: the first CLI argument, else `ORCHARD_WORKSPACE`.
#[tauri::command]
fn default_workspace_path() -> Option<String> {
    std::env::args()
        .nth(1)
        .or_else(|| std::env::var("ORCHARD_WORKSPACE").ok())
}

/// A typed path, with a leading `~` as the home folder.
fn typed_path(app: &AppHandle, path: String) -> PathBuf {
    match (path.strip_prefix('~'), app.path().home_dir().ok()) {
        (Some(rest), Some(home)) if rest.is_empty() || rest.starts_with(['/', '\\']) => {
            home.join(rest.trim_start_matches(['/', '\\']))
        }
        _ => PathBuf::from(path),
    }
}

/// Opens the repository containing `path` (closing another Workspace open in the window).
#[tauri::command]
async fn open_workspace(
    app: AppHandle,
    core: State<'_, Core>,
    path: String,
) -> CommandResult<WorkspaceInfo> {
    core.open_workspace(&typed_path(&app, path))
        .await
        .map_err(|e| e.to_string())
}

/// The Workspace opening `path` would open, without opening it.
#[tauri::command]
async fn workspace_for(
    app: AppHandle,
    core: State<'_, Core>,
    path: String,
) -> CommandResult<WorkspaceInfo> {
    core.workspace_for(&typed_path(&app, path))
        .await
        .map_err(|e| e.to_string())
}

/// Closes every popped-out Manual editor window, unsaved changes or not (the caller asked).
#[tauri::command]
fn close_pop_outs(app: AppHandle) {
    for (label, window) in app.webview_windows() {
        if label.starts_with("editor-") {
            let _ = window.destroy();
        }
    }
}

/// The Workspace picker's list: the Recent Workspaces matching `query`.
#[tauri::command]
fn recent_workspaces(core: State<'_, Core>, query: String) -> Vec<RecentWorkspace> {
    core.recent_workspaces(&query)
}

/// The native open-files dialog (the composer's paperclip); empty if it was cancelled.
#[tauri::command]
async fn pick_files(window: tauri::Window) -> Vec<String> {
    use tauri_plugin_dialog::DialogExt;
    window
        .dialog()
        .file()
        .set_title("Attach files")
        .set_parent(&window)
        .blocking_pick_files()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|file| file.into_path().ok())
        .map(|path| path.to_string_lossy().into_owned())
        .collect()
}

#[tauri::command]
fn remove_recent_workspace(core: State<'_, Core>, root: String) -> CommandResult<()> {
    core.remove_recent_workspace(root.as_ref())
        .map_err(|e| e.to_string())
}

/// The native folder dialog (the Workspace picker's Browse…); `None` if it was cancelled.
#[tauri::command]
async fn pick_folder(window: tauri::Window) -> Option<String> {
    use tauri_plugin_dialog::DialogExt;
    window
        .dialog()
        .file()
        .set_title("Open a repository")
        .set_parent(&window)
        .blocking_pick_folder()
        .and_then(|folder| folder.into_path().ok())
        .map(|path| path.to_string_lossy().into_owned())
}

#[tauri::command]
async fn new_session(core: State<'_, Core>) -> CommandResult<SessionId> {
    core.new_session().await.map_err(|e| e.to_string())
}

#[tauri::command]
async fn new_session_in(core: State<'_, Core>, worktree: String) -> CommandResult<SessionId> {
    core.new_session_in(worktree.as_ref())
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn worktrees(core: State<'_, Core>) -> Vec<WorktreeInfo> {
    core.worktrees()
}

#[tauri::command]
async fn create_worktree(
    core: State<'_, Core>,
    spec: NewWorktree,
) -> CommandResult<CreatedWorktree> {
    core.create_worktree(spec).await.map_err(|e| e.to_string())
}

/// Every Worktree with whether its branch is merged into its Base (see `Core::merge_overview`).
#[tauri::command]
async fn merge_overview(core: State<'_, Core>) -> CommandResult<Vec<WorktreeMerge>> {
    core.merge_overview().await.map_err(|e| e.to_string())
}

#[tauri::command]
async fn removal_check(core: State<'_, Core>, worktree: String) -> CommandResult<RemovalCheck> {
    core.removal_check(worktree.as_ref())
        .await
        .map_err(|e| e.to_string())
}

/// Stops the Worktree's sessions and setup, then removes it (see `Core::remove_worktree`).
#[tauri::command]
async fn remove_worktree(
    core: State<'_, Core>,
    worktree: String,
    options: RemoveWorktree,
) -> CommandResult<RemovedWorktree> {
    core.remove_worktree(worktree.as_ref(), options)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn branches(core: State<'_, Core>) -> CommandResult<BranchList> {
    core.branches().await.map_err(|e| e.to_string())
}

#[tauri::command]
async fn suggest_branch_name(core: State<'_, Core>) -> CommandResult<String> {
    core.suggest_branch_name().await.map_err(|e| e.to_string())
}

#[tauri::command]
async fn default_start_point(core: State<'_, Core>) -> CommandResult<String> {
    core.default_start_point().await.map_err(|e| e.to_string())
}
#[tauri::command]
fn settings(core: State<'_, Core>) -> LoadedSettings {
    core.settings()
}

/// Adds this repo's section to the settings file if need be; the file's path, to open in the
/// Manual editor.
#[tauri::command]
async fn open_repo_settings(core: State<'_, Core>) -> CommandResult<String> {
    core.open_repo_settings()
        .await
        .map(|path| path.display().to_string())
        .map_err(|e| e.to_string())
}

/// The fonts installed on this machine, for the settings page's font pickers.
#[tauri::command]
async fn installed_fonts(core: State<'_, Core>) -> CommandResult<Vec<FontFamily>> {
    Ok(core.installed_fonts().await)
}

/// The settings page's view of the open repo's settings.
#[tauri::command]
async fn repo_settings(core: State<'_, Core>) -> CommandResult<RepoSettings> {
    core.repo_settings().await.map_err(|e| e.to_string())
}

/// One change from the settings page, written to the settings file.
#[tauri::command]
async fn change_setting(
    core: State<'_, Core>,
    change: SettingChange,
) -> CommandResult<LoadedSettings> {
    core.change_setting(change).await.map_err(|e| e.to_string())
}

/// Opens a file for the Manual editor.
#[tauri::command]
async fn read_file(core: State<'_, Core>, path: String) -> CommandResult<OpenedFile> {
    core.read_file(path.as_ref())
        .await
        .map_err(|e| e.to_string())
}

/// What a save came to: saved (the file's new version), or refused because the file changed on
/// disk since it was read (the editor then asks before overwriting).
#[derive(serde::Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
enum SaveOutcome {
    Saved { version: String },
    ChangedOnDisk,
}

/// Saves a Manual editor's text (`\n` line endings, written as `line_ending`) over `over`.
#[tauri::command]
async fn save_file(
    core: State<'_, Core>,
    window: tauri::Window,
    path: String,
    text: String,
    line_ending: String,
    over: SaveOver,
) -> CommandResult<SaveOutcome> {
    match core
        .save_file(path.as_ref(), &text, &line_ending, over, window.label())
        .await
    {
        Ok(version) => Ok(SaveOutcome::Saved { version }),
        Err(CoreError::FileChangedOnDisk(_)) => Ok(SaveOutcome::ChangedOnDisk),
        Err(err) => Err(err.to_string()),
    }
}
#[tauri::command]
fn setups(core: State<'_, Core>) -> Vec<SetupInfo> {
    core.setups()
}

#[tauri::command]
async fn retry_setup(core: State<'_, Core>, worktree: String) -> CommandResult<()> {
    core.retry_setup(worktree.as_ref())
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn start_anyway(core: State<'_, Core>, worktree: String) -> CommandResult<()> {
    core.start_anyway(worktree.as_ref())
        .map_err(|e| e.to_string())
}

/// Called when the window regains focus, in case Worktrees changed while the editor was away.
#[tauri::command]
async fn refresh_worktrees(core: State<'_, Core>) -> CommandResult<()> {
    core.refresh_worktrees().await;
    Ok(())
}

#[tauri::command]
async fn send_prompt(
    core: State<'_, Core>,
    session_id: SessionId,
    text: String,
    attachments: Vec<Attachment>,
) -> CommandResult<()> {
    core.send_prompt_with(session_id, &text, attachments)
        .await
        .map_err(|e| e.to_string())
}

/// A message written while the Agent works: sent when the turn ends Idle (or now, if it has ended).
#[tauri::command]
async fn queue_prompt(
    core: State<'_, Core>,
    session_id: SessionId,
    text: String,
    attachments: Vec<Attachment>,
) -> CommandResult<()> {
    core.queue_prompt(session_id, &text, attachments)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn queued_prompt(
    core: State<'_, Core>,
    session_id: SessionId,
) -> CommandResult<Option<QueuedPrompt>> {
    core.queued_prompt(session_id).map_err(|e| e.to_string())
}

/// Takes the queued message back into the composer (or drops it): it won't be sent.
#[tauri::command]
fn take_queued_prompt(
    core: State<'_, Core>,
    session_id: SessionId,
) -> CommandResult<Option<QueuedPrompt>> {
    core.take_queued_prompt(session_id)
        .map_err(|e| e.to_string())
}

/// Stop: cancels the turn in progress.
#[tauri::command]
fn cancel_turn(core: State<'_, Core>, session_id: SessionId) -> CommandResult<()> {
    core.cancel_turn(session_id).map_err(|e| e.to_string())
}

/// A file (picked, or dropped on the composer) to attach to the next prompt, or why it can't be.
#[tauri::command]
async fn attach_file(
    core: State<'_, Core>,
    session_id: SessionId,
    path: String,
) -> CommandResult<Attachment> {
    core.attach_file(session_id, path.as_ref())
        .await
        .map_err(|e| e.to_string())
}

/// Something pasted into the composer (base64), to attach to the next prompt.
#[tauri::command]
async fn attach_data(
    core: State<'_, Core>,
    session_id: SessionId,
    name: String,
    mime_type: Option<String>,
    data: String,
) -> CommandResult<Attachment> {
    core.attach_data(session_id, &name, mime_type.as_deref(), &data)
        .await
        .map_err(|e| e.to_string())
}

/// A Manual editor in the calling window opened a file.
#[tauri::command]
async fn document_opened(
    core: State<'_, Core>,
    window: tauri::Window,
    path: String,
    version: String,
) -> CommandResult<()> {
    core.document_opened(path.as_ref(), window.label(), &version)
        .await
        .map_err(|e| e.to_string())
}

/// A file in the calling window got (or lost) unsaved changes.
#[tauri::command]
fn document_changed(core: State<'_, Core>, window: tauri::Window, path: String, dirty: bool) {
    core.document_changed(path.as_ref(), window.label(), dirty);
}

/// The files open in Manual editors, in every window (which have unsaved changes).
#[tauri::command]
fn open_documents(core: State<'_, Core>) -> Vec<editor_core::OpenDocument> {
    core.open_documents()
}

/// The diff from a Manual editor's text to the file on disk.
#[tauri::command]
async fn diff_with_disk(
    core: State<'_, Core>,
    path: String,
    text: String,
) -> CommandResult<Vec<editor_core::DiffLine>> {
    core.diff_with_disk(path.as_ref(), &text)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn document_closed(core: State<'_, Core>, window: tauri::Window, path: String) {
    core.document_closed(path.as_ref(), window.label());
}

/// If the file is open in another window, brings that window forward (rather than opening a second
/// copy that could be edited); whether it did.
#[tauri::command]
fn show_where_open(
    app: AppHandle,
    core: State<'_, Core>,
    window: tauri::Window,
    path: String,
) -> bool {
    let elsewhere = core
        .windows_with(path.as_ref())
        .into_iter()
        .find(|label| label != window.label());
    let Some(other) = elsewhere.and_then(|label| app.get_webview_window(&label)) else {
        return false;
    };
    let _ = other.unminimize();
    other.set_focus().is_ok()
}

/// Pops a file out of the calling window's pane into a window of its own, carrying its text and
/// cursor. Async: building a window from a synchronous command can deadlock on Windows.
#[tauri::command]
async fn pop_out(
    app: AppHandle,
    core: State<'_, Core>,
    window: tauri::Window,
    file: PoppedOutFile,
) -> CommandResult<String> {
    let name = file
        .path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    // Which Worktree it's in, as the same file name is common across them.
    let worktree = core
        .worktrees()
        .into_iter()
        .filter(|w| file.path.starts_with(&w.path))
        .max_by_key(|w| w.path.as_os_str().len())
        .and_then(|w| {
            w.branch
                .or_else(|| w.path.file_name().map(|n| n.to_string_lossy().into_owned()))
        })
        .unwrap_or_default();
    let label = core
        .pop_out(window.label(), file)
        .map_err(|e| e.to_string())?;
    let built = tauri::WebviewWindowBuilder::new(
        &app,
        &label,
        tauri::WebviewUrl::App("index.html#popout".into()),
    )
    .title(format!("{name} - {worktree} - Orchard"))
    .inner_size(900.0, 720.0)
    .decorations(false) // (it draws its own title bar, like the main window)
    .build();
    match built {
        Ok(_) => Ok(label),
        Err(err) => {
            core.cancel_pop_out(&label);
            Err(format!("couldn't open a window: {err}"))
        }
    }
}

/// A popped-out window collects its file (again, after a reload).
#[tauri::command]
async fn collect_pop_out(
    core: State<'_, Core>,
    window: tauri::Window,
) -> CommandResult<PoppedOutFile> {
    core.collect_pop_out(window.label())
        .await
        .map_err(|e| e.to_string())
}

/// A popped-out window's file as it is now (a reload of the window then loses nothing); also the
/// pane's text typed while the pop-out was opening, for window `label`.
#[tauri::command]
fn update_pop_out(
    core: State<'_, Core>,
    window: tauri::Window,
    label: Option<String>,
    text: String,
    saved_text: String,
    version: String,
) {
    let label = label.unwrap_or_else(|| window.label().to_owned());
    core.update_pop_out(&label, text, saved_text, version);
}
/// The Worktree being looked at (its files are indexed and watched).
#[tauri::command]
async fn show_worktree(core: State<'_, Core>, worktree: String) -> CommandResult<()> {
    core.show_worktree(worktree.as_ref())
        .await
        .map_err(|e| e.to_string())
}

/// Ctrl+P.
#[tauri::command]
async fn find_files(
    core: State<'_, Core>,
    worktree: String,
    query: String,
    limit: usize,
) -> CommandResult<Vec<FileMatch>> {
    core.find_files(worktree.as_ref(), &query, limit)
        .await
        .map_err(|e| e.to_string())
}

/// The Files drawer.
#[tauri::command]
async fn list_dir(
    core: State<'_, Core>,
    worktree: String,
    dir: String,
) -> CommandResult<Vec<DirEntry>> {
    core.list_dir(worktree.as_ref(), &dir)
        .await
        .map_err(|e| e.to_string())
}

/// The open Tabs, in order (after a restart, the restored ones).
#[tauri::command]
fn sessions(core: State<'_, Core>) -> Vec<SessionInfo> {
    core.sessions()
}

/// Closes a Tab; its session goes to the Worktree's Recent sessions.
#[tauri::command]
async fn close_tab(core: State<'_, Core>, session_id: SessionId) -> CommandResult<()> {
    core.close_tab(session_id).await.map_err(|e| e.to_string())
}

/// The slash commands (and skills) a session's Agent offers, for the composer's `/` menu.
#[tauri::command]
fn available_commands(
    core: State<'_, Core>,
    session_id: SessionId,
) -> CommandResult<Vec<SlashCommand>> {
    core.available_commands(session_id)
        .map_err(|e| e.to_string())
}

/// The Worktrees pinned as columns of the Columns view.
#[tauri::command]
fn pinned_worktrees(core: State<'_, Core>) -> Vec<PathBuf> {
    core.pinned_worktrees()
}

/// The columns' widths, as shares of the row.
#[tauri::command]
fn column_shares(core: State<'_, Core>) -> std::collections::BTreeMap<PathBuf, u32> {
    core.column_shares()
}

/// Sets the columns' widths (shares of the row), remembered across restarts.
#[tauri::command]
fn set_column_shares(
    core: State<'_, Core>,
    shares: std::collections::BTreeMap<PathBuf, u32>,
) -> CommandResult<()> {
    core.set_column_shares(shares).map_err(|e| e.to_string())
}

/// Pins or unpins a Worktree as a column; returns the pinned Worktrees now.
#[tauri::command]
fn set_pinned(
    core: State<'_, Core>,
    worktree: String,
    pinned: bool,
) -> CommandResult<Vec<PathBuf>> {
    core.set_pinned(worktree.as_ref(), pinned)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn recent_sessions(core: State<'_, Core>, worktree: String) -> Vec<RecentSession> {
    core.recent_sessions(worktree.as_ref())
}

#[tauri::command]
async fn reopen_session(core: State<'_, Core>, acp_id: String) -> CommandResult<SessionId> {
    core.reopen_session(&acp_id)
        .await
        .map_err(|e| e.to_string())
}

/// The Agent's conversations in a Worktree that the editor doesn't have (started in a terminal…).
#[tauri::command]
async fn other_conversations(
    core: State<'_, Core>,
    worktree: String,
) -> CommandResult<Vec<OtherConversation>> {
    core.other_conversations(worktree.as_ref())
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn open_conversation(
    core: State<'_, Core>,
    worktree: String,
    acp_id: String,
    title: Option<String>,
) -> CommandResult<SessionId> {
    core.open_conversation(worktree.as_ref(), &acp_id, title.as_deref())
        .await
        .map_err(|e| e.to_string())
}

/// `Ctrl+Shift+T`; `None` when there's nothing to reopen.
#[tauri::command]
async fn reopen_last_closed(core: State<'_, Core>) -> CommandResult<Option<SessionId>> {
    core.reopen_last_closed().await.map_err(|e| e.to_string())
}

/// The Tab shown last (before the restart, if it's been restored).
#[tauri::command]
fn last_active_session(core: State<'_, Core>) -> Option<SessionId> {
    core.last_active_session()
}

/// Stops an Idle session's Agent process; the next prompt resumes it.
#[tauri::command]
async fn suspend_session(core: State<'_, Core>, session_id: SessionId) -> CommandResult<()> {
    core.suspend_session(session_id)
        .await
        .map_err(|e| e.to_string())
}

/// Brings a Suspended or Exited session back.
#[tauri::command]
async fn resume_session(core: State<'_, Core>, session_id: SessionId) -> CommandResult<()> {
    core.resume_session(session_id)
        .await
        .map_err(|e| e.to_string())
}

/// The Git drawer: the Worktree's branch, upstream standing and changed files.
#[tauri::command]
async fn git_status(
    core: State<'_, Core>,
    worktree: String,
) -> CommandResult<editor_core::GitStatus> {
    core.git_status(worktree.as_ref())
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn git_stage(
    core: State<'_, Core>,
    worktree: String,
    paths: Vec<String>,
) -> CommandResult<()> {
    core.stage(worktree.as_ref(), &paths)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn git_unstage(
    core: State<'_, Core>,
    worktree: String,
    paths: Vec<String>,
) -> CommandResult<()> {
    core.unstage(worktree.as_ref(), &paths)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn git_stage_all(core: State<'_, Core>, worktree: String) -> CommandResult<()> {
    core.stage_all(worktree.as_ref())
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn git_unstage_all(core: State<'_, Core>, worktree: String) -> CommandResult<()> {
    core.unstage_all(worktree.as_ref())
        .await
        .map_err(|e| e.to_string())
}

/// Commits what's staged, or says what to ask the user first.
#[tauri::command]
async fn git_commit(
    core: State<'_, Core>,
    worktree: String,
    request: editor_core::CommitRequest,
) -> CommandResult<editor_core::CommitOutcome> {
    core.commit(worktree.as_ref(), request)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn git_fetch(core: State<'_, Core>, worktree: String) -> CommandResult<()> {
    core.fetch(worktree.as_ref())
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn git_push(
    core: State<'_, Core>,
    worktree: String,
) -> CommandResult<editor_core::PushOutcome> {
    core.push(worktree.as_ref())
        .await
        .map_err(|e| e.to_string())
}

/// Fast-forward only: says so when the branch has diverged instead (or asks first while a session
/// there is mid-turn).
#[tauri::command]
async fn git_pull(
    core: State<'_, Core>,
    worktree: String,
    even_if_working: bool,
) -> CommandResult<editor_core::PullOutcome> {
    core.pull(worktree.as_ref(), even_if_working)
        .await
        .map_err(|e| e.to_string())
}

/// The window got focus: the Worktrees are listed again, and fetched if due (5 minutes at most).
#[tauri::command]
async fn window_focused(core: State<'_, Core>) -> CommandResult<()> {
    core.window_focused().await;
    Ok(())
}

/// Switches the Worktree to another branch (refused while a session there is mid-turn).
#[tauri::command]
async fn switch_branch(
    core: State<'_, Core>,
    worktree: String,
    branch: String,
) -> CommandResult<()> {
    core.switch_branch(worktree.as_ref(), &branch)
        .await
        .map_err(|e| e.to_string())
}

/// "Changes vs base": what the Worktree's branch changed since it split from its Base.
#[tauri::command]
async fn changes_vs_base(
    core: State<'_, Core>,
    worktree: String,
) -> CommandResult<editor_core::BaseChanges> {
    core.changes_vs_base(worktree.as_ref())
        .await
        .map_err(|e| e.to_string())
}

/// Sets the Worktree's Base (null: back to the default).
#[tauri::command]
async fn set_base(
    core: State<'_, Core>,
    worktree: String,
    base: Option<String>,
) -> CommandResult<()> {
    core.set_base(worktree.as_ref(), base.as_deref())
        .await
        .map_err(|e| e.to_string())
}

/// One file's change since the branch split from its Base (`split`, as the list gave it).
#[tauri::command]
async fn diff_vs_base(
    core: State<'_, Core>,
    worktree: String,
    split: String,
    path: String,
    renamed_from: Option<String>,
    change: editor_core::ChangeKind,
) -> CommandResult<Vec<editor_core::DiffLine>> {
    core.diff_vs_base(
        worktree.as_ref(),
        &split,
        &path,
        renamed_from.as_deref(),
        change,
    )
    .await
    .map_err(|e| e.to_string())
}

/// Aborts the merge, rebase, cherry-pick or revert in progress.
#[tauri::command]
async fn abort_operation(core: State<'_, Core>, worktree: String) -> CommandResult<()> {
    core.abort_operation(worktree.as_ref())
        .await
        .map_err(|e| e.to_string())
}

/// Throws away every change to one file (the drawer has asked).
#[tauri::command]
async fn git_discard(core: State<'_, Core>, worktree: String, path: String) -> CommandResult<()> {
    core.discard(worktree.as_ref(), &path)
        .await
        .map_err(|e| e.to_string())
}

/// The session's Edit notes waiting for its next prompt.
#[tauri::command]
fn edit_notes(
    core: State<'_, Core>,
    session_id: SessionId,
) -> CommandResult<Vec<editor_core::EditNote>> {
    core.edit_notes(session_id).map_err(|e| e.to_string())
}

/// The user removed an Edit note: the Agent isn't told about that change.
#[tauri::command]
fn remove_edit_note(
    core: State<'_, Core>,
    session_id: SessionId,
    path: String,
) -> CommandResult<()> {
    core.remove_edit_note(session_id, path.as_ref())
        .map_err(|e| e.to_string())
}

/// The oldest permission card waiting in the session (for the Board).
#[tauri::command]
fn pending_permission(
    core: State<'_, Core>,
    session_id: SessionId,
) -> CommandResult<Option<editor_core::PermissionRequest>> {
    core.pending_permission(session_id)
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn answer_permission(
    core: State<'_, Core>,
    session_id: SessionId,
    tool_call_id: String,
    option_id: String,
) -> CommandResult<()> {
    core.answer_permission(session_id, &tool_call_id, &option_id)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn set_permission_mode(
    core: State<'_, Core>,
    session_id: SessionId,
    mode: PermissionMode,
) -> CommandResult<()> {
    core.set_permission_mode(session_id, mode)
        .await
        .map_err(|e| e.to_string())
}

/// Benchmark mode (ticket 05): `ORCHARD_BENCH` names a file the frontend's scripted scenario
/// appends phase markers to, so `scripts/bench-memory.ps1` knows when to measure.
#[tauri::command]
fn bench_mode() -> bool {
    std::env::var_os("ORCHARD_BENCH").is_some()
}

#[tauri::command]
fn bench_mark(phase: String) -> CommandResult<()> {
    use std::io::Write;
    let Some(path) = std::env::var_os("ORCHARD_BENCH") else {
        return Ok(());
    };
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|e| e.to_string())?;
    writeln!(file, "{phase}").map_err(|e| e.to_string())
}

/// Shows an OS notification about a session; clicking it opens that session's Tab.
#[tauri::command]
fn notify_session(app: AppHandle, session_id: SessionId, title: String, body: String) {
    notifications::notify(app, session_id, title, body);
}

#[tauri::command]
async fn transcript_page_before(
    core: State<'_, Core>,
    session_id: SessionId,
    before: usize,
) -> CommandResult<TranscriptPage> {
    core.transcript_page_before(session_id, before)
        .map_err(|e| e.to_string())
}

/// No Tab visible in view `slot` (a Worktree without sessions is selected there, or a column
/// closed), or in any slot without one: the Tab shown there stops streaming.
#[tauri::command]
fn hide_tabs(core: State<'_, Core>, slot: Option<String>) {
    match slot {
        Some(slot) => core.hide_tab_in(&slot),
        None => core.hide_tabs(),
    }
}

/// Makes a session the visible Tab of view `slot` (default: the Tabs view's) and streams its
/// transcript over a dedicated channel; the Tab that slot showed stops streaming. Async so the
/// core's streaming task runs on Tauri's runtime.
#[tauri::command]
async fn show_session(
    core: State<'_, Core>,
    session_id: SessionId,
    slot: Option<String>,
    on_batch: Channel<Vec<TranscriptDelta>>,
) -> CommandResult<()> {
    let slot = slot.as_deref().unwrap_or(editor_core::TABS_SLOT);
    let mut stream = core
        .show_session_in(slot, session_id)
        .map_err(|e| e.to_string())?;
    tauri::async_runtime::spawn(async move {
        while let Some(batch) = stream.next().await {
            if on_batch.send(batch).is_err() {
                break;
            }
        }
    });
    Ok(())
}

/// Every running shell, oldest first.
#[tauri::command]
fn terminals(core: State<'_, Core>) -> Vec<TerminalInfo> {
    core.terminals()
}

/// Shows a shell of the Worktree in view slot `slot`'s Terminal panel: `id` if it still runs
/// there, else the Worktree's oldest, else a new one (`cols` x `rows`). Its output streams over a
/// dedicated channel: what it kept first, then what it prints, whatever has arrived sent in one
/// go. Whatever the slot showed before stops sending there.
#[tauri::command]
async fn open_terminal(
    core: State<'_, Core>,
    slot: String,
    worktree: PathBuf,
    id: Option<TerminalId>,
    cols: u16,
    rows: u16,
    on_output: Channel<Vec<TerminalOutput>>,
) -> CommandResult<TerminalInfo> {
    let (info, stream) = core
        .open_terminal(&slot, &worktree, id, cols, rows)
        .await
        .map_err(|e| e.to_string())?;
    forward_terminal(stream, on_output);
    Ok(info)
}

/// Starts another shell in the Worktree and shows it in view slot `slot`'s Terminal panel.
#[tauri::command]
async fn new_terminal(
    core: State<'_, Core>,
    slot: String,
    worktree: PathBuf,
    cols: u16,
    rows: u16,
    on_output: Channel<Vec<TerminalOutput>>,
) -> CommandResult<TerminalInfo> {
    let (info, stream) = core
        .new_terminal(&slot, &worktree, cols, rows)
        .await
        .map_err(|e| e.to_string())?;
    forward_terminal(stream, on_output);
    Ok(info)
}

fn forward_terminal(mut stream: TerminalStream, on_output: Channel<Vec<TerminalOutput>>) {
    tauri::async_runtime::spawn(async move {
        while let Some(first) = stream.next().await {
            let mut batch = vec![first];
            batch.extend(std::iter::from_fn(|| stream.try_next()));
            if on_output.send(batch).is_err() {
                break;
            }
        }
    });
}

#[tauri::command]
fn terminal_input(core: State<'_, Core>, id: TerminalId, data: String) -> CommandResult<()> {
    core.terminal_input(id, &data).map_err(|e| e.to_string())
}

#[tauri::command]
fn resize_terminal(
    core: State<'_, Core>,
    id: TerminalId,
    cols: u16,
    rows: u16,
) -> CommandResult<()> {
    core.resize_terminal(id, cols, rows)
        .map_err(|e| e.to_string())
}

/// View slot `slot`'s Terminal panel is hidden; the shells keep running.
#[tauri::command]
fn hide_terminal(core: State<'_, Core>, slot: String) {
    core.hide_terminal(&slot);
}

/// Stops a shell and everything it started.
#[tauri::command]
async fn close_terminal(core: State<'_, Core>, id: TerminalId) -> CommandResult<()> {
    core.close_terminal(id).await;
    Ok(())
}

/// The identifier the app had as Agent Editor, which names its old config and data folders.
const OLD_IDENTIFIER: &str = "dev.agent-editor.app";

/// Moves a folder from before the rename to Orchard into place, unless Orchard already has one.
fn adopt_old_dir(dir: Option<std::path::PathBuf>) {
    let Some(dir) = dir else { return };
    let Some(old) = dir.parent().map(|parent| parent.join(OLD_IDENTIFIER)) else {
        return;
    };
    if !dir.exists() && old.is_dir() {
        let _ = std::fs::rename(&old, &dir);
    }
}

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        // A closed window's files are no longer open (however it closed).
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::Destroyed = event {
                window.state::<Core>().window_closed(window.label());
            }
        })
        // A (re)loading page has nothing open yet: its editors say again what they open.
        .on_page_load(|webview, payload| {
            if payload.event() == tauri::webview::PageLoadEvent::Started {
                webview.state::<Core>().page_loading(webview.label());
            }
        })
        .setup(|app| {
            // On Windows both are the same folder, so the second finds it already moved.
            adopt_old_dir(app.path().app_config_dir().ok());
            adopt_old_dir(app.path().app_data_dir().ok());
            let core = Core::new(CoreConfig {
                settings_path: app
                    .path()
                    .app_config_dir()
                    .ok()
                    .map(|dir| dir.join("settings.toml")),
                // The benchmark starts from nothing each run (and leaves nothing behind).
                state_path: app
                    .path()
                    .app_data_dir()
                    .ok()
                    .filter(|_| !bench_mode())
                    .map(|dir| dir.join("state.json")),
                ..CoreConfig::new(adapter_command(app.path().app_data_dir().ok().as_deref()))
            });
            let mut events = core.subscribe();
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                use tokio::sync::broadcast::error::RecvError;
                loop {
                    match events.recv().await {
                        Ok(event) => {
                            let _ = handle.emit("core-event", event);
                        }
                        // Fell behind (e.g. a noisy setup): skip what was missed, keep relaying.
                        Err(RecvError::Lagged(_)) => continue,
                        Err(RecvError::Closed) => break,
                    }
                }
            });
            app.manage(core);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            prerequisites,
            adapter_missing,
            install_adapter,
            default_workspace_path,
            open_workspace,
            workspace_for,
            close_pop_outs,
            recent_workspaces,
            remove_recent_workspace,
            pick_folder,
            pick_files,
            new_session,
            new_session_in,
            worktrees,
            refresh_worktrees,
            create_worktree,
            merge_overview,
            removal_check,
            remove_worktree,
            branches,
            suggest_branch_name,
            default_start_point,
            settings,
            open_repo_settings,
            repo_settings,
            installed_fonts,
            change_setting,
            read_file,
            document_opened,
            document_changed,
            document_closed,
            open_documents,
            diff_with_disk,
            show_where_open,
            pop_out,
            collect_pop_out,
            update_pop_out,
            save_file,
            setups,
            retry_setup,
            start_anyway,
            send_prompt,
            queue_prompt,
            queued_prompt,
            take_queued_prompt,
            cancel_turn,
            attach_file,
            attach_data,
            sessions,
            show_worktree,
            find_files,
            list_dir,
            close_tab,
            recent_sessions,
            reopen_session,
            other_conversations,
            open_conversation,
            reopen_last_closed,
            last_active_session,
            suspend_session,
            resume_session,
            answer_permission,
            pending_permission,
            edit_notes,
            remove_edit_note,
            git_status,
            git_stage,
            git_unstage,
            git_stage_all,
            git_unstage_all,
            git_commit,
            git_discard,
            git_fetch,
            git_push,
            git_pull,
            window_focused,
            switch_branch,
            abort_operation,
            changes_vs_base,
            set_base,
            diff_vs_base,
            set_permission_mode,
            transcript_page_before,
            notify_session,
            bench_mode,
            bench_mark,
            show_session,
            hide_tabs,
            pinned_worktrees,
            set_pinned,
            column_shares,
            set_column_shares,
            available_commands,
            terminals,
            open_terminal,
            new_terminal,
            terminal_input,
            resize_terminal,
            hide_terminal,
            close_terminal
        ])
        .run(tauri::generate_context!())
        .expect("error while running the editor");
}

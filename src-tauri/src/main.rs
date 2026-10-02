//! Tauri shell: forwards commands to `editor-core` and relays its events. No state lives here
//! (ADR 0003).
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::path::PathBuf;

use editor_core::{
    check_prerequisites, AdapterCommand, BranchList, Core, CoreConfig, CoreError, CreatedWorktree,
    DirEntry, FileMatch, LoadedSettings, MissingPrerequisite, NewWorktree, OpenedFile,
    PermissionMode, PoppedOutFile, RecentSession, RemovalCheck, RemoveWorktree, RemovedWorktree,
    SaveOver, SessionId, SessionInfo, SetupInfo, Tools, TranscriptDelta, TranscriptPage,
    WorkspaceInfo, WorktreeInfo,
};
use tauri::ipc::Channel;
use tauri::{AppHandle, Emitter, Manager, State};

mod notifications;

/// The pinned ACP adapter installed by `pnpm install` (see the root `package.json`).
/// `AGENT_EDITOR_ACP_ADAPTER` swaps in another executable, e.g. the fake agent for demos.
/// The path is fixed at build time, which only suits dev builds; distribution is out of scope for v1.
fn adapter_command() -> AdapterCommand {
    if let Some(program) = std::env::var_os("AGENT_EDITOR_ACP_ADAPTER") {
        return AdapterCommand {
            program: program.into(),
            args: vec![],
            env: vec![],
        };
    }
    let script = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../node_modules/@agentclientprotocol/claude-agent-acp/dist/index.js");
    AdapterCommand {
        program: "node".into(),
        args: vec![script.display().to_string()],
        env: vec![],
    }
}

type CommandResult<T> = Result<T, String>;

#[tauri::command]
async fn prerequisites() -> Vec<MissingPrerequisite> {
    check_prerequisites(&Tools::default()).await
}

/// The repo to offer on startup: the first CLI argument, else `AGENT_EDITOR_WORKSPACE`.
#[tauri::command]
fn default_workspace_path() -> Option<String> {
    std::env::args()
        .nth(1)
        .or_else(|| std::env::var("AGENT_EDITOR_WORKSPACE").ok())
}

#[tauri::command]
async fn open_workspace(core: State<'_, Core>, path: String) -> CommandResult<WorkspaceInfo> {
    core.open_workspace(path.as_ref())
        .await
        .map_err(|e| e.to_string())
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
) -> CommandResult<()> {
    core.send_prompt(session_id, &text)
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
    .title(format!("{name} - {worktree} - Agent Editor"))
    .inner_size(900.0, 720.0)
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

/// Benchmark mode (ticket 05): `AGENT_EDITOR_BENCH` names a file the frontend's scripted scenario
/// appends phase markers to, so `scripts/bench-memory.ps1` knows when to measure.
#[tauri::command]
fn bench_mode() -> bool {
    std::env::var_os("AGENT_EDITOR_BENCH").is_some()
}

#[tauri::command]
fn bench_mark(phase: String) -> CommandResult<()> {
    use std::io::Write;
    let Some(path) = std::env::var_os("AGENT_EDITOR_BENCH") else {
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

/// No Tab visible (a Worktree without sessions is selected): the previous Tab stops streaming.
#[tauri::command]
fn hide_tabs(core: State<'_, Core>) {
    core.hide_tabs();
}

/// Makes a session the visible Tab and streams its transcript over a dedicated channel; the
/// previously visible Tab's stream ends. Async so the core's streaming task runs on Tauri's runtime.
#[tauri::command]
async fn show_session(
    core: State<'_, Core>,
    session_id: SessionId,
    on_batch: Channel<Vec<TranscriptDelta>>,
) -> CommandResult<()> {
    let mut stream = core.show_session(session_id).map_err(|e| e.to_string())?;
    tauri::async_runtime::spawn(async move {
        while let Some(batch) = stream.next().await {
            if on_batch.send(batch).is_err() {
                break;
            }
        }
    });
    Ok(())
}

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
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
                ..CoreConfig::new(adapter_command())
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
            default_workspace_path,
            open_workspace,
            new_session,
            new_session_in,
            worktrees,
            refresh_worktrees,
            create_worktree,
            removal_check,
            remove_worktree,
            branches,
            suggest_branch_name,
            default_start_point,
            settings,
            open_repo_settings,
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
            sessions,
            show_worktree,
            find_files,
            list_dir,
            close_tab,
            recent_sessions,
            reopen_session,
            reopen_last_closed,
            last_active_session,
            suspend_session,
            resume_session,
            answer_permission,
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
            set_permission_mode,
            transcript_page_before,
            notify_session,
            bench_mode,
            bench_mark,
            show_session,
            hide_tabs
        ])
        .run(tauri::generate_context!())
        .expect("error while running the editor");
}

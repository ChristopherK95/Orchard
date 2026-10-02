//! Tauri shell: forwards commands to `editor-core` and relays its events. No state lives here
//! (ADR 0003).
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::path::PathBuf;

use editor_core::{
    check_prerequisites, AdapterCommand, BranchList, Core, CoreConfig, CreatedWorktree,
    LoadedSettings, MissingPrerequisite, NewWorktree, PermissionMode, RemovalCheck, RemoveWorktree,
    RemovedWorktree, SessionId, SetupInfo, Tools, TranscriptDelta, TranscriptPage, WorkspaceInfo,
    WorktreeInfo,
};
use tauri::ipc::Channel;
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_opener::OpenerExt;

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

/// Adds this repo's section to the settings file if need be, then opens the file in the OS's
/// editor for TOML (the Manual editor takes this over in ticket 14). With no app for `.toml`
/// files, it shows the file in its folder instead.
#[tauri::command]
async fn open_repo_settings(app: AppHandle, core: State<'_, Core>) -> CommandResult<()> {
    let path = core.open_repo_settings().await.map_err(|e| e.to_string())?;
    let opener = app.opener();
    opener
        .open_path(path.display().to_string(), None::<&str>)
        .or_else(|_| opener.reveal_item_in_dir(&path))
        .map_err(|e| format!("couldn't open {}: {e}", path.display()))
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
        .setup(|app| {
            let core = Core::new(CoreConfig {
                adapter: adapter_command(),
                settings_path: app
                    .path()
                    .app_config_dir()
                    .ok()
                    .map(|dir| dir.join("settings.toml")),
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
            setups,
            retry_setup,
            start_anyway,
            send_prompt,
            suspend_session,
            resume_session,
            answer_permission,
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

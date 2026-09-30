//! Tauri shell: forwards commands to `editor-core` and relays its events. No state lives here
//! (ADR 0003).
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::path::PathBuf;

use editor_core::{
    check_prerequisites, AdapterCommand, Core, CoreConfig, MissingPrerequisite, PermissionMode, SessionId, Tools,
    TranscriptDelta, WorkspaceInfo,
};
use tauri::ipc::Channel;
use tauri::{Emitter, Manager, State};

/// The pinned ACP adapter installed by `pnpm install` (see the root `package.json`).
/// `AGENT_EDITOR_ACP_ADAPTER` swaps in another executable, e.g. the fake agent for demos.
/// The path is fixed at build time, which only suits dev builds; distribution is out of scope for v1.
fn adapter_command() -> AdapterCommand {
    if let Some(program) = std::env::var_os("AGENT_EDITOR_ACP_ADAPTER") {
        return AdapterCommand { program: program.into(), args: vec![], env: vec![] };
    }
    let script = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../node_modules/@agentclientprotocol/claude-agent-acp/dist/index.js");
    AdapterCommand { program: "node".into(), args: vec![script.display().to_string()], env: vec![] }
}

type CommandResult<T> = Result<T, String>;

#[tauri::command]
async fn prerequisites() -> Vec<MissingPrerequisite> {
    check_prerequisites(&Tools::default()).await
}

/// The repo to offer on startup: the first CLI argument, else `AGENT_EDITOR_WORKSPACE`.
#[tauri::command]
fn default_workspace_path() -> Option<String> {
    std::env::args().nth(1).or_else(|| std::env::var("AGENT_EDITOR_WORKSPACE").ok())
}

#[tauri::command]
async fn open_workspace(core: State<'_, Core>, path: String) -> CommandResult<WorkspaceInfo> {
    core.open_workspace(path.as_ref()).await.map_err(|e| e.to_string())
}

#[tauri::command]
async fn new_session(core: State<'_, Core>) -> CommandResult<SessionId> {
    core.new_session().await.map_err(|e| e.to_string())
}

#[tauri::command]
async fn send_prompt(core: State<'_, Core>, session_id: SessionId, text: String) -> CommandResult<()> {
    core.send_prompt(session_id, &text).await.map_err(|e| e.to_string())
}

#[tauri::command]
async fn answer_permission(
    core: State<'_, Core>,
    session_id: SessionId,
    tool_call_id: String,
    option_id: String,
) -> CommandResult<()> {
    core.answer_permission(session_id, &tool_call_id, &option_id).await.map_err(|e| e.to_string())
}

#[tauri::command]
async fn set_permission_mode(core: State<'_, Core>, session_id: SessionId, mode: PermissionMode) -> CommandResult<()> {
    core.set_permission_mode(session_id, mode).await.map_err(|e| e.to_string())
}

/// Streams a session's transcript to the visible Tab over a dedicated channel.
/// Async so it runs on Tauri's runtime: the core spawns the streaming task there.
#[tauri::command]
async fn watch_session(
    core: State<'_, Core>,
    session_id: SessionId,
    on_batch: Channel<Vec<TranscriptDelta>>,
) -> CommandResult<()> {
    let mut stream = core.watch_session(session_id).map_err(|e| e.to_string())?;
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
        .setup(|app| {
            let core = Core::new(CoreConfig { adapter: adapter_command() });
            let mut events = core.subscribe();
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                while let Ok(event) = events.recv().await {
                    let _ = handle.emit("core-event", event);
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
            send_prompt,
            answer_permission,
            set_permission_mode,
            watch_session
        ])
        .run(tauri::generate_context!())
        .expect("error while running the editor");
}

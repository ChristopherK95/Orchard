//! Tauri shell: forwards commands to `editor-core` and relays its events. No state lives here
//! (ADR 0003).
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::path::PathBuf;

use editor_core::{
    check_prerequisites, AdapterCommand, Core, CoreConfig, MissingPrerequisite, PermissionMode,
    SessionId, Tools, TranscriptDelta, TranscriptPage, WorkspaceInfo,
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
async fn send_prompt(
    core: State<'_, Core>,
    session_id: SessionId,
    text: String,
) -> CommandResult<()> {
    core.send_prompt(session_id, &text)
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
    let Some(path) = std::env::var_os("AGENT_EDITOR_BENCH") else { return Ok(()) };
    let mut file = std::fs::OpenOptions::new().create(true).append(true).open(path).map_err(|e| e.to_string())?;
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
        .setup(|app| {
            let core = Core::new(CoreConfig {
                adapter: adapter_command(),
            });
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
            transcript_page_before,
            notify_session,
            bench_mode,
            bench_mark,
            show_session
        ])
        .run(tauri::generate_context!())
        .expect("error while running the editor");
}

//! Opt-in check against the real, pinned `claude-agent-acp` (run `pnpm install` first, and be logged
//! in with `claude /login`). It only initialises and creates a session, so no prompt is sent and no
//! subscription usage is spent. Run with: `cargo test -p editor-core --test real_adapter -- --ignored`

mod support;

use std::path::PathBuf;

use editor_core::{AdapterCommand, Core, CoreConfig};

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs Node, `pnpm install` and a Claude login"]
async fn the_real_adapter_accepts_the_handshake_and_creates_a_session() {
    let script = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../node_modules/@agentclientprotocol/claude-agent-acp/dist/index.js");
    assert!(
        script.exists(),
        "run `pnpm install` first ({})",
        script.display()
    );
    let repo = support::git_repo();
    let core = Core::new(CoreConfig {
        adapter: AdapterCommand {
            program: "node".into(),
            args: vec![script.display().to_string()],
            env: vec![],
        },
        settings_path: None,
    });
    core.open_workspace(repo.path()).await.unwrap();
    let mut events = core.subscribe();

    let session = tokio::time::timeout(support::TIMEOUT * 6, core.new_session())
        .await
        .expect("timed out")
        .unwrap();

    assert_eq!(core.transcript(session).unwrap(), vec![]);
    assert!(events.try_recv().is_ok(), "SessionCreated was broadcast");
}

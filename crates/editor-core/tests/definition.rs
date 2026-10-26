//! Go to definition in the Manual editor: a name clicked in a file leads to where it's defined,
//! found through the file's import line, in a real temp repository.

mod support;

use std::path::{Path, PathBuf};

use editor_core::Core;
use support::*;

fn write(root: &Path, rel: &str, text: &str) {
    let path = root.join(rel);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

/// The core with the Worktree shown (that's what indexes its files), and the Worktree's root.
async fn workspace(repo: &Path) -> (FakeAgent, Core, PathBuf) {
    let fake = FakeAgent::new(r#"{"turns":[]}"#);
    let core = core_with(&fake);
    let root = core.open_workspace(repo).await.unwrap().root;
    core.show_worktree(&root).await.unwrap();
    (fake, core, root)
}

#[tokio::test(flavor = "multi_thread")]
async fn an_imported_name_leads_to_its_line_in_the_file_it_comes_from() {
    let repo = git_repo();
    let app = "import { parseConfig } from \"./config\";\nparseConfig();\n";
    write(repo.path(), "src/app.ts", app);
    write(repo.path(), "src/config.ts", "// config\nexport function parseConfig() {}\n");
    write(repo.path(), "src/other.ts", "export function parseConfig() {}\n");
    let (_fake, core, root) = workspace(repo.path()).await;

    let found = core
        .find_definition(&root.join("src/app.ts"), app, "parseConfig")
        .await
        .unwrap();

    assert_eq!(found.len(), 1, "{found:?}");
    assert!(found[0].path.ends_with("config.ts"), "{found:?}");
    assert_eq!(found[0].line, 2);
}

#[tokio::test(flavor = "multi_thread")]
async fn unsaved_text_is_what_is_searched_and_unknown_names_find_nothing() {
    let repo = git_repo();
    write(repo.path(), "lib.py", "pass\n");
    let (_fake, core, root) = workspace(repo.path()).await;
    let unsaved = "x = 1\n\ndef fresh():\n    pass\n";

    let found = core
        .find_definition(&root.join("lib.py"), unsaved, "fresh")
        .await
        .unwrap();
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0].line, 3);

    let none = core
        .find_definition(&root.join("lib.py"), unsaved, "nothing_here")
        .await
        .unwrap();
    assert!(none.is_empty());
}

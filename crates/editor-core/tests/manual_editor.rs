//! Ticket 14: the Manual editor reads and saves files through the core. Every save carries the
//! version the editor last read; a file that changed on disk since is never overwritten unasked.

mod support;

use std::path::Path;

use editor_core::{Core, CoreError, FileContent, SaveOver};
use support::*;

fn over(version: &str) -> SaveOver {
    SaveOver::Version {
        version: version.to_owned(),
    }
}

async fn workspace(fake: &FakeAgent, repo: &Path) -> Core {
    let core = core_with(fake);
    core.open_workspace(repo).await.unwrap();
    core
}

fn text_of(content: &FileContent) -> &str {
    match content {
        FileContent::Text { text, .. } => text,
        other => panic!("not text: {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_file_saves_over_the_version_it_was_read_at() {
    let repo = git_repo();
    let fake = FakeAgent::new(r#"{"turns":[]}"#);
    let core = workspace(&fake, repo.path()).await;
    let path = repo.path().join("notes.md");
    std::fs::write(&path, "# Notes\n").unwrap();

    let opened = core.read_file(&path).await.unwrap();
    assert_eq!(text_of(&opened.content), "# Notes\n");
    let saved = core
        .save_file(
            &path,
            "# Notes\nmore\n",
            "\n",
            over(&opened.version),
            "main",
        )
        .await
        .unwrap();
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "# Notes\nmore\n");
    assert_ne!(saved, opened.version);

    // Saving again from the new version works too.
    core.save_file(&path, "# Notes\n", "\n", over(&saved), "main")
        .await
        .unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn saving_over_a_newer_file_on_disk_is_refused_until_confirmed() {
    let repo = git_repo();
    let fake = FakeAgent::new(r#"{"turns":[]}"#);
    let core = workspace(&fake, repo.path()).await;
    let path = repo.path().join("lib.rs");
    std::fs::write(&path, "fn mine() {}\n").unwrap();
    let opened = core.read_file(&path).await.unwrap();

    // An Agent writes the file meanwhile.
    std::fs::write(&path, "fn agents() {}\n").unwrap();
    let refused = core
        .save_file(
            &path,
            "fn mine_edited() {}\n",
            "\n",
            over(&opened.version),
            "main",
        )
        .await;
    assert!(
        matches!(refused, Err(CoreError::FileChangedOnDisk(_))),
        "{refused:?}"
    );
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "fn agents() {}\n",
        "untouched"
    );

    // The user chose to overwrite.
    core.save_file(
        &path,
        "fn mine_edited() {}\n",
        "\n",
        SaveOver::Anything,
        "main",
    )
    .await
    .unwrap();
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "fn mine_edited() {}\n"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_file_deleted_on_disk_counts_as_changed() {
    let repo = git_repo();
    let fake = FakeAgent::new(r#"{"turns":[]}"#);
    let core = workspace(&fake, repo.path()).await;
    let path = repo.path().join("gone.txt");
    std::fs::write(&path, "here").unwrap();
    let opened = core.read_file(&path).await.unwrap();
    std::fs::remove_file(&path).unwrap();
    assert!(matches!(
        core.save_file(&path, "back", "\n", over(&opened.version), "main")
            .await,
        Err(CoreError::FileChangedOnDisk(_))
    ));
}

#[tokio::test(flavor = "multi_thread")]
async fn odd_files_follow_the_rules() {
    let repo = git_repo();
    let fake = FakeAgent::new(r#"{"turns":[]}"#);
    let core = workspace(&fake, repo.path()).await;

    std::fs::write(
        repo.path().join("image.png"),
        [0x89, b'P', b'N', b'G', 0, 0, 1, 2],
    )
    .unwrap();
    let binary = core
        .read_file(&repo.path().join("image.png"))
        .await
        .unwrap();
    assert!(matches!(binary.content, FileContent::Binary { bytes: 8 }));

    let big = "x".repeat(6 * 1024 * 1024);
    std::fs::write(repo.path().join("big.log"), &big).unwrap();
    let opened = core.read_file(&repo.path().join("big.log")).await.unwrap();
    assert!(matches!(
        opened.content,
        FileContent::Text {
            read_only: true,
            ..
        }
    ));

    let minified = format!("var a={};", "1,".repeat(5000));
    std::fs::write(repo.path().join("app.min.js"), &minified).unwrap();
    let opened = core
        .read_file(&repo.path().join("app.min.js"))
        .await
        .unwrap();
    assert!(matches!(
        opened.content,
        FileContent::Text {
            minified: true,
            read_only: false,
            ..
        }
    ));

    std::fs::write(repo.path().join("plain.rs"), "fn a() {}\nfn b() {}\n").unwrap();
    let opened = core.read_file(&repo.path().join("plain.rs")).await.unwrap();
    assert!(matches!(
        opened.content,
        FileContent::Text {
            minified: false,
            read_only: false,
            ..
        }
    ));
}

#[tokio::test(flavor = "multi_thread")]
async fn windows_line_endings_survive_an_edit_and_a_save() {
    let repo = git_repo();
    let fake = FakeAgent::new(r#"{"turns":[]}"#);
    let core = workspace(&fake, repo.path()).await;
    let path = repo.path().join("crlf.txt");
    std::fs::write(&path, "one\r\ntwo\r\n").unwrap();
    let opened = core.read_file(&path).await.unwrap();
    let FileContent::Text {
        text, line_ending, ..
    } = &opened.content
    else {
        panic!("text")
    };
    assert_eq!(text, "one\ntwo\n", "the editor works in \\n");
    assert_eq!(line_ending, "\r\n");

    core.save_file(
        &path,
        "one\ntwo\nthree\n",
        line_ending,
        over(&opened.version),
        "main",
    )
    .await
    .unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), b"one\r\ntwo\r\nthree\r\n");
}

#[tokio::test(flavor = "multi_thread")]
async fn mixed_line_endings_are_reported_and_made_one_kind_on_save() {
    let repo = git_repo();
    let fake = FakeAgent::new(r#"{"turns":[]}"#);
    let core = workspace(&fake, repo.path()).await;
    let path = repo.path().join("mixed.txt");
    std::fs::write(&path, "a\r\nb\r\nc\n").unwrap();
    let opened = core.read_file(&path).await.unwrap();
    assert!(matches!(
        &opened.content,
        FileContent::Text { line_ending, mixed_line_endings: true, text, .. }
            if line_ending == "\r\n" && text == "a\nb\nc\n"
    ));
    core.save_file(&path, "a\nb\nc\n", "\r\n", over(&opened.version), "main")
        .await
        .unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), b"a\r\nb\r\nc\r\n");
}
#[tokio::test(flavor = "multi_thread")]
async fn only_the_workspaces_files_and_the_settings_file_can_be_edited() {
    let repo = git_repo();
    let elsewhere = tempfile::tempdir().unwrap();
    let outside = elsewhere.path().join("secret.txt");
    std::fs::write(&outside, "not yours").unwrap();
    let fake = FakeAgent::new(r#"{"turns":[]}"#);
    let core = core_with_settings(&fake, "[editor]\nvim = true\n");
    core.open_workspace(repo.path()).await.unwrap();

    assert!(matches!(
        core.read_file(&outside).await,
        Err(CoreError::NotEditable(_))
    ));
    let escape = repo
        .path()
        .join("..")
        .join(elsewhere.path().file_name().unwrap())
        .join("secret.txt");
    assert!(matches!(
        core.read_file(&escape).await,
        Err(CoreError::NotEditable(_))
    ));
    assert!(matches!(
        core.save_file(&outside, "mine now", "\n", SaveOver::Anything, "main")
            .await,
        Err(CoreError::NotEditable(_))
    ));
    assert_eq!(std::fs::read_to_string(&outside).unwrap(), "not yours");

    let settings = core.read_file(&fake.settings_path()).await.unwrap();
    assert!(text_of(&settings.content).contains("vim = true"));
    assert!(core.settings().settings.editor.vim);

    // The repo's git folder isn't for hand edits.
    assert!(matches!(
        core.read_file(&repo.path().join(".git").join("config"))
            .await,
        Err(CoreError::NotEditable(_))
    ));
}

#[tokio::test(flavor = "multi_thread")]
async fn saving_the_settings_file_applies_it_at_once() {
    let repo = git_repo();
    let fake = FakeAgent::new(r#"{"turns":[]}"#);
    let core = core_with_settings(&fake, "[editor]\nvim = false\n");
    core.open_workspace(repo.path()).await.unwrap();
    let opened = core.read_file(&fake.settings_path()).await.unwrap();
    core.save_file(
        &fake.settings_path(),
        "[editor]\nvim = true\n",
        "\n",
        over(&opened.version),
        "main",
    )
    .await
    .unwrap();
    assert!(
        core.settings().settings.editor.vim,
        "no waiting for the file watch"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn text_that_isnt_utf8_is_a_placeholder_not_garbage() {
    let repo = git_repo();
    let fake = FakeAgent::new(r#"{"turns":[]}"#);
    let core = workspace(&fake, repo.path()).await;
    let path = repo.path().join("latin1.txt");
    std::fs::write(&path, b"caf\xe9\n").unwrap();
    assert!(matches!(
        core.read_file(&path).await.unwrap().content,
        FileContent::NotUtf8 { bytes: 5 }
    ));
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn a_dangling_link_out_of_the_worktree_is_refused() {
    let repo = git_repo();
    let elsewhere = tempfile::tempdir().unwrap();
    let fake = FakeAgent::new(r#"{"turns":[]}"#);
    let core = workspace(&fake, repo.path()).await;
    let link = repo.path().join("link.txt");
    std::os::unix::fs::symlink(elsewhere.path().join("not-yet.txt"), &link).unwrap();
    assert!(matches!(
        core.save_file(&link, "escape", "\n", SaveOver::Anything, "main")
            .await,
        Err(CoreError::NotEditable(_))
    ));
}

//! What one of the user's PRs conflicts in with the branch it goes into (the Dashboard opens it from
//! the PR's row), and the prompt that starts an Agent session to resolve it. GitHub only says that a
//! PR conflicts, not where, so the core merges the two branches as origin has them with
//! `git merge-tree`, which touches no Worktree.

use serde::{Deserialize, Serialize};

/// Where one PR conflicts with its base branch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MergeConflicts {
    pub number: u64,
    pub title: String,
    /// The PR's branch.
    pub head: String,
    /// The branch it goes into.
    pub base: String,
    /// Empty when the two merge cleanly now (GitHub's answer may lag behind a push).
    pub files: Vec<ConflictedFile>,
}

/// One file that merging the base branch in would leave conflicted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConflictedFile {
    pub path: String,
    /// git's kind of conflict ("content", "modify/delete", "rename/delete", …), if it named one.
    pub kind: Option<String>,
    /// What git said about it ("CONFLICT (modify/delete): a.rs deleted in … and modified in …").
    pub messages: Vec<String>,
}

/// Reads `git merge-tree --write-tree -z --name-only`'s output: the tree, each conflicted path, an
/// empty field, then its informational messages (how many paths, the paths, the message's type,
/// the message). Only the `CONFLICT` messages are kept, on each path they name.
pub(crate) fn conflicted_files(stdout: &str) -> Vec<ConflictedFile> {
    let mut fields = stdout.split('\0');
    fields.next(); // (The merged tree.)
    let mut files: Vec<ConflictedFile> = fields
        .by_ref()
        .take_while(|f| !f.is_empty())
        .map(|path| ConflictedFile {
            path: path.to_owned(),
            kind: None,
            messages: vec![],
        })
        .collect();
    while let Some(count) = fields.next().and_then(|n| n.parse::<usize>().ok()) {
        let paths: Vec<&str> = fields.by_ref().take(count).collect();
        let (Some(kind), Some(message)) = (fields.next(), fields.next()) else {
            break;
        };
        let Some(kind) = kind.strip_prefix("CONFLICT") else {
            continue;
        };
        let kind = kind.trim().trim_start_matches('(').trim_end_matches(')');
        // (Its type says "contents" where its message says "content".)
        let kind = if kind == "contents" { "content" } else { kind };
        for file in files
            .iter_mut()
            .filter(|f| paths.contains(&f.path.as_str()))
        {
            file.kind.get_or_insert_with(|| kind.to_owned());
            file.messages.push(message.trim().to_owned());
        }
    }
    files
}

/// The prompt that starts an Agent session, in a Worktree on the PR's branch, to merge its base
/// branch in and resolve the conflicts. It's left to the user to push, once they've looked.
pub fn merge_conflict_prompt(conflicts: &MergeConflicts) -> String {
    let MergeConflicts {
        number,
        title,
        head,
        base,
        files,
    } = conflicts;
    let mut text = format!(
        "PR #{number} ({title}) can't be merged: it conflicts with `{base}`. On its branch \
         `{head}` here, first bring it up to date with `origin/{head}`, then run \
         `git fetch origin {base}` and `git merge origin/{base}`. Resolve each conflict so that \
         what both sides meant still works together; if one side can't be kept without dropping \
         the other, stop and ask me. Then check it builds and its tests pass, and commit the \
         merge. Don't rebase or push: I'll look at it first.\n"
    );
    if !files.is_empty() {
        text.push_str("\nThe files that conflict:\n");
    }
    for file in files {
        match &file.kind {
            Some(kind) => text.push_str(&format!("\n## {} ({kind})\n", file.path)),
            None => text.push_str(&format!("\n## {}\n", file.path)),
        }
        for message in &file.messages {
            text.push_str(&format!("{message}\n"));
        }
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conflicted_files_have_git_s_kind_and_messages() {
        let out = "2711bb6e\0f.txt\0g.txt\0s p.txt\0\0\
            1\0f.txt\0Auto-merging\0Auto-merging f.txt\n\0\
            1\0f.txt\0CONFLICT (contents)\0CONFLICT (content): Merge conflict in f.txt\n\0\
            1\0g.txt\0CONFLICT (modify/delete)\0CONFLICT (modify/delete): g.txt deleted in pr and modified in main.\n\0\
            1\0s p.txt\0CONFLICT (contents)\0CONFLICT (add/add): Merge conflict in s p.txt\n\0";
        let files = conflicted_files(out);
        let kinds: Vec<_> = files
            .iter()
            .map(|f| (f.path.as_str(), f.kind.as_deref()))
            .collect();
        assert_eq!(
            kinds,
            [
                ("f.txt", Some("content")),
                ("g.txt", Some("modify/delete")),
                ("s p.txt", Some("content"))
            ]
        );
        assert_eq!(
            files[0].messages,
            ["CONFLICT (content): Merge conflict in f.txt"]
        );
    }

    #[test]
    fn a_clean_merge_has_no_conflicted_files() {
        assert_eq!(conflicted_files("2711bb6e\0"), []);
    }

    #[test]
    fn the_prompt_says_what_to_merge_and_each_file() {
        let prompt = merge_conflict_prompt(&MergeConflicts {
            number: 12,
            title: "Fix the parser".into(),
            head: "fix".into(),
            base: "main".into(),
            files: vec![ConflictedFile {
                path: "src/parse.rs".into(),
                kind: Some("content".into()),
                messages: vec!["CONFLICT (content): Merge conflict in src/parse.rs".into()],
            }],
        });
        assert!(prompt.contains("git merge origin/main"), "{prompt}");
        assert!(prompt.contains("## src/parse.rs (content)"), "{prompt}");
        assert!(prompt.contains("Don't rebase or push"), "{prompt}");
    }
}

//! CodeRabbit's review comments on the user's PRs: how many are still open (the Dashboard shows
//! them on the PR's row), what each says, and the prompt that starts an Agent session on the ones
//! chosen. CodeRabbit asks for changes in review threads on the code, mostly in reviews that only
//! comment, so its open threads (not GitHub's review decision) are what it still wants changed.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::dashboard::{Login, Nodes};

/// CodeRabbit's login: GraphQL gives it bare, the REST API with `[bot]`.
pub(crate) fn is_code_rabbit(login: &str) -> bool {
    matches!(login, "coderabbitai" | "coderabbitai[bot]")
}

/// What `gh api graphql` is asked for the open threads on the user's open PRs (`$q` is
/// `OPEN_SEARCH`).
pub(crate) const COUNT_QUERY: &str = "query($q: String!) {
  search(query: $q, type: ISSUE, first: 100) {
    nodes {
      ... on PullRequest {
        number
        reviewThreads(first: 100) { nodes { isResolved comments(first: 1) { nodes { author { login } } } } }
      }
    }
  }
}";

/// The user's open PRs in the Worktree's repo (`gh` fills it in).
pub(crate) const OPEN_SEARCH: &str = "repo:{owner}/{repo} is:pr is:open author:@me";

/// Reads `COUNT_QUERY`'s answer: by PR number, how many threads CodeRabbit started that aren't
/// resolved (PRs with none left out).
pub(crate) fn open_counts(stdout: &str) -> Result<HashMap<u64, u32>, String> {
    #[derive(Deserialize)]
    struct Answer {
        data: Data,
    }
    #[derive(Deserialize)]
    struct Data {
        search: Nodes<Pr>,
    }
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Pr {
        // (A search node that isn't a PR comes as `{}`.)
        number: Option<u64>,
        #[serde(default)]
        review_threads: Nodes<Thread>,
    }
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Thread {
        is_resolved: bool,
        #[serde(default)]
        comments: Nodes<Comment>,
    }
    let answer: Answer = serde_json::from_str(stdout.trim())
        .map_err(|e| format!("gh said something unexpected: {e}"))?;
    Ok(answer
        .data
        .search
        .nodes
        .into_iter()
        .filter_map(|pr| {
            let open = pr
                .review_threads
                .nodes
                .iter()
                .filter(|t| !t.is_resolved && started_by_code_rabbit(&t.comments.nodes))
                .count() as u32;
            (open > 0).then_some((pr.number?, open))
        })
        .collect())
}

fn started_by_code_rabbit<C: Author>(comments: &[C]) -> bool {
    comments
        .first()
        .and_then(|c| c.login())
        .is_some_and(is_code_rabbit)
}

trait Author {
    fn login(&self) -> Option<&str>;
}

/// CodeRabbit's open review comments on one PR.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodeRabbitFeedback {
    pub number: u64,
    pub title: String,
    pub comments: Vec<CodeRabbitComment>,
}

/// One review thread CodeRabbit started that isn't resolved.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodeRabbitComment {
    /// The thread's first comment's id: what a reply through GitHub's REST API goes to.
    pub id: u64,
    pub path: String,
    /// The line it's on now, else (outdated) the one it was made on.
    pub line: Option<u64>,
    /// The code it was made on has changed since.
    pub outdated: bool,
    /// How much it matters, as CodeRabbit heads the comment: `Critical`, `Major`, `Minor`, `Trivial`
    /// or `Info`.
    pub severity: Option<String>,
    /// The rest of that heading: its kind, its area, … ("Functional Correctness", "Quick win").
    pub tags: Vec<String>,
    /// What it's about in a line: its bold title, else its first line.
    pub headline: String,
    /// The rest, in Markdown: without CodeRabbit's hidden HTML comments, its analysis chains and
    /// static analysis (the scripts it ran to check the comment), the learnings it used and its prompt for AI agents,
    /// its other collapsed sections opened out and its HTML (icons, mostly) gone, as the HTML
    /// isn't rendered.
    pub body: String,
    /// What CodeRabbit wrote for an AI agent to act on.
    pub agent_prompt: Option<String>,
    pub url: String,
    /// What's been said on the thread since, in order.
    pub replies: Vec<CodeRabbitReply>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodeRabbitReply {
    pub author: String,
    pub body: String,
}

/// What `gh api graphql` is asked for a PR's title and review threads (`$owner`, `$name`, `$number`).
pub(crate) const THREAD_QUERY: &str = "query($owner: String!, $name: String!, $number: Int!) {
  repository(owner: $owner, name: $name) {
    pullRequest(number: $number) {
      title
      reviewThreads(first: 100) {
        nodes { isResolved isOutdated path line originalLine
          comments(first: 50) { nodes { databaseId url author { login } body } } }
      }
    }
  }
}";

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ThreadComment {
    #[serde(default)]
    database_id: u64,
    #[serde(default)]
    url: String,
    author: Option<Login>,
    #[serde(default)]
    body: String,
}

impl Author for ThreadComment {
    fn login(&self) -> Option<&str> {
        self.author.as_ref()?.login.as_deref()
    }
}

impl Author for Comment {
    fn login(&self) -> Option<&str> {
        self.author.as_ref()?.login.as_deref()
    }
}

#[derive(Deserialize)]
struct Comment {
    author: Option<Login>,
}

/// Reads `THREAD_QUERY`'s answer for PR `number`: CodeRabbit's threads that aren't resolved, in
/// the order GitHub gives them.
pub(crate) fn open_comments(number: u64, stdout: &str) -> Result<CodeRabbitFeedback, String> {
    #[derive(Deserialize)]
    struct Answer {
        data: Data,
    }
    #[derive(Deserialize)]
    struct Data {
        repository: Repository,
    }
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Repository {
        pull_request: PullRequest,
    }
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct PullRequest {
        #[serde(default)]
        title: String,
        review_threads: Nodes<Thread>,
    }
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Thread {
        is_resolved: bool,
        #[serde(default)]
        is_outdated: bool,
        path: String,
        line: Option<u64>,
        original_line: Option<u64>,
        #[serde(default)]
        comments: Nodes<ThreadComment>,
    }
    let answer: Answer = serde_json::from_str(stdout.trim())
        .map_err(|e| format!("gh said something unexpected: {e}"))?;
    let pr = answer.data.repository.pull_request;
    let comments = pr
        .review_threads
        .nodes
        .into_iter()
        .filter(|t| !t.is_resolved && started_by_code_rabbit(&t.comments.nodes))
        .filter_map(|t| {
            let mut said = t.comments.nodes.into_iter();
            let first = said.next()?;
            let read = read_comment(&first.body);
            Some(CodeRabbitComment {
                id: first.database_id,
                path: t.path,
                line: t.line.or(t.original_line),
                outdated: t.is_outdated,
                severity: read.severity,
                tags: read.tags,
                headline: read.headline,
                body: read.body,
                agent_prompt: read.agent_prompt,
                url: first.url,
                replies: said
                    .map(|c| CodeRabbitReply {
                        author: c.login().unwrap_or("ghost").to_string(),
                        body: without_html(&without_hidden(&c.body)),
                    })
                    .collect(),
            })
        })
        .collect();
    Ok(CodeRabbitFeedback {
        number,
        title: pr.title,
        comments,
    })
}

/// A CodeRabbit comment read into its parts.
struct ReadComment {
    severity: Option<String>,
    tags: Vec<String>,
    headline: String,
    body: String,
    agent_prompt: Option<String>,
}

/// How CodeRabbit names how much a comment matters.
const SEVERITIES: &[&str] = &["Critical", "Major", "Minor", "Trivial", "Info"];

/// Reads a CodeRabbit comment. It's headed by a line of labels, bold ones apart with `·` (`**<icon>
/// Minor** · **Functional Correctness** · **Quick win**`) or, before, italic ones apart with `|`
/// (`_⚠️ Potential issue_ | _🟠 Major_`), then its title in bold.
fn read_comment(raw: &str) -> ReadComment {
    let (rest, prompts) = split_details(raw, |summary| summary.contains("Prompt for AI Agents"));
    let (rest, _) = split_details(&rest, |summary| {
        // (What it ran to check the comment: before its title in comments since October 2026.)
        summary.contains("Analysis chain")
            || summary.contains("Supported by static analysis")
            || summary.contains("Learnings used")
    });
    let agent_prompt = prompts
        .first()
        .map(|block| {
            // (Without its summary: the "Prompt for AI Agents" it was picked by.)
            let block = match (block.find("<summary>"), block.find("</summary>")) {
                (Some(start), Some(end)) if start < end => {
                    format!("{}{}", &block[..start], &block[end + "</summary>".len()..])
                }
                _ => block.clone(),
            };
            fenced_or_text(&without_html(&without_hidden(&block)))
        })
        .filter(|p| !p.is_empty());
    let text = without_html(&without_hidden(&rest));
    let lines: Vec<&str> = text.lines().collect();
    let mut at = lines
        .iter()
        .position(|l| !l.trim().is_empty())
        .unwrap_or(lines.len());
    let mut labels: Vec<String> = vec![];
    if let Some(first) = lines.get(at).map(|l| l.trim()) {
        let parts: Option<Vec<&str>> = if first.starts_with('_') {
            Some(first.split('|').collect())
        } else if first.starts_with("**") && first.contains("** · **") {
            Some(first.split('·').collect())
        } else {
            None
        };
        if let Some(parts) = parts {
            labels = parts
                .into_iter()
                .map(|p| {
                    let p = p.trim().trim_matches(|c| c == '_' || c == '*').trim();
                    // (Without the emoji some begin with.)
                    p.trim_start_matches(|c: char| !c.is_alphanumeric())
                        .to_string()
                })
                .filter(|p| !p.is_empty())
                .collect();
            at += 1;
        }
    }
    let severity = labels
        .iter()
        .position(|l| SEVERITIES.iter().any(|s| s.eq_ignore_ascii_case(l)))
        .map(|i| labels.remove(i));
    while lines.get(at).is_some_and(|l| l.trim().is_empty()) {
        at += 1;
    }
    let bold = lines.get(at).and_then(|l| {
        let l = l.trim();
        let inner = l.strip_prefix("**")?.strip_suffix("**")?;
        (!inner.contains("**")).then(|| inner.to_string())
    });
    let (headline, body_from) = match bold {
        Some(title) => (title, at + 1),
        None => (
            lines
                .get(at)
                .map(|l| l.trim().to_string())
                .unwrap_or_default(),
            at,
        ),
    };
    ReadComment {
        severity,
        tags: labels,
        headline: headline.chars().take(200).collect(),
        body: lines
            .get(body_from..)
            .unwrap_or_default()
            .join("\n")
            .trim()
            .to_string(),
        agent_prompt,
    }
}

/// A comment of CodeRabbit's as Markdown to show on the code it's about: without its hidden HTML
/// comments, the scripts it ran, the learnings it used, its prompt for AI agents and its HTML,
/// its other collapsed sections opened out.
pub(crate) fn readable(raw: &str) -> String {
    let (rest, _) = split_details(raw, |summary| {
        summary.contains("Analysis chain")
            || summary.contains("Supported by static analysis")
            || summary.contains("Learnings used")
            || summary.contains("Prompt for AI Agents")
    });
    without_html(&without_hidden(&rest))
}

/// What's in a block's first fenced code block, else all its text.
fn fenced_or_text(block: &str) -> String {
    let mut lines = block.lines();
    if lines.by_ref().any(|l| l.trim_start().starts_with("```")) {
        let inside: Vec<&str> = lines
            .take_while(|l| !l.trim_start().starts_with("```"))
            .collect();
        return inside.join("\n").trim().to_string();
    }
    block.trim().to_string()
}

/// `body` without its HTML comments (CodeRabbit hides its bookkeeping in them).
fn without_hidden(body: &str) -> String {
    let mut out = String::new();
    let mut rest = body;
    while let Some(start) = rest.find("<!--") {
        out.push_str(&rest[..start]);
        rest = match rest[start..].find("-->") {
            Some(end) => &rest[start + end + 3..],
            None => "",
        };
    }
    out.push_str(rest);
    out.trim().to_string()
}

/// The HTML tags CodeRabbit lays its comments out with, which the Markdown shown and sent does
/// without.
const LAYOUT_TAGS: &[&str] = &[
    "details",
    "summary",
    "picture",
    "source",
    "img",
    "dl",
    "dd",
    "dt",
    "blockquote",
    "br",
    "sub",
    "sup",
    "div",
    "span",
    "p",
];

/// `body` without `LAYOUT_TAGS` (a `<summary>`'s text made bold instead) outside code: fenced
/// blocks and inline code keep theirs.
fn without_html(body: &str) -> String {
    let mut out = String::new();
    let mut fenced = false;
    for line in body.lines() {
        if line.trim_start().starts_with("```") {
            fenced = !fenced;
        }
        if !fenced {
            let mut in_code = false;
            let mut rest = line;
            while let Some(i) = rest.find(['<', '`']) {
                out.push_str(&rest[..i]);
                rest = &rest[i..];
                if rest.starts_with('`') {
                    in_code = !in_code;
                    out.push('`');
                    rest = &rest[1..];
                    continue;
                }
                let tag = rest[1..].trim_start_matches('/');
                let name: String = tag
                    .chars()
                    .take_while(|c| c.is_ascii_alphanumeric())
                    .collect();
                let end = rest.find('>');
                match end {
                    Some(end)
                        if !in_code
                            && LAYOUT_TAGS.contains(&name.to_ascii_lowercase().as_str()) =>
                    {
                        if name.eq_ignore_ascii_case("summary") {
                            out.push_str("**");
                        }
                        rest = &rest[end + 1..];
                    }
                    _ => {
                        out.push('<');
                        rest = &rest[1..];
                    }
                }
            }
            out.push_str(rest);
        } else {
            out.push_str(line);
        }
        out.push('\n');
    }
    // (A summary's text came out between two `**`: on its own line, with spaces trimmed.)
    let tidied: Vec<String> = out
        .lines()
        .map(|l| {
            match l
                .trim()
                .strip_prefix("**")
                .and_then(|l| l.strip_suffix("**"))
            {
                Some(inner) => format!("**{}**", inner.trim()),
                None => l.trim_end().to_string(),
            }
        })
        .collect();
    let mut text = tidied.join("\n");
    while text.contains("\n\n\n") {
        text = text.replace("\n\n\n", "\n\n");
    }
    text.trim().to_string()
}

/// `body` without the `<details>` blocks (nested ones within them too) whose summary `pick` says
/// to take out, and those blocks.
fn split_details(body: &str, pick: impl Fn(&str) -> bool) -> (String, Vec<String>) {
    let mut out = String::new();
    let mut picked = vec![];
    let mut rest = body;
    while let Some(start) = rest.find("<details>") {
        out.push_str(&rest[..start]);
        let block = &rest[start..];
        // Where it ends, counting the ones opened inside it.
        let mut depth = 0;
        let mut end = block.len();
        let mut at = 0;
        while at < block.len() {
            let open = block[at..].find("<details>").map(|i| at + i);
            let close = block[at..].find("</details>").map(|i| at + i);
            match (open, close) {
                (Some(o), Some(c)) if o < c => {
                    depth += 1;
                    at = o + "<details>".len();
                }
                (_, Some(c)) => {
                    depth -= 1;
                    at = c + "</details>".len();
                    if depth == 0 {
                        end = at;
                        break;
                    }
                }
                _ => break,
            }
        }
        let summary = block[..end]
            .find("<summary>")
            .and_then(|s| {
                let from = s + "<summary>".len();
                block[from..end]
                    .find("</summary>")
                    .map(|e| &block[from..from + e])
            })
            .unwrap_or_default();
        if pick(summary) {
            picked.push(block[..end].to_string());
        } else {
            out.push_str(&block[..end]);
        }
        rest = &block[end..];
    }
    out.push_str(rest);
    (out.trim().to_string(), picked)
}

/// The prompt that starts an Agent session on the comments of `feedback` whose ids are in
/// `chosen`: each with its file and line and what's been said since, the Agent to make the change
/// asked for or else reply on its thread saying why not.
pub fn code_rabbit_prompt(feedback: &CodeRabbitFeedback, chosen: &[u64]) -> String {
    let number = feedback.number;
    let mut text = format!(
        "CodeRabbit reviewed PR #{number} ({}) and asked for the changes below. For each one, \
         decide whether it's right. If it is, make the change. If it isn't, leave the code as it \
         is and reply on its thread on GitHub saying why, with \
         `gh api repos/{{owner}}/{{repo}}/pulls/{number}/comments/<comment id>/replies -f body='<why>'`.\n",
        feedback.title
    );
    for comment in feedback.comments.iter().filter(|c| chosen.contains(&c.id)) {
        let place = match comment.line {
            Some(line) => format!("{}:{line}", comment.path),
            None => comment.path.clone(),
        };
        let outdated = if comment.outdated { ", outdated" } else { "" };
        text.push_str(&format!(
            "\n## {place} (comment id {}{outdated})\n\n",
            comment.id
        ));
        let mut labels: Vec<&str> = comment.severity.iter().map(String::as_str).collect();
        labels.extend(comment.tags.iter().map(String::as_str));
        if !labels.is_empty() {
            text.push_str(&format!("{}\n\n", labels.join(" · ")));
        }
        text.push_str(&format!("**{}**\n\n", comment.headline));
        if !comment.body.is_empty() {
            text.push_str(&comment.body);
            text.push('\n');
        }
        if let Some(prompt) = &comment.agent_prompt {
            text.push_str("\nCodeRabbit's prompt for AI agents:\n\n");
            text.push_str(&format!("```\n{prompt}\n```\n"));
        }
        for reply in &comment.replies {
            text.push_str(&format!(
                "\nReply from {}:\n{}\n",
                reply.author,
                reply.body.trim()
            ));
        }
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_counts_are_of_threads_code_rabbit_started_and_not_resolved() {
        let out = r#"{"data":{"search":{"nodes":[
            {"number":4,"reviewThreads":{"nodes":[
                {"isResolved":false,"comments":{"nodes":[{"author":{"login":"coderabbitai"}}]}},
                {"isResolved":true,"comments":{"nodes":[{"author":{"login":"coderabbitai"}}]}},
                {"isResolved":false,"comments":{"nodes":[{"author":{"login":"ann"}}]}},
                {"isResolved":false,"comments":{"nodes":[{"author":{"login":"coderabbitai"}}]}}]}},
            {"number":7,"reviewThreads":{"nodes":[
                {"isResolved":false,"comments":{"nodes":[{"author":{"login":"ann"}}]}}]}},
            {}
        ]}}}"#;
        let counts = open_counts(out).unwrap();
        assert_eq!(counts, HashMap::from([(4, 2)]));
    }

    const THREADS: &str = r#"{"data":{"repository":{"pullRequest":{"title":"Fix the parser","reviewThreads":{"nodes":[
        {"isResolved":false,"isOutdated":false,"path":"src/parse.rs","line":42,"originalLine":40,
         "comments":{"nodes":[
            {"databaseId":101,"url":"https://github.com/o/r/pull/12#discussion_r101","author":{"login":"coderabbitai"},
             "body":"_⚠️ Potential issue_ | _🟠 Major_\n\n**Guard against an empty input.**\n\nIt panics on `\"\"`.\n\n<details>\n<summary>🧩 Analysis chain</summary>\n\n<details><summary>nested</summary>rg parse</details>\n\nScript output\n</details>\n\n<details>\n<summary>🤖 Prompt for AI Agents</summary>\n\nIn src/parse.rs around line 42, return early when empty.\n</details>\n\n<!-- fingerprinting:abc -->"},
            {"databaseId":102,"url":"u","author":{"login":"me"},"body":"Is it reachable?"}]}},
        {"isResolved":true,"isOutdated":false,"path":"src/old.rs","line":3,"originalLine":3,
         "comments":{"nodes":[{"databaseId":103,"url":"u","author":{"login":"coderabbitai"},"body":"**Done.**"}]}},
        {"isResolved":false,"isOutdated":true,"path":"src/lex.rs","line":null,"originalLine":9,
         "comments":{"nodes":[{"databaseId":104,"url":"u","author":{"login":"coderabbitai"},"body":"Rename `x`."}]}},
        {"isResolved":false,"isOutdated":false,"path":"src/main.rs","line":1,"originalLine":1,
         "comments":{"nodes":[{"databaseId":105,"url":"u","author":{"login":"ann"},"body":"Why?"}]}}
    ]}}}}}"#;

    #[test]
    fn open_comments_are_code_rabbits_unresolved_threads_with_their_heading() {
        let feedback = open_comments(12, THREADS).unwrap();
        assert_eq!(feedback.title, "Fix the parser");
        let seen: Vec<_> = feedback
            .comments
            .iter()
            .map(|c| (c.id, c.path.as_str(), c.line, c.outdated))
            .collect();
        assert_eq!(
            seen,
            [
                (101, "src/parse.rs", Some(42), false),
                (104, "src/lex.rs", Some(9), true)
            ]
        );
        let first = &feedback.comments[0];
        assert_eq!(first.severity.as_deref(), Some("Major"));
        assert_eq!(first.tags, ["Potential issue"]);
        assert_eq!(first.headline, "Guard against an empty input.");
        assert_eq!(first.body, "It panics on `\"\"`.");
        assert_eq!(
            first.agent_prompt.as_deref(),
            Some("In src/parse.rs around line 42, return early when empty.")
        );
        assert_eq!(
            first.replies,
            [CodeRabbitReply {
                author: "me".into(),
                body: "Is it reachable?".into()
            }]
        );
        assert_eq!(feedback.comments[1].severity, None);
        assert_eq!(feedback.comments[1].headline, "Rename `x`.");
    }

    /// As CodeRabbit writes them now (October 2026): icons in HTML, labels apart with `·`, and its
    /// prompt for AI agents in a fenced block between markers.
    const NOW: &str = "<!-- coderabbit-ai-generated:v1 -->\n\n**<picture><img src=\"https://storage.googleapis.com/coderabbit_public_assets/review-comment-icons-v1/amber-16.svg\" width=\"16\" height=\"16\" align=\"absmiddle\" alt=\"\"></picture> Minor** · **Functional Correctness** · **Quick win**\n\n**Find the nearest React-owned ancestor.**\n\nIf a component inserts DOM elements, those have no `__reactFiber$` key, so `<Overlay>` shows no name.\n<!-- coderabbit-global-learning v1 -->\n\n<!-- ai-agent-prompt:start -->\n\n<details>\n<summary><picture><source media=\"(prefers-color-scheme: dark)\" srcset=\"bot-dark.svg\"><img src=\"bot.svg\" width=\"16\" height=\"16\" align=\"absmiddle\" alt=\"\"></picture> Prompt for AI Agents</summary>\n<dl>\n<dd>\n\n```\nReview comment at get-component-name.ts at\nline 23:\nCheck the ancestors for a <div> fiber.\n```\n\n</dd>\n</dl>\n</details>\n\n<!-- ai-agent-prompt:end -->\n\n<!-- fingerprinting:phantom:poseidon:wombat -->\n\n_Source: Learnings_\n\n<!-- This is an auto-generated comment by CodeRabbit -->";

    #[test]
    fn a_comment_as_code_rabbit_writes_them_now_is_read_into_its_parts() {
        let read = read_comment(NOW);
        assert_eq!(read.severity.as_deref(), Some("Minor"));
        assert_eq!(read.tags, ["Functional Correctness", "Quick win"]);
        assert_eq!(read.headline, "Find the nearest React-owned ancestor.");
        assert_eq!(
            read.body,
            "If a component inserts DOM elements, those have no `__reactFiber$` key, so \
             `<Overlay>` shows no name.\n\n_Source: Learnings_"
        );
        assert_eq!(
            read.agent_prompt.as_deref(),
            Some("Review comment at get-component-name.ts at\nline 23:\nCheck the ancestors for a <div> fiber.")
        );
    }

    /// With the scripts CodeRabbit ran to check it, collapsed between its labels and its title.
    const WITH_STATIC_ANALYSIS: &str = "<!-- coderabbit-ai-generated:v1 -->\n\n**<picture><img src=\"major-16.svg\" alt=\"\"></picture> Major** · **Maintainability & Code Quality** · **Heavy lift**\n\n<details>\n<summary><picture><img src=\"review-details.svg\" alt=\"\"></picture> Supported by static analysis</summary>\n<dl>\n<dd>\n\nScript executed:\n\n```bash\nrg -n 'logAction\\(' server/src\n```\n\nRepository: o/r\n\nLength of output: 4792\n\n---\n\n</dd>\n</dl>\n</details>\n\n**Use `securedMutation` for this changed endpoint.**\n\nThe handler still uses `endpoint`.\n\n<details>\n<summary>Suggested fix</summary>\n<dl>\n<dd>\n\n```diff\n-export default endpoint(\n+export default securedMutation(\n```\n\n</dd>\n</dl>\n</details>\n\n<!-- This is an auto-generated comment by CodeRabbit -->";

    #[test]
    fn a_comment_supported_by_static_analysis_is_read_without_its_scripts() {
        let read = read_comment(WITH_STATIC_ANALYSIS);
        assert_eq!(read.severity.as_deref(), Some("Major"));
        assert_eq!(read.tags, ["Maintainability & Code Quality", "Heavy lift"]);
        assert_eq!(
            read.headline,
            "Use `securedMutation` for this changed endpoint."
        );
        assert_eq!(
            read.body,
            "The handler still uses `endpoint`.\n\n**Suggested fix**\n\n```diff\n-export default endpoint(\n+export default securedMutation(\n```"
        );
    }

    #[test]
    fn a_comment_shown_on_its_code_has_no_scripts_prompt_or_html() {
        assert_eq!(
            readable(WITH_STATIC_ANALYSIS),
            "**Major** · **Maintainability & Code Quality** · **Heavy lift**\n\n**Use `securedMutation` for this changed endpoint.**\n\nThe handler still uses `endpoint`.\n\n**Suggested fix**\n\n```diff\n-export default endpoint(\n+export default securedMutation(\n```"
        );
        let shown = readable(NOW);
        assert!(shown.starts_with("**Minor** · **Functional Correctness** · **Quick win**"));
        assert!(!shown.contains("Prompt for AI Agents") && !shown.contains("<picture>"));
    }

    #[test]
    fn the_prompt_has_the_chosen_comments_without_analysis_chains() {
        let feedback = open_comments(12, THREADS).unwrap();
        let prompt = code_rabbit_prompt(&feedback, &[101]);
        assert!(prompt.contains("#12 (Fix the parser)"), "{prompt}");
        assert!(
            prompt.contains("gh api repos/{owner}/{repo}/pulls/12/comments/<comment id>/replies"),
            "{prompt}"
        );
        assert!(
            prompt.contains("## src/parse.rs:42 (comment id 101)"),
            "{prompt}"
        );
        assert!(prompt.contains("Major · Potential issue"), "{prompt}");
        assert!(
            prompt.contains("**Guard against an empty input.**"),
            "{prompt}"
        );
        assert!(prompt.contains("return early when empty"), "{prompt}");
        assert!(!prompt.contains("Analysis chain"), "{prompt}");
        assert!(!prompt.contains("rg parse"), "{prompt}");
        assert!(!prompt.contains("Script output"), "{prompt}");
        assert!(
            prompt.contains("Reply from me:\nIs it reachable?"),
            "{prompt}"
        );
        assert!(!prompt.contains("src/lex.rs"), "{prompt}");

        let both = code_rabbit_prompt(&feedback, &[101, 104]);
        assert!(
            both.contains("## src/lex.rs:9 (comment id 104, outdated)"),
            "{both}"
        );
    }
}

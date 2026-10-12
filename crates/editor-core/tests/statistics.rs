//! Issue 47: the Dashboard's statistics, asked of a fake `gh`, and the period kept per Workspace.

mod support;

#[cfg(unix)]
use editor_core::CoreError;
use support::*;

#[cfg(unix)]
const DAY: u64 = 86_400;

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn statistics_are_read_from_gh_for_the_period_and_the_one_before() {
    let setup = RepoWithOrigin::new();
    let fake = FakeAgent::new(r#"{"turns":[]}"#);
    let page = |next: Option<&str>, nodes: String| {
        let info = match next {
            Some(cursor) => format!(r#"{{"hasNextPage":true,"endCursor":"{cursor}"}}"#),
            None => r#"{"hasNextPage":false,"endCursor":null}"#.to_owned(),
        };
        format!(
            r#"{{"data":{{"viewer":{{"login":"me"}},"search":{{"pageInfo":{info},"nodes":[{nodes}]}}}}}}"#
        )
    };
    let authored = |created: u64, merged: Option<u64>, reviews: &str| {
        let merged_at = merged.map_or("null".to_owned(), |m| format!(r#""{}""#, utc_ago(m)));
        let state = if merged.is_some() { "MERGED" } else { "OPEN" };
        format!(
            r#"{{"createdAt":"{}","mergedAt":{merged_at},"isDraft":false,"state":"{state}",
                "timelineItems":{{"nodes":[]}},"reviews":{{"nodes":[{reviews}]}}}}"#,
            utc_ago(created)
        )
    };
    let review = |login: &str, ago: u64| {
        format!(
            r#"{{"submittedAt":"{}","author":{{"__typename":"User","login":"{login}"}}}}"#,
            utc_ago(ago)
        )
    };
    let first = page(
        Some("c1"),
        authored(2 * DAY, Some(DAY), &review("ann", DAY + DAY / 2)),
    );
    let second = page(
        None,
        [
            authored(12 * DAY, Some(10 * DAY), ""),
            authored(3 * DAY, None, &review("bot[bot]", 2 * DAY)),
        ]
        .join(","),
    );
    let reviewed = page(
        None,
        format!(
            r#"{{"author":{{"login":"ann"}},"reviews":{{"nodes":[{}]}}}}"#,
            review("me", 2 * DAY)
        ),
    );
    let open = format!(
        r#"{{"data":{{"search":{{"nodes":[{{"number":4,"title":"Old one","createdAt":"{}"}}]}}}}}}"#,
        utc_ago(40 * DAY)
    );
    let requests = format!(
        r#"{{"data":{{"viewer":{{"login":"me"}},"search":{{"nodes":[
          {{"number":9,"title":"Asked","url":"u9","author":{{"login":"ann"}},"headRefName":"feat",
            "latestReviews":{{"nodes":[]}},
            "timelineItems":{{"nodes":[{{"createdAt":"{}","requestedReviewer":{{"login":"me"}}}}]}},
            "commits":{{"nodes":[]}}}}]}}}}}}"#,
        utc_ago(5 * DAY)
    );
    let gh = FakeGh::new(&format!(
        r#"case "$*" in
*"is:open author:@me"*) cat <<'JSON'
{open}
JSON
;;
*"reviewed-by:@me"*) cat <<'JSON'
{reviewed}
JSON
;;
*"user-review-requested:@me"*) cat <<'JSON'
{requests}
JSON
;;
*"after=c1"*) cat <<'JSON'
{second}
JSON
;;
*"author:@me"*) cat <<'JSON'
{first}
JSON
;;
esac"#
    ));
    let core = core_with_gh(&fake, &gh);
    let root = core.open_workspace(&setup.repo()).await.unwrap().root;

    let stats = core.statistics(&root, 7).await.unwrap();
    assert_eq!(stats.days, 7);
    assert_eq!((stats.merged.now, stats.merged.before), (1, 1));
    assert_eq!(stats.time_to_first_review.now.count, 1);
    // Half a day, give or take a second ticking between the fake's times.
    let wait = stats.time_to_first_review.now.seconds.unwrap();
    assert!((wait - DAY as i64 / 2).abs() <= 2, "{wait}");
    assert_eq!(stats.still_waiting, 1);
    assert_eq!(stats.reviews_given.now, 1);
    assert_eq!(stats.oldest_open.map(|pr| pr.number), Some(4));
    assert_eq!(stats.longest_request.map(|pr| pr.number), Some(9));

    let calls = gh.calls();
    assert_eq!(calls.len(), 5, "{calls:?}");
    assert!(calls.iter().all(|call| call[..2] == ["api", "graphql"]));
    let asked = |text: &str| calls.iter().filter(|c| c.join(" ").contains(text)).count();
    assert_eq!(asked("after=c1"), 1, "{calls:?}");
    assert_eq!(asked("updated:>="), 3, "{calls:?}");
    assert_eq!(asked("reviewed-by:@me -author:@me"), 1, "{calls:?}");
    assert_eq!(asked("sort:created-asc"), 1, "{calls:?}");

    assert!(matches!(
        core.statistics(&root, 14).await,
        Err(CoreError::UnknownPeriod(14))
    ));
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn statistics_say_when_gh_cant_be_used() {
    let setup = RepoWithOrigin::new();
    let fake = FakeAgent::new(r#"{"turns":[]}"#);
    let gh = FakeGh::new(
        "echo 'To get started with GitHub CLI, please run:  gh auth login' >&2\nexit 1",
    );
    let core = core_with_gh(&fake, &gh);
    let root = core.open_workspace(&setup.repo()).await.unwrap().root;

    let error = core.statistics(&root, 30).await.unwrap_err().to_string();
    assert!(error.contains("gh auth login"), "{error}");
    assert!(!error.contains("Creating a PR"), "{error}");
}

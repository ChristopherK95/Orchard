//! Statistics on the Dashboard (issue 47): the figures of the user's work in the Workspace's repo
//! over a period, and the period before it, worked out from what `gh` answers. Times are GitHub's
//! ISO strings read by hand into seconds since the Unix epoch (no date crate).

use std::path::Path;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::dashboard::{self, Login, Nodes, ReviewRequest};
use crate::pull_request::gh_text;

/// A day in seconds.
const DAY: i64 = 86_400;

/// The periods the statistics can cover, in days.
pub(crate) const PERIODS: [u32; 3] = [7, 30, 90];

/// The period they cover unless the Workspace has another.
pub(crate) const DEFAULT_PERIOD: u32 = 30;

/// Reads GitHub's `YYYY-MM-DDTHH:MM:SSZ` (fractional seconds ignored) into seconds since the Unix
/// epoch; anything else is `None`.
pub(crate) fn parse_utc(text: &str) -> Option<i64> {
    let bytes = text.as_bytes();
    if bytes.len() < 20 || !text.ends_with('Z') {
        return None;
    }
    let number = |from: usize, to: usize| -> Option<i64> {
        let part = text.get(from..to)?;
        part.bytes().all(|b| b.is_ascii_digit()).then_some(())?;
        part.parse().ok()
    };
    let separators = [(4, b'-'), (7, b'-'), (10, b'T'), (13, b':'), (16, b':')];
    if separators.iter().any(|&(at, sep)| bytes[at] != sep) {
        return None;
    }
    let fraction = text.get(19..text.len() - 1)?;
    if !(fraction.is_empty()
        || (fraction.len() > 1
            && fraction.starts_with('.')
            && fraction[1..].bytes().all(|b| b.is_ascii_digit())))
    {
        return None;
    }
    let (year, month, day) = (number(0, 4)?, number(5, 7)?, number(8, 10)?);
    let (hour, minute, second) = (number(11, 13)?, number(14, 16)?, number(17, 19)?);
    if !(1..=12).contains(&month)
        || !(1..=31).contains(&day)
        || hour > 23
        || minute > 59
        || second > 60
    {
        return None;
    }
    Some(days_from_civil(year, month, day) * DAY + hour * 3600 + minute * 60 + second)
}

/// The UTC date of a time, `YYYY-MM-DD`, as GitHub's search qualifiers take it.
pub(crate) fn utc_date(secs: i64) -> String {
    let (year, month, day) = civil_from_days(secs.div_euclid(DAY));
    format!("{year:04}-{month:02}-{day:02}")
}

/// Days since the Unix epoch of a date in the proleptic Gregorian calendar.
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let year_of_era = year - era * 400;
    let day_of_year = (153 * (month + if month > 2 { -3 } else { 9 }) + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

/// The date `days` after the Unix epoch: the inverse of `days_from_civil`.
fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let days = days + 719_468;
    let era = days.div_euclid(146_097);
    let day_of_era = days - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let shifted_month = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * shifted_month + 2) / 5 + 1;
    let month = if shifted_month < 10 {
        shifted_month + 3
    } else {
        shifted_month - 9
    };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

/// The period the figures cover, `(now − P, now]`, and the one before it, `(now − 2P, now − P]`,
/// P being `days` days.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Periods {
    pub(crate) now: i64,
    pub(crate) days: u32,
}

impl Periods {
    fn length(&self) -> i64 {
        i64::from(self.days) * DAY
    }

    pub(crate) fn in_current(&self, t: i64) -> bool {
        self.now - self.length() < t && t <= self.now
    }

    pub(crate) fn in_previous(&self, t: i64) -> bool {
        self.now - 2 * self.length() < t && t <= self.now - self.length()
    }

    /// The date the searches ask from: GitHub's qualifiers are day-granular, so they ask from the
    /// day the previous period starts and the exact times are filtered here.
    pub(crate) fn search_from(&self) -> String {
        utc_date(self.now - 2 * self.length())
    }
}

/// A median time in seconds and how many PRs it's over (none: nothing to count).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Median {
    pub seconds: Option<i64>,
    pub count: u32,
}

/// The middle value; with an even count, the mean of the middle two (rounded down).
pub(crate) fn median(mut values: Vec<i64>) -> Median {
    values.sort_unstable();
    let count = values.len();
    let seconds = match count {
        0 => None,
        _ if count % 2 == 1 => Some(values[count / 2]),
        _ => Some((values[count / 2 - 1] + values[count / 2]).div_euclid(2)),
    };
    Median {
        seconds,
        count: count as u32,
    }
}

/// A figure for the period and for the one before it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Compared<T> {
    pub now: T,
    pub before: T,
}

/// A PR going to draft (`ConvertToDraftEvent`) or becoming ready for review
/// (`ReadyForReviewEvent`), from its timeline.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct DraftEvent {
    pub(crate) at: i64,
    pub(crate) to_draft: bool,
}

fn sorted(events: &[DraftEvent]) -> Vec<DraftEvent> {
    let mut events = events.to_vec();
    events.sort_by_key(|e| e.at);
    events
}

/// When a PR first became ready for review: when it was opened, unless it was opened as a draft
/// (its first event makes it ready), then that event's; `None` if it never has been.
pub(crate) fn first_ready(created: i64, is_draft_now: bool, events: &[DraftEvent]) -> Option<i64> {
    match sorted(events).first() {
        Some(first) if !first.to_draft => Some(first.at),
        Some(_) => Some(created),
        None => (!is_draft_now).then_some(created),
    }
}

/// How long a PR was ready for review from when it first was to `until`: every spell it spent as
/// a draft in between is left out. 0 if it never was ready.
pub(crate) fn ready_time(
    created: i64,
    is_draft_now: bool,
    events: &[DraftEvent],
    until: i64,
) -> i64 {
    let Some(start) = first_ready(created, is_draft_now, events) else {
        return 0;
    };
    let mut total = 0;
    let mut ready_since = Some(start);
    for event in sorted(events) {
        if event.at < start || event.at >= until {
            continue;
        }
        match (event.to_draft, ready_since) {
            (true, Some(since)) => {
                total += event.at - since;
                ready_since = None;
            }
            (false, None) => ready_since = Some(event.at),
            _ => {}
        }
    }
    if let Some(since) = ready_since {
        total += until - since;
    }
    total.max(0)
}

/// One of the user's PRs, as the authored search reads it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AuthoredPr {
    pub(crate) created_at: i64,
    pub(crate) merged_at: Option<i64>,
    pub(crate) open: bool,
    pub(crate) is_draft: bool,
    pub(crate) draft_events: Vec<DraftEvent>,
    pub(crate) reviews: Vec<Reviewed>,
}

/// A submitted review: when, and by whom.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Reviewed {
    pub(crate) at: i64,
    pub(crate) login: String,
    pub(crate) bot: bool,
}

/// A PR the user reviewed: its author, and when the user's reviews were submitted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ReviewedPr {
    pub(crate) author: String,
    pub(crate) mine: Vec<i64>,
}

/// A PR that has waited since a time: the user's oldest open PR, or the review request to the
/// user that has waited longest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Waiting {
    pub number: u64,
    pub title: String,
    /// As GitHub gave it (ISO).
    pub since: String,
}

/// The six figures of the Dashboard's Statistics, for `days` days.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Statistics {
    pub days: u32,
    /// The user's PRs merged, into any base branch.
    pub merged: Compared<u32>,
    /// From first ready for review to merged, draft spells left out.
    pub time_to_merge: Compared<Median>,
    /// From first ready to the first review by someone else (bots left out), over the PRs that
    /// first became ready in the period.
    pub time_to_first_review: Compared<Median>,
    /// The period's open, non-draft PRs that nobody else has reviewed yet.
    pub still_waiting: u32,
    /// PRs by others the user reviewed (several reviews of one count once).
    pub reviews_given: Compared<u32>,
    pub oldest_open: Option<Waiting>,
    pub longest_request: Option<Waiting>,
}

/// Works out the figures from what the searches found.
pub(crate) fn figures(
    periods: Periods,
    viewer: &str,
    authored: &[AuthoredPr],
    reviewed: &[ReviewedPr],
    oldest_open: Option<Waiting>,
    requests: &[ReviewRequest],
) -> Statistics {
    let (merged, time_to_merge) = merges(periods, authored);
    let (time_to_first_review, still_waiting) = first_reviews(periods, viewer, authored);
    Statistics {
        days: periods.days,
        merged,
        time_to_merge,
        time_to_first_review,
        still_waiting,
        reviews_given: reviews_given(periods, viewer, reviewed),
        oldest_open,
        longest_request: longest_request(requests),
    }
}

/// Values sorted into the period and the one before by their times (others dropped).
fn split(periods: Periods, timed: impl IntoIterator<Item = (i64, i64)>) -> Compared<Vec<i64>> {
    let mut split = Compared {
        now: vec![],
        before: vec![],
    };
    for (at, value) in timed {
        if periods.in_current(at) {
            split.now.push(value);
        } else if periods.in_previous(at) {
            split.before.push(value);
        }
    }
    split
}

/// How many PRs were merged in each period, and the median time to merge.
fn merges(periods: Periods, authored: &[AuthoredPr]) -> (Compared<u32>, Compared<Median>) {
    let times = split(
        periods,
        authored.iter().filter_map(|pr| {
            let merged_at = pr.merged_at?;
            let time = ready_time(pr.created_at, pr.is_draft, &pr.draft_events, merged_at);
            Some((merged_at, time))
        }),
    );
    let counts = Compared {
        now: times.now.len() as u32,
        before: times.before.len() as u32,
    };
    let medians = Compared {
        now: median(times.now),
        before: median(times.before),
    };
    (counts, medians)
}

/// The median time to first review over the PRs that first became ready in each period, and how
/// many of this period's open, non-draft ones nobody has reviewed yet. Only reviews by someone
/// other than the user, and not a bot, submitted since the PR was first ready count.
fn first_reviews(
    periods: Periods,
    viewer: &str,
    authored: &[AuthoredPr],
) -> (Compared<Median>, u32) {
    let mut still_waiting = 0;
    let mut waits = vec![];
    for pr in authored {
        let Some(ready) = first_ready(pr.created_at, pr.is_draft, &pr.draft_events) else {
            continue;
        };
        let first = pr
            .reviews
            .iter()
            .filter(|r| !r.bot && r.login != viewer && r.at >= ready)
            .map(|r| r.at)
            .min();
        match first {
            Some(at) => waits.push((ready, at - ready)),
            None if pr.open && !pr.is_draft && periods.in_current(ready) => still_waiting += 1,
            None => {}
        }
    }
    let waits = split(periods, waits);
    let medians = Compared {
        now: median(waits.now),
        before: median(waits.before),
    };
    (medians, still_waiting)
}

/// How many PRs by others the user reviewed in each period, each counted once per period.
fn reviews_given(periods: Periods, viewer: &str, reviewed: &[ReviewedPr]) -> Compared<u32> {
    let mut given = Compared { now: 0, before: 0 };
    for pr in reviewed.iter().filter(|pr| pr.author != viewer) {
        if pr.mine.iter().any(|&at| periods.in_current(at)) {
            given.now += 1;
        }
        if pr.mine.iter().any(|&at| periods.in_previous(at)) {
            given.before += 1;
        }
    }
    given
}

/// The review request to the user asked longest ago (none says when: none).
fn longest_request(requests: &[ReviewRequest]) -> Option<Waiting> {
    requests
        .iter()
        .filter_map(|r| {
            let since = r.requested_at.as_ref()?;
            Some((parse_utc(since)?, r, since))
        })
        .min_by_key(|(at, ..)| *at)
        .map(|(_, r, since)| Waiting {
            number: r.number,
            title: r.title.clone(),
            since: since.clone(),
        })
}

/// What `gh api graphql` is asked for the user's PRs: `$q` is `authored_search`, `$after` the
/// page's cursor.
pub(crate) const AUTHORED_QUERY: &str = "query($q: String!, $after: String) {
  viewer { login }
  search(query: $q, type: ISSUE, first: 100, after: $after) {
    pageInfo { hasNextPage endCursor }
    nodes {
      ... on PullRequest {
        createdAt mergedAt isDraft state
        timelineItems(itemTypes: [READY_FOR_REVIEW_EVENT, CONVERT_TO_DRAFT_EVENT], first: 50) {
          nodes { __typename ... on ReadyForReviewEvent { createdAt } ... on ConvertToDraftEvent { createdAt } }
        }
        reviews(first: 50) { nodes { submittedAt author { __typename login } } }
      }
    }
  }
}";

/// The search for the user's PRs (`gh` fills in the Worktree's repo): any PR merged or readied
/// since `from` has been updated since.
pub(crate) fn authored_search(from: &str) -> String {
    format!("repo:{{owner}}/{{repo}} is:pr author:@me updated:>={from}")
}

/// What `gh api graphql` is asked for the PRs the user reviewed (`$q`: `reviewed_search`).
pub(crate) const REVIEWED_QUERY: &str = "query($q: String!, $after: String) {
  viewer { login }
  search(query: $q, type: ISSUE, first: 100, after: $after) {
    pageInfo { hasNextPage endCursor }
    nodes {
      ... on PullRequest {
        author { login }
        reviews(last: 100) { nodes { submittedAt author { login } } }
      }
    }
  }
}";

/// The search for others' PRs the user reviewed.
pub(crate) fn reviewed_search(from: &str) -> String {
    format!("repo:{{owner}}/{{repo}} is:pr reviewed-by:@me -author:@me updated:>={from}")
}

/// What `gh api graphql` is asked for the user's oldest open PR (`$q`: `OPEN_SEARCH`). Only `$q`:
/// GitHub rejects a variable the query doesn't use.
pub(crate) const OPEN_QUERY: &str = "query($q: String!) {
  search(query: $q, type: ISSUE, first: 1) {
    nodes { ... on PullRequest { number title createdAt } }
  }
}";

/// The search for the user's open PRs, the oldest first (drafts too: they're open).
pub(crate) const OPEN_SEARCH: &str =
    "repo:{owner}/{repo} is:pr is:open author:@me sort:created-asc";

/// A page of a search's answer: who the user is, what it found, and the next page's cursor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Page<T> {
    pub(crate) viewer: String,
    pub(crate) items: Vec<T>,
    pub(crate) next: Option<String>,
}

/// Reads a paged search's answer, each node through `item` (`None` drops it).
fn page<N: DeserializeOwned, T>(
    stdout: &str,
    mut item: impl FnMut(N, &str) -> Option<T>,
) -> Result<Page<T>, String> {
    #[derive(Deserialize)]
    struct Answer<N> {
        data: Data<N>,
    }
    #[derive(Deserialize)]
    struct Data<N> {
        #[serde(default)]
        viewer: Login,
        search: Search<N>,
    }
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Search<N> {
        #[serde(default)]
        page_info: PageInfo,
        #[serde(default = "Vec::new")]
        nodes: Vec<N>,
    }
    #[derive(Deserialize, Default)]
    #[serde(default, rename_all = "camelCase")]
    struct PageInfo {
        has_next_page: bool,
        end_cursor: Option<String>,
    }
    let answer: Answer<N> = serde_json::from_str(stdout.trim())
        .map_err(|e| format!("gh said something unexpected: {e}"))?;
    let viewer = answer.data.viewer.login.unwrap_or_default();
    let search = answer.data.search;
    let items = search
        .nodes
        .into_iter()
        .filter_map(|node| item(node, &viewer))
        .collect();
    let next = search
        .page_info
        .end_cursor
        .filter(|_| search.page_info.has_next_page);
    Ok(Page {
        viewer,
        items,
        next,
    })
}

/// A review's author with what they are: a bot (`__typename` "Bot", or a login ending `[bot]`) or
/// a person.
#[derive(Deserialize, Default)]
#[serde(default)]
struct Author {
    #[serde(rename = "__typename")]
    typename: Option<String>,
    login: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ReviewNode<A> {
    submitted_at: Option<String>,
    author: Option<A>,
}

/// Reads an `AUTHORED_QUERY` page. Pending reviews (not submitted) are skipped; a deleted user's
/// review counts as someone else's.
pub(crate) fn authored_page(stdout: &str) -> Result<Page<AuthoredPr>, String> {
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Pr {
        created_at: String,
        merged_at: Option<String>,
        #[serde(default)]
        is_draft: bool,
        #[serde(default)]
        state: String,
        #[serde(default)]
        timeline_items: Nodes<Event>,
        #[serde(default)]
        reviews: Nodes<ReviewNode<Author>>,
    }
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Event {
        #[serde(rename = "__typename")]
        typename: String,
        created_at: Option<String>,
    }
    page(stdout, |pr: Pr, _| {
        Some(AuthoredPr {
            created_at: parse_utc(&pr.created_at)?,
            merged_at: pr.merged_at.as_deref().and_then(parse_utc),
            open: pr.state == "OPEN",
            is_draft: pr.is_draft,
            draft_events: pr
                .timeline_items
                .nodes
                .into_iter()
                .filter_map(|event| {
                    let to_draft = match event.typename.as_str() {
                        "ConvertToDraftEvent" => true,
                        "ReadyForReviewEvent" => false,
                        _ => return None,
                    };
                    let at = parse_utc(event.created_at.as_deref()?)?;
                    Some(DraftEvent { at, to_draft })
                })
                .collect(),
            reviews: pr
                .reviews
                .nodes
                .into_iter()
                .filter_map(|review| {
                    let at = parse_utc(review.submitted_at.as_deref()?)?;
                    let author = review.author.unwrap_or_default();
                    let login = author.login.unwrap_or_else(|| "ghost".into());
                    let bot = author.typename.as_deref() == Some("Bot") || login.ends_with("[bot]");
                    Some(Reviewed { at, login, bot })
                })
                .collect(),
        })
    })
}

/// Reads a `REVIEWED_QUERY` page: each PR's author, and when the user's own reviews of it were
/// submitted.
pub(crate) fn reviewed_page(stdout: &str) -> Result<Page<ReviewedPr>, String> {
    #[derive(Deserialize)]
    struct Pr {
        author: Option<Login>,
        #[serde(default)]
        reviews: Nodes<ReviewNode<Login>>,
    }
    page(stdout, |pr: Pr, viewer| {
        Some(ReviewedPr {
            author: pr
                .author
                .and_then(|a| a.login)
                .unwrap_or_else(|| "ghost".into()),
            mine: pr
                .reviews
                .nodes
                .into_iter()
                .filter(|review| {
                    review.author.as_ref().and_then(|a| a.login.as_deref()) == Some(viewer)
                })
                .filter_map(|review| parse_utc(review.submitted_at.as_deref()?))
                .collect(),
        })
    })
}

/// Reads `OPEN_QUERY`'s answer: the user's oldest open PR, if they have one.
pub(crate) fn oldest_open(stdout: &str) -> Result<Option<Waiting>, String> {
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Pr {
        number: u64,
        title: String,
        created_at: String,
    }
    Ok(page(stdout, |pr: Pr, _| {
        Some(Waiting {
            number: pr.number,
            title: pr.title,
            since: pr.created_at,
        })
    })?
    .items
    .into_iter()
    .next())
}

/// How many pages of a search are read at most: GitHub's search stops at 1,000 results.
const SEARCH_PAGES: usize = 10;

/// Every page of a paged search (`query` with `$q` = `search`), each read by `read`: who the user
/// is, and everything found.
async fn search_all<T>(
    gh: &Path,
    worktree: &Path,
    query: &str,
    search: &str,
    read: fn(&str) -> Result<Page<T>, String>,
) -> Result<(String, Vec<T>), String> {
    let search = format!("q={search}");
    let query = format!("query={query}");
    let (mut viewer, mut items, mut after) = (String::new(), vec![], None::<String>);
    for _ in 0..SEARCH_PAGES {
        let cursor = after.as_ref().map(|cursor| format!("after={cursor}"));
        let mut args = vec!["api", "graphql", "-F", &search, "-f", &query];
        if let Some(cursor) = &cursor {
            args.extend(["-f", cursor]);
        }
        let page = read(&gh_text(gh, worktree, &args).await?)?;
        viewer = page.viewer;
        items.extend(page.items);
        after = page.next;
        if after.is_none() {
            break;
        }
    }
    Ok((viewer, items))
}

/// Asks `gh` (in `worktree`, so for its repo) for everything the figures are worked out from, all
/// at once, and works them out.
pub(crate) async fn read(
    gh: &Path,
    worktree: &Path,
    periods: Periods,
) -> Result<Statistics, String> {
    let from = periods.search_from();
    let (authored_search, reviewed_search) = (authored_search(&from), reviewed_search(&from));
    let ask = |query: &'static str, search: &'static str| async move {
        let search = format!("q={search}");
        let query = format!("query={query}");
        gh_text(
            gh,
            worktree,
            &["api", "graphql", "-F", &search, "-f", &query],
        )
        .await
    };
    let ((viewer, authored), (_, reviewed), open, requests) = tokio::try_join!(
        search_all(
            gh,
            worktree,
            AUTHORED_QUERY,
            &authored_search,
            authored_page
        ),
        search_all(
            gh,
            worktree,
            REVIEWED_QUERY,
            &reviewed_search,
            reviewed_page
        ),
        ask(OPEN_QUERY, OPEN_SEARCH),
        ask(dashboard::REVIEW_QUERY, dashboard::REVIEW_SEARCH),
    )?;
    Ok(figures(
        periods,
        &viewer,
        &authored,
        &reviewed,
        oldest_open(&open)?,
        &dashboard::review_requests_listed(&requests)?,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn times_periods_and_medians_are_exact() {
        assert_eq!(parse_utc("1970-01-02T00:00:01Z"), Some(86_401));
        let leap = parse_utc("2024-02-29T12:00:00Z").unwrap();
        assert_eq!(utc_date(leap), "2024-02-29");
        assert_eq!(
            parse_utc("2026-10-08T12:00:00.123Z"),
            parse_utc("2026-10-08T12:00:00Z")
        );
        assert_eq!(parse_utc("yesterday"), None);
        // Not a time, and not split on a character's bytes either.
        assert_eq!(parse_utc("2026-10-08T12:00:0é.5Z"), None);
        assert_eq!(parse_utc("2026-10-08T12:00:00é5Z"), None);

        let periods = Periods {
            now: 100 * DAY,
            days: 7,
        };
        assert!(!periods.in_current(93 * DAY));
        assert!(periods.in_current(93 * DAY + 1));
        assert!(periods.in_previous(93 * DAY));
        assert!(!periods.in_previous(86 * DAY));
        assert_eq!(periods.search_from(), utc_date(86 * DAY));

        assert_eq!(
            median(vec![5, 1, 3]),
            Median {
                seconds: Some(3),
                count: 3
            }
        );
        assert_eq!(
            median(vec![4, 1]),
            Median {
                seconds: Some(2),
                count: 2
            }
        );
        assert_eq!(
            median(vec![]),
            Median {
                seconds: None,
                count: 0
            }
        );
    }

    #[test]
    fn draft_spells_are_left_out_of_a_prs_time() {
        let ready = |at| DraftEvent {
            at,
            to_draft: false,
        };
        let draft = |at| DraftEvent { at, to_draft: true };
        // Opened ready, merged.
        assert_eq!(first_ready(0, false, &[]), Some(0));
        assert_eq!(ready_time(0, false, &[], 100), 100);
        // Opened as a draft, ready later.
        assert_eq!(first_ready(0, false, &[ready(30)]), Some(30));
        assert_eq!(ready_time(0, false, &[ready(30)], 100), 70);
        // Opened ready, back to draft for a spell.
        let spell = [draft(20), ready(50)];
        assert_eq!(first_ready(0, false, &spell), Some(0));
        assert_eq!(ready_time(0, false, &spell, 100), 70);
        // Opened as a draft and still one: never ready.
        assert_eq!(first_ready(0, true, &[]), None);
        assert_eq!(ready_time(0, true, &[], 100), 0);
        // Events out of order give the same answers.
        let shuffled = [ready(50), draft(20)];
        assert_eq!(first_ready(0, false, &shuffled), Some(0));
        assert_eq!(ready_time(0, false, &shuffled, 100), 70);
        let shuffled = [ready(80), ready(30), draft(60)];
        assert_eq!(first_ready(0, false, &shuffled), Some(30));
        assert_eq!(ready_time(0, false, &shuffled, 100), 50);
    }

    /// A week up to day 100.
    const WEEK: Periods = Periods {
        now: 100 * DAY,
        days: 7,
    };

    fn pr(created_at: i64) -> AuthoredPr {
        AuthoredPr {
            created_at,
            merged_at: None,
            open: true,
            is_draft: false,
            draft_events: vec![],
            reviews: vec![],
        }
    }

    fn merged(created_at: i64, merged_at: i64) -> AuthoredPr {
        AuthoredPr {
            merged_at: Some(merged_at),
            open: false,
            ..pr(created_at)
        }
    }

    fn by(login: &str, at: i64) -> Reviewed {
        Reviewed {
            at,
            login: login.into(),
            bot: false,
        }
    }

    fn of(authored: &[AuthoredPr]) -> Statistics {
        figures(WEEK, "me", authored, &[], None, &[])
    }

    #[test]
    fn merged_prs_and_time_to_merge_per_period() {
        let authored = [
            merged(95 * DAY, 99 * DAY),
            // Back to draft for a day, which doesn't count.
            AuthoredPr {
                draft_events: vec![
                    DraftEvent {
                        at: 95 * DAY,
                        to_draft: true,
                    },
                    DraftEvent {
                        at: 96 * DAY,
                        to_draft: false,
                    },
                ],
                ..merged(94 * DAY, 98 * DAY)
            },
            merged(88 * DAY, 90 * DAY),
            merged(70 * DAY, 80 * DAY),
            pr(99 * DAY),
        ];
        let stats = of(&authored);
        assert_eq!(stats.days, 7);
        assert_eq!(stats.merged, Compared { now: 2, before: 1 });
        assert_eq!(
            stats.time_to_merge,
            Compared {
                now: Median {
                    seconds: Some(3 * DAY + DAY / 2),
                    count: 2
                },
                before: Median {
                    seconds: Some(2 * DAY),
                    count: 1
                },
            }
        );
        let stats = of(&authored[..1]);
        assert_eq!(stats.merged, Compared { now: 1, before: 0 });
        assert_eq!(
            stats.time_to_merge.before,
            Median {
                seconds: None,
                count: 0
            }
        );
    }

    #[test]
    fn first_review_leaves_out_bots_and_the_user() {
        let authored = [
            // A bot first, then ann: ann's review counts.
            AuthoredPr {
                reviews: vec![
                    Reviewed {
                        bot: true,
                        ..by("dependabot[bot]", 95 * DAY + DAY / 2)
                    },
                    by("ann", 96 * DAY),
                ],
                ..merged(95 * DAY, 97 * DAY)
            },
            // Only a bot and the user: still waiting.
            AuthoredPr {
                reviews: vec![
                    Reviewed {
                        bot: true,
                        ..by("ci", 96 * DAY + DAY / 2)
                    },
                    by("me", 97 * DAY),
                ],
                ..pr(96 * DAY)
            },
            // Reviewed only while a draft, ready since: still waiting.
            AuthoredPr {
                draft_events: vec![DraftEvent {
                    at: 97 * DAY,
                    to_draft: false,
                }],
                reviews: vec![by("ann", 95 * DAY)],
                ..pr(94 * DAY)
            },
            // Merged with no review: not waiting.
            merged(95 * DAY, 96 * DAY),
            // Ready in the period before.
            AuthoredPr {
                reviews: vec![by("bob", 90 * DAY)],
                ..merged(88 * DAY, 91 * DAY)
            },
        ];
        let stats = of(&authored);
        assert_eq!(
            stats.time_to_first_review,
            Compared {
                now: Median {
                    seconds: Some(DAY),
                    count: 1
                },
                before: Median {
                    seconds: Some(2 * DAY),
                    count: 1
                },
            }
        );
        assert_eq!(stats.still_waiting, 2);
    }

    #[test]
    fn reviews_given_count_each_pr_once_per_period() {
        let reviewed = [
            ReviewedPr {
                author: "ann".into(),
                mine: vec![95 * DAY, 96 * DAY, 97 * DAY],
            },
            ReviewedPr {
                author: "bob".into(),
                mine: vec![90 * DAY, 98 * DAY],
            },
            ReviewedPr {
                author: "me".into(),
                mine: vec![99 * DAY],
            },
        ];
        let stats = figures(WEEK, "me", &[], &reviewed, None, &[]);
        assert_eq!(stats.reviews_given, Compared { now: 2, before: 1 });
    }

    #[test]
    fn the_oldest_open_pr_and_longest_request_are_found() {
        let request = |number: u64, requested_at: Option<&str>| ReviewRequest {
            number,
            title: format!("PR {number}"),
            url: format!("u{number}"),
            author: "ann".into(),
            head: "feat".into(),
            checks: vec![],
            requested_at: requested_at.map(str::to_owned),
            re_requested: false,
        };
        let oldest = Waiting {
            number: 4,
            title: "Old one".into(),
            since: "2026-09-01T09:00:00Z".into(),
        };
        let requests = [
            request(7, Some("2026-10-05T09:00:00Z")),
            request(9, Some("2026-10-01T09:00:00Z")),
            request(11, None),
        ];
        let stats = figures(WEEK, "me", &[], &[], Some(oldest.clone()), &requests);
        assert_eq!(stats.oldest_open, Some(oldest));
        assert_eq!(
            stats.longest_request,
            Some(Waiting {
                number: 9,
                title: "PR 9".into(),
                since: "2026-10-01T09:00:00Z".into(),
            })
        );
        let stats = figures(WEEK, "me", &[], &[], None, &[]);
        assert_eq!(stats.oldest_open, None);
        assert_eq!(stats.longest_request, None);
    }

    #[test]
    fn githubs_answers_are_read_into_prs_and_reviews() {
        let authored = authored_page(
            r#"{"data":{"viewer":{"login":"me"},"search":{
              "pageInfo":{"hasNextPage":true,"endCursor":"c1"},
              "nodes":[{"createdAt":"1970-01-01T00:00:10Z","mergedAt":"1970-01-01T00:01:40Z",
                "isDraft":false,"state":"MERGED",
                "timelineItems":{"nodes":[
                  {"__typename":"ConvertToDraftEvent","createdAt":"1970-01-01T00:00:20Z"},
                  {"__typename":"ReadyForReviewEvent","createdAt":"1970-01-01T00:00:50Z"}]},
                "reviews":{"nodes":[
                  {"submittedAt":"1970-01-01T00:00:30Z","author":{"__typename":"Bot","login":"ci"}},
                  {"submittedAt":"1970-01-01T00:00:40Z","author":{"__typename":"User","login":"renovate[bot]"}},
                  {"submittedAt":"1970-01-01T00:01:00Z","author":{"__typename":"User","login":"ann"}},
                  {"submittedAt":"1970-01-01T00:01:10Z","author":null},
                  {"submittedAt":null,"author":{"__typename":"User","login":"bob"}}]}}]}}}"#,
        )
        .unwrap();
        assert_eq!(authored.viewer, "me");
        assert_eq!(authored.next.as_deref(), Some("c1"));
        assert_eq!(
            authored.items,
            [AuthoredPr {
                created_at: 10,
                merged_at: Some(100),
                open: false,
                is_draft: false,
                draft_events: vec![
                    DraftEvent {
                        at: 20,
                        to_draft: true
                    },
                    DraftEvent {
                        at: 50,
                        to_draft: false
                    },
                ],
                reviews: vec![
                    Reviewed {
                        bot: true,
                        ..by("ci", 30)
                    },
                    Reviewed {
                        bot: true,
                        ..by("renovate[bot]", 40)
                    },
                    by("ann", 60),
                    by("ghost", 70),
                ],
            }]
        );

        let reviewed = reviewed_page(
            r#"{"data":{"viewer":{"login":"me"},"search":{
              "pageInfo":{"hasNextPage":false,"endCursor":"c9"},
              "nodes":[{"author":{"login":"ann"},"reviews":{"nodes":[
                {"submittedAt":"1970-01-01T00:00:05Z","author":{"login":"me"}},
                {"submittedAt":"1970-01-01T00:00:06Z","author":{"login":"bob"}},
                {"submittedAt":null,"author":{"login":"me"}},
                {"submittedAt":"1970-01-01T00:00:07Z","author":{"login":"me"}}]}}]}}}"#,
        )
        .unwrap();
        assert_eq!(reviewed.viewer, "me");
        assert_eq!(reviewed.next, None);
        assert_eq!(
            reviewed.items,
            [ReviewedPr {
                author: "ann".into(),
                mine: vec![5, 7]
            }]
        );

        let open = oldest_open(
            r#"{"data":{"search":{"nodes":[
              {"number":4,"title":"Old one","createdAt":"2026-09-01T09:00:00Z"}]}}}"#,
        )
        .unwrap();
        assert_eq!(
            open,
            Some(Waiting {
                number: 4,
                title: "Old one".into(),
                since: "2026-09-01T09:00:00Z".into(),
            })
        );
        assert_eq!(
            oldest_open(r#"{"data":{"search":{"nodes":[]}}}"#).unwrap(),
            None
        );
        assert!(authored_page("not json")
            .unwrap_err()
            .starts_with("gh said something unexpected"));
    }
}

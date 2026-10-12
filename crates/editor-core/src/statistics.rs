//! Statistics on the Dashboard (issue 47): the figures of the user's work in the Workspace's repo
//! over a period, and the period before it, worked out from what `gh` answers. Times are GitHub's
//! ISO strings read by hand into seconds since the Unix epoch (no date crate).

use serde::Serialize;

/// A day in seconds.
const DAY: i64 = 86_400;

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
    let fraction = &text[19..text.len() - 1];
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
}

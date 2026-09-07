//! restic-style keep-by-rule (spec §7). Pure: no IO, no clock. A file is kept if
//! ANY rule claims it; everything else is deleted. Rules count only periods that
//! actually contain a file, so gaps (PC off at the weekend) never lose a tier.

use chrono::{Datelike, NaiveDate, NaiveDateTime};
use serde::Serialize;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Keep {
    pub daily: u32,
    pub weekly: u32,
    pub monthly: u32,
    pub yearly: u32,
}

#[derive(Clone, Debug)]
pub struct Candidate {
    pub id: i64,
    pub created_at: NaiveDateTime,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Reason {
    Latest,
    Daily,
    Weekly,
    Monthly,
    Yearly,
}

#[derive(Debug, Default, PartialEq)]
pub struct Decision {
    pub keep: Vec<(i64, Vec<Reason>)>,
    pub delete: Vec<i64>,
}

/// Thai fiscal year: Oct–Sep. 2025-10-01 → 2026, 2025-09-30 → 2025.
pub fn fiscal_year(d: NaiveDate) -> i32 {
    if d.month() >= 10 { d.year() + 1 } else { d.year() }
}

fn bucket(reason: Reason, d: NaiveDate) -> (i32, u32) {
    match reason {
        Reason::Daily => (d.year(), d.ordinal()),
        Reason::Weekly => (d.iso_week().year(), d.iso_week().week()),
        Reason::Monthly => (d.year(), d.month()),
        Reason::Yearly => (fiscal_year(d), 0),
        Reason::Latest => (0, 0),
    }
}

pub fn decide(files: &[Candidate], keep: &Keep, now: NaiveDateTime) -> Decision {
    let mut sorted: Vec<&Candidate> = files.iter().collect();
    sorted.sort_by(|a, b| b.created_at.cmp(&a.created_at).then(b.id.cmp(&a.id)));
    let mut reasons: Vec<Vec<Reason>> = vec![Vec::new(); sorted.len()];

    let all_zero = keep.daily == 0 && keep.weekly == 0 && keep.monthly == 0 && keep.yearly == 0;
    let mut newest_seen = false;
    for (i, c) in sorted.iter().enumerate() {
        // Future-dated (clock went backwards) → keep; newest real file → keep; all rules 0 → keep all.
        if c.created_at > now || !newest_seen || all_zero {
            reasons[i].push(Reason::Latest);
        }
        if c.created_at <= now {
            newest_seen = true;
        }
    }

    for (reason, n) in [(Reason::Daily, keep.daily), (Reason::Weekly, keep.weekly), (Reason::Monthly, keep.monthly), (Reason::Yearly, keep.yearly)] {
        if n == 0 {
            continue;
        }
        let mut seen: Vec<(i32, u32)> = Vec::new();
        for (i, c) in sorted.iter().enumerate() {
            if c.created_at > now {
                continue;
            }
            let b = bucket(reason, c.created_at.date());
            if seen.contains(&b) {
                continue;
            }
            seen.push(b);
            reasons[i].push(reason);
            if seen.len() as u32 >= n {
                break;
            }
        }
    }

    let mut out = Decision::default();
    for (c, rs) in sorted.iter().zip(reasons) {
        if rs.is_empty() { out.delete.push(c.id) } else { out.keep.push((c.id, rs)) }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration, Weekday};

    fn at(y: i32, m: u32, d: u32, h: u32) -> NaiveDateTime {
        NaiveDate::from_ymd_opt(y, m, d).unwrap().and_hms_opt(h, 30, 0).unwrap()
    }
    fn daily(from: NaiveDateTime, days: i64, weekdays_only: bool) -> Vec<Candidate> {
        (0..days)
            .map(|i| from + Duration::days(i))
            .filter(|t| !weekdays_only || !matches!(t.weekday(), Weekday::Sat | Weekday::Sun))
            .enumerate()
            .map(|(i, t)| Candidate { id: i as i64 + 1, created_at: t })
            .collect()
    }
    fn count(d: &Decision, r: Reason) -> usize {
        d.keep.iter().filter(|(_, rs)| rs.contains(&r)).count()
    }
    fn keep(daily: u32, weekly: u32, monthly: u32, yearly: u32) -> Keep {
        Keep { daily, weekly, monthly, yearly }
    }

    #[test]
    fn three_years_of_daily_backups() {
        let files = daily(at(2023, 10, 1, 3), 1073, false); // to 2026-09-07
        let now = at(2026, 9, 7, 12);
        let d = decide(&files, &keep(14, 12, 24, 5), now);
        assert_eq!(count(&d, Reason::Daily), 14);
        assert_eq!(count(&d, Reason::Weekly), 12);
        assert_eq!(count(&d, Reason::Monthly), 24);
        assert_eq!(count(&d, Reason::Yearly), 3, "FY2024, FY2025, FY2026 present");
        assert_eq!(d.keep.len() + d.delete.len(), files.len());
        assert!(d.keep.len() < 60 && d.keep.len() > 40);
        // newest file has Latest + Daily
        let newest = d.keep.iter().find(|(id, _)| *id == files.last().unwrap().id).unwrap();
        assert!(newest.1.contains(&Reason::Latest) && newest.1.contains(&Reason::Daily));
    }

    #[test]
    fn weekend_gap_keeps_friday_as_weekly() {
        let files = daily(at(2026, 8, 3, 3), 35, true); // Mon 3 Aug .. Sun 6 Sep, weekdays only
        let d = decide(&files, &keep(1, 2, 0, 0), at(2026, 9, 7, 12));
        let weekly: Vec<Weekday> = d.keep.iter().filter(|(_, rs)| rs.contains(&Reason::Weekly))
            .map(|(id, _)| files.iter().find(|f| f.id == *id).unwrap().created_at.weekday()).collect();
        assert_eq!(weekly, vec![Weekday::Fri, Weekday::Fri]);
    }

    #[test]
    fn newest_always_kept_and_all_zero_keeps_everything() {
        let files = daily(at(2026, 9, 1, 3), 5, false);
        let d = decide(&files, &keep(0, 0, 0, 1), at(2026, 9, 7, 12));
        assert!(d.keep.iter().any(|(id, rs)| *id == 5 && rs.contains(&Reason::Latest)));
        let all = decide(&files, &keep(0, 0, 0, 0), at(2026, 9, 7, 12));
        assert!(all.delete.is_empty());
        assert_eq!(all.keep.len(), 5);
    }

    #[test]
    fn fiscal_year_boundary_is_october() {
        assert_eq!(fiscal_year(NaiveDate::from_ymd_opt(2025, 9, 30).unwrap()), 2025);
        assert_eq!(fiscal_year(NaiveDate::from_ymd_opt(2025, 10, 1).unwrap()), 2026);
        let files = vec![
            Candidate { id: 1, created_at: at(2025, 9, 30, 3) },
            Candidate { id: 2, created_at: at(2025, 10, 1, 3) },
        ];
        let d = decide(&files, &keep(0, 0, 0, 2), at(2026, 9, 7, 12));
        assert_eq!(count(&d, Reason::Yearly), 2);
        assert!(d.delete.is_empty());
    }

    #[test]
    fn several_runs_same_day_keep_newest_only() {
        let files = vec![
            Candidate { id: 1, created_at: at(2026, 9, 7, 3) },
            Candidate { id: 2, created_at: at(2026, 9, 7, 9) },
            Candidate { id: 3, created_at: at(2026, 9, 7, 15) },
        ];
        let d = decide(&files, &keep(1, 0, 0, 0), at(2026, 9, 7, 20));
        assert_eq!(d.keep.iter().map(|(id, _)| *id).collect::<Vec<_>>(), vec![3]);
        assert_eq!(d.delete, vec![2, 1]);
    }

    #[test]
    fn future_dated_file_is_kept_not_counted() {
        let files = vec![
            Candidate { id: 1, created_at: at(2026, 9, 6, 3) },
            Candidate { id: 2, created_at: at(2027, 1, 1, 3) },
        ];
        let d = decide(&files, &keep(1, 0, 0, 0), at(2026, 9, 7, 12));
        assert!(d.delete.is_empty());
        assert!(d.keep.iter().any(|(id, rs)| *id == 1 && rs.contains(&Reason::Daily)));
    }
}

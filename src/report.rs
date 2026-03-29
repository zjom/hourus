use crate::entry::{Entry, EntryKind, EntryLine, Interval};
use crate::error::ParseError;
use anyhow::{Result, anyhow};
use chrono::{NaiveDateTime, TimeDelta};
use itertools::Itertools;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::str::FromStr;

#[derive(Serialize, Deserialize, Debug)]
pub struct Report {
    pub entries: HashMap<String, Vec<Interval>>,
    pub entry_lines: Vec<EntryLine>,
}

impl Report {
    pub fn build_start_entries(&self, desc: &str, now: NaiveDateTime) -> Result<Vec<EntryLine>> {
        if desc.is_empty() {
            return Err(anyhow!("Must specify description"));
        }
        let desc = desc.to_lowercase();

        if let Some(last) = self.entry_lines.last()
            && last.kind == EntryKind::Start
        {
            if last.dt >= now {
                return Err(ParseError::EndBeforeStart.into());
            }
            Ok(vec![
                EntryLine {
                    kind: EntryKind::End,
                    dt: now,
                    desc: last.desc.clone(),
                },
                EntryLine {
                    kind: EntryKind::Start,
                    dt: now + TimeDelta::seconds(1),
                    desc,
                },
            ])
        } else {
            Ok(vec![EntryLine {
                kind: EntryKind::Start,
                dt: now,
                desc,
            }])
        }
    }

    pub fn summarize(&self) -> Vec<(String, TimeDelta)> {
        self.entries
            .iter()
            .map(|(desc, v)| {
                (
                    desc.to_owned(),
                    v.iter().map(Interval::duration).sum::<TimeDelta>(),
                )
            })
            .sorted_by(|a, b| Ord::cmp(&b.1, &a.1))
            .collect()
    }

    pub fn total_duration(&self) -> TimeDelta {
        self.entries
            .values()
            .flat_map(|v| v.iter().map(Interval::duration))
            .sum()
    }
}

impl FromStr for Report {
    type Err = ParseError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let entry_lines: Vec<EntryLine> = s
            .lines()
            .map(EntryLine::from_str)
            .collect::<Result<_, _>>()?;

        let entries = entry_lines
            .iter()
            .tuples()
            .map(|(a, b)| Entry::new(a, b))
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .map(|e| (e.desc, e.interval))
            .into_group_map();

        Ok(Report {
            entry_lines,
            entries,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_dt(s: &str) -> NaiveDateTime {
        NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S").unwrap()
    }

    // A well-formed two-entry report (1 hour of "coding")
    const ONE_PAIR: &str = "START - 2024-01-01 09:00:00 - coding\nEND - 2024-01-01 10:00:00 - coding";

    // Two tasks: coding 2 h, review 30 min
    const TWO_TASKS: &str = "\
START - 2024-01-01 09:00:00 - coding\n\
END - 2024-01-01 11:00:00 - coding\n\
START - 2024-01-01 11:00:00 - review\n\
END - 2024-01-01 11:30:00 - review";

    // Same task logged twice: 1 h + 90 min
    const REPEATED_TASK: &str = "\
START - 2024-01-01 09:00:00 - coding\n\
END - 2024-01-01 10:00:00 - coding\n\
START - 2024-01-01 10:00:00 - coding\n\
END - 2024-01-01 11:30:00 - coding";

    // --- Report::from_str ---

    #[test]
    fn from_str_empty_input_produces_empty_report() {
        let report: Report = "".parse().unwrap();
        assert!(report.entries.is_empty());
        assert!(report.entry_lines.is_empty());
    }

    #[test]
    fn from_str_single_pair_parses_correctly() {
        let report: Report = ONE_PAIR.parse().unwrap();
        assert_eq!(report.entry_lines.len(), 2);
        assert_eq!(report.entries["coding"].len(), 1);
    }

    #[test]
    fn from_str_multiple_pairs_parsed() {
        let report: Report = TWO_TASKS.parse().unwrap();
        assert_eq!(report.entry_lines.len(), 4);
        assert!(report.entries.contains_key("coding"));
        assert!(report.entries.contains_key("review"));
    }

    #[test]
    fn from_str_invalid_line_returns_err() {
        let input = "BOGUS - 2024-01-01 09:00:00 - coding\nEND - 2024-01-01 10:00:00 - coding";
        assert!(input.parse::<Report>().is_err());
    }

    #[test]
    fn from_str_mismatched_desc_pair_returns_err() {
        let input = "START - 2024-01-01 09:00:00 - coding\nEND - 2024-01-01 10:00:00 - review";
        assert!(matches!(
            input.parse::<Report>().unwrap_err(),
            ParseError::DescMismatch
        ));
    }

    #[test]
    fn from_str_end_before_start_time_returns_err() {
        let input = "START - 2024-01-01 10:00:00 - coding\nEND - 2024-01-01 09:00:00 - coding";
        assert!(matches!(
            input.parse::<Report>().unwrap_err(),
            ParseError::EndBeforeStart
        ));
    }

    // --- Report::total_duration ---

    #[test]
    fn total_duration_empty_report_is_zero() {
        let report: Report = "".parse().unwrap();
        assert_eq!(report.total_duration().num_seconds(), 0);
    }

    #[test]
    fn total_duration_single_pair() {
        let report: Report = ONE_PAIR.parse().unwrap();
        assert_eq!(report.total_duration().num_hours(), 1);
    }

    #[test]
    fn total_duration_multiple_tasks_summed() {
        let report: Report = TWO_TASKS.parse().unwrap();
        assert_eq!(report.total_duration().num_minutes(), 150); // 2h + 30m
    }

    #[test]
    fn total_duration_repeated_task_accumulates() {
        let report: Report = REPEATED_TASK.parse().unwrap();
        assert_eq!(report.total_duration().num_minutes(), 150); // 1h + 90m
    }

    // --- Report::summarize ---

    #[test]
    fn summarize_empty_report_is_empty() {
        let report: Report = "".parse().unwrap();
        assert!(report.summarize().is_empty());
    }

    #[test]
    fn summarize_single_task() {
        let report: Report = ONE_PAIR.parse().unwrap();
        let summary = report.summarize();
        assert_eq!(summary.len(), 1);
        assert_eq!(summary[0].0, "coding");
        assert_eq!(summary[0].1.num_hours(), 1);
    }

    #[test]
    fn summarize_sorted_descending_by_duration() {
        let report: Report = TWO_TASKS.parse().unwrap();
        let summary = report.summarize();
        assert_eq!(summary.len(), 2);
        // coding (2 h) should come before review (30 min)
        assert_eq!(summary[0].0, "coding");
        assert_eq!(summary[1].0, "review");
    }

    #[test]
    fn summarize_repeated_task_aggregated() {
        let report: Report = REPEATED_TASK.parse().unwrap();
        let summary = report.summarize();
        assert_eq!(summary.len(), 1);
        assert_eq!(summary[0].0, "coding");
        assert_eq!(summary[0].1.num_minutes(), 150);
    }

    // --- Report::build_start_entries ---

    #[test]
    fn build_start_entries_empty_desc_returns_err() {
        let report: Report = "".parse().unwrap();
        let now = parse_dt("2024-01-01 12:00:00");
        assert!(report.build_start_entries("", now).is_err());
    }

    #[test]
    fn build_start_entries_no_open_session_returns_single_start() {
        let report: Report = ONE_PAIR.parse().unwrap();
        let now = parse_dt("2024-01-01 12:00:00");
        let entries = report.build_start_entries("new task", now).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].kind, EntryKind::Start);
        assert_eq!(entries[0].desc, "new task");
        assert_eq!(entries[0].dt, now);
    }

    #[test]
    fn build_start_entries_empty_report_returns_single_start() {
        let report: Report = "".parse().unwrap();
        let now = parse_dt("2024-01-01 12:00:00");
        let entries = report.build_start_entries("task", now).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].kind, EntryKind::Start);
    }

    #[test]
    fn build_start_entries_open_session_returns_end_and_start() {
        // Report ending with an open START
        let input = "START - 2024-01-01 09:00:00 - coding\nEND - 2024-01-01 10:00:00 - coding\nSTART - 2024-01-01 10:00:00 - coding";
        // The last line is unpaired (tuples() drops it), but entry_lines has it
        let report: Report = input.parse().unwrap();
        let now = parse_dt("2024-01-01 12:00:00");
        let entries = report.build_start_entries("review", now).unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].kind, EntryKind::End);
        assert_eq!(entries[0].desc, "coding");
        assert_eq!(entries[0].dt, now);
        assert_eq!(entries[1].kind, EntryKind::Start);
        assert_eq!(entries[1].desc, "review");
    }

    #[test]
    fn build_start_entries_open_session_desc_lowercased() {
        let input = "START - 2024-01-01 09:00:00 - coding";
        let report: Report = input.parse().unwrap();
        let now = parse_dt("2024-01-01 12:00:00");
        let entries = report.build_start_entries("MY TASK", now).unwrap();
        assert_eq!(entries[1].desc, "my task");
    }

    #[test]
    fn build_start_entries_open_session_now_before_last_returns_err() {
        let input = "START - 2024-01-01 10:00:00 - coding";
        let report: Report = input.parse().unwrap();
        let now = parse_dt("2024-01-01 09:00:00"); // before the open START
        assert!(report.build_start_entries("review", now).is_err());
    }

    #[test]
    fn build_start_entries_new_start_is_one_second_after_end() {
        let input = "START - 2024-01-01 09:00:00 - coding";
        let report: Report = input.parse().unwrap();
        let now = parse_dt("2024-01-01 12:00:00");
        let entries = report.build_start_entries("review", now).unwrap();
        assert_eq!(entries[1].dt, now + chrono::TimeDelta::seconds(1));
    }
}

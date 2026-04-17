use crate::error::ParseError;
use chrono::{DateTime, Local, NaiveDateTime, TimeDelta, Utc};
use serde::{Deserialize, Serialize};
use std::{fmt, str::FromStr, sync::Arc};

#[derive(Serialize, Deserialize, PartialEq, Debug, Clone)]
pub enum EntryKind {
    Start,
    End,
}

impl fmt::Display for EntryKind {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Self::Start => write!(f, "START"),
            Self::End => write!(f, "END"),
        }
    }
}

impl FromStr for EntryKind {
    type Err = ParseError;
    fn from_str(s: &str) -> Result<Self, ParseError> {
        match s.to_lowercase().trim() {
            "start" => Ok(EntryKind::Start),
            "end" => Ok(EntryKind::End),
            _ => Err(ParseError::UnknownEntryKind),
        }
    }
}

#[derive(Serialize, Deserialize, Debug, PartialEq, Clone)]
pub struct EntryLine {
    pub kind: EntryKind,
    pub desc: Arc<str>,
    pub dt: DateTime<Utc>,
}

impl fmt::Display for EntryLine {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(
            f,
            "{} - {} - {}",
            self.kind,
            self.dt.format("%Y-%m-%d %H:%M:%S"),
            self.desc
        )
    }
}

impl FromStr for EntryLine {
    type Err = ParseError;
    fn from_str(s: &str) -> Result<Self, ParseError> {
        let data = s.splitn(3, " - ").collect::<Vec<&str>>();
        if data.len() != 3 {
            return Err(ParseError::Malformatted);
        }
        let kind: EntryKind = data[0].parse()?;
        let dt_str = data[1].trim();
        // TODO this is very hacky.
        // Should probably pass in format specifier from user config
        let dt = NaiveDateTime::parse_from_str(dt_str, "%Y-%m-%d %H:%M:%S")
            .or_else(|_| NaiveDateTime::parse_from_str(dt_str, "%Y-%m-%dT%H:%M:%S"))?
            .and_utc();
        let desc = data[2].trim().to_lowercase();

        Ok(EntryLine {
            kind,
            desc: desc.into(),
            dt,
        })
    }
}

#[derive(Serialize, Deserialize, Debug, Copy, Clone)]
pub struct Interval {
    pub start: DateTime<Utc>,
    pub end: Option<DateTime<Utc>>,
}
impl Interval {
    pub fn duration(&self) -> TimeDelta {
        let end = self.end.unwrap_or(Local::now().to_utc());
        end - self.start
    }
}

#[derive(Debug, Clone)]
pub struct Entry {
    pub desc: Arc<str>,
    pub interval: Interval,
}

impl Entry {
    pub fn new(a: &EntryLine, b: &EntryLine) -> Result<Entry, ParseError> {
        if a.kind != EntryKind::Start || b.kind != EntryKind::End {
            return Err(ParseError::StartNoEnd);
        }
        if a.desc != b.desc {
            return Err(ParseError::DescMismatch);
        }
        if b.dt < a.dt {
            return Err(ParseError::EndBeforeStart);
        }

        Ok(Entry {
            desc: a.desc.clone(),
            interval: Interval {
                start: a.dt,
                end: Some(b.dt),
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_dt(s: &str) -> DateTime<Utc> {
        NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S")
            .unwrap()
            .and_utc()
    }

    fn base_dt() -> DateTime<Utc> {
        parse_dt("2015-09-05 23:56:04")
    }

    fn start_line() -> EntryLine {
        EntryLine {
            kind: EntryKind::Start,
            desc: "desc".into(),
            dt: base_dt(),
        }
    }

    fn end_line() -> EntryLine {
        EntryLine {
            kind: EntryKind::End,
            desc: "desc".into(),
            dt: parse_dt("2015-09-06 00:56:04"),
        }
    }

    // --- EntryKind ---

    #[test]
    fn entry_kind_display_start() {
        assert_eq!(format!("{}", EntryKind::Start), "START");
    }

    #[test]
    fn entry_kind_display_end() {
        assert_eq!(format!("{}", EntryKind::End), "END");
    }

    #[test]
    fn entry_kind_from_str_lowercase() {
        assert_eq!("start".parse::<EntryKind>().unwrap(), EntryKind::Start);
        assert_eq!("end".parse::<EntryKind>().unwrap(), EntryKind::End);
    }

    #[test]
    fn entry_kind_from_str_uppercase() {
        assert_eq!("START".parse::<EntryKind>().unwrap(), EntryKind::Start);
        assert_eq!("END".parse::<EntryKind>().unwrap(), EntryKind::End);
    }

    #[test]
    fn entry_kind_from_str_mixed_case() {
        assert_eq!("Start".parse::<EntryKind>().unwrap(), EntryKind::Start);
        assert_eq!("End".parse::<EntryKind>().unwrap(), EntryKind::End);
    }

    #[test]
    fn entry_kind_from_str_with_whitespace() {
        assert_eq!("  start  ".parse::<EntryKind>().unwrap(), EntryKind::Start);
    }

    #[test]
    fn entry_kind_from_str_unknown_returns_err() {
        assert!(matches!(
            "STOP".parse::<EntryKind>().unwrap_err(),
            ParseError::UnknownEntryKind
        ));
    }

    #[test]
    fn entry_kind_from_str_empty_returns_err() {
        assert!(matches!(
            "".parse::<EntryKind>().unwrap_err(),
            ParseError::UnknownEntryKind
        ));
    }

    // --- EntryLine Display ---

    #[test]
    fn entry_line_display_should_format_correctly() {
        assert_eq!(
            format!("{}", start_line()),
            "START - 2015-09-05 23:56:04 - desc"
        );
    }

    #[test]
    fn entry_line_display_end_kind() {
        let e = EntryLine {
            kind: EntryKind::End,
            desc: "task".into(),
            dt: base_dt(),
        };
        assert_eq!(format!("{}", e), "END - 2015-09-05 23:56:04 - task");
    }

    // --- EntryLine FromStr ---

    #[test]
    fn entry_line_from_str_should_ok_with_nice_input() {
        assert_eq!(
            "START - 2015-09-05 23:56:04 - desc"
                .parse::<EntryLine>()
                .unwrap(),
            start_line()
        );
    }

    #[test]
    fn entry_line_from_str_end_kind() {
        let e: EntryLine = "END - 2015-09-05 23:56:04 - desc".parse().unwrap();
        assert_eq!(e.kind, EntryKind::End);
        assert_eq!(e.desc, "desc".into());
        assert_eq!(e.dt, base_dt());
    }

    #[test]
    fn entry_line_from_str_desc_is_lowercased() {
        let e: EntryLine = "START - 2015-09-05 23:56:04 - My Task".parse().unwrap();
        assert_eq!(e.desc, "my task".into());
    }

    #[test]
    fn entry_line_from_str_desc_with_embedded_dashes_preserved() {
        // splitn(3) means only the first two separators split; the rest stays in desc
        let e: EntryLine = "START - 2015-09-05 23:56:04 - a - b - c".parse().unwrap();
        assert_eq!(e.desc, "a - b - c".into());
    }

    #[test]
    fn entry_line_from_str_should_err_missing_desc() {
        assert!(matches!(
            "START - 2015-09-05 23:56:04"
                .parse::<EntryLine>()
                .unwrap_err(),
            ParseError::Malformatted
        ));
    }

    #[test]
    fn entry_line_from_str_should_err_only_one_part() {
        assert!(matches!(
            "START".parse::<EntryLine>().unwrap_err(),
            ParseError::Malformatted
        ));
    }

    #[test]
    fn entry_line_from_str_should_err_invalid_kind() {
        assert!(matches!(
            "PAUSE - 2015-09-05 23:56:04 - desc"
                .parse::<EntryLine>()
                .unwrap_err(),
            ParseError::UnknownEntryKind
        ));
    }

    #[test]
    fn entry_line_from_str_should_err_invalid_datetime() {
        assert!(matches!(
            "START - not-a-date - desc"
                .parse::<EntryLine>()
                .unwrap_err(),
            ParseError::TimeFormat(_)
        ));
    }

    // --- Interval ---

    #[test]
    fn interval_duration_is_correct() {
        let interval = Interval {
            start: parse_dt("2024-01-01 09:00:00"),
            end: Some(parse_dt("2024-01-01 10:30:00")),
        };
        assert_eq!(interval.duration().num_minutes(), 90);
    }

    #[test]
    fn interval_duration_zero_when_start_equals_end() {
        let d = parse_dt("2024-01-01 09:00:00");
        let interval = Interval {
            start: d,
            end: Some(d),
        };
        assert_eq!(interval.duration().num_seconds(), 0);
    }

    // --- Entry::new ---

    #[test]
    fn entry_new_valid_pair() {
        let entry = Entry::new(&start_line(), &end_line()).unwrap();
        assert_eq!(entry.desc, "desc".into());
        assert_eq!(entry.interval.start, base_dt());
        assert_eq!(entry.interval.end, Some(end_line().dt));
    }

    #[test]
    fn entry_new_both_start_returns_start_no_end() {
        assert!(matches!(
            Entry::new(&start_line(), &start_line()).unwrap_err(),
            ParseError::StartNoEnd
        ));
    }

    #[test]
    fn entry_new_end_then_start_returns_start_no_end() {
        assert!(matches!(
            Entry::new(&end_line(), &start_line()).unwrap_err(),
            ParseError::StartNoEnd
        ));
    }

    #[test]
    fn entry_new_desc_mismatch_returns_err() {
        let b = EntryLine {
            kind: EntryKind::End,
            desc: "different".into(),
            dt: end_line().dt,
        };
        assert!(matches!(
            Entry::new(&start_line(), &b).unwrap_err(),
            ParseError::DescMismatch
        ));
    }

    #[test]
    fn entry_new_end_time_before_start_time_returns_err() {
        let a = EntryLine {
            kind: EntryKind::Start,
            desc: "desc".into(),
            dt: parse_dt("2024-01-01 10:00:00"),
        };
        let b = EntryLine {
            kind: EntryKind::End,
            desc: "desc".into(),
            dt: parse_dt("2024-01-01 09:00:00"),
        };
        assert!(matches!(
            Entry::new(&a, &b).unwrap_err(),
            ParseError::EndBeforeStart
        ));
    }

    #[test]
    fn entry_new_equal_start_and_end_time_is_valid() {
        let d = parse_dt("2024-01-01 09:00:00");
        let a = EntryLine {
            kind: EntryKind::Start,
            desc: "desc".into(),
            dt: d,
        };
        let b = EntryLine {
            kind: EntryKind::End,
            desc: "desc".into(),
            dt: d,
        };
        assert!(Entry::new(&a, &b).is_ok());
    }
}

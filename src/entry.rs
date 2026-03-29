use crate::error::ParseError;
use chrono::{NaiveDateTime, TimeDelta};
use serde::{Deserialize, Serialize};
use std::{fmt, str::FromStr};

#[derive(Serialize, Deserialize, PartialEq, Debug)]
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

#[derive(Serialize, Deserialize, Debug, PartialEq)]
pub struct EntryLine {
    pub kind: EntryKind,
    pub desc: String,
    pub dt: NaiveDateTime,
}

impl fmt::Display for EntryLine {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{} - {} - {}", self.kind, self.dt, self.desc)
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
        let dt: NaiveDateTime = data[1].parse()?;
        let desc = data[2].trim().to_lowercase().to_owned();

        Ok(EntryLine { kind, desc, dt })
    }
}

#[derive(Serialize, Deserialize, Debug)]
pub struct Interval {
    pub start: NaiveDateTime,
    pub end: NaiveDateTime,
}
impl Interval {
    pub fn duration(&self) -> TimeDelta {
        self.end - self.start
    }
}

#[derive(Debug)]
pub struct Entry {
    pub desc: String,
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
            desc: a.desc.to_owned(),
            interval: Interval {
                start: a.dt,
                end: b.dt,
            },
        })
    }
}

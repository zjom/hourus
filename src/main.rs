use chrono::{NaiveDateTime, TimeDelta};
use itertools::Itertools;
use serde::{Deserialize, Serialize};
use std::io::{self, Read};
use std::str::FromStr;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum ParseError {
    #[error("start time before end time")]
    StartBeforeEnd,

    #[error("end not found for start")]
    StartNoEnd,

    #[error("description on pair do not match")]
    DescMismatch,

    #[error("failed to parse time")]
    TimeFormat(#[from] chrono::ParseError),

    #[error("malformatted line")]
    Malformatted,

    #[error("unknown entry line keyword")]
    UnknownEntryKind,

    #[error("unknown data store error")]
    Unknown,
}

#[derive(Serialize, Deserialize, PartialEq, Debug)]
enum EntryKind {
    Start,
    End,
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

#[derive(Serialize, Deserialize, Debug)]
struct EntryLine {
    kind: EntryKind,
    desc: String,
    dt: NaiveDateTime,
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
pub struct Entry {
    desc: String,
    start: NaiveDateTime,
    end: NaiveDateTime,
    duration: TimeDelta,
}

impl Entry {
    fn new(a: EntryLine, b: EntryLine) -> Result<Entry, ParseError> {
        if a.kind != EntryKind::Start || b.kind != EntryKind::End {
            return Err(ParseError::StartNoEnd);
        }
        if a.desc != b.desc {
            return Err(ParseError::DescMismatch);
        }
        let duration = b.dt - a.dt;
        if duration.lt(&TimeDelta::zero()) {
            return Err(ParseError::StartBeforeEnd);
        }

        Ok(Entry {
            desc: a.desc,
            start: a.dt,
            end: b.dt,
            duration: duration,
        })
    }
}

fn main() {
    let mut input = String::new();
    io::stdin()
        .read_to_string(&mut input)
        .expect("Failed to read stdin");

    input
        .lines()
        .enumerate()
        .map(|(i, line)| EntryLine::from_str(line).expect(&format!("failed to parse line {i}")))
        .tuples::<(_, _)>()
        .enumerate()
        .map(|(i, (a, b))| Entry::new(a, b).expect(&format!("failed to parse line {i}")))
        .for_each(|entry| println!("{:#?}", entry));
}

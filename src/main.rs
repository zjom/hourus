use anyhow::Result;
use chrono::{NaiveDateTime, TimeDelta};
use itertools::Itertools;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs::File;
use std::io::{self, Read, Write};
use std::str::FromStr;
use std::{fmt, fs, process};

#[derive(thiserror::Error, Debug)]
pub enum ParseError {
    #[error("end time is before start time")]
    EndBeforeStart,

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
}

#[derive(Serialize, Deserialize, PartialEq, Debug)]
enum EntryKind {
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

#[derive(Serialize, Deserialize, Debug)]
struct EntryLine {
    kind: EntryKind,
    desc: String,
    dt: NaiveDateTime,
}

impl fmt::Display for EntryLine {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{} - {} - {}", self.kind, self.desc, self.dt)
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
struct Entry {
    desc: String,
    start: NaiveDateTime,
    end: NaiveDateTime,
    duration: TimeDelta,
}

impl Entry {
    fn new(a: &EntryLine, b: &EntryLine) -> Result<Entry, ParseError> {
        if a.kind != EntryKind::Start || b.kind != EntryKind::End {
            return Err(ParseError::StartNoEnd);
        }
        if a.desc != b.desc {
            return Err(ParseError::DescMismatch);
        }
        let duration = b.dt - a.dt;
        if duration < TimeDelta::zero() {
            return Err(ParseError::EndBeforeStart);
        }

        Ok(Entry {
            desc: a.desc.to_owned(),
            start: a.dt,
            end: b.dt,
            duration,
        })
    }
}

#[derive(Serialize, Deserialize, Debug)]
pub struct Report {
    entries: HashMap<String, Vec<(NaiveDateTime, NaiveDateTime)>>,
    entry_lines: Vec<EntryLine>,
}

impl Report {
    fn summarize(&self) -> Vec<(String, TimeDelta)> {
        self.entries
            .iter()
            .map(|(desc, v)| {
                (
                    desc.to_owned(),
                    v.iter()
                        .map(|(start, end)| *end - *start)
                        .sum::<TimeDelta>(),
                )
            })
            .sorted_by(|a, b| Ord::cmp(&b.1, &a.1))
            .collect()
    }

    fn total_duration(&self) -> TimeDelta {
        self.entries
            .values()
            .flat_map(|v| v.iter().map(|(start, end)| *end - *start))
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
            .map(|e| (e.desc, (e.start, e.end)))
            .into_group_map();

        Ok(Report {
            entry_lines,
            entries,
        })
    }
}

use clap::{Parser, Subcommand};

/// Parses and summarises .hours log file. Data can be passed in via the --path flag or stdin
/// (default).
#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Cli {
    /// Path to .hours file
    #[arg(short, long)]
    path: Option<String>,

    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Prints breakdown of hours by task
    Breakdown,
    /// Appends new session to file, ending current if exists. Outputs to stdout if no path
    /// specified
    Start {
        /// Description of the entry
        #[arg(short)]
        desc: String,
    },
}

fn get_file_reader(opt_path: &Option<String>) -> Result<Box<dyn Read>> {
    Ok(match opt_path {
        Some(path) => Box::new(File::open(path)?),
        None => Box::new(io::stdin()),
    })
}
fn get_file_writer(opt_path: &Option<String>) -> Result<Box<dyn Write>> {
    Ok(match opt_path {
        Some(path) => Box::new(fs::OpenOptions::new().append(true).open(path)?),
        None => Box::new(io::stdout()),
    })
}

fn run() -> Result<(), anyhow::Error> {
    let cli = Cli::parse();
    let mut reader = get_file_reader(&cli.path)?;
    let mut input_buf = String::new();
    reader.read_to_string(&mut input_buf)?;
    let report: Report = input_buf.parse()?;

    match &cli.command {
        Some(Commands::Breakdown) => {
            let summary = report.summarize();
            for (desc, dur) in &summary {
                println!("{desc}: {}h {}m", dur.num_hours(), dur.num_minutes() % 60);
            }
            let total = report.total_duration();
            println!(
                "Total: {}h {}m",
                total.num_hours(),
                total.num_minutes() % 60
            );
        }
        Some(Commands::Start { desc }) => {
            if desc.is_empty() {
                eprintln!("Must specify description");
                process::exit(1);
            }

            let now = chrono::Local::now().naive_local();

            let output = if let Some(last) = report.entry_lines.last()
                && last.kind == EntryKind::Start
            {
                if last.dt >= now {
                    return Err(ParseError::EndBeforeStart.into());
                }

                let end_entry = EntryLine {
                    kind: EntryKind::End,
                    dt: now,
                    desc: last.desc.clone(),
                };
                let next_start_dt = now + TimeDelta::new(1, 0).unwrap();
                let start_entry = EntryLine {
                    kind: EntryKind::Start,
                    dt: next_start_dt,
                    desc: desc.to_owned(),
                };

                format!("\n{end_entry}\n{start_entry}")
            } else {
                let new = EntryLine {
                    kind: EntryKind::Start,
                    desc: desc.to_owned(),
                    dt: now,
                };
                format!("\n{new}")
            };

            let mut writer = get_file_writer(&cli.path)?;
            writer.write(output.as_bytes())?;
        }
        None => {
            let total = report.total_duration();
            println!(
                "Total: {}h {}m",
                total.num_hours(),
                total.num_minutes() % 60
            );
        }
    }

    Ok(())
}

fn main() {
    if let Err(err) = run() {
        eprintln!("{err}");
        process::exit(1);
    }
}

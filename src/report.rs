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

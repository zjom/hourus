use crate::entry::{Entry, EntryKind, EntryLine, Interval};
use crate::error::StorageError;
use anyhow::{Result, anyhow};
use chrono::{DateTime, TimeDelta, Utc};
use std::fs;
use std::io::{self, BufRead, BufReader, Read, Seek, Write};
use std::path::PathBuf;
use std::str::FromStr;

#[derive(Default, Clone)]
pub struct QueryOpts {
    pub from: Option<DateTime<Utc>>,
    pub to: Option<DateTime<Utc>>,
    pub limit: Option<usize>,
    pub offset: Option<usize>,
}

pub trait Repository {
    fn list(&self, opts: QueryOpts) -> Result<Vec<&Entry>, StorageError>;

    fn start_session(&mut self, desc: &str, dt: DateTime<Utc>) -> Result<()>;

    fn end_session(&mut self, dt: DateTime<Utc>) -> Result<()>;
}

/// File-backed repository of time-tracking entries.
///
/// Loads the full log once on construction.  Writes are append-only:
/// `start_session` and `end_session` write to the backing file *before*
/// updating in-memory state, so a crash after a successful write leaves the
/// file as the authoritative source of truth and the next load will
/// reconstruct the correct state.
pub struct FileRepository {
    path: Option<PathBuf>,
    entries: Vec<Entry>,
}

impl FileRepository {
    pub fn new(path: Option<PathBuf>) -> anyhow::Result<FileRepository> {
        let reader: Box<dyn io::Read> = match &path {
            Some(p) => Box::new(fs::File::open(p)?),
            None => Box::new(io::stdin()),
        };

        Ok(FileRepository {
            path,
            entries: FileRepository::load(reader)?,
        })
    }

    fn load(reader: Box<dyn io::Read>) -> anyhow::Result<Vec<Entry>> {
        let lines: Vec<EntryLine> = BufReader::new(reader)
            .lines()
            .filter_map(|line| {
                let line = line.ok()?;
                (!line.trim().is_empty()).then(|| EntryLine::from_str(&line))
            })
            .collect::<Result<_, _>>()?;

        let (pairs, tail) = if lines.last().map_or(false, |l| l.kind == EntryKind::Start) {
            (&lines[..lines.len() - 1], lines.last())
        } else {
            (&lines[..], None)
        };

        let mut entries: Vec<Entry> = pairs
            .chunks(2)
            .map(|pair| Entry::new(&pair[0], &pair[1]).map_err(Into::into))
            .collect::<Result<_, StorageError>>()?;

        if let Some(ongoing) = tail {
            entries.push(Entry {
                desc: ongoing.desc.clone(),
                interval: Interval {
                    start: ongoing.dt,
                    end: None,
                },
            });
        }

        Ok(entries)
    }

    /// Append `lines` to the backing file (or stdout when path is None).
    ///
    /// Ensures the file ends with a newline before appending so the first new
    /// line is never merged with existing content.
    fn append_lines(&self, lines: &[EntryLine]) -> Result<(), StorageError> {
        let mut writer: Box<dyn Write> = match &self.path {
            Some(path) => {
                if let Ok(metadata) = fs::metadata(path) {
                    if metadata.len() > 0 {
                        let mut file = fs::File::open(path)?;
                        file.seek(io::SeekFrom::End(-1))?;
                        let mut buf = [0u8; 1];
                        file.read_exact(&mut buf)?;
                        if buf[0] != b'\n' {
                            let mut f = fs::OpenOptions::new().append(true).open(path)?;
                            writeln!(f)?;
                        }
                    }
                }
                Box::new(fs::OpenOptions::new().append(true).open(path)?)
            }
            None => Box::new(io::stdout()),
        };
        for line in lines {
            writeln!(writer, "{line}")?;
        }
        Ok(())
    }
}

impl Repository for FileRepository {
    fn list(&self, opts: QueryOpts) -> Result<Vec<&Entry>, StorageError> {
        Ok(self
            .entries
            .iter()
            .filter(|e| {
                e.interval.start >= opts.from.unwrap_or(DateTime::<Utc>::MIN_UTC)
                    && e.interval.start <= opts.to.unwrap_or(DateTime::<Utc>::MAX_UTC)
            })
            .skip(opts.offset.unwrap_or(0))
            .take(opts.limit.unwrap_or(usize::MAX))
            .collect())
    }

    fn start_session(&mut self, desc: &str, dt: DateTime<Utc>) -> Result<()> {
        let mut to_write: Vec<EntryLine> = Vec::with_capacity(2);

        // Auto-close any in-progress entry one second before the new start.
        if let Some(entry) = self.entries.pop_if(|e| e.interval.end.is_none()) {
            let end_dt = dt - TimeDelta::seconds(1);
            to_write.push(EntryLine {
                kind: EntryKind::End,
                desc: entry.desc.clone(),
                dt: end_dt,
            });
            self.entries.push(Entry {
                desc: entry.desc,
                interval: Interval {
                    start: entry.interval.start,
                    end: Some(end_dt),
                },
            });
        }

        to_write.push(EntryLine {
            kind: EntryKind::Start,
            desc: desc.to_owned(),
            dt,
        });

        self.append_lines(&to_write)?;

        self.entries.push(Entry {
            desc: desc.to_owned(),
            interval: Interval {
                start: dt,
                end: None,
            },
        });

        Ok(())
    }

    fn end_session(&mut self, dt: DateTime<Utc>) -> Result<()> {
        let entry = self
            .entries
            .pop_if(|e| e.interval.end.is_none())
            .ok_or_else(|| anyhow!("tried to end but nothing was started"))?;

        self.append_lines(&[EntryLine {
            kind: EntryKind::End,
            desc: entry.desc.clone(),
            dt,
        }])?;

        self.entries.push(Entry {
            desc: entry.desc,
            interval: Interval {
                start: entry.interval.start,
                end: Some(dt),
            },
        });

        Ok(())
    }
}

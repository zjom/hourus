use std::fs::{self, OpenOptions};
use std::path::PathBuf;
use std::str::FromStr;

use crate::entry::{Entry, EntryKind, EntryLine, Interval};
use crate::error::StorageError;
use crate::repository::{QueryOpts, Repository};
use anyhow::{Result, anyhow};
use chrono::{DateTime, TimeDelta, Utc};
use std::io::{self, BufRead, BufReader, Read, Seek, Write};

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
    /// Number of entries present at load time. Used by `flush` to know which
    /// entries are new and need to be written to stdout when path is None.
    initial_entry_count: usize,
}

impl FileRepository {
    pub fn new(path: Option<PathBuf>) -> anyhow::Result<FileRepository> {
        let reader: Box<dyn io::Read> = match &path {
            Some(p) => Box::new(
                OpenOptions::new()
                    .read(true)
                    .create(true)
                    .truncate(false)
                    .write(true)
                    .open(p)?,
            ),
            None => Box::new(io::stdin()),
        };

        let entries = FileRepository::load(reader)?;
        let initial_entry_count = entries.len();
        Ok(FileRepository {
            path,
            entries,
            initial_entry_count,
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

        let (pairs, tail) = if lines.last().is_some_and(|l| l.kind == EntryKind::Start) {
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
                if let Ok(metadata) = fs::metadata(path)
                    && metadata.len() > 0
                {
                    let mut file = fs::File::open(path)?;
                    file.seek(io::SeekFrom::End(-1))?;
                    let mut buf = [0u8; 1];
                    file.read_exact(&mut buf)?;
                    if buf[0] != b'\n' {
                        let mut f = fs::OpenOptions::new().append(true).open(path)?;
                        writeln!(f)?;
                    }
                }
                Box::new(fs::OpenOptions::new().append(true).open(path)?)
            }
            // When path is None the writes are deferred to `flush` so they
            // don't corrupt the TUI while it is rendering to the same terminal.
            None => return Ok(()),
        };
        for line in lines {
            writeln!(writer, "{line}")?;
        }
        Ok(())
    }
}

impl Repository for FileRepository {
    fn list(&self, opts: QueryOpts) -> Result<Vec<Entry>, StorageError> {
        Ok(self
            .entries
            .iter()
            .filter(|e| {
                e.interval.start >= opts.from.unwrap_or(DateTime::<Utc>::MIN_UTC)
                    && e.interval.start <= opts.to.unwrap_or(DateTime::<Utc>::MAX_UTC)
            })
            .skip(opts.offset.unwrap_or(0))
            .take(opts.limit.unwrap_or(usize::MAX))
            .map(|e| e.to_owned())
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

    fn flush(&mut self) -> Result<()> {
        if self.path.is_some() {
            return Ok(());
        }
        let mut stdout = io::stdout();
        for entry in &self.entries[self.initial_entry_count..] {
            writeln!(
                stdout,
                "{}",
                EntryLine {
                    kind: EntryKind::Start,
                    desc: entry.desc.clone(),
                    dt: entry.interval.start,
                }
            )?;
            if let Some(end) = entry.interval.end {
                writeln!(
                    stdout,
                    "{}",
                    EntryLine {
                        kind: EntryKind::End,
                        desc: entry.desc.clone(),
                        dt: end,
                    }
                )?;
            }
        }
        Ok(())
    }

    fn rename_current(&mut self, new_desc: &str) -> Result<()> {
        if let Some(entry) = self.entries.pop_if(|e| e.interval.end.is_none()) {
            if let Some(path) = self.path.as_ref() {
                let last_line =
                    get_last_line(path)?.and_then(|s| EntryLine::from_str(s.as_ref()).ok());
                if let Some(EntryLine {
                    kind: EntryKind::Start,
                    ..
                }) = last_line
                {
                    delete_last_line(path)?;
                }

                self.append_lines(&[EntryLine {
                    desc: new_desc.to_string(),
                    kind: EntryKind::Start,
                    dt: entry.interval.start,
                }])?;
            }
            self.entries.push(Entry {
                desc: new_desc.to_string(),
                ..entry
            });
        }
        Ok(())
    }
}

fn get_last_line(path: &PathBuf) -> io::Result<Option<String>> {
    let file = fs::File::open(path)?;
    let file_size = file.metadata()?.len();

    if file_size == 0 {
        return Ok(None);
    }

    let mut reader = BufReader::new(file);
    let mut pos = file_size;
    let mut last_line = String::new();

    // Move backward from the end to find the last newline
    while pos > 0 {
        pos -= 1;
        reader.seek(io::SeekFrom::Start(pos))?;
        let mut buffer = [0; 1];
        reader.read_exact(&mut buffer)?;

        if buffer[0] == b'\n' && pos < file_size - 1 {
            break;
        }
    }

    // Read from the identified position to the end
    reader.read_line(&mut last_line)?;
    Ok(Some(last_line.trim_end().to_string()))
}

fn delete_last_line(path: &PathBuf) -> io::Result<()> {
    let mut file = OpenOptions::new().read(true).write(true).open(path)?;
    let file_size = file.metadata()?.len();
    if file_size == 0 {
        return Ok(());
    }

    let mut pos = file_size - 1;
    let mut buffer = [0; 1];

    // Find the start of the last line
    while pos > 0 {
        pos -= 1;
        file.seek(io::SeekFrom::Start(pos))?;
        file.read_exact(&mut buffer)?;

        if buffer[0] == b'\n' {
            // We found the newline of the second-to-last line.
            // We want to keep this newline, so we truncate at pos + 1.
            pos += 1;
            break;
        }
    }

    Ok(file.set_len(pos)?)
}

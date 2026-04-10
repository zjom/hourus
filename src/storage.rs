use crate::entry::EntryLine;
use crate::error::StorageError;
use std::fs;
use std::io::{self, BufRead, BufReader, Write};
use std::io::{Read, Seek};
use std::path::PathBuf;
use std::str::FromStr;
/// Abstraction over where entry lines are stored and retrieved.
///
/// Implementations may target flat files, SQLite, remote APIs, etc.
/// The storage layer is responsible for serialization; the domain layer
/// (`Report`, `ReportBuilder`) only works with `Vec<EntryLine>`.
pub trait Storage {
    /// Load all entry lines from the backing store, in order.
    fn load(&self) -> Result<Vec<EntryLine>, StorageError>;

    /// Append entry lines to the backing store.
    fn append(&mut self, lines: &[EntryLine]) -> Result<(), StorageError>;
}

/// File-based storage: reads from a `.hours` file (or stdin) and appends to it.
///
/// `path = None` means stdin for reads and stdout for writes.
pub struct FileStorage {
    path: Option<PathBuf>,
}

impl FileStorage {
    pub fn new(path: Option<PathBuf>) -> Self {
        FileStorage { path }
    }
}

impl Storage for FileStorage {
    fn load(&self) -> Result<Vec<EntryLine>, StorageError> {
        let reader: Box<dyn io::Read> = match &self.path {
            Some(path) => Box::new(fs::File::open(path)?),
            None => Box::new(io::stdin()),
        };

        BufReader::new(reader)
            .lines()
            .map(|line| Ok(EntryLine::from_str(&line?)?))
            .collect()
    }

    fn append(&mut self, lines: &[EntryLine]) -> Result<(), StorageError> {
        let mut writer: Box<dyn Write> = match &self.path {
            Some(path) => {
                // Ensure file ends with a newline before appending
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

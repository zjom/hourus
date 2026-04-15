/// Errors arising from parsing entry lines or assembling entries.
/// Pure domain errors — no I/O.
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

/// Errors arising from reading or writing the backing store.
#[derive(thiserror::Error, Debug)]
pub enum StorageError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[cfg(feature = "sqlite")]
    #[error("sqlite error: {0}")]
    SqliteError(#[from] rusqlite::Error),

    #[error("parse error: {0}")]
    Parse(#[from] ParseError),
}

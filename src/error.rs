use std::io;

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

    #[error("io error")]
    IOError(#[from] io::Error),
}

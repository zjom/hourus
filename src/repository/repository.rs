use crate::entry::Entry;
use crate::error::StorageError;
use anyhow::Result;
use chrono::{DateTime, Utc};

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

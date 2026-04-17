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
    fn list(&self, opts: QueryOpts) -> Result<Vec<Entry>, StorageError>;

    fn start_session(&mut self, desc: &str, dt: DateTime<Utc>) -> Result<()>;

    fn end_session(&mut self, dt: DateTime<Utc>) -> Result<()>;

    fn rename_current(&mut self, new_desc: &str) -> Result<()>;

    /// Flush any buffered output. Only meaningful for stdout-backed repositories
    /// where writes were deferred (e.g. while a TUI was active). Default is a no-op.
    fn flush(&mut self) -> Result<()> {
        Ok(())
    }
}

impl Repository for Box<dyn Repository> {
    fn list(&self, opts: QueryOpts) -> Result<Vec<Entry>, StorageError> {
        (**self).list(opts)
    }

    fn start_session(
        &mut self,
        desc: &str,
        dt: chrono::DateTime<chrono::Utc>,
    ) -> anyhow::Result<()> {
        (**self).start_session(desc, dt)
    }

    fn end_session(&mut self, dt: chrono::DateTime<chrono::Utc>) -> anyhow::Result<()> {
        (**self).end_session(dt)
    }

    fn rename_current(&mut self, new_desc: &str) -> Result<()> {
        (**self).rename_current(new_desc)
    }

    fn flush(&mut self) -> Result<()> {
        (**self).flush()
    }
}

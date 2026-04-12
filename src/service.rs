use crate::entry::Entry;
use crate::error::StorageError;
use crate::repository::{QueryOpts, Repository};
use anyhow::Result;
use chrono::{DateTime, TimeDelta, Utc};

// ---------------------------------------------------------------------------
// Session state
// ---------------------------------------------------------------------------

/// The observable state of the current tracking session.
pub enum SessionStatus {
    Idle,
    Active {
        desc: String,
        started_at: DateTime<Utc>,
    },
    Paused {
        desc: String,
    },
}

// ---------------------------------------------------------------------------
// SessionService
// ---------------------------------------------------------------------------

/// Orchestrates session transitions on top of a `Repository`.
///
/// Deliberately lean: only the current session state is kept in memory.
/// Callers that need historical aggregations (summary, today-total, etc.)
/// should query via [`SessionService::list`] and compute on the result.
pub struct SessionService<R: Repository> {
    repo: R,
    status: SessionStatus,
}

impl<R: Repository> SessionService<R> {
    /// Build a `SessionService` by loading all entries from `repo` once.
    pub fn new(repo: R) -> Result<Self> {
        let status = {
            let entries = repo.list(QueryOpts::default())?;
            match entries.last() {
                Some(e) if e.interval.end.is_none() => SessionStatus::Active {
                    desc: e.desc.clone(),
                    started_at: e.interval.start,
                },
                _ => SessionStatus::Idle,
            }
        };

        Ok(SessionService { repo, status })
    }

    // -----------------------------------------------------------------------
    // Commands — each writes to the repository before mutating in-memory state
    // -----------------------------------------------------------------------

    /// Start a new session with `desc`, auto-closing the current one if active.
    pub fn start(&mut self, desc: &str, now: DateTime<Utc>) -> Result<()> {
        self.repo.start_session(desc, now)?;
        self.status = SessionStatus::Active {
            desc: desc.to_owned(),
            started_at: now,
        };
        Ok(())
    }

    /// Pause the active session, writing an END line.
    pub fn pause(&mut self, now: DateTime<Utc>) -> Result<()> {
        let SessionStatus::Active { desc, .. } = &self.status else {
            return Ok(());
        };
        let desc = desc.clone();
        self.repo.end_session(now)?;
        self.status = SessionStatus::Paused { desc };
        Ok(())
    }

    /// Resume a paused session, writing a START line.
    pub fn resume(&mut self, now: DateTime<Utc>) -> Result<()> {
        let SessionStatus::Paused { desc } = &self.status else {
            return Ok(());
        };
        let desc = desc.clone();
        self.repo.start_session(&desc, now)?;
        self.status = SessionStatus::Active {
            desc,
            started_at: now,
        };
        Ok(())
    }

    /// End the active session, writing an END line.
    /// Returns an error if no session is currently active.
    pub fn end(&mut self, now: DateTime<Utc>) -> Result<()> {
        if !matches!(self.status, SessionStatus::Active { .. }) {
            anyhow::bail!("tried to end but nothing was started");
        }
        self.repo.end_session(now)?;
        self.status = SessionStatus::Idle;
        Ok(())
    }

    /// Discard a paused session. No file write — the END line was already
    /// written when the session was paused.
    pub fn discard_paused(&mut self) {
        if matches!(self.status, SessionStatus::Paused { .. }) {
            self.status = SessionStatus::Idle;
        }
    }

    // -----------------------------------------------------------------------
    // Queries
    // -----------------------------------------------------------------------

    pub fn status(&self) -> &SessionStatus {
        &self.status
    }

    /// Return all entries that satisfy `opts`, borrowing from the repository.
    pub fn list(&self, opts: QueryOpts) -> Result<Vec<&Entry>, StorageError> {
        self.repo.list(opts)
    }
}

// ---------------------------------------------------------------------------
// Utilities
// ---------------------------------------------------------------------------

/// Aggregate completed entries by description, sorted by total duration desc.
pub fn summarize(entries: &[&Entry]) -> Vec<(String, TimeDelta)> {
    use std::collections::HashMap;
    let mut map: HashMap<&str, TimeDelta> = HashMap::new();
    for e in entries {
        if e.interval.end.is_some() {
            *map.entry(e.desc.as_str()).or_default() += e.interval.duration();
        }
    }
    let mut result: Vec<(String, TimeDelta)> =
        map.into_iter().map(|(k, v)| (k.to_owned(), v)).collect();
    result.sort_by(|a, b| b.1.cmp(&a.1));
    result
}

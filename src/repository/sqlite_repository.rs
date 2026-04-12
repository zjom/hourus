use crate::entry::{Entry, Interval};
use crate::error::StorageError;
use anyhow::{Result, anyhow};
use chrono::{DateTime, TimeDelta, Utc};
use rusqlite::{Connection, OptionalExtension, Row, params, types::Value};
use std::path::Path;

use crate::repository::Repository;

pub struct SqliteRepository {
    conn: Connection,
}

impl SqliteRepository {
    pub fn new<P>(path: P) -> Result<Self>
    where
        P: AsRef<Path>,
    {
        let conn = Connection::open(path)?;
        let repo = SqliteRepository { conn };
        repo.migrate()?;
        Ok(repo)
    }

    fn migrate(&self) -> Result<()> {
        if self.conn.table_exists(None, "session")? {
            return Ok(());
        }

        self.conn.execute(
            "CREATE TABLE IF NOT EXISTS session (id integer primary key, desc text not null, start_ts integer not null, end_ts integer)",
            (),
        )?;

        self.conn.execute(
            "CREATE UNIQUE INDEX if not exists idx_session_start_ts on session (start_ts)",
            (),
        )?;

        Ok(())
    }

    fn last_open(&self) -> Result<Option<Entry>> {
        let query = "SELECT desc, start_ts, end_ts FROM session WHERE end_ts IS NULL ORDER BY start_ts DESC LIMIT 1";
        Ok(self
            .conn
            .query_row(query, (), SqliteRepository::map_row_to_entry)
            .optional()?)
    }

    fn map_row_to_entry(row: &Row) -> rusqlite::Result<Entry> {
        let start_ts: i64 = row.get(1)?;
        let end_ts: Option<i64> = row.get(2)?;
        Ok(Entry {
            desc: row.get(0)?,
            interval: Interval {
                start: DateTime::from_timestamp(start_ts, 0).unwrap_or_default(),
                end: end_ts.and_then(|ts| DateTime::from_timestamp(ts, 0)),
            },
        })
    }
}

impl Repository for SqliteRepository {
    fn list(&self, opts: super::QueryOpts) -> Result<Vec<Entry>, StorageError> {
        let mut query = String::from("SELECT desc, start_ts, end_ts FROM session WHERE 1=1");
        let mut params: Vec<Value> = Vec::new();

        if let Some(from) = opts.from {
            query.push_str(" AND start_ts >= ?");
            params.push(Value::Integer(from.timestamp()));
        }

        if let Some(to) = opts.to {
            query.push_str(" AND start_ts <= ?");
            params.push(Value::Integer(to.timestamp()));
        }

        query.push_str(" ORDER BY start_ts ASC");

        let limit = opts.limit.map(|l| l as i64);
        let offset = opts.offset.map(|o| o as i64);

        if let Some(l) = limit {
            query.push_str(" LIMIT ?");
            params.push(Value::Integer(l));
        } else if opts.offset.is_some() {
            query.push_str(" LIMIT -1");
        }

        if let Some(o) = offset {
            query.push_str(" OFFSET ?");
            params.push(Value::Integer(o));
        }

        let mut stmt = self.conn.prepare(&query)?;
        let entries: Vec<Entry> = stmt
            .query_map(rusqlite::params_from_iter(params), SqliteRepository::map_row_to_entry)?
            .collect::<Result<_, _>>()?;

        Ok(entries)
    }

    fn start_session(&mut self, desc: &str, dt: DateTime<Utc>) -> Result<()> {
        // Auto-close any open session one second before the new start.
        if let Some(Entry {
            interval: Interval { start, end: None },
            ..
        }) = self.last_open()?
        {
            let end_ts = (dt - TimeDelta::seconds(1)).timestamp();
            self.conn.execute(
                "UPDATE session SET end_ts = ? WHERE start_ts = ? AND end_ts IS NULL",
                params![end_ts, start.timestamp()],
            )?;
        }

        self.conn.execute(
            "INSERT INTO session (desc, start_ts) VALUES (?, ?)",
            params![desc, dt.timestamp()],
        )?;

        Ok(())
    }

    fn end_session(&mut self, dt: DateTime<Utc>) -> Result<()> {
        let open = self
            .last_open()?
            .ok_or_else(|| anyhow!("tried to end but nothing was started"))?;

        self.conn.execute(
            "UPDATE session SET end_ts = ? WHERE start_ts = ? AND end_ts IS NULL",
            params![dt.timestamp(), open.interval.start.timestamp()],
        )?;

        Ok(())
    }
}

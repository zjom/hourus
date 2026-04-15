use anyhow::Result;
use chrono::{NaiveDate, NaiveTime, TimeZone, Utc};
use clap::{Parser, Subcommand};
use std::env;
use std::io;
use std::path::PathBuf;

use crate::output::OutputFormat;
use crate::repository::Repository;
#[cfg(feature = "sqlite")]
use crate::repository::SqliteRepository;
use crate::repository::{FileRepository, QueryOpts};
use crate::service::{SessionService, summarize};
use crate::tui;

/// Parses and summarises .hours log files.
///
/// Data can be passed via `--path`, the `HOURUS_DEFAULT_FILE` environment variable, or stdin.
#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
pub struct Cli {
    /// Path to the .hours file.
    /// Falls back to the HOURUS_DEFAULT_FILE environment variable unless --no-env is set.
    #[arg(short, long)]
    pub path: Option<String>,

    /// Ignore the HOURUS_DEFAULT_FILE environment variable.
    #[arg(long)]
    pub no_env: bool,

    /// Only include entries on or after this date.
    #[arg(short, long)]
    pub from: Option<NaiveDate>,

    /// Only include entries on or before this date.
    #[arg(short, long)]
    pub to: Option<NaiveDate>,

    #[command(subcommand)]
    pub command: Option<Commands>,

    /// Output format.
    #[arg(long, value_enum, default_value_t = OutputFormat::Pretty)]
    format: OutputFormat,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Print a breakdown of hours by task.
    Breakdown {
        #[arg(short, long)]
        from: Option<NaiveDate>,

        #[arg(short, long)]
        to: Option<NaiveDate>,

        /// Output format.
        #[arg(long, value_enum, default_value_t = OutputFormat::Pretty)]
        format: OutputFormat,
    },
    /// Start a new session, auto-ending the current one if open.
    /// Ignores --from and --to.
    Start {
        /// Description of the task being started. Required unless --interactive is set.
        desc: Option<String>,

        /// Start in interactive mode.
        #[arg(short, long)]
        interactive: bool,
    },
    /// End the current session. Fails if no session is ongoing.
    /// Ignores --from and --to.
    End {},
}

static ENV_KEY: &str = "HOURUS_DEFAULT_FILE";

#[cfg(feature = "sqlite")]
fn repo_for_path(p: PathBuf) -> Result<Box<dyn Repository>> {
    let is_db = p
        .extension()
        .and_then(|ext| ext.to_str())
        .map_or(false, |ext| ["db", "sqlite"].contains(&ext));
    if is_db {
        Ok(Box::new(SqliteRepository::new(p)?))
    } else {
        Ok(Box::new(FileRepository::new(Some(p))?))
    }
}

#[cfg(not(feature = "sqlite"))]
fn repo_for_path(p: PathBuf) -> Result<Box<dyn Repository>> {
    Ok(Box::new(FileRepository::new(Some(p))?))
}

pub fn run() -> Result<()> {
    let cli = Cli::parse();

    // Resolve the file path: explicit --path takes priority; then the env var (unless
    // --no-env is set); then fall back to stdin/stdout.
    let path: Option<PathBuf> = cli.path.map(PathBuf::from).or_else(|| {
        if cli.no_env {
            None
        } else {
            env::var(ENV_KEY).ok().map(PathBuf::from)
        }
    });
    let repo: Box<dyn Repository> = match path {
        Some(p) => repo_for_path(p)?,
        None => Box::new(FileRepository::new(None)?),
    };

    let mut service = SessionService::new(repo)?;
    let stdout = &mut io::stdout();

    match &cli.command {
        Some(Commands::Breakdown { from, to, format }) => {
            let entries = service.list(QueryOpts {
                from: from.map(date_start),
                to: to.map(date_end),
                ..Default::default()
            })?;
            let summary = summarize(&entries);
            let total = summary.iter().map(|(_, d)| *d).sum();
            format.write_breakdown(stdout, &summary, total)?;
        }
        Some(Commands::Start {
            desc,
            interactive: true,
        }) => tui::run(service, desc.clone())?,
        Some(Commands::Start {
            desc: Some(desc), ..
        }) => service.start(desc, Utc::now())?,
        Some(Commands::Start { desc: None, .. }) => {
            anyhow::bail!("a description is required when not using --interactive");
        }
        Some(Commands::End {}) => service.end(Utc::now())?,
        None => {
            let entries = service.list(QueryOpts {
                from: cli.from.map(date_start),
                to: cli.to.map(date_end),
                ..Default::default()
            })?;
            let total = summarize(&entries).iter().map(|(_, d)| *d).sum();
            cli.format.write_total(stdout, total)?
        }
    }

    Ok(())
}

/// Resolve a date to the start of that day in UTC.
fn date_start(date: NaiveDate) -> chrono::DateTime<Utc> {
    Utc.from_utc_datetime(&date.and_time(NaiveTime::MIN))
}

/// Resolve a date to the last second of that day in UTC.
fn date_end(date: NaiveDate) -> chrono::DateTime<Utc> {
    Utc.from_utc_datetime(&date.and_hms_opt(23, 59, 59).unwrap())
}

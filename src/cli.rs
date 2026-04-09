use anyhow::Result;
use chrono::{Local, NaiveDate, NaiveDateTime, NaiveTime};
use clap::{Parser, Subcommand};
use std::env;
use std::io;
use std::path::PathBuf;

use crate::output::OutputFormat;
use crate::report::Report;
use crate::storage::{FileStorage, Storage};
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
        /// Description of the task being started.
        desc: String,

        /// Start in interactive mode.
        #[arg(short, long)]
        interactive: bool,
    },
    /// End the current session. Fails if no session is ongoing.
    /// Ignores --from and --to.
    End {},
}

static ENV_KEY: &str = "HOURUS_DEFAULT_FILE";

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

    let mut storage = FileStorage::new(path);
    let stdout = &mut io::stdout();

    match &cli.command {
        Some(Commands::Breakdown { from, to, format }) => {
            let report = Report::new()
                .with_lines(storage.load()?)
                .from(date_start(from))
                .to(date_end(to))
                .build()?;

            format.write_breakdown(stdout, &report.summarize(), report.total_duration())?;
        }
        Some(Commands::Start { desc, interactive }) if *interactive => {
            return tui::run(storage);
        }
        Some(Commands::Start { desc, .. }) => {
            let report = Report::new().with_lines(storage.load()?).build()?;
            let entries = report.build_start_entries(desc, Local::now().naive_local())?;
            storage.append(&entries)?;
        }
        Some(Commands::End {}) => {
            let report = Report::new().with_lines(storage.load()?).build()?;
            let entry = report.build_end_entry(Local::now().naive_local())?;
            storage.append(&[entry])?;
        }
        None => {
            let report = Report::new()
                .with_lines(storage.load()?)
                .from(date_start(&cli.from))
                .to(date_end(&cli.to))
                .build()?;

            cli.format.write_total(stdout, report.total_duration())?
        }
    }

    Ok(())
}

/// Resolve an optional date to the start of that day (or the minimum date if absent).
fn date_start(date: &Option<NaiveDate>) -> NaiveDateTime {
    date.unwrap_or(NaiveDate::MIN).and_time(NaiveTime::MIN)
}

/// Resolve an optional date to the last second of that day (or today if absent).
fn date_end(date: &Option<NaiveDate>) -> NaiveDateTime {
    date.unwrap_or_else(|| Local::now().date_naive())
        .and_time(NaiveTime::from_hms_opt(23, 59, 59).unwrap())
}

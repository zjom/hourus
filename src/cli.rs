use anyhow::Result;
use chrono::{Local, NaiveDate, NaiveTime};
use clap::{Parser, Subcommand};
use std::fs::{self, File};
use std::io::{self, Read, Write};

use crate::report::Report;

/// Parses and summarises .hours log file. Data can be passed in via the --path flag or stdin
/// (default).
#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
pub struct Cli {
    /// Path to .hours file.
    #[arg(short, long)]
    pub path: Option<String>,

    #[arg(short, long)]
    pub from: Option<NaiveDate>,

    #[arg(short, long)]
    pub to: Option<NaiveDate>,

    #[command(subcommand)]
    pub command: Option<Commands>,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Prints breakdown of hours by task
    Breakdown {
        #[arg(short, long)]
        from: Option<NaiveDate>,

        #[arg(short, long)]
        to: Option<NaiveDate>,
    },
    /// Appends new session to file, ending current if exists.
    /// specified. Does not respect --from and --to flags.
    Start {
        /// Description of the entry
        desc: String,
    },
    /// Ends current session. Fails if no session is ongoing.
    /// Does not respect --from and --to flags.
    End {},
}

pub fn run() -> Result<()> {
    let cli = Cli::parse();
    let reader = get_file_reader(cli.path.as_deref())?;
    let report_builder = Report::new().with_reader(reader);
    match &cli.command {
        Some(Commands::Breakdown { from, to }) => {
            let from = from.unwrap_or(NaiveDate::MIN).and_time(NaiveTime::MIN);
            let to = to
                .unwrap_or(Local::now().date_naive())
                .and_time(NaiveTime::MIN);

            let report = report_builder.from(from).to(to).build()?;

            let summary = report.summarize();
            for (desc, dur) in &summary {
                println!("{desc}: {}h {}m", dur.num_hours(), dur.num_minutes() % 60);
            }

            let total = report.total_duration();
            println!(
                "Total: {}h {}m",
                total.num_hours(),
                total.num_minutes() % 60
            );
        }
        Some(Commands::Start { desc }) => {
            let report = report_builder.build()?;
            let now = chrono::Local::now().naive_local();
            let mut writer = get_file_writer(cli.path.as_deref())?;
            let entries = report.build_start_entries(desc, now)?;
            for entry in entries {
                write!(writer, "\n{entry}")?;
            }
        }
        Some(Commands::End {}) => {
            let report = report_builder.build()?;
            let now = chrono::Local::now().naive_local();
            let mut writer = get_file_writer(cli.path.as_deref())?;
            let entry = report.build_end_entry(now)?;
            write!(writer, "\n{entry}")?;
        }
        None => {
            let from = cli.from.unwrap_or(NaiveDate::MIN).and_time(NaiveTime::MIN);
            let to = cli
                .to
                .unwrap_or(Local::now().date_naive())
                .and_time(NaiveTime::MIN);
            let report = report_builder.from(from).to(to).build()?;

            let total = report.total_duration();
            println!(
                "Total: {}h {}m",
                total.num_hours(),
                total.num_minutes() % 60
            );
        }
    }

    Ok(())
}

fn get_file_reader(path: Option<&str>) -> Result<Box<dyn Read>> {
    Ok(match path {
        Some(path) => Box::new(File::open(path)?),
        None => Box::new(io::stdin()),
    })
}

fn get_file_writer(path: Option<&str>) -> Result<Box<dyn Write>> {
    Ok(match path {
        Some(path) => Box::new(fs::OpenOptions::new().append(true).open(path)?),
        None => Box::new(io::stdout()),
    })
}

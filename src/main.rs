mod cli;
mod entry;
mod error;
mod report;

use anyhow::Result;
use clap::Parser;
use std::io::Read;
use std::process;

use cli::{Cli, Commands, get_file_reader, get_file_writer};

fn run() -> Result<()> {
    let cli = Cli::parse();
    let mut reader = get_file_reader(cli.path.as_deref())?;
    let mut input_buf = String::new();
    reader.read_to_string(&mut input_buf)?;
    let report: report::Report = input_buf.parse()?;

    match &cli.command {
        Some(Commands::Breakdown) => {
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
            let now = chrono::Local::now().naive_local();
            let mut writer = get_file_writer(cli.path.as_deref())?;
            let entries = report.build_start_entries(desc, now)?;
            for entry in entries {
                write!(writer, "\n{entry}")?;
            }
        }
        None => {
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

fn main() {
    if let Err(err) = run() {
        eprintln!("{err}");
        process::exit(1);
    }
}

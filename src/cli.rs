use anyhow::Result;
use clap::{Parser, Subcommand};
use std::fs::{self, File};
use std::io::{self, Read, Write};

/// Parses and summarises .hours log file. Data can be passed in via the --path flag or stdin
/// (default).
#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
pub struct Cli {
    /// Path to .hours file
    #[arg(short, long)]
    pub path: Option<String>,

    #[command(subcommand)]
    pub command: Option<Commands>,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Prints breakdown of hours by task
    Breakdown,
    /// Appends new session to file, ending current if exists. Outputs to stdout if no path
    /// specified
    Start {
        /// Description of the entry
        #[arg(short)]
        desc: String,
    },
    /// Ends current session. Fails if no session is ongoing.
    End {},
}

pub fn get_file_reader(path: Option<&str>) -> Result<Box<dyn Read>> {
    Ok(match path {
        Some(path) => Box::new(File::open(path)?),
        None => Box::new(io::stdin()),
    })
}

pub fn get_file_writer(path: Option<&str>) -> Result<Box<dyn Write>> {
    Ok(match path {
        Some(path) => Box::new(fs::OpenOptions::new().append(true).open(path)?),
        None => Box::new(io::stdout()),
    })
}

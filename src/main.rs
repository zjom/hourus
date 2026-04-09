mod cli;
mod entry;
mod error;
mod output;
mod report;
mod storage;
mod tui;

use std::process;

use cli::run;

fn main() {
    if let Err(err) = run() {
        eprintln!("{err}");
        process::exit(1);
    }
}

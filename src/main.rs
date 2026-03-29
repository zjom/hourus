mod cli;
mod entry;
mod error;
mod report;

use std::process;

use cli::run;

fn main() {
    if let Err(err) = run() {
        eprintln!("{err}");
        process::exit(1);
    }
}

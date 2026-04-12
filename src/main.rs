mod cli;
mod entry;
mod error;
mod history;
mod output;
mod repository;
mod service;
mod tui;

use std::process;

use cli::run;

fn main() {
    if let Err(err) = run() {
        eprintln!("{err}");
        process::exit(1);
    }
}

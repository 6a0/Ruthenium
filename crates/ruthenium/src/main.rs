//! Ruthenium: composition root.

#![forbid(unsafe_code)]

mod prepare_cmd;

use std::process::ExitCode;

/// Shown on every first-run, About and title screen (Minecraft Usage Guidelines).
pub const DISCLAIMER: &str =
    "NOT AN OFFICIAL MINECRAFT PRODUCT. NOT APPROVED BY OR ASSOCIATED WITH MOJANG OR MICROSOFT.";

const USAGE: &str = "\
usage: ruthenium [command]

commands:
  locate  [--from <folder>]
      List the launcher folders searched for a verified game install.
  prepare [--from <folder>] [--data-dir <folder>] [--java <java>]
      Import the game files from your launcher into Ruthenium's data folder and
      run Mojang's data generator on them. Runs only what is out of date.";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        None => {
            println!("Ruthenium {}", env!("CARGO_PKG_VERSION"));
            println!("{DISCLAIMER}");
            ExitCode::SUCCESS
        }
        Some("locate") => prepare_cmd::locate(&args[1..]),
        Some("prepare") => prepare_cmd::prepare(&args[1..]),
        Some(_) => {
            eprintln!("{USAGE}");
            ExitCode::FAILURE
        }
    }
}

//! Developer tasks. Run with `cargo xtask <task>`.

#![forbid(unsafe_code)]

mod check_content;

use std::process::ExitCode;

const USAGE: &str = "\
usage: cargo xtask <task>

tasks:
  check-content [--jar <client.jar>]... [--asset-index <index.json>]...
      Fail if any tracked or untracked (non-ignored) file looks like Mojang
      content. With --jar / --asset-index, also fail if any file is byte-identical
      to an entry of that jar or an object of that asset index. If no --jar is
      given and reference/26.3/jars/client.jar exists, it is used automatically.
  codegen
      Generate ruthenium-data tables (not implemented yet).";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("check-content") => check_content::run(&args[1..]),
        Some("codegen") => {
            eprintln!("codegen: not implemented yet");
            ExitCode::FAILURE
        }
        _ => {
            eprintln!("{USAGE}");
            ExitCode::FAILURE
        }
    }
}

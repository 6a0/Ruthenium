//! Ruthenium: composition root.

#![forbid(unsafe_code)]

mod account;
mod assets_cmd;
mod auth_cmd;
mod client;
mod menu;
mod prepare_cmd;
mod servers;

use std::process::ExitCode;
use std::time::Instant;

/// Printed by `ruthenium version` and at start-up, and for the first-run and About screens
/// (Minecraft Usage Guidelines). Not on the title screen: no launcher puts it there.
pub const DISCLAIMER: &str =
    "NOT AN OFFICIAL MINECRAFT PRODUCT. NOT APPROVED BY OR ASSOCIATED WITH MOJANG OR MICROSOFT.";

const USAGE: &str = "\
usage: ruthenium [command]

With no command, opens the game window. Options:
  --vsync | --no-vsync   VSync on (default) or off (tearing, else mailbox)
  --fps-cap <fps>        frame limiter (sleep, then spin, to exact deadlines)
  --gpu <name>           prefer a GPU whose name contains <name> (or RUTHENIUM_GPU)
  --validation | --no-validation
                         Vulkan validation layer (default: on in debug builds if
                         installed; RUTHENIUM_VALIDATION=1 requires it)
  --stats                print frame-time statistics every second
  --overlay              start with the frame-stats overlay shown (F3 toggles it)
  --bench-out <file>     write a JSON summary of the run at exit (cargo xtask bench ab)
  --quit-after <s>       close after <s> seconds (for benchmarks and tests)
  --exercise-swapchain   resize and toggle VSync every few frames (development)
  --no-pipeline-cache    don't load or save the pipeline cache
  --no-transfer-queue    upload on the graphics queue even if there is a copy queue
  --exercise-uploads     check data pushed through the upload ring every frame
                         (development; a mismatch fails the run)
  --screenshot <file>    write the window to a PNG, then quit
                         (development)
  --data-dir <folder>    Ruthenium's data folder (accounts, servers); default:
                         the per-user one
In the window, V toggles VSync and F3 the frame-stats overlay.

commands:
  version
      Print the version and the disclaimer.
  locate  [--from <folder>]
      List the launcher folders searched for a verified game install.
  prepare [--from <folder>] [--data-dir <folder>] [--java <java>] [--exporters <jar>]
      Import the game files from your launcher into Ruthenium's data folder,
      then run Mojang's data generator and Ruthenium's exporters on them. Runs
      only what is out of date.
  assets  [--data-dir <folder>] [--pack <zip or folder>]... [--mip-levels <0-4>]
          [--dump <folder>]
      Build the sprite atlases from the imported game files and the given
      resource packs (lowest first), print their sizes, and optionally write
      every atlas level as PNG with a sprite index.
  auth login [--device] | list | check [<name>] [--certificates] | logout <name>
      Sign in with a Microsoft account (in the browser, or with a code to enter
      on another device), list the accounts signed in, check one by signing in
      again from its saved sign-in, or sign one out. Saved sign-ins live in the
      system's credential store.";

fn main() -> ExitCode {
    let started = Instant::now();
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("version") => {
            println!("Ruthenium {}", env!("CARGO_PKG_VERSION"));
            println!("{DISCLAIMER}");
            ExitCode::SUCCESS
        }
        Some("locate") => prepare_cmd::locate(&args[1..]),
        Some("prepare") => prepare_cmd::prepare(&args[1..]),
        Some("assets") => assets_cmd::run(&args[1..]),
        Some("auth") => auth_cmd::run(&args[1..]),
        Some("help" | "--help" | "-h") => {
            println!("{USAGE}");
            ExitCode::SUCCESS
        }
        a if a.is_none_or(|a| a.starts_with("--")) => match client::Options::parse(&args) {
            Ok(opts) => {
                println!("Ruthenium {} - {DISCLAIMER}", env!("CARGO_PKG_VERSION"));
                client::run(&opts, started)
            }
            Err(e) => {
                eprintln!("ruthenium: {e}\n\n{USAGE}");
                ExitCode::FAILURE
            }
        },
        _ => {
            eprintln!("{USAGE}");
            ExitCode::FAILURE
        }
    }
}

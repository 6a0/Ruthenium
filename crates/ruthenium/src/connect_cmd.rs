//! `ruthenium connect`: joins a server without a window and prints what happens, to
//! try the connection path from a shell (development).

use std::path::PathBuf;
use std::process::ExitCode;
use std::time::{Duration, Instant};

use ruthenium_prepare::discover::Env;
use ruthenium_sim::session::SessionEvent;

use crate::account::{AccountStatus, Accounts};
use crate::join::{Joiner, closed_text, stage_text};

pub fn run(args: &[String]) -> ExitCode {
    match run_inner(args) {
        Ok(code) => code,
        Err(e) => {
            eprintln!("ruthenium connect: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run_inner(args: &[String]) -> Result<ExitCode, String> {
    let mut address = None;
    let mut seconds = 10.0;
    let mut data_dir = None;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--seconds" => {
                let v = it.next().ok_or("--seconds needs a number")?;
                seconds = v
                    .parse::<f64>()
                    .ok()
                    .filter(|s| (0.0..=86_400.0).contains(s))
                    .ok_or(format!("--seconds: not a number of seconds: {v}"))?;
            }
            "--data-dir" => {
                data_dir = Some(PathBuf::from(it.next().ok_or("--data-dir needs a folder")?));
            }
            other if !other.starts_with("--") && address.is_none() => {
                address = Some(other.to_owned());
            }
            other => return Err(format!("unknown option {other}")),
        }
    }
    let address = address.ok_or("give the server's address")?;
    let env = Env::from_process();
    let data_dir = data_dir
        .or_else(|| ruthenium_prepare::default_data_dir(&env))
        .ok_or("no data folder: set --data-dir")?;
    let cache_dir = ruthenium_prepare::default_cache_dir(&env);
    let mut accounts = Accounts::start(Some(data_dir.clone()), cache_dir.as_deref());
    // The account in use signs in again first (for online servers).
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        accounts.poll();
        if !matches!(accounts.status(), AccountStatus::Checking(_)) || Instant::now() > deadline {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let mut joiner = Joiner::new(Some(data_dir.join("game")), cache_dir);
    let data = joiner.data();
    let session = joiner.join(&address, &accounts)?;
    let started = Instant::now();
    let stay = Duration::from_secs_f64(seconds);
    let mut playing = false;
    let mut leaving = false;
    loop {
        let mut closed = None;
        for e in session.poll() {
            let t = started.elapsed().as_secs_f64();
            match &e {
                SessionEvent::Stage(s) => println!("{t:7.3}s  {}", stage_text(*s)),
                SessionEvent::LoggedIn { name } => println!("{t:7.3}s  logged in as {name}"),
                SessionEvent::CodeOfConduct(text) => {
                    println!(
                        "{t:7.3}s  code of conduct (declined: answer it in the window):\n{text}"
                    );
                    session.answer_code_of_conduct(false);
                }
                SessionEvent::Playing => {
                    playing = true;
                    println!("{t:7.3}s  playing");
                }
                SessionEvent::Reconfiguring => println!("{t:7.3}s  reconfiguring"),
                SessionEvent::Closed(c) => {
                    let (title, reason) = closed_text(c, &data.lang);
                    println!("{t:7.3}s  closed: {title}: {reason}");
                    closed = Some(c.clone());
                }
            }
        }
        if let Some(c) = closed {
            let ok = playing && (leaving || crate::join::left_on_purpose(&c));
            return Ok(if ok {
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            });
        }
        if !leaving && started.elapsed() >= stay {
            leaving = true;
            session.disconnect();
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

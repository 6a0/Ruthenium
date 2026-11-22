//! `ruthenium auth`: sign in, list, check and sign out accounts from the command line.

use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::atomic::AtomicBool;

use ruthenium_auth::{AccountStore, Auth, SignedIn, open_browser};
use ruthenium_prepare::discover::Env;

fn data_dir(args: &[String]) -> Result<PathBuf, String> {
    match args.iter().position(|a| a == "--data-dir") {
        Some(i) => args
            .get(i + 1)
            .map(PathBuf::from)
            .ok_or("--data-dir needs a folder".into()),
        None => ruthenium_prepare::default_data_dir(&Env::from_process())
            .ok_or_else(|| "no data folder: set --data-dir".into()),
    }
}

fn report(s: &SignedIn) {
    let p = &s.session.profile;
    let v = &s.session.privileges;
    println!("Signed in as {} ({})", p.name, p.id);
    println!(
        "  multiplayer: {}, chat: {}, realms: {}",
        if v.multiplayer_server {
            "allowed"
        } else {
            "not allowed"
        },
        if v.online_chat {
            "allowed"
        } else {
            "not allowed"
        },
        if v.multiplayer_realms {
            "allowed"
        } else {
            "not allowed"
        },
    );
    if let Some(b) = &v.multiplayer_ban {
        println!("  banned from multiplayer: {}", b.reason);
    }
}

pub fn run(args: &[String]) -> ExitCode {
    match run_inner(args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("ruthenium auth: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run_inner(args: &[String]) -> Result<(), String> {
    let dir = data_dir(args)?;
    let mut store = AccountStore::open(&dir);
    let cancel = AtomicBool::new(false);
    match args.first().map(String::as_str) {
        Some("login") => {
            let mut auth = Auth::new();
            let signed = if args.iter().any(|a| a == "--device") {
                auth.sign_in_device(
                    &mut |code| {
                        println!("{}", code.message);
                    },
                    &cancel,
                )
            } else {
                auth.sign_in_browser(
                    &mut |url| {
                        println!("Opening the Microsoft sign-in page in your browser. If it doesn't open, go to:\n{url}");
                        if let Err(e) = open_browser(url) {
                            eprintln!("(couldn't open a browser: {e})");
                        }
                    },
                    &cancel,
                )
            }
            .map_err(|e| e.to_string())?;
            let p = &signed.session.profile;
            store
                .signed_in(p, &signed.refresh_token)
                .map_err(|e| e.to_string())?;
            report(&signed);
            Ok(())
        }
        Some("list") => {
            if store.accounts().is_empty() {
                println!("No accounts. Sign in with: ruthenium auth login");
            }
            for (i, a) in store.accounts().iter().enumerate() {
                println!("{} {} ({})", if i == 0 { "*" } else { " " }, a.name, a.id);
            }
            Ok(())
        }
        Some("check") => {
            let who = args.get(1).filter(|a| !a.starts_with("--"));
            let account = match who {
                Some(w) => store.find(w),
                None => store.current(),
            }
            .cloned()
            .ok_or("no such account (see: ruthenium auth list)")?;
            let token = store
                .refresh_token(&account.id)
                .map_err(|e| e.to_string())?
                .ok_or("this account's sign-in is missing from the credential store: sign in again")?;
            let mut auth = Auth::new();
            let signed = auth.resume(&token).map_err(|e| e.to_string())?;
            store
                .signed_in(&signed.session.profile, &signed.refresh_token)
                .map_err(|e| e.to_string())?;
            report(&signed);
            if args.iter().any(|a| a == "--certificates") {
                let c = auth.certificates(&signed.session).map_err(|e| e.to_string())?;
                println!("  chat certificates valid until {} (kept in memory only)", c.expires_at);
            }
            Ok(())
        }
        Some("logout") => {
            let who = args.get(1).ok_or("logout needs an account name")?;
            let id = store.find(who).ok_or("no such account")?.id.clone();
            store.remove(&id).map_err(|e| e.to_string())?;
            println!("Signed out {who}.");
            Ok(())
        }
        _ => Err("usage: ruthenium auth login [--device] | list | check [name] [--certificates] | logout <name>  [--data-dir <folder>]".into()),
    }
}

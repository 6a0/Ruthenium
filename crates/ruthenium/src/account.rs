//! The launch-time sign-in check: the most recently used account signs in again from its
//! saved sign-in on a background thread, and the menus learn how it went.

use std::path::PathBuf;
use std::sync::mpsc::{Receiver, channel};

use ruthenium_auth::{AccountStore, Auth, AuthError};

use crate::menu::AccountStatus;

/// A short reason for the title screen.
pub fn short_reason(e: &AuthError) -> String {
    match e {
        AuthError::NotApproved => "waiting for Mojang to approve Ruthenium".into(),
        AuthError::SignInExpired => "signed out, sign in again".into(),
        AuthError::Network(_) => "offline, couldn't sign in".into(),
        AuthError::NoProfile => "this account doesn't own Java Edition".into(),
        AuthError::Xbox(r) => r.to_string(),
        AuthError::Keystore(_) => "couldn't read the saved sign-in".into(),
        _ => "couldn't sign in".into(),
    }
}

/// Starts the check: the status to show now, and where the result arrives.
pub fn check_at_launch(
    data_dir: Option<PathBuf>,
) -> (AccountStatus, Option<Receiver<AccountStatus>>) {
    let Some(dir) = data_dir else {
        return (AccountStatus::None, None);
    };
    // Reading the account list is a small file read; the credential store and the network
    // wait for the thread.
    let Some(account) = AccountStore::open(&dir).current().cloned() else {
        return (AccountStatus::None, None);
    };
    let (tx, rx) = channel();
    let name = account.name.clone();
    let spawned = std::thread::Builder::new()
        .name("sign-in".into())
        .spawn(move || {
            let mut store = AccountStore::open(&dir);
            let failed = |e: &AuthError| AccountStatus::Failed {
                name: account.name.clone(),
                reason: short_reason(e),
            };
            let status = match store.refresh_token(&account.id) {
                Ok(Some(token)) => match Auth::new().resume(&token) {
                    Ok(s) => {
                        let p = &s.session.profile;
                        if let Err(e) = store.signed_in(&p.id, &p.name, &s.refresh_token) {
                            eprintln!("[ruthenium] couldn't save the renewed sign-in: {e}");
                        }
                        AccountStatus::SignedIn {
                            name: p.name.clone(),
                            may_play_online: s.session.privileges.may_play_online(),
                        }
                    }
                    Err(e) => {
                        eprintln!("[ruthenium] sign-in for {}: {e}", account.name);
                        failed(&e)
                    }
                },
                Ok(None) => failed(&AuthError::SignInExpired),
                Err(e) => failed(&e),
            };
            let _ = tx.send(status);
        });
    match spawned {
        Ok(_) => (AccountStatus::Checking(name), Some(rx)),
        Err(_) => (AccountStatus::None, None),
    }
}

//! Signed-in accounts: a list in `accounts.json` in Ruthenium's data folder (profile id,
//! name, when last used; nothing secret) and each account's Microsoft refresh token in the
//! operating system's credential store (Keychain, Credential Manager, Secret Service).
//! Minecraft access tokens and chat certificates are never stored.

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::AuthError;

const SERVICE: &str = "Ruthenium";

/// Where secrets live.
pub trait Secrets {
    fn get(&self, key: &str) -> Result<Option<String>, AuthError>;
    fn set(&self, key: &str, value: &str) -> Result<(), AuthError>;
    fn delete(&self, key: &str) -> Result<(), AuthError>;
}

/// The operating system's credential store.
#[derive(Debug, Default)]
pub struct Keyring;

fn keyring_error(e: keyring::Error) -> AuthError {
    AuthError::Keystore(e.to_string())
}

impl Secrets for Keyring {
    fn get(&self, key: &str) -> Result<Option<String>, AuthError> {
        let entry = keyring::Entry::new(SERVICE, key).map_err(keyring_error)?;
        match entry.get_password() {
            Ok(s) => Ok(Some(s)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(keyring_error(e)),
        }
    }

    fn set(&self, key: &str, value: &str) -> Result<(), AuthError> {
        keyring::Entry::new(SERVICE, key)
            .and_then(|e| e.set_password(value))
            .map_err(keyring_error)
    }

    fn delete(&self, key: &str) -> Result<(), AuthError> {
        let entry = keyring::Entry::new(SERVICE, key).map_err(keyring_error)?;
        match entry.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(keyring_error(e)),
        }
    }
}

/// A signed-in account.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Account {
    /// The Java profile id (no dashes).
    pub id: String,
    pub name: String,
    /// Unix seconds.
    pub last_used: u64,
}

/// See the module docs.
pub struct AccountStore {
    path: PathBuf,
    accounts: Vec<Account>,
    secrets: Box<dyn Secrets + Send>,
}

impl std::fmt::Debug for AccountStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AccountStore")
            .field("path", &self.path)
            .field("accounts", &self.accounts)
            .finish_non_exhaustive()
    }
}

fn secret_key(id: &str) -> String {
    format!("msa-refresh-token/{id}")
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

impl AccountStore {
    /// The store in `data_dir`, with secrets in the OS credential store.
    pub fn open(data_dir: &Path) -> AccountStore {
        AccountStore::with_secrets(data_dir, Box::new(Keyring))
    }

    pub fn with_secrets(data_dir: &Path, secrets: Box<dyn Secrets + Send>) -> AccountStore {
        let path = data_dir.join("accounts.json");
        let accounts = std::fs::read_to_string(&path)
            .ok()
            .and_then(|s| serde_json::from_str::<Value>(&s).ok())
            .and_then(|v| {
                Some(
                    v.get("accounts")?
                        .as_array()?
                        .iter()
                        .filter_map(|a| {
                            Some(Account {
                                id: a.get("id")?.as_str()?.to_owned(),
                                name: a.get("name")?.as_str()?.to_owned(),
                                last_used: a.get("lastUsed").and_then(Value::as_u64).unwrap_or(0),
                            })
                        })
                        .collect(),
                )
            })
            .unwrap_or_default();
        let mut store = AccountStore {
            path,
            accounts,
            secrets,
        };
        // Saved newest first; the sort is stable, so equal times keep that order.
        store
            .accounts
            .sort_by_key(|a| std::cmp::Reverse(a.last_used));
        store
    }

    /// Moves `account` to the front as the one used now (never older than the one it
    /// replaces there, so the order survives a reload even if the clock went back).
    fn put_first(&mut self, mut account: Account) {
        account.last_used = now().max(self.accounts.first().map_or(0, |a| a.last_used));
        self.accounts.retain(|a| a.id != account.id);
        self.accounts.insert(0, account);
    }

    /// Accounts, most recently used first.
    pub fn accounts(&self) -> &[Account] {
        &self.accounts
    }

    /// The most recently used account.
    pub fn current(&self) -> Option<&Account> {
        self.accounts.first()
    }

    /// The account whose name or id is `who` (names ignore case).
    pub fn find(&self, who: &str) -> Option<&Account> {
        self.accounts
            .iter()
            .find(|a| a.id == who || a.name.eq_ignore_ascii_case(who))
    }

    fn save(&self) -> Result<(), AuthError> {
        let list: Vec<Value> = self
            .accounts
            .iter()
            .map(|a| json!({ "id": a.id, "name": a.name, "lastUsed": a.last_used }))
            .collect();
        let text = serde_json::to_string_pretty(&json!({ "accounts": list })).expect("JSON");
        let io = |e: std::io::Error| AuthError::Keystore(format!("{}: {e}", self.path.display()));
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir).map_err(io)?;
        }
        let tmp = self.path.with_extension("json.new");
        std::fs::write(&tmp, text).map_err(io)?;
        std::fs::rename(&tmp, &self.path).map_err(io)
    }

    /// Records a sign-in: the account becomes the current one and its refresh token is
    /// replaced.
    pub fn signed_in(
        &mut self,
        id: &str,
        name: &str,
        refresh_token: &str,
    ) -> Result<(), AuthError> {
        self.secrets.set(&secret_key(id), refresh_token)?;
        self.put_first(Account {
            id: id.to_owned(),
            name: name.to_owned(),
            last_used: 0,
        });
        self.save()
    }

    /// Records a refresh: the account's new refresh token and its name as the services
    /// now give it, without changing which account is current.
    pub fn refreshed(
        &mut self,
        id: &str,
        name: &str,
        refresh_token: &str,
    ) -> Result<(), AuthError> {
        self.secrets.set(&secret_key(id), refresh_token)?;
        match self.accounts.iter_mut().find(|a| a.id == id) {
            Some(a) if a.name != name => {
                a.name = name.to_owned();
                self.save()
            }
            Some(_) => Ok(()),
            // Signed out meanwhile: keep it signed out.
            None => self.secrets.delete(&secret_key(id)),
        }
    }

    /// Makes `id` the current account (switching accounts). False if there is none.
    pub fn select(&mut self, id: &str) -> Result<bool, AuthError> {
        let Some(a) = self.accounts.iter().find(|a| a.id == id).cloned() else {
            return Ok(false);
        };
        self.put_first(a);
        self.save().map(|()| true)
    }

    pub fn refresh_token(&self, id: &str) -> Result<Option<String>, AuthError> {
        self.secrets.get(&secret_key(id))
    }

    /// Signs an account out: forgets it and deletes its refresh token.
    pub fn remove(&mut self, id: &str) -> Result<(), AuthError> {
        self.secrets.delete(&secret_key(id))?;
        self.accounts.retain(|a| a.id != id);
        self.save()
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use std::collections::HashMap;
    use std::sync::Mutex;

    use super::*;

    #[derive(Default)]
    pub(crate) struct MemorySecrets(pub Mutex<HashMap<String, String>>);

    impl Secrets for MemorySecrets {
        fn get(&self, key: &str) -> Result<Option<String>, AuthError> {
            Ok(self.0.lock().unwrap().get(key).cloned())
        }

        fn set(&self, key: &str, value: &str) -> Result<(), AuthError> {
            self.0.lock().unwrap().insert(key.into(), value.into());
            Ok(())
        }

        fn delete(&self, key: &str) -> Result<(), AuthError> {
            self.0.lock().unwrap().remove(key);
            Ok(())
        }
    }

    #[test]
    fn remembers_accounts_without_their_secrets() {
        let dir = std::env::temp_dir().join(format!("ruthenium-accounts-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut s = AccountStore::with_secrets(&dir, Box::new(MemorySecrets::default()));
        s.signed_in("aaa", "Alex", "r1").unwrap();
        // Within the same second: the order still holds, here and after a reload.
        s.signed_in("bbb", "Steve", "r2").unwrap();
        assert_eq!(s.current().unwrap().name, "Steve");
        assert_eq!(s.find("alex").unwrap().id, "aaa");
        assert_eq!(s.refresh_token("aaa").unwrap().as_deref(), Some("r1"));
        let file = std::fs::read_to_string(dir.join("accounts.json")).unwrap();
        assert!(file.contains("Steve") && !file.contains("r2"));
        // Reopened from disk, newest first.
        let again = AccountStore::with_secrets(&dir, Box::new(MemorySecrets::default()));
        assert_eq!(
            again
                .accounts()
                .iter()
                .map(|a| a.name.as_str())
                .collect::<Vec<_>>(),
            ["Steve", "Alex"]
        );
        // A refresh renames but doesn't switch; selecting switches.
        s.refreshed("aaa", "Alex2", "r3").unwrap();
        assert_eq!(s.current().unwrap().name, "Steve");
        assert_eq!(s.refresh_token("aaa").unwrap().as_deref(), Some("r3"));
        assert!(s.select("aaa").unwrap());
        assert!(!s.select("nobody").unwrap());
        let again = AccountStore::with_secrets(&dir, Box::new(MemorySecrets::default()));
        assert_eq!(again.current().unwrap().name, "Alex2");
        s.remove("bbb").unwrap();
        assert_eq!(s.current().unwrap().name, "Alex2");
        assert_eq!(s.refresh_token("bbb").unwrap(), None);
        // A refresh that lands after a sign-out doesn't bring the secret back.
        s.refreshed("bbb", "Steve", "late").unwrap();
        assert_eq!(s.refresh_token("bbb").unwrap(), None);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}

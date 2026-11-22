//! The accounts the menus show and change. One worker thread does everything that waits
//! on the network or the credential store, in the order it was asked: signing in again
//! from a saved sign-in (at launch, and on switching), signing in a new account in the
//! browser or with a device code, switching and signing out. The menus read the list,
//! the current account's status and any sign-in in progress from [`Accounts`].

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};

use ruthenium_auth::{Account, AccountStore, Auth, AuthError, DeviceCode, SignedIn};

/// The account in use, as the menus show it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AccountStatus {
    /// No account signed in.
    None,
    Checking(String),
    SignedIn {
        name: String,
        may_play_online: bool,
    },
    Failed {
        name: String,
        reason: String,
    },
}

impl AccountStatus {
    fn signed_in(s: &SignedIn) -> AccountStatus {
        AccountStatus::SignedIn {
            name: s.session.profile.name.clone(),
            may_play_online: s.session.privileges.may_play_online(),
        }
    }
}

/// A short reason, for the title's account chip and the Accounts screen.
pub fn short_reason(e: &AuthError) -> String {
    match e {
        AuthError::NotApproved => "waiting for Mojang to approve Ruthenium".into(),
        AuthError::SignInExpired => "signed out, sign in again".into(),
        AuthError::Network(_) => "offline, couldn't sign in".into(),
        AuthError::NoProfile => "this account doesn't own Java Edition".into(),
        AuthError::NoProfileName => "choose a profile name at minecraft.net first".into(),
        AuthError::Xbox(r) => r.to_string(),
        AuthError::Keystore(_) => "couldn't read the saved sign-in".into(),
        _ => "couldn't sign in".into(),
    }
}

/// Why a sign-in failed, in lines short enough for a form.
pub fn failure_lines(e: &AuthError) -> Vec<String> {
    let lines: &[&str] = match e {
        AuthError::NotApproved => &[
            "Mojang hasn't approved Ruthenium",
            "for the Minecraft services yet.",
        ],
        AuthError::NoProfile => &["This account doesn't own", "Minecraft: Java Edition."],
        AuthError::NoProfileName => &[
            "This account owns Java Edition but has",
            "no profile yet. Choose a name at",
            "minecraft.net, then try again.",
        ],
        AuthError::Network(_) => &[
            "Couldn't reach the sign-in service.",
            "Check your connection and try again.",
        ],
        AuthError::TimedOut => &["The sign-in took too long."],
        AuthError::Denied(_) => &["The sign-in was declined."],
        AuthError::Keystore(_) => &[
            "Couldn't save the sign-in in your",
            "system's credential store.",
        ],
        AuthError::Xbox(_) => return vec![format!("Xbox Live: {}.", short_reason(e))],
        _ => &["Something went wrong signing in.", "Try again in a moment."],
    };
    lines.iter().map(|&l| l.to_owned()).collect()
}

/// Signs in; the worker owns one. [`Auth`] is the real one.
pub trait Backend {
    fn resume(&mut self, refresh_token: &str) -> Result<SignedIn, AuthError>;
    fn sign_in_browser(
        &mut self,
        open: &mut dyn FnMut(&str),
        cancel: &AtomicBool,
    ) -> Result<SignedIn, AuthError>;
    fn sign_in_device(
        &mut self,
        show: &mut dyn FnMut(&DeviceCode),
        cancel: &AtomicBool,
    ) -> Result<SignedIn, AuthError>;
}

impl Backend for Auth {
    fn resume(&mut self, refresh_token: &str) -> Result<SignedIn, AuthError> {
        Auth::resume(self, refresh_token)
    }

    fn sign_in_browser(
        &mut self,
        open: &mut dyn FnMut(&str),
        cancel: &AtomicBool,
    ) -> Result<SignedIn, AuthError> {
        Auth::sign_in_browser(self, open, cancel)
    }

    fn sign_in_device(
        &mut self,
        show: &mut dyn FnMut(&DeviceCode),
        cancel: &AtomicBool,
    ) -> Result<SignedIn, AuthError> {
        Auth::sign_in_device(self, show, cancel)
    }
}

/// Opens the worker's store and backend, on the worker thread.
pub type Open = Box<dyn FnOnce() -> (AccountStore, Box<dyn Backend>) + Send>;

/// How a sign-in of a new account is going.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Stage {
    /// Waiting for the sign-in page (browser) or the code (device).
    Starting {
        device: bool,
    },
    /// The browser was sent to this page.
    Browser {
        url: String,
    },
    Device(DeviceCode),
    Failed {
        error: AuthError,
        device: bool,
    },
}

#[derive(Debug)]
struct Flow {
    id: u64,
    cancel: Arc<AtomicBool>,
    stage: Stage,
}

enum Job {
    Resume {
        generation: u64,
        id: String,
    },
    Select {
        generation: u64,
        id: String,
    },
    Remove {
        generation: u64,
        id: String,
    },
    SignIn {
        flow: u64,
        device: bool,
        cancel: Arc<AtomicBool>,
    },
}

enum Update {
    /// The store's accounts after a change.
    List(Vec<Account>),
    Status {
        generation: u64,
        status: AccountStatus,
    },
    Page {
        flow: u64,
        url: String,
    },
    Code {
        flow: u64,
        code: DeviceCode,
    },
    SignedIn {
        flow: u64,
        status: AccountStatus,
    },
    SignInFailed {
        flow: u64,
        error: AuthError,
    },
}

/// See the module docs.
pub struct Accounts {
    jobs: Option<Sender<Job>>,
    updates: Receiver<Update>,
    list: Vec<Account>,
    status: AccountStatus,
    /// Bumped by every change of the account in use; older status reports are dropped.
    generation: u64,
    flow: Option<Flow>,
    next_flow: u64,
    /// Bumped whenever the list, the status or the sign-in changes.
    revision: u64,
    open_page: Box<dyn Fn(&str)>,
}

impl std::fmt::Debug for Accounts {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Accounts")
            .field("list", &self.list)
            .field("status", &self.status)
            .field("flow", &self.flow)
            .finish_non_exhaustive()
    }
}

/// Opens `url` in the system browser, saying so if it can't.
fn open_in_browser(url: &str) {
    if let Err(e) = ruthenium_auth::open_browser(url) {
        eprintln!("[ruthenium] couldn't open a browser ({e}); the sign-in page is {url}");
    }
}

impl Accounts {
    /// The accounts in `data_dir` (none: no accounts, and signing in can't be kept); the
    /// discovery document is kept in `cache_dir`. The account in use signs in again.
    pub fn start(data_dir: Option<PathBuf>, cache_dir: Option<&Path>) -> Accounts {
        let Some(dir) = data_dir else {
            return Accounts::disabled();
        };
        let discovery = cache_dir.map(|d| d.join("discovery.json"));
        let worker_dir = dir.clone();
        let open: Open = Box::new(move || {
            let mut auth = Auth::new();
            if let Some(path) = &discovery {
                auth = auth.with_discovery_cache(path);
            }
            (AccountStore::open(&worker_dir), Box::new(auth))
        });
        // Reading the list is a small file read; the credential store and the network wait
        // for the worker.
        let list = AccountStore::open(&dir).accounts().to_vec();
        Accounts::with_worker(list, open, Box::new(open_in_browser))
    }

    /// No data folder: nothing to show, and signing in fails.
    pub fn disabled() -> Accounts {
        Accounts::new(None, channel().1, Vec::new(), Box::new(|_| {}))
    }

    fn new(
        jobs: Option<Sender<Job>>,
        updates: Receiver<Update>,
        list: Vec<Account>,
        open_page: Box<dyn Fn(&str)>,
    ) -> Accounts {
        Accounts {
            jobs,
            updates,
            list,
            status: AccountStatus::None,
            generation: 0,
            flow: None,
            next_flow: 0,
            revision: 0,
            open_page,
        }
    }

    /// Starts the worker with `open`, showing `list` until it reports; pages to sign in
    /// on go to `open_page`.
    pub fn with_worker(list: Vec<Account>, open: Open, open_page: Box<dyn Fn(&str)>) -> Accounts {
        let (jobs, job_rx) = channel();
        let (update_tx, updates) = channel();
        let spawned = std::thread::Builder::new()
            .name("accounts".into())
            .spawn(move || work(open, &job_rx, &update_tx));
        let mut accounts = Accounts::new(spawned.is_ok().then_some(jobs), updates, list, open_page);
        if let Some(a) = accounts.list.first().cloned() {
            accounts.status = AccountStatus::Checking(a.name);
            accounts.send(Job::Resume {
                generation: 0,
                id: a.id,
            });
        }
        accounts
    }

    fn send(&mut self, job: Job) {
        if let Some(tx) = &self.jobs
            && tx.send(job).is_err()
        {
            eprintln!("[ruthenium] the accounts thread has stopped");
            self.jobs = None;
        }
    }

    /// Accounts, the one in use first.
    pub fn list(&self) -> &[Account] {
        &self.list
    }

    pub fn status(&self) -> &AccountStatus {
        &self.status
    }

    /// The sign-in in progress, if any.
    pub fn stage(&self) -> Option<&Stage> {
        self.flow.as_ref().map(|f| &f.stage)
    }

    /// Changes whenever anything here does.
    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// Takes in the worker's reports; true if anything changed.
    pub fn poll(&mut self) -> bool {
        let before = self.revision;
        while let Ok(u) = self.updates.try_recv() {
            let flow = self.flow.as_ref().map(|f| f.id);
            match u {
                Update::List(list) => self.list = list,
                Update::Status { generation, status } => {
                    if generation != self.generation {
                        continue;
                    }
                    self.status = status;
                }
                Update::Page { flow: f, url } if Some(f) == flow => {
                    (self.open_page)(&url);
                    self.set_stage(Stage::Browser { url });
                }
                Update::Code { flow: f, code } if Some(f) == flow => {
                    self.set_stage(Stage::Device(code));
                }
                // Signed in, even if the player moved on meanwhile: the account is now in
                // use, so its status wins over any older report.
                Update::SignedIn { flow: f, status } => {
                    self.generation += 1;
                    self.status = status;
                    if Some(f) == flow {
                        self.flow = None;
                    }
                }
                Update::SignInFailed { flow: f, error } if Some(f) == flow => {
                    if error == AuthError::Cancelled {
                        self.flow = None;
                    } else {
                        let device = matches!(
                            self.stage(),
                            Some(Stage::Starting { device: true } | Stage::Device(_))
                        );
                        self.set_stage(Stage::Failed { error, device });
                    }
                }
                // About a sign-in given up on.
                Update::Page { .. } | Update::Code { .. } | Update::SignInFailed { .. } => {
                    continue;
                }
            }
            self.revision += 1;
        }
        self.revision != before
    }

    fn set_stage(&mut self, stage: Stage) {
        if let Some(f) = &mut self.flow {
            f.stage = stage;
        }
    }

    /// Starts signing in a new account, in the browser or with a device code; any
    /// sign-in in progress is given up.
    pub fn sign_in(&mut self, device: bool) {
        self.cancel_sign_in();
        let id = self.next_flow;
        self.next_flow += 1;
        let cancel = Arc::new(AtomicBool::new(false));
        self.flow = Some(Flow {
            id,
            cancel: cancel.clone(),
            stage: if self.jobs.is_some() {
                Stage::Starting { device }
            } else {
                Stage::Failed {
                    error: AuthError::Keystore("no data folder".into()),
                    device,
                }
            },
        });
        self.revision += 1;
        self.send(Job::SignIn {
            flow: id,
            device,
            cancel,
        });
    }

    /// Gives up the sign-in in progress.
    pub fn cancel_sign_in(&mut self) {
        if let Some(f) = self.flow.take() {
            f.cancel.store(true, Ordering::Relaxed);
            self.revision += 1;
        }
    }

    /// Opens the sign-in page again (browser), or the page to enter the code on.
    pub fn open_page_again(&self) {
        match self.stage() {
            Some(Stage::Browser { url }) => (self.open_page)(url),
            Some(Stage::Device(code)) => (self.open_page)(&code.verification_uri),
            _ => {}
        }
    }

    /// Switches to account `id`, which then signs in again.
    pub fn use_account(&mut self, id: &str) {
        let Some(i) = self.list.iter().position(|a| a.id == id) else {
            return;
        };
        let a = self.list.remove(i);
        self.status = AccountStatus::Checking(a.name.clone());
        self.list.insert(0, a);
        self.generation += 1;
        self.revision += 1;
        self.send(Job::Select {
            generation: self.generation,
            id: id.to_owned(),
        });
    }

    /// Signs account `id` out (forgets it and its saved sign-in). If it was in use, the
    /// next one takes over.
    pub fn sign_out(&mut self, id: &str) {
        let was_current = self.list.first().is_some_and(|a| a.id == id);
        self.list.retain(|a| a.id != id);
        if was_current {
            self.generation += 1;
            self.status = match self.list.first() {
                Some(a) => AccountStatus::Checking(a.name.clone()),
                None => AccountStatus::None,
            };
        }
        self.revision += 1;
        self.send(Job::Remove {
            generation: self.generation,
            id: id.to_owned(),
        });
    }
}

impl Drop for Accounts {
    fn drop(&mut self) {
        self.cancel_sign_in();
    }
}

/// The worker: runs jobs in order until the menus go away.
fn work(open: Open, jobs: &Receiver<Job>, updates: &Sender<Update>) {
    let (mut store, mut backend) = open();
    let send = |u: Update| {
        let _ = updates.send(u);
    };
    let resume = |store: &mut AccountStore, backend: &mut dyn Backend, id: &str| {
        let Some(account) = store.accounts().iter().find(|a| a.id == id).cloned() else {
            return AccountStatus::None;
        };
        let failed = |e: &AuthError| {
            eprintln!("[ruthenium] sign-in for {}: {e}", account.name);
            AccountStatus::Failed {
                name: account.name.clone(),
                reason: short_reason(e),
            }
        };
        let token = match store.refresh_token(&account.id) {
            Ok(Some(t)) => t,
            Ok(None) => return failed(&AuthError::SignInExpired),
            Err(e) => return failed(&e),
        };
        match backend.resume(&token) {
            Ok(s) => {
                let p = &s.session.profile;
                if let Err(e) = store.refreshed(&account.id, &p.name, &s.refresh_token) {
                    eprintln!("[ruthenium] couldn't save the renewed sign-in: {e}");
                }
                AccountStatus::signed_in(&s)
            }
            Err(e) => failed(&e),
        }
    };
    for job in jobs {
        match job {
            Job::Resume { generation, id } => {
                let status = resume(&mut store, &mut *backend, &id);
                send(Update::Status { generation, status });
            }
            Job::Select { generation, id } => {
                if let Err(e) = store.select(&id) {
                    eprintln!("[ruthenium] couldn't save the account in use: {e}");
                }
                send(Update::List(store.accounts().to_vec()));
                let status = resume(&mut store, &mut *backend, &id);
                send(Update::Status { generation, status });
            }
            Job::Remove { generation, id } => {
                let was_current = store.current().is_some_and(|a| a.id == id);
                if let Err(e) = store.remove(&id) {
                    eprintln!("[ruthenium] couldn't sign out: {e}");
                }
                send(Update::List(store.accounts().to_vec()));
                if was_current {
                    let status = match store.current().map(|a| a.id.clone()) {
                        Some(next) => resume(&mut store, &mut *backend, &next),
                        None => AccountStatus::None,
                    };
                    send(Update::Status { generation, status });
                }
            }
            Job::SignIn {
                flow,
                device,
                cancel,
            } => {
                if cancel.load(Ordering::Relaxed) {
                    continue;
                }
                let result = if device {
                    backend.sign_in_device(
                        &mut |code| {
                            send(Update::Code {
                                flow,
                                code: code.clone(),
                            });
                        },
                        &cancel,
                    )
                } else {
                    backend.sign_in_browser(
                        &mut |url| {
                            send(Update::Page {
                                flow,
                                url: url.to_owned(),
                            });
                        },
                        &cancel,
                    )
                };
                let saved = result.and_then(|s| {
                    let p = &s.session.profile;
                    store.signed_in(&p.id, &p.name, &s.refresh_token)?;
                    Ok(s)
                });
                match saved {
                    Ok(s) => {
                        send(Update::List(store.accounts().to_vec()));
                        send(Update::SignedIn {
                            flow,
                            status: AccountStatus::signed_in(&s),
                        });
                    }
                    Err(error) => {
                        if error != AuthError::Cancelled {
                            eprintln!("[ruthenium] sign-in: {error}");
                        }
                        send(Update::SignInFailed { flow, error });
                    }
                }
            }
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use std::collections::HashMap;
    use std::sync::Mutex;
    use std::time::{Duration, Instant};

    use ruthenium_auth::store::Secrets;
    use ruthenium_auth::{Privileges, Profile, Session};

    use super::*;

    #[derive(Default, Clone)]
    pub(crate) struct MemorySecrets(pub Arc<Mutex<HashMap<String, String>>>);

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

    /// Signs in whoever the refresh token names ("token-<name>"); a new sign-in gives
    /// `next`'s account, or waits to be cancelled when there is none.
    pub(crate) struct FakeBackend {
        pub next: Option<Result<&'static str, AuthError>>,
    }

    /// "Child" may not play online.
    pub(crate) fn signed(name: &str) -> SignedIn {
        SignedIn {
            session: Session {
                profile: Profile {
                    id: format!("id-{name}"),
                    name: name.to_owned(),
                    skin: None,
                },
                access_token: "mc".into(),
                expires_at: Instant::now(),
                privileges: Privileges {
                    multiplayer_server: name != "Child",
                    ..Privileges::default()
                },
            },
            refresh_token: format!("token-{name}"),
        }
    }

    impl FakeBackend {
        fn new_sign_in(&mut self, cancel: &AtomicBool) -> Result<SignedIn, AuthError> {
            match self.next.take() {
                Some(r) => r.map(signed),
                None => {
                    while !cancel.load(Ordering::Relaxed) {
                        std::thread::sleep(Duration::from_millis(5));
                    }
                    Err(AuthError::Cancelled)
                }
            }
        }
    }

    impl Backend for FakeBackend {
        fn resume(&mut self, refresh_token: &str) -> Result<SignedIn, AuthError> {
            match refresh_token.strip_prefix("token-") {
                Some("Expired") => Err(AuthError::SignInExpired),
                Some(name) => Ok(signed(name)),
                None => Err(AuthError::SignInExpired),
            }
        }

        fn sign_in_browser(
            &mut self,
            open: &mut dyn FnMut(&str),
            cancel: &AtomicBool,
        ) -> Result<SignedIn, AuthError> {
            open("https://login.example/authorize");
            self.new_sign_in(cancel)
        }

        fn sign_in_device(
            &mut self,
            show: &mut dyn FnMut(&DeviceCode),
            cancel: &AtomicBool,
        ) -> Result<SignedIn, AuthError> {
            show(&DeviceCode {
                user_code: "ABCD-1234".into(),
                verification_uri: "https://www.microsoft.com/link".into(),
                message: String::new(),
                expires_in: 900,
            });
            self.new_sign_in(cancel)
        }
    }

    /// A data folder with `names` signed in (the first in use).
    pub(crate) fn accounts_in(
        dir: &Path,
        names: &[&str],
        next: Option<Result<&'static str, AuthError>>,
    ) -> (Accounts, Arc<Mutex<Vec<String>>>) {
        let _ = std::fs::remove_dir_all(dir);
        let secrets = MemorySecrets::default();
        let mut store = AccountStore::with_secrets(dir, Box::new(secrets.clone()));
        for n in names.iter().rev() {
            let s = signed(n);
            store
                .signed_in(&s.session.profile.id, n, &s.refresh_token)
                .unwrap();
        }
        let list = store.accounts().to_vec();
        let dir = dir.to_owned();
        let pages = Arc::new(Mutex::new(Vec::new()));
        let seen = pages.clone();
        let open: Open = Box::new(move || {
            let store = AccountStore::with_secrets(&dir, Box::new(secrets));
            (store, Box::new(FakeBackend { next }))
        });
        let accounts = Accounts::with_worker(
            list,
            open,
            Box::new(move |url| seen.lock().unwrap().push(url.to_owned())),
        );
        (accounts, pages)
    }

    /// Polls until `done` holds (or fails after a few seconds).
    pub(crate) fn wait(a: &mut Accounts, done: impl Fn(&Accounts) -> bool) {
        let until = Instant::now() + Duration::from_secs(5);
        while !done(a) {
            assert!(Instant::now() < until, "timed out: {a:?}");
            a.poll();
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    fn names(a: &Accounts) -> Vec<&str> {
        a.list().iter().map(|a| a.name.as_str()).collect()
    }

    fn temp(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("ruthenium-{name}-{}", std::process::id()))
    }

    #[test]
    fn the_account_in_use_signs_in_again_and_switching_follows() {
        let dir = temp("accounts-switch");
        let (mut a, _) = accounts_in(&dir, &["Alex", "Steve"], None);
        assert_eq!(a.status(), &AccountStatus::Checking("Alex".into()));
        wait(&mut a, |a| {
            matches!(a.status(), AccountStatus::SignedIn { .. })
        });
        assert_eq!(
            a.status(),
            &AccountStatus::SignedIn {
                name: "Alex".into(),
                may_play_online: true
            }
        );
        a.use_account("id-Steve");
        assert_eq!(names(&a), ["Steve", "Alex"]);
        wait(&mut a, |a| {
            a.status()
                == &AccountStatus::SignedIn {
                    name: "Steve".into(),
                    may_play_online: true,
                }
        });
        // Kept on disk.
        let store = AccountStore::with_secrets(&dir, Box::new(MemorySecrets::default()));
        assert_eq!(store.current().unwrap().name, "Steve");
        // Signing out the one in use hands over to the next.
        a.sign_out("id-Steve");
        assert_eq!(a.status(), &AccountStatus::Checking("Alex".into()));
        wait(
            &mut a,
            |a| matches!(a.status(), AccountStatus::SignedIn { name, .. } if name == "Alex"),
        );
        a.sign_out("id-Alex");
        assert_eq!(a.status(), &AccountStatus::None);
        wait(&mut a, |a| a.list().is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_stale_report_does_not_win() {
        let dir = temp("accounts-stale");
        let (mut a, _) = accounts_in(&dir, &["Alex", "Expired"], None);
        // Switch before Alex's launch check reports: only Expired's result counts.
        a.use_account("id-Expired");
        wait(&mut a, |a| {
            matches!(a.status(), AccountStatus::Failed { .. })
        });
        assert_eq!(
            a.status(),
            &AccountStatus::Failed {
                name: "Expired".into(),
                reason: short_reason(&AuthError::SignInExpired)
            }
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn signing_in_in_the_browser_or_with_a_code() {
        let dir = temp("accounts-sign-in");
        let (mut a, pages) = accounts_in(&dir, &[], None);
        assert_eq!(a.status(), &AccountStatus::None);
        a.sign_in(false);
        wait(&mut a, |a| matches!(a.stage(), Some(Stage::Browser { .. })));
        assert_eq!(
            pages.lock().unwrap().as_slice(),
            ["https://login.example/authorize"]
        );
        // Switching to a code gives up the browser sign-in.
        a.sign_in(true);
        wait(&mut a, |a| matches!(a.stage(), Some(Stage::Device(_))));
        a.open_page_again();
        assert_eq!(pages.lock().unwrap()[1], "https://www.microsoft.com/link");
        a.cancel_sign_in();
        assert_eq!(a.stage(), None);
        let _ = std::fs::remove_dir_all(&dir);

        let (mut a, _) = accounts_in(&dir, &["Alex"], Some(Ok("Notch")));
        wait(&mut a, |a| {
            matches!(a.status(), AccountStatus::SignedIn { .. })
        });
        a.sign_in(true);
        wait(&mut a, |a| a.stage().is_none());
        assert_eq!(names(&a), ["Notch", "Alex"]);
        assert!(matches!(a.status(), AccountStatus::SignedIn { name, .. } if name == "Notch"));
        let _ = std::fs::remove_dir_all(&dir);

        let (mut a, _) = accounts_in(&dir, &[], Some(Err(AuthError::NotApproved)));
        a.sign_in(false);
        wait(&mut a, |a| matches!(a.stage(), Some(Stage::Failed { .. })));
        assert_eq!(
            a.stage(),
            Some(&Stage::Failed {
                error: AuthError::NotApproved,
                device: false
            })
        );
        assert!(a.list().is_empty(), "no profile, no account");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn without_a_data_folder_signing_in_fails() {
        let mut a = Accounts::disabled();
        a.sign_in(false);
        assert!(matches!(a.stage(), Some(Stage::Failed { .. })));
        assert!(!a.poll());
    }
}

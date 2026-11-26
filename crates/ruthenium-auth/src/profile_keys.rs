//! The chat signing key pair (`AccountProfileKeyPairManager`): fetched from the player
//! certificates endpoint off the caller's thread, kept in memory only (vanilla writes it
//! to a file only when run from a development environment), and fetched again once Mojang
//! says it is due (`refreshedAfter`), checked at most once an hour.

use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use base64::Engine;

use crate::{AuthError, Certificates};

/// The least time between two looks at whether a new key pair is due
/// (`MINIMUM_PROFILE_KEY_REFRESH_INTERVAL`).
const REFRESH_INTERVAL: Duration = Duration::from_secs(60 * 60);

/// A key pair as the client signs with it (`ProfileKeyPair`).
#[derive(Clone, PartialEq, Eq)]
pub struct ProfileKeyPair {
    /// PKCS#8 DER.
    pub private_key: Vec<u8>,
    /// X.509 DER.
    pub public_key: Vec<u8>,
    /// Mojang's signature over the public key, its expiry and the profile's id.
    pub key_signature: Vec<u8>,
    /// Milliseconds of the epoch.
    pub expires_at: i64,
    pub refresh_after: i64,
}

impl std::fmt::Debug for ProfileKeyPair {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProfileKeyPair")
            .field("expires_at", &self.expires_at)
            .field("refresh_after", &self.refresh_after)
            .finish_non_exhaustive()
    }
}

impl ProfileKeyPair {
    /// `AccountProfileKeyPairManager.fetchProfileKeyPair`'s reading of the answer: the
    /// keys' PEM (`Crypt.stringToPemRsaPrivateKey`, `stringToRsaPublicKey`), the
    /// signature's Base64 and the times' ISO 8601.
    pub fn from_certificates(c: &Certificates) -> Result<ProfileKeyPair, AuthError> {
        let bad = |what: &str| AuthError::BadResponse(format!("certificates with a bad {what}"));
        let private_key = pem(
            &c.private_key_pem,
            "-----BEGIN RSA PRIVATE KEY-----",
            "-----END RSA PRIVATE KEY-----",
        )
        .ok_or_else(|| bad("private key"))?;
        let public_key = pem(
            &c.public_key_pem,
            "-----BEGIN RSA PUBLIC KEY-----",
            "-----END RSA PUBLIC KEY-----",
        )
        .ok_or_else(|| bad("public key"))?;
        {
            use rsa::pkcs8::{DecodePrivateKey, DecodePublicKey};
            rsa::PrivateKey::from_pkcs8_der(&private_key).map_err(|_| bad("private key"))?;
            rsa::PublicKey::from_public_key_der(&public_key).map_err(|_| bad("public key"))?;
        }
        let key_signature = base64::engine::general_purpose::STANDARD
            .decode(c.signature_v2.trim())
            .ok()
            .filter(|s| !s.is_empty())
            .ok_or_else(|| bad("key signature"))?;
        Ok(ProfileKeyPair {
            private_key,
            public_key,
            key_signature,
            expires_at: iso_millis(&c.expires_at).ok_or_else(|| bad("expiry"))?,
            refresh_after: iso_millis(&c.refreshed_after).ok_or_else(|| bad("refresh time"))?,
        })
    }

    /// `dueRefresh`: the refresh time has come.
    pub fn due_refresh(&self, now: i64) -> bool {
        now >= self.refresh_after
    }
}

/// The bytes of a PEM block (`Crypt.rsaStringToKey`): what lies between the header and
/// the footer if there is a header, else everything, as MIME Base64 (other characters
/// are skipped).
fn pem(text: &str, header: &str, footer: &str) -> Option<Vec<u8>> {
    let body = match text.find(header) {
        Some(at) => {
            let rest = &text[at + header.len()..];
            &rest[..rest.find(footer)?]
        }
        None => text,
    };
    let clean: String = body
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '/' | '='))
        .collect();
    base64::engine::general_purpose::STANDARD.decode(clean).ok()
}

/// Milliseconds of the epoch of an ISO 8601 instant (`Instant.parse`:
/// `2026-11-26T12:00:00.123456Z`, or with an offset).
pub fn iso_millis(s: &str) -> Option<i64> {
    let s = s.trim();
    let (date, time) = s.split_once('T')?;
    let mut d = date.splitn(3, '-');
    let year: i64 = d.next()?.parse().ok()?;
    let month: i64 = d.next()?.parse().ok()?;
    let day: i64 = d.next()?.parse().ok()?;
    let (clock, offset) = if let Some(c) = time.strip_suffix('Z') {
        (c, 0)
    } else {
        let at = time.rfind(['+', '-'])?;
        let (c, o) = time.split_at(at);
        let sign = if o.starts_with('-') { -1 } else { 1 };
        let (h, m) = o[1..].split_once(':')?;
        (
            c,
            sign * (h.parse::<i64>().ok()? * 60 + m.parse::<i64>().ok()?),
        )
    };
    let (hms, fraction) = clock.split_once('.').unwrap_or((clock, ""));
    let mut t = hms.splitn(3, ':');
    let hour: i64 = t.next()?.parse().ok()?;
    let minute: i64 = t.next()?.parse().ok()?;
    let second: i64 = t.next().unwrap_or("0").parse().ok()?;
    if !(1..=12).contains(&month)
        || !(1..=31).contains(&day)
        || hour > 23
        || minute > 59
        || second > 60
    {
        return None;
    }
    if !fraction.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let millis: i64 = format!("{:0<3}", &fraction[..fraction.len().min(3)])
        .parse()
        .ok()?;
    // Days since the epoch of a proleptic Gregorian date (Howard Hinnant's
    // `days_from_civil`).
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (month + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    let seconds = days * 86_400 + hour * 3600 + minute * 60 + second - offset * 60;
    Some(seconds * 1000 + millis)
}

/// Now, in milliseconds of the epoch.
pub fn now_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64)
}

/// What fetches the certificates (blocking: the manager runs it on a thread of its own).
pub type Fetch = dyn Fn() -> Result<Certificates, AuthError> + Send + Sync;

/// How the key pair stands.
#[derive(Debug)]
enum Job {
    /// Nothing asked for, or the last answer taken.
    Idle,
    Running(JoinHandle<Option<Arc<ProfileKeyPair>>>),
    /// Done, not yet taken.
    Done(Option<Arc<ProfileKeyPair>>),
}

#[derive(Debug)]
struct State {
    /// The pair the last fetch left (`keyPair`'s value).
    kept: Option<Arc<ProfileKeyPair>>,
    job: Job,
    next_refresh: Option<Instant>,
}

/// `AccountProfileKeyPairManager` for one signed-in profile.
pub struct ProfileKeys {
    fetch: Arc<Fetch>,
    state: Mutex<State>,
}

impl std::fmt::Debug for ProfileKeys {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProfileKeys").finish_non_exhaustive()
    }
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

impl ProfileKeys {
    pub fn new(fetch: Arc<Fetch>) -> ProfileKeys {
        ProfileKeys {
            fetch,
            state: Mutex::new(State {
                kept: None,
                job: Job::Idle,
                next_refresh: None,
            }),
        }
    }

    /// Settles a finished fetch into the state.
    fn settle(state: &mut State) {
        if matches!(&state.job, Job::Running(h) if h.is_finished()) {
            let Job::Running(h) = std::mem::replace(&mut state.job, Job::Idle) else {
                unreachable!()
            };
            let pair = h.join().unwrap_or(None);
            state.kept.clone_from(&pair);
            state.job = Job::Done(pair);
        }
    }

    /// `prepareKeyPair`: the kept pair if it is not due, else a new one (the kept one
    /// if that fails). Nothing new starts while a fetch is under way.
    pub fn prepare(&self) {
        let mut state = lock(&self.state);
        Self::settle(&mut state);
        state.next_refresh = Some(Instant::now() + REFRESH_INTERVAL);
        if matches!(state.job, Job::Running(_)) {
            return;
        }
        let kept = state.kept.clone();
        if let Some(k) = kept.as_ref().filter(|k| !k.due_refresh(now_millis())) {
            state.job = Job::Done(Some(k.clone()));
            return;
        }
        let fetch = self.fetch.clone();
        let spawned = std::thread::Builder::new()
            .name("profile-keys".into())
            .spawn(
                move || match fetch().and_then(|c| ProfileKeyPair::from_certificates(&c)) {
                    Ok(pair) => Some(Arc::new(pair)),
                    Err(e) => {
                        eprintln!("[auth] the chat key pair couldn't be had: {e}");
                        kept
                    }
                },
            );
        state.job = match spawned {
            Ok(h) => Job::Running(h),
            Err(_) => Job::Done(state.kept.clone()),
        };
    }

    /// The pair the last [`ProfileKeys::prepare`] got, once it is done: `None` while it
    /// runs (or once taken), `Some(None)` if there is no pair.
    pub fn poll(&self) -> Option<Option<Arc<ProfileKeyPair>>> {
        let mut state = lock(&self.state);
        Self::settle(&mut state);
        match std::mem::replace(&mut state.job, Job::Idle) {
            Job::Done(pair) => Some(pair),
            other => {
                state.job = other;
                None
            }
        }
    }

    /// `shouldRefreshKeyPair`: nothing under way, an hour since the last prepare, and the
    /// pair kept due for a new one (or none kept).
    pub fn should_refresh(&self) -> bool {
        let mut state = lock(&self.state);
        Self::settle(&mut state);
        if matches!(state.job, Job::Running(_))
            || state.next_refresh.is_some_and(|t| Instant::now() <= t)
        {
            return false;
        }
        state
            .kept
            .as_ref()
            .is_none_or(|k| k.due_refresh(now_millis()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    #[test]
    fn instants_read_as_java_reads_them() {
        assert_eq!(iso_millis("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(
            iso_millis("2026-11-26T12:00:00.123456Z"),
            Some(1_795_694_400_123)
        );
        assert_eq!(
            iso_millis("2026-11-26T13:30:00.5+01:30"),
            Some(1_795_694_400_500)
        );
        assert_eq!(iso_millis("2024-02-29T23:59:59Z"), Some(1_709_251_199_000));
        assert_eq!(iso_millis("1969-12-31T23:59:59.9Z"), Some(-100));
        assert_eq!(iso_millis("2026-13-01T00:00:00Z"), None);
        assert_eq!(iso_millis("2026-11-26"), None);
        assert_eq!(iso_millis("2026-11-26T12:00:00.1xZ"), None);
    }

    fn certificates() -> Certificates {
        // A key made for this test (the chat signing golden's), as Mojang writes them.
        use rsa::pkcs8::{EncodePrivateKey, EncodePublicKey};
        let key = test_key();
        let pem = |label: &str, der: &[u8]| {
            let b64 = base64::engine::general_purpose::STANDARD.encode(der);
            let lines: Vec<&str> = b64
                .as_bytes()
                .chunks(76)
                .map(|c| std::str::from_utf8(c).unwrap())
                .collect();
            format!(
                "-----BEGIN RSA {label} KEY-----\n{}\n-----END RSA {label} KEY-----\n",
                lines.join("\n")
            )
        };
        Certificates {
            private_key_pem: pem("PRIVATE", key.to_pkcs8_der().unwrap().as_bytes()),
            public_key_pem: pem(
                "PUBLIC",
                key.to_public_key().to_public_key_der().unwrap().as_bytes(),
            ),
            signature_v2: base64::engine::general_purpose::STANDARD.encode([7u8; 512]),
            expires_at: "2026-11-27T12:00:00Z".into(),
            refreshed_after: "2026-11-27T04:00:00.25Z".into(),
        }
    }

    fn test_key() -> rsa::PrivateKey {
        use rsa::pkcs8::DecodePrivateKey;
        let hex: String = include_str!("profile_keys_test_key.hex")
            .split_whitespace()
            .collect();
        let der: Vec<u8> = (0..hex.len() / 2)
            .map(|i| u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).unwrap())
            .collect();
        rsa::PrivateKey::from_pkcs8_der(&der).unwrap()
    }

    #[test]
    fn certificates_become_a_key_pair() {
        use rsa::pkcs8::EncodePrivateKey;
        let pair = ProfileKeyPair::from_certificates(&certificates()).unwrap();
        assert_eq!(
            pair.private_key,
            test_key().to_pkcs8_der().unwrap().as_bytes()
        );
        assert_eq!(pair.key_signature, [7u8; 512]);
        assert_eq!(pair.expires_at, 1_795_780_800_000);
        assert_eq!(pair.refresh_after, 1_795_752_000_250);
        assert!(!pair.due_refresh(1_795_752_000_249));
        assert!(pair.due_refresh(1_795_752_000_250));
        // Without the PEM lines the Base64 alone reads the same.
        let mut bare = certificates();
        bare.public_key_pem = bare
            .public_key_pem
            .lines()
            .filter(|l| !l.starts_with("-----"))
            .collect();
        assert_eq!(ProfileKeyPair::from_certificates(&bare).unwrap(), pair);
        for broken in [
            Certificates {
                private_key_pem:
                    "-----BEGIN RSA PRIVATE KEY-----\nAAAA\n-----END RSA PRIVATE KEY-----".into(),
                ..certificates()
            },
            Certificates {
                signature_v2: String::new(),
                ..certificates()
            },
            Certificates {
                expires_at: "soon".into(),
                ..certificates()
            },
        ] {
            assert!(ProfileKeyPair::from_certificates(&broken).is_err());
        }
    }

    fn wait(keys: &ProfileKeys) -> Option<Arc<ProfileKeyPair>> {
        for _ in 0..500 {
            if let Some(pair) = keys.poll() {
                return pair;
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        panic!("the fetch never ended");
    }

    #[test]
    fn a_pair_is_fetched_once_until_it_is_due() {
        let fetches = Arc::new(AtomicU32::new(0));
        let counted = fetches.clone();
        let due = Arc::new(Mutex::new(String::from("2100-01-01T00:00:00Z")));
        let refresh = due.clone();
        let keys = ProfileKeys::new(Arc::new(move || {
            counted.fetch_add(1, Ordering::SeqCst);
            Ok(Certificates {
                refreshed_after: lock(&refresh).clone(),
                ..certificates()
            })
        }));
        assert_eq!(keys.poll(), None);
        // Nothing kept: due.
        assert!(keys.should_refresh());
        keys.prepare();
        let first = wait(&keys).unwrap();
        assert_eq!(keys.poll(), None);
        // Within the hour, never; the kept pair is not due anyway.
        assert!(!keys.should_refresh());
        keys.prepare();
        assert_eq!(wait(&keys), Some(first.clone()));
        assert_eq!(fetches.load(Ordering::SeqCst), 1);
        // Due: fetched again, but not within the hour of the last prepare.
        *lock(&due) = "2000-01-01T00:00:00Z".into();
        lock(&keys.state).kept = Some(Arc::new(ProfileKeyPair {
            refresh_after: 0,
            ..(*first).clone()
        }));
        assert!(!keys.should_refresh());
        lock(&keys.state).next_refresh = Some(Instant::now());
        std::thread::sleep(Duration::from_millis(2));
        assert!(keys.should_refresh());
        keys.prepare();
        let second = wait(&keys).unwrap();
        assert_eq!(fetches.load(Ordering::SeqCst), 2);
        assert_eq!(second.refresh_after, 946_684_800_000);
    }

    #[test]
    fn a_failed_fetch_keeps_what_was_had() {
        let fail = Arc::new(AtomicU32::new(0));
        let failing = fail.clone();
        let keys = ProfileKeys::new(Arc::new(move || {
            if failing.load(Ordering::SeqCst) == 1 {
                Err(AuthError::BadResponse("down".into()))
            } else {
                Ok(Certificates {
                    refreshed_after: "2000-01-01T00:00:00Z".into(),
                    ..certificates()
                })
            }
        }));
        keys.prepare();
        let had = wait(&keys).unwrap();
        fail.store(1, Ordering::SeqCst);
        keys.prepare();
        assert_eq!(wait(&keys), Some(had));
        // With nothing kept, nothing.
        let none = ProfileKeys::new(Arc::new(|| Err(AuthError::BadResponse("down".into()))));
        none.prepare();
        assert_eq!(wait(&none), None);
    }
}

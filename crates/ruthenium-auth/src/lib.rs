//! Signing in with a Microsoft account to play Minecraft: Java Edition.
//!
//! 1. Microsoft identity platform ([`msa`]): browser sign-in with PKCE and a loopback
//!    redirect, or a device code; refresh tokens after that.
//! 2. Xbox Live user token, then XSTS for the Minecraft services ([`xbox`]).
//! 3. Minecraft services ([`minecraft`]): access token, then the Java profile, which is
//!    required (no profile, no play: Ruthenium has no offline mode), and the player's
//!    privileges, which can turn Multiplayer off. Without a profile, the store
//!    entitlements say whether the account owns the game but hasn't picked a name yet.
//!
//! Ruthenium signs in as its own Azure app, [`CLIENT_ID`]. Until Mojang approves that app
//! for the Minecraft API, step 3 answers 403: [`AuthError::NotApproved`].

#![forbid(unsafe_code)]

pub mod http;
pub mod minecraft;
pub mod msa;
pub mod pkce;
pub mod skin;
pub mod store;
pub mod xbox;

use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant};

pub use http::{Http, UreqHttp};
pub use minecraft::{Certificates, Endpoints, Privileges, Profile};
pub use msa::DeviceCode;
pub use store::{Account, AccountStore};
pub use xbox::XboxRefusal;

/// Ruthenium's Azure app (personal Microsoft accounts, public client). Not a secret.
pub const CLIENT_ID: &str = "5738b34f-eb22-45b4-9e69-84db5fce328a";

/// The client ID to sign in as: `RUTHENIUM_CLIENT_ID` if set (development), else
/// [`CLIENT_ID`].
pub fn client_id() -> String {
    std::env::var("RUTHENIUM_CLIENT_ID")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| CLIENT_ID.to_owned())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthError {
    Network(String),
    Cancelled,
    TimedOut,
    /// The player declined at Microsoft's sign-in page.
    Denied(String),
    /// The saved sign-in is no longer valid: sign in again.
    SignInExpired,
    Microsoft(String),
    Xbox(XboxRefusal),
    /// Mojang hasn't approved Ruthenium for the Minecraft API yet.
    NotApproved,
    /// The account doesn't own Minecraft: Java Edition (no profile).
    NoProfile,
    /// The account owns Java Edition but has no profile yet (no name chosen).
    NoProfileName,
    Mojang(u16, String),
    Keystore(String),
    BadResponse(String),
}

impl std::fmt::Display for AuthError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AuthError::Network(e) => write!(f, "couldn't reach the sign-in service: {e}"),
            AuthError::Cancelled => f.write_str("sign-in cancelled"),
            AuthError::TimedOut => f.write_str("sign-in timed out"),
            AuthError::Denied(d) => write!(f, "sign-in was declined: {d}"),
            AuthError::SignInExpired => f.write_str("the saved sign-in has expired: sign in again"),
            AuthError::Microsoft(e) => write!(f, "Microsoft sign-in failed: {e}"),
            AuthError::Xbox(r) => write!(f, "Xbox Live: {r}"),
            AuthError::NotApproved => f.write_str(
                "Mojang hasn't approved Ruthenium for the Minecraft services yet, so it can't sign in",
            ),
            AuthError::NoProfile => {
                f.write_str("this account doesn't own Minecraft: Java Edition (it has no Java profile)")
            }
            AuthError::NoProfileName => f.write_str(
                "this account owns Minecraft: Java Edition but has no profile yet: choose a name at minecraft.net or in the official launcher",
            ),
            AuthError::Mojang(s, e) => write!(f, "Minecraft services error (HTTP {s}): {e}"),
            AuthError::Keystore(e) => write!(f, "couldn't use the credential store: {e}"),
            AuthError::BadResponse(e) => write!(f, "unexpected answer from the sign-in service: {e}"),
        }
    }
}

impl std::error::Error for AuthError {}

/// A signed-in player, in memory.
#[derive(Clone, PartialEq, Eq)]
pub struct Session {
    pub profile: Profile,
    pub access_token: String,
    pub expires_at: Instant,
    pub privileges: Privileges,
}

impl std::fmt::Debug for Session {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Session")
            .field("profile", &self.profile)
            .field("privileges", &self.privileges)
            .finish_non_exhaustive()
    }
}

/// A finished sign-in: the session and the Microsoft refresh token to keep.
#[derive(Clone)]
pub struct SignedIn {
    pub session: Session,
    pub refresh_token: String,
}

impl std::fmt::Debug for SignedIn {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SignedIn")
            .field("session", &self.session)
            .finish_non_exhaustive()
    }
}

/// Runs the sign-in flows. Every call blocks on the network: run it off the main thread.
pub struct Auth {
    http: Box<dyn Http>,
    client_id: String,
    endpoints: Option<Endpoints>,
    discovery_cache: Option<PathBuf>,
}

impl std::fmt::Debug for Auth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Auth")
            .field("client_id", &self.client_id)
            .finish_non_exhaustive()
    }
}

impl Auth {
    /// Signs in as [`client_id()`] over HTTPS.
    pub fn new() -> Auth {
        Auth::with_http(Box::new(UreqHttp::new()), &client_id())
    }

    pub fn with_http(http: Box<dyn Http>, client_id: &str) -> Auth {
        Auth {
            http,
            client_id: client_id.to_owned(),
            endpoints: None,
            discovery_cache: None,
        }
    }

    /// Keeps the services' discovery document in the file `path` (see
    /// [`Endpoints::discover_cached`]).
    pub fn with_discovery_cache(mut self, path: &Path) -> Auth {
        self.discovery_cache = Some(path.to_owned());
        self
    }

    fn endpoints(&mut self) -> Endpoints {
        let (http, cache) = (&*self.http, self.discovery_cache.as_deref());
        self.endpoints
            .get_or_insert_with(|| match cache {
                Some(path) => Endpoints::discover_cached(http, path),
                None => Endpoints::discover(http),
            })
            .clone()
    }

    /// Signs in through the system browser; `open` is handed the page to open.
    pub fn sign_in_browser(
        &mut self,
        open: &mut dyn FnMut(&str),
        cancel: &AtomicBool,
    ) -> Result<SignedIn, AuthError> {
        let tokens = msa::sign_in_browser(&*self.http, &self.client_id, open, cancel)?;
        self.finish(tokens)
    }

    /// Signs in with a device code; `show` is handed the code to enter.
    pub fn sign_in_device(
        &mut self,
        show: &mut dyn FnMut(&DeviceCode),
        cancel: &AtomicBool,
    ) -> Result<SignedIn, AuthError> {
        let tokens = msa::sign_in_device(&*self.http, &self.client_id, show, cancel)?;
        self.finish(tokens)
    }

    /// Signs in again from a saved refresh token.
    pub fn resume(&mut self, refresh_token: &str) -> Result<SignedIn, AuthError> {
        let tokens = msa::refresh(&*self.http, &self.client_id, refresh_token)?;
        self.finish(tokens)
    }

    /// The chat signing certificates for a session (keep them in memory only).
    pub fn certificates(&mut self, session: &Session) -> Result<Certificates, AuthError> {
        let endpoints = self.endpoints();
        let token = minecraft::McToken {
            access_token: session.access_token.clone(),
            expires_in: 0,
        };
        minecraft::certificates(&*self.http, &endpoints, &token)
    }

    fn finish(&mut self, tokens: msa::MsaTokens) -> Result<SignedIn, AuthError> {
        let started = Instant::now();
        let http = &*self.http;
        let user = xbox::user_token(http, &tokens.access_token)?;
        let xsts = xbox::xsts_token(http, &user)?;
        let endpoints = self.endpoints();
        let http = &*self.http;
        let token = minecraft::login(http, &endpoints, &xsts)?;
        let profile = match minecraft::profile(http, &token) {
            // Bought but not set up yet, or not bought at all? Not knowing is "not bought".
            Err(AuthError::NoProfile) if minecraft::owns_java(http, &token) == Ok(true) => {
                return Err(AuthError::NoProfileName);
            }
            r => r?,
        };
        // An account without attributes (or a hiccup reading them) keeps the defaults.
        let privileges = minecraft::privileges(http, &endpoints, &token).unwrap_or_default();
        Ok(SignedIn {
            session: Session {
                profile,
                expires_at: started + Duration::from_secs(token.expires_in),
                access_token: token.access_token,
                privileges,
            },
            refresh_token: tokens.refresh_token,
        })
    }
}

impl Default for Auth {
    fn default() -> Auth {
        Auth::new()
    }
}

/// Opens `url` in the system browser.
pub fn open_browser(url: &str) -> std::io::Result<()> {
    use std::process::{Command, Stdio};
    let mut cmd = if cfg!(target_os = "windows") {
        let mut c = Command::new("rundll32");
        c.args(["url.dll,FileProtocolHandler", url]);
        c
    } else if cfg!(target_os = "macos") {
        let mut c = Command::new("open");
        c.arg(url);
        c
    } else {
        let mut c = Command::new("xdg-open");
        c.arg(url);
        c
    };
    cmd.stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map(drop)
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use serde_json::{Value, json};

    use super::*;
    use crate::http::Reply;

    /// Answers by URL, recording what was asked.
    struct FakeHttp {
        routes: Vec<(&'static str, u16, Value)>,
        log: Mutex<Vec<String>>,
    }

    impl FakeHttp {
        fn reply(&self, method: &str, url: &str, body: String) -> Result<Reply, AuthError> {
            self.log
                .lock()
                .unwrap()
                .push(format!("{method} {url} {body}"));
            let (_, status, v) = self
                .routes
                .iter()
                .find(|(u, _, _)| url.contains(u))
                .ok_or_else(|| AuthError::Network(format!("no route for {url}")))?;
            Ok(Reply {
                status: *status,
                body: v.to_string(),
            })
        }
    }

    impl Http for FakeHttp {
        fn post_form(&self, url: &str, form: &[(&str, &str)]) -> Result<Reply, AuthError> {
            self.reply("FORM", url, format!("{form:?}"))
        }

        fn post_json(
            &self,
            url: &str,
            body: &Value,
            bearer: Option<&str>,
        ) -> Result<Reply, AuthError> {
            self.reply("POST", url, format!("{body} {bearer:?}"))
        }

        fn get(&self, url: &str, bearer: Option<&str>) -> Result<Reply, AuthError> {
            self.reply("GET", url, format!("{bearer:?}"))
        }
    }

    fn services(login_status: u16, profile_status: u16) -> FakeHttp {
        FakeHttp {
            routes: vec![
                (
                    "oauth2/v2.0/token",
                    200,
                    json!({"access_token": "msa", "refresh_token": "new-refresh", "expires_in": 3600}),
                ),
                (
                    "user.auth.xboxlive.com",
                    200,
                    json!({"Token": "xbl", "DisplayClaims": {"xui": [{"uhs": "42"}]}}),
                ),
                (
                    "xsts.auth.xboxlive.com",
                    200,
                    json!({"Token": "xsts", "DisplayClaims": {"xui": [{"uhs": "42"}]}}),
                ),
                ("discovery.minecraftservices.com", 500, json!({})),
                (
                    "login_with_xbox",
                    login_status,
                    json!({"access_token": "mc", "expires_in": 86400}),
                ),
                (
                    "minecraft/profile",
                    profile_status,
                    json!({"id": "abc123", "name": "Alex", "skins": [{"state": "ACTIVE", "url": "https://textures.minecraft.net/texture/x"}]}),
                ),
                (
                    "player/attributes",
                    200,
                    json!({"privileges": {"onlineChat": {"enabled": true}, "multiplayerServer": {"enabled": true}, "multiplayerRealms": {"enabled": true}}}),
                ),
            ],
            log: Mutex::new(Vec::new()),
        }
    }

    #[test]
    fn resume_runs_the_whole_chain() {
        let http = services(200, 200);
        let mut auth = Auth::with_http(Box::new(http), "cid");
        let s = auth.resume("old-refresh").unwrap();
        assert_eq!(s.refresh_token, "new-refresh");
        assert_eq!(s.session.profile.name, "Alex");
        assert_eq!(s.session.access_token, "mc");
        assert!(s.session.privileges.may_play_online());
        assert!(s.session.profile.skin.is_some());
    }

    #[test]
    fn the_chain_sends_the_right_tokens() {
        let http = std::sync::Arc::new(services(200, 200));
        struct Shared(std::sync::Arc<FakeHttp>);
        impl Http for Shared {
            fn post_form(&self, u: &str, f: &[(&str, &str)]) -> Result<Reply, AuthError> {
                self.0.post_form(u, f)
            }
            fn post_json(&self, u: &str, b: &Value, t: Option<&str>) -> Result<Reply, AuthError> {
                self.0.post_json(u, b, t)
            }
            fn get(&self, u: &str, t: Option<&str>) -> Result<Reply, AuthError> {
                self.0.get(u, t)
            }
        }
        let mut auth = Auth::with_http(Box::new(Shared(http.clone())), "cid");
        auth.resume("old-refresh").unwrap();
        let log = http.log.lock().unwrap().join("\n");
        assert!(log.contains("(\"grant_type\", \"refresh_token\")") && log.contains("old-refresh"));
        assert!(log.contains("\"RpsTicket\":\"d=msa\""));
        assert!(log.contains("\"UserTokens\":[\"xbl\"]"));
        assert!(log.contains("XBL3.0 x=42;xsts"));
        assert!(
            log.contains("GET https://api.minecraftservices.com/minecraft/profile Some(\"mc\")")
        );
    }

    #[test]
    fn refusals_are_errors_not_offline_play() {
        let mut auth = Auth::with_http(Box::new(services(403, 200)), "cid");
        assert_eq!(auth.resume("r").unwrap_err(), AuthError::NotApproved);
        let mut auth = Auth::with_http(Box::new(services(200, 404)), "cid");
        assert_eq!(auth.resume("r").unwrap_err(), AuthError::NoProfile);
        let mut http = services(200, 200);
        http.routes[2] = (
            "xsts.auth.xboxlive.com",
            401,
            json!({"XErr": 2148916238u64}),
        );
        let mut auth = Auth::with_http(Box::new(http), "cid");
        assert_eq!(
            auth.resume("r").unwrap_err(),
            AuthError::Xbox(XboxRefusal::ChildAccount)
        );
    }

    #[test]
    fn entitlements_explain_a_missing_profile() {
        let with_items = |items: Value| {
            let mut http = services(200, 404);
            http.routes
                .push(("entitlements/mcstore", 200, json!({ "items": items })));
            Auth::with_http(Box::new(http), "cid")
                .resume("r")
                .unwrap_err()
        };
        assert_eq!(
            with_items(json!([{"name": "product_minecraft"}, {"name": "game_minecraft"}])),
            AuthError::NoProfileName
        );
        assert_eq!(with_items(json!([])), AuthError::NoProfile);
        // The profile is the gate: entitlements are only asked about when it is missing.
        let mut http = services(200, 200);
        http.routes
            .insert(0, ("entitlements/mcstore", 500, json!({})));
        assert!(Auth::with_http(Box::new(http), "cid").resume("r").is_ok());
    }
}

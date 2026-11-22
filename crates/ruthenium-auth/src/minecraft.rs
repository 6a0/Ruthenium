//! The Minecraft services: the access token from XSTS, the Java profile (the ownership
//! gate: no profile, no play), the player's privileges and ban status, and the chat
//! signing certificates (kept in memory only, never stored).
//!
//! Endpoints come from the services' discovery document, kept on disk for a day and read
//! again after that; when it can't be fetched, the copy on disk (however old), else the
//! values it held on 2026-11-22.

use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};

use crate::AuthError;
use crate::http::Http;
use crate::xbox::XboxToken;

const DISCOVERY: &str = "https://discovery.minecraftservices.com/minecraft/client";
const PROFILE: &str = "https://api.minecraftservices.com/minecraft/profile";
const ENTITLEMENTS: &str = "https://api.minecraftservices.com/entitlements/mcstore";
/// How long a discovery document on disk is used before it is fetched again.
pub const DISCOVERY_MAX_AGE: Duration = Duration::from_secs(24 * 60 * 60);

/// The service addresses sign-in uses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Endpoints {
    pub login_xbox: String,
    pub attributes: String,
    pub certificates: String,
    pub session_join: String,
}

impl Default for Endpoints {
    fn default() -> Endpoints {
        Endpoints {
            login_xbox: "https://api.minecraftservices.com/authentication/login_with_xbox".into(),
            attributes: "https://api.minecraftservices.com/player/attributes".into(),
            certificates: "https://api.minecraftservices.com/player/certificates".into(),
            session_join: "https://sessionserver.mojang.com/session/minecraft/join".into(),
        }
    }
}

impl Endpoints {
    /// Reads the discovery document; each address it lacks keeps its default.
    pub fn from_discovery(doc: &Value) -> Endpoints {
        let mut e = Endpoints::default();
        let get = |path: &str| doc.pointer(path).and_then(Value::as_str).map(str::to_owned);
        let d = "/discovery";
        if let Some(u) = get(&format!("{d}/authentication/endpoints/loginXbox/uri")) {
            e.login_xbox = u;
        }
        if let Some(u) = get(&format!("{d}/player/endpoints/getAttributes/uri")) {
            e.attributes = u;
        }
        if let Some(u) = get(&format!("{d}/player/endpoints/getCertificates/uri")) {
            e.certificates = u;
        }
        if let Some(u) = get(&format!("{d}/session/endpoints/join/uri")) {
            e.session_join = u;
        }
        e
    }

    /// Fetches the discovery document, falling back to the defaults.
    pub fn discover(http: &dyn Http) -> Endpoints {
        fetch_discovery(http).map_or_else(Endpoints::default, |doc| Endpoints::from_discovery(&doc))
    }

    /// Like [`Self::discover`], with the document kept in the file `cache`: a copy younger
    /// than [`DISCOVERY_MAX_AGE`] is used as it is; an older one is fetched again, and
    /// used if that fails.
    pub fn discover_cached(http: &dyn Http, cache: &Path) -> Endpoints {
        let saved = std::fs::read_to_string(cache)
            .ok()
            .and_then(|s| serde_json::from_str::<Value>(&s).ok())
            .and_then(|v| Some((v.get("fetched")?.as_u64()?, v.get("document")?.clone())));
        let now = unix_now();
        if let Some((fetched, doc)) = &saved
            && now.saturating_sub(*fetched) < DISCOVERY_MAX_AGE.as_secs()
            && *fetched <= now
        {
            return Endpoints::from_discovery(doc);
        }
        match fetch_discovery(http) {
            Some(doc) => {
                let text = json!({ "fetched": now, "document": doc }).to_string();
                if let Err(e) = write_atomically(cache, &text) {
                    eprintln!("[ruthenium] couldn't keep the services' discovery document: {e}");
                }
                Endpoints::from_discovery(&doc)
            }
            None => saved.map_or_else(Endpoints::default, |(_, doc)| {
                Endpoints::from_discovery(&doc)
            }),
        }
    }
}

fn fetch_discovery(http: &dyn Http) -> Option<Value> {
    let r = http.get(DISCOVERY, None).ok().filter(|r| r.ok())?;
    let doc = r.json();
    doc.get("discovery").is_some().then_some(doc)
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

fn write_atomically(path: &Path, text: &str) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("json.new");
    std::fs::write(&tmp, text)?;
    std::fs::rename(&tmp, path)
}

/// A Minecraft services access token.
#[derive(Clone, PartialEq, Eq)]
pub struct McToken {
    pub access_token: String,
    pub expires_in: u64,
}

impl std::fmt::Debug for McToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("McToken")
            .field("expires_in", &self.expires_in)
            .finish_non_exhaustive()
    }
}

/// Exchanges the XSTS token. A 403 means Mojang hasn't approved this app for the
/// Minecraft API (yet): a first-class error, never a reason to play offline.
pub fn login(
    http: &dyn Http,
    endpoints: &Endpoints,
    xsts: &XboxToken,
) -> Result<McToken, AuthError> {
    let body = json!({ "identityToken": format!("XBL3.0 x={};{}", xsts.user_hash, xsts.token) });
    let r = http.post_json(&endpoints.login_xbox, &body, None)?;
    match r.status {
        403 => return Err(AuthError::NotApproved),
        s if !r.ok() => return Err(AuthError::Mojang(s, error_message(&r.json()))),
        _ => {}
    }
    let v = r.json();
    Ok(McToken {
        access_token: v
            .get("access_token")
            .and_then(Value::as_str)
            .ok_or_else(|| AuthError::BadResponse("no Minecraft access token".into()))?
            .to_owned(),
        expires_in: v
            .get("expires_in")
            .and_then(Value::as_u64)
            .unwrap_or(86_400),
    })
}

fn error_message(v: &Value) -> String {
    ["errorMessage", "error", "path"]
        .iter()
        .find_map(|k| v.get(*k).and_then(Value::as_str))
        .unwrap_or("no details")
        .to_owned()
}

/// A Java Edition profile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Profile {
    /// Without dashes, as the services send it.
    pub id: String,
    pub name: String,
    /// The active skin's texture URL.
    pub skin: Option<String>,
}

/// The account's Java profile; an account without one does not own the game.
pub fn profile(http: &dyn Http, token: &McToken) -> Result<Profile, AuthError> {
    let r = http.get(PROFILE, Some(&token.access_token))?;
    match r.status {
        404 => return Err(AuthError::NoProfile),
        s if !r.ok() => return Err(AuthError::Mojang(s, error_message(&r.json()))),
        _ => {}
    }
    let v = r.json();
    let s = |k: &str| v.get(k).and_then(Value::as_str).map(str::to_owned);
    Ok(Profile {
        id: s("id").ok_or_else(|| AuthError::BadResponse("profile without an id".into()))?,
        name: s("name").ok_or_else(|| AuthError::BadResponse("profile without a name".into()))?,
        skin: v
            .get("skins")
            .and_then(Value::as_array)
            .and_then(|skins| {
                skins
                    .iter()
                    .find(|k| k.get("state").and_then(Value::as_str) == Some("ACTIVE"))
            })
            .and_then(|k| k.get("url")?.as_str().map(str::to_owned)),
    })
}

/// Whether the account's Minecraft store entitlements include Java Edition
/// (`game_minecraft` or `product_minecraft`, Game Pass included). The answer is a signed
/// token whose signature is not checked: it only explains a missing profile, it never
/// lets anyone play (the profile does).
pub fn owns_java(http: &dyn Http, token: &McToken) -> Result<bool, AuthError> {
    let r = http.get(ENTITLEMENTS, Some(&token.access_token))?;
    if !r.ok() {
        return Err(AuthError::Mojang(r.status, error_message(&r.json())));
    }
    Ok(r.json()
        .get("items")
        .and_then(Value::as_array)
        .is_some_and(|items| {
            items.iter().any(|i| {
                matches!(
                    i.get("name").and_then(Value::as_str),
                    Some("game_minecraft" | "product_minecraft")
                )
            })
        }))
}

/// A ban from one of the services.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ban {
    pub reason: String,
    /// Epoch milliseconds, or none for a permanent ban.
    pub expires: Option<i64>,
}

/// What the account may do (`/player/attributes`). Missing privileges count as allowed
/// only when the whole answer is missing (an older account without attributes).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Privileges {
    pub online_chat: bool,
    /// Parental controls can turn this off; then Multiplayer is disabled.
    pub multiplayer_server: bool,
    pub multiplayer_realms: bool,
    pub multiplayer_ban: Option<Ban>,
    pub profanity_filter: bool,
}

impl Default for Privileges {
    fn default() -> Privileges {
        Privileges {
            online_chat: true,
            multiplayer_server: true,
            multiplayer_realms: true,
            multiplayer_ban: None,
            profanity_filter: false,
        }
    }
}

impl Privileges {
    pub fn from_json(v: &Value) -> Privileges {
        let on = |name: &str| {
            v.pointer(&format!("/privileges/{name}/enabled"))
                .and_then(Value::as_bool)
                .unwrap_or(false)
        };
        Privileges {
            online_chat: on("onlineChat"),
            multiplayer_server: on("multiplayerServer"),
            multiplayer_realms: on("multiplayerRealms"),
            multiplayer_ban: v
                .pointer("/banStatus/bannedScopes/MULTIPLAYER")
                .map(|b| Ban {
                    reason: b
                        .get("reasonMessage")
                        .or_else(|| b.get("reason"))
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_owned(),
                    expires: b.get("expires").and_then(Value::as_i64),
                }),
            profanity_filter: v
                .pointer("/profanityFilterPreferences/profanityFilterOn")
                .and_then(Value::as_bool)
                .unwrap_or(false),
        }
    }

    /// Whether the player may join servers.
    pub fn may_play_online(&self) -> bool {
        self.multiplayer_server && self.multiplayer_ban.is_none()
    }
}

pub fn privileges(
    http: &dyn Http,
    endpoints: &Endpoints,
    token: &McToken,
) -> Result<Privileges, AuthError> {
    let r = http.get(&endpoints.attributes, Some(&token.access_token))?;
    if !r.ok() {
        return Err(AuthError::Mojang(r.status, error_message(&r.json())));
    }
    Ok(Privileges::from_json(&r.json()))
}

/// The chat signing key pair and Mojang's signature over its public half. Memory only.
#[derive(Clone, PartialEq, Eq)]
pub struct Certificates {
    pub private_key_pem: String,
    pub public_key_pem: String,
    /// Base64.
    pub signature_v2: String,
    /// ISO 8601.
    pub expires_at: String,
    pub refreshed_after: String,
}

impl std::fmt::Debug for Certificates {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Certificates")
            .field("expires_at", &self.expires_at)
            .finish_non_exhaustive()
    }
}

pub fn certificates(
    http: &dyn Http,
    endpoints: &Endpoints,
    token: &McToken,
) -> Result<Certificates, AuthError> {
    let r = http.post_json(
        &endpoints.certificates,
        &Value::Null,
        Some(&token.access_token),
    )?;
    if !r.ok() {
        return Err(AuthError::Mojang(r.status, error_message(&r.json())));
    }
    let v = r.json();
    let s = |p: &str| {
        v.pointer(p)
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned()
    };
    let c = Certificates {
        private_key_pem: s("/keyPair/privateKey"),
        public_key_pem: s("/keyPair/publicKey"),
        signature_v2: s("/publicKeySignatureV2"),
        expires_at: s("/expiresAt"),
        refreshed_after: s("/refreshedAfter"),
    };
    if c.private_key_pem.is_empty() || c.public_key_pem.is_empty() {
        return Err(AuthError::BadResponse(
            "certificates without a key pair".into(),
        ));
    }
    Ok(c)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discovery_overrides_defaults() {
        let doc = json!({"discovery": {"authentication": {"endpoints": {"loginXbox": {"uri": "https://x/login"}}}}});
        let e = Endpoints::from_discovery(&doc);
        assert_eq!(e.login_xbox, "https://x/login");
        assert_eq!(e.attributes, Endpoints::default().attributes);
    }

    #[test]
    fn discovery_is_kept_for_a_day() {
        use crate::http::Reply;
        use std::sync::atomic::{AtomicUsize, Ordering};
        struct Doc(AtomicUsize, u16);
        impl Http for Doc {
            fn post_form(&self, _: &str, _: &[(&str, &str)]) -> Result<Reply, AuthError> {
                unreachable!()
            }
            fn post_json(&self, _: &str, _: &Value, _: Option<&str>) -> Result<Reply, AuthError> {
                unreachable!()
            }
            fn get(&self, url: &str, _: Option<&str>) -> Result<Reply, AuthError> {
                assert_eq!(url, DISCOVERY);
                self.0.fetch_add(1, Ordering::Relaxed);
                Ok(Reply {
                    status: self.1,
                    body: json!({"discovery": {"player": {"endpoints": {"getAttributes": {"uri": "https://x/attr"}}}}}).to_string(),
                })
            }
        }
        let dir = std::env::temp_dir().join(format!("ruthenium-discovery-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let cache = dir.join("discovery.json");
        let up = Doc(AtomicUsize::new(0), 200);
        assert_eq!(
            Endpoints::discover_cached(&up, &cache).attributes,
            "https://x/attr"
        );
        assert_eq!(
            Endpoints::discover_cached(&up, &cache).attributes,
            "https://x/attr"
        );
        assert_eq!(
            up.0.load(Ordering::Relaxed),
            1,
            "the second read comes from disk"
        );
        // Stale: fetched again; when that fails, the stale copy still beats the defaults.
        let old = json!({"fetched": 1, "document": {"discovery": {"player": {"endpoints": {"getAttributes": {"uri": "https://old/attr"}}}}}});
        std::fs::write(&cache, old.to_string()).unwrap();
        let down = Doc(AtomicUsize::new(0), 503);
        assert_eq!(
            Endpoints::discover_cached(&down, &cache).attributes,
            "https://old/attr"
        );
        assert_eq!(down.0.load(Ordering::Relaxed), 1);
        assert_eq!(
            Endpoints::discover_cached(&up, &cache).attributes,
            "https://x/attr"
        );
        std::fs::remove_file(&cache).unwrap();
        assert_eq!(
            Endpoints::discover_cached(&down, &cache),
            Endpoints::default()
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn privileges_and_bans() {
        let v = json!({
            "privileges": {
                "onlineChat": {"enabled": true},
                "multiplayerServer": {"enabled": false},
                "multiplayerRealms": {"enabled": true}
            },
            "profanityFilterPreferences": {"profanityFilterOn": true},
            "banStatus": {"bannedScopes": {"MULTIPLAYER": {"banId": "x", "expires": 1700000000000i64, "reasonMessage": "Hate speech"}}}
        });
        let p = Privileges::from_json(&v);
        assert!(p.online_chat && !p.multiplayer_server && p.profanity_filter);
        assert_eq!(p.multiplayer_ban.as_ref().unwrap().reason, "Hate speech");
        assert!(!p.may_play_online());
        assert!(Privileges::default().may_play_online());
    }
}

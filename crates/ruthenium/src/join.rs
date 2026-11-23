//! Joining a server from the menus: who joins (the account's session, or in development
//! builds an offline name), with what (the imported game's language and data packs), and
//! the words the connecting and disconnected screens use.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::thread::JoinHandle;

use ruthenium_assets::zip::Zip;
use ruthenium_net::connection::{Auth, Closed, ConnectOptions, Identity, Local, Reason, Stage};
use ruthenium_protocol::packets::{ClientInformation, KnownPack};
use ruthenium_protocol::text::StyledText;
use ruthenium_sim::blocks::Blocks;
use ruthenium_sim::enchantment::Enchantments;
use ruthenium_sim::entity::EntityTypes;
use ruthenium_sim::item::Items;
use ruthenium_sim::session::{DimensionType, Physics, Session, SessionOptions};
use ruthenium_sim::use_on::Transformers;

use crate::account::Accounts;

/// The brand Ruthenium sends (PLAN D6).
pub const BRAND: &str = "ruthenium";

/// Present in development builds that can join offline-mode servers, and only there: CI
/// checks that release binaries don't contain it (PLAN D3).
#[cfg(feature = "dev-offline")]
pub const DEV_OFFLINE_MARKER: &str = "RUTHENIUM-DEV-OFFLINE-BUILD";

const LANG: &str = "assets/minecraft/lang/en_us.json";
const DATAPACKS: &str = "data/minecraft/datapacks/";

/// What joining needs from the imported game.
#[derive(Debug, Default)]
pub struct GameData {
    /// English translations, for the servers' messages.
    pub lang: HashMap<String, String>,
    /// Vanilla's data packs at the game's version: `core` and the feature packs in the
    /// jar (`ServerPacksSource.createVanillaTrustedRepository`).
    pub known_packs: Vec<KnownPack>,
    /// What movement needs: the exported block physics and vanilla's dimension heights.
    pub physics: Option<Physics>,
}

impl GameData {
    /// Reads the imported game in `game`; empty without it.
    pub fn load(game: &Path) -> GameData {
        let version = ruthenium_prepare::supported_version();
        let Some(imported) = ruthenium_prepare::import::open(version, game) else {
            return GameData::default();
        };
        let Ok(jar) = Zip::open(&imported.client_jar) else {
            return GameData::default();
        };
        let lang = jar
            .find(LANG)
            .and_then(|i| jar.read(i).ok())
            .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
            .and_then(|v| match v {
                serde_json::Value::Object(o) => Some(
                    o.into_iter()
                        .filter_map(|(k, v)| Some((k, v.as_str()?.to_owned())))
                        .collect(),
                ),
                _ => None,
            })
            .unwrap_or_default();
        let id = ruthenium_data::v26_3::version::ID;
        let mut known_packs = vec![KnownPack::vanilla("core", id)];
        let mut features: Vec<String> = jar
            .with_prefix(DATAPACKS)
            .filter_map(|i| {
                let rest = jar.name(i).strip_prefix(DATAPACKS)?;
                let (pack, file) = rest.split_once('/')?;
                (file == "pack.mcmeta").then(|| pack.to_owned())
            })
            .collect();
        features.sort();
        features.dedup();
        known_packs.extend(features.iter().map(|f| KnownPack::vanilla(f, id)));
        let exports = ruthenium_prepare::exporters::exports_dir(&imported);
        let physics = match Blocks::load(&exports) {
            Ok(blocks) => {
                // Without entity types the player still moves; it just doesn't see
                // other entities (an export from before they were added, say).
                let entity_types = EntityTypes::load(&exports).unwrap_or_else(|e| {
                    eprintln!(
                        "[ruthenium] no entity types ({e}); run `ruthenium prepare`. Other entities won't push the player"
                    );
                    EntityTypes::default()
                });
                // Without items nothing in the hands can be used.
                let reports = ruthenium_prepare::datagen::reports_dir(&imported);
                let items = Items::load(&exports, &reports).unwrap_or_else(|e| {
                    eprintln!(
                        "[ruthenium] no item data ({e}); run `ruthenium prepare`. Items can't be used"
                    );
                    Items::default()
                });
                Some(Physics {
                    blocks: Arc::new(blocks),
                    entity_types: Arc::new(entity_types),
                    dimension_types: dimension_types(&jar),
                    items: Arc::new(items),
                    block_transformers: Arc::new(block_transformers(&jar)),
                    enchantments: Arc::new(enchantments(&jar)),
                    biomes: biomes(&jar),
                })
            }
            Err(e) => {
                eprintln!("[ruthenium] no block physics ({e}); run `ruthenium prepare`");
                None
            }
        };
        GameData {
            lang,
            known_packs,
            physics,
        }
    }
}

/// Vanilla's dimension types from the jar's data pack: heights and fast lava, by name.
fn dimension_types(jar: &Zip) -> HashMap<String, DimensionType> {
    const DIR: &str = "data/minecraft/dimension_type/";
    jar.with_prefix(DIR)
        .filter_map(|i| {
            let file = jar.name(i).strip_prefix(DIR)?.strip_suffix(".json")?;
            let v: serde_json::Value = serde_json::from_slice(&jar.read(i).ok()?).ok()?;
            let n = |k: &str| v.get(k)?.as_i64().and_then(|n| i32::try_from(n).ok());
            let fast_lava = v["attributes"][DimensionType::FAST_LAVA]
                .as_bool()
                .unwrap_or(false);
            let t = DimensionType {
                min_y: n("min_y")?,
                height: n("height")?,
                fast_lava,
                has_skylight: v.get("has_skylight")?.as_bool()?,
                has_ceiling: v.get("has_ceiling")?.as_bool()?,
            };
            Some((format!("minecraft:{file}"), t))
        })
        .collect()
}

/// Vanilla's biomes' climates from the jar's data pack, by name.
fn biomes(jar: &Zip) -> HashMap<String, ruthenium_sim::biome::Climate> {
    const DIR: &str = "data/minecraft/worldgen/biome/";
    jar.with_prefix(DIR)
        .filter_map(|i| {
            let file = jar.name(i).strip_prefix(DIR)?.strip_suffix(".json")?;
            let v: serde_json::Value = serde_json::from_slice(&jar.read(i).ok()?).ok()?;
            Some((
                format!("minecraft:{file}"),
                ruthenium_sim::biome::Climate::from_json(&v)?,
            ))
        })
        .collect()
}

/// Vanilla's block transformers from the jar's data pack (`data/minecraft/block_transformer`).
fn block_transformers(jar: &Zip) -> Transformers {
    const DIR: &str = "data/minecraft/block_transformer/";
    let files: Vec<(String, std::borrow::Cow<'_, [u8]>)> = jar
        .with_prefix(DIR)
        .filter_map(|i| Some((jar.name(i).to_owned(), jar.read(i).ok()?)))
        .collect();
    Transformers::from_json_files(files)
}

/// Vanilla's enchantments from the jar's data pack (`data/minecraft/enchantment`).
fn enchantments(jar: &Zip) -> Enchantments {
    const DIR: &str = "data/minecraft/enchantment/";
    let files: Vec<(String, std::borrow::Cow<'_, [u8]>)> = jar
        .with_prefix(DIR)
        .filter_map(|i| Some((jar.name(i).to_owned(), jar.read(i).ok()?)))
        .collect();
    Enchantments::from_json_files(files)
}

/// Starts joining servers: loads the game data on a thread meanwhile.
#[derive(Debug)]
pub struct Joiner {
    data: Data,
    /// For the services' discovery document (online logins).
    #[cfg_attr(feature = "dev-offline", allow(dead_code))]
    cache_dir: Option<PathBuf>,
}

#[derive(Debug)]
enum Data {
    Loading(JoinHandle<GameData>),
    Ready(Arc<GameData>),
}

impl Joiner {
    /// `game`: the imported game files; `cache_dir`: for the services' discovery document.
    pub fn new(game: Option<PathBuf>, cache_dir: Option<PathBuf>) -> Joiner {
        let data = match game {
            Some(dir) => match std::thread::Builder::new()
                .name("game-data".into())
                .spawn(move || GameData::load(&dir))
            {
                Ok(h) => Data::Loading(h),
                Err(_) => Data::Ready(Arc::default()),
            },
            None => Data::Ready(Arc::default()),
        };
        Joiner { data, cache_dir }
    }

    /// The game data, waiting for it if it is still loading.
    pub fn data(&mut self) -> Arc<GameData> {
        if let Data::Loading(_) = &self.data {
            let Data::Loading(h) = std::mem::replace(&mut self.data, Data::Ready(Arc::default()))
            else {
                unreachable!()
            };
            self.data = Data::Ready(Arc::new(h.join().unwrap_or_default()));
        }
        match &self.data {
            Data::Ready(d) => d.clone(),
            Data::Loading(_) => unreachable!(),
        }
    }

    /// Starts a session with the server at `address` (as typed), or says why it can't.
    pub fn join(&mut self, address: &str, accounts: &Accounts) -> Result<Session, String> {
        let (identity, auth, local_only) = self.who(accounts)?;
        let data = self.data();
        Session::start(SessionOptions {
            connect: ConnectOptions {
                address: address.to_owned(),
                identity,
                auth,
                brand: BRAND.into(),
                information: ClientInformation::default(),
                local_only,
            },
            known_packs: data.known_packs.clone(),
            physics: data.physics.clone(),
        })
        .map_err(|e| format!("couldn't start the connection: {e}"))
    }

    /// Development builds join as an offline player, only on this machine or the local
    /// network: `RUTHENIUM_OFFLINE_NAME`, else the account's name, else "Developer".
    #[cfg(feature = "dev-offline")]
    fn who(&self, accounts: &Accounts) -> Result<(Identity, Auth, bool), String> {
        let name = std::env::var("RUTHENIUM_OFFLINE_NAME")
            .ok()
            .or_else(|| accounts.session().map(|s| s.profile.name.clone()))
            .or_else(|| accounts.list().first().map(|a| a.name.clone()))
            .unwrap_or_else(|| "Developer".into());
        if !valid_name(&name) {
            return Err(format!("{name:?} isn't a valid player name"));
        }
        eprintln!("[ruthenium] {DEV_OFFLINE_MARKER}: joining offline as {name}");
        let id = ruthenium_net::crypt::offline_uuid(&name);
        Ok((Identity { name, id }, Auth::Offline, true))
    }

    /// Release builds join as the signed-in account, with its session.
    #[cfg(not(feature = "dev-offline"))]
    fn who(&self, accounts: &Accounts) -> Result<(Identity, Auth, bool), String> {
        let Some(session) = accounts.session() else {
            return Err(match accounts.status() {
                crate::account::AccountStatus::Checking(_) => {
                    "Still signing in, try again in a moment".into()
                }
                crate::account::AccountStatus::Failed { reason, .. } => {
                    format!("Not signed in: {reason}")
                }
                _ => "Sign in with a Microsoft account to play on servers".into(),
            });
        };
        let id = u128::from_str_radix(&session.profile.id, 16)
            .map_err(|_| "the account's profile id isn't a UUID".to_owned())?;
        let join = ruthenium_auth::ServerJoin::new(
            session,
            self.cache_dir
                .as_deref()
                .map(|d| d.join("discovery.json"))
                .as_deref(),
        );
        Ok((
            Identity {
                name: session.profile.name.clone(),
                id,
            },
            Auth::Online(Arc::new(Join(join))),
            false,
        ))
    }
}

/// A name a server accepts (`StringUtil.isValidPlayerName`): at most 16 characters,
/// printable ASCII without spaces.
#[cfg(feature = "dev-offline")]
fn valid_name(name: &str) -> bool {
    !name.is_empty() && name.len() <= 16 && name.bytes().all(|b| (33..127).contains(&b))
}

#[cfg(not(feature = "dev-offline"))]
struct Join(ruthenium_auth::ServerJoin);

#[cfg(not(feature = "dev-offline"))]
impl ruthenium_net::connection::SessionJoin for Join {
    fn join(&self, server_hash: &str) -> Result<(), String> {
        self.0.join(server_hash).map_err(|e| e.to_string())
    }
}

/// The connecting screen's line for `stage` (vanilla's `connect.*`).
pub fn stage_text(stage: Stage) -> &'static str {
    match stage {
        Stage::Connecting => "Connecting to the server...",
        Stage::Authorizing => "Logging in...",
        Stage::Encrypting => "Encrypting...",
        Stage::Negotiating => "Negotiating...",
        Stage::Joining => "Joining world...",
    }
}

/// The disconnected screen's title and reason.
pub fn closed_text(c: &Closed, lang: &HashMap<String, String>) -> (String, String) {
    let title = if c.during_login {
        "Couldn't connect to the server"
    } else {
        "Connection lost"
    };
    let reason = match &c.reason {
        // Vanilla reads it leniently; what doesn't parse shows as it is.
        Reason::Json(j) => match serde_json::from_str::<serde_json::Value>(j) {
            Ok(v) => StyledText::from_json_in(&v, lang).plain(),
            Err(_) => j.clone(),
        },
        Reason::Text(t) => StyledText::from_nbt_in(t, lang).plain(),
        Reason::Local(l) => l.to_string(),
    };
    (title.into(), reason)
}

/// A text component from the server (a death message, say) as plain text, translated
/// with the game's `en_us.json`.
pub fn plain_text(t: &ruthenium_protocol::nbt::Tag, lang: &HashMap<String, String>) -> String {
    StyledText::from_nbt_in(t, lang).plain()
}

/// Whether the session ended because the player left (no screen to show).
pub fn left_on_purpose(c: &Closed) -> bool {
    c.reason == Reason::Local(Local::Closed)
}

/// `text` in lines of at most `width` characters, broken at spaces where it can be.
pub fn wrap(text: &str, width: usize) -> Vec<String> {
    let mut lines = Vec::new();
    for para in text.split('\n') {
        let mut line = String::new();
        for word in para.split(' ') {
            let mut word = word.to_owned();
            while word.chars().count() > width {
                if !line.is_empty() {
                    lines.push(std::mem::take(&mut line));
                }
                let cut: String = word.chars().take(width).collect();
                word = word.chars().skip(width).collect();
                lines.push(cut);
            }
            let need = line.chars().count() + usize::from(!line.is_empty()) + word.chars().count();
            if need > width && !line.is_empty() {
                lines.push(std::mem::take(&mut line));
            }
            if !line.is_empty() {
                line.push(' ');
            }
            line.push_str(&word);
        }
        lines.push(line);
    }
    while lines.last().is_some_and(String::is_empty) && lines.len() > 1 {
        lines.pop();
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;
    use ruthenium_protocol::nbt::Tag;

    #[test]
    fn wraps_at_spaces() {
        assert_eq!(wrap("one two three", 7), ["one two", "three"]);
        assert_eq!(wrap("abcdefghij", 4), ["abcd", "efgh", "ij"]);
        assert_eq!(wrap("a\nb", 10), ["a", "b"]);
    }

    #[test]
    fn reasons_read_as_text() {
        let mut lang = HashMap::new();
        lang.insert(
            "multiplayer.disconnect.not_whitelisted".to_owned(),
            "You are not white-listed on this server!".to_owned(),
        );
        let c = Closed {
            during_login: true,
            reason: Reason::Json(
                r#"{"translate":"multiplayer.disconnect.not_whitelisted"}"#.into(),
            ),
        };
        assert_eq!(
            closed_text(&c, &lang),
            (
                "Couldn't connect to the server".into(),
                "You are not white-listed on this server!".into()
            )
        );
        let c = Closed {
            during_login: false,
            reason: Reason::Text(Tag::String("Server closed".into())),
        };
        assert_eq!(closed_text(&c, &lang).1, "Server closed");
        assert!(left_on_purpose(&Closed {
            during_login: false,
            reason: Reason::Local(Local::Closed)
        }));
    }
}

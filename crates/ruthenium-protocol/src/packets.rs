//! Packets of the login and configuration phases, and the few play packets every phase
//! shares (keep-alive, ping, disconnect, cookies, resource packs, transfer), with 26.3's
//! layouts. Decoding is exact: a packet with bytes left after its fields is refused, as
//! vanilla's `PacketDecoder` does.
//!
//! Serverbound packets are built as their bytes before framing: the id as a VarInt, then
//! the fields ([`wire::packet`]).

use ruthenium_data::v26_3::packets::{configuration as cfg, login as lg, play as pl};

use crate::item::ItemStack;
use crate::nbt::Tag;
use crate::wire::{self, MAX_STRING, Reader, WireError};

/// The longest player name (`ByteBufCodecs.PLAYER_NAME`).
pub const MAX_NAME: usize = 16;
/// Custom payloads and query payloads carry at most this much (`DiscardedPayload`).
pub const MAX_PAYLOAD: usize = 1 << 20;
/// A stored cookie holds at most this much (`ClientboundStoreCookiePacket`).
pub const MAX_COOKIE: usize = 5120;
/// A login disconnect's reason is lenient JSON of at most this many characters.
pub const MAX_LOGIN_REASON: usize = 262_144;

/// A game profile property: textures, usually (`ByteBufCodecs.GAME_PROFILE_PROPERTIES`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Property {
    pub name: String,
    pub value: String,
    pub signature: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GameProfile {
    pub id: u128,
    pub name: String,
    pub properties: Vec<Property>,
}

impl GameProfile {
    fn read(r: &mut Reader<'_>) -> Result<GameProfile, WireError> {
        let id = r.uuid()?;
        let name = r.string(MAX_NAME)?.to_owned();
        let n = r.count(16)?;
        let mut properties = Vec::with_capacity(n);
        for _ in 0..n {
            let name = r.string(64)?.to_owned();
            let value = r.string(MAX_STRING)?.to_owned();
            let signature = if r.bool()? {
                Some(r.string(1024)?.to_owned())
            } else {
                None
            };
            properties.push(Property {
                name,
                value,
                signature,
            });
        }
        Ok(GameProfile {
            id,
            name,
            properties,
        })
    }
}

/// A data pack the server and client may both have (`KnownPack`).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct KnownPack {
    pub namespace: String,
    pub id: String,
    pub version: String,
}

impl KnownPack {
    /// One of vanilla's own packs at `version` (`KnownPack.vanilla`).
    pub fn vanilla(id: &str, version: &str) -> KnownPack {
        KnownPack {
            namespace: "minecraft".into(),
            id: id.into(),
            version: version.into(),
        }
    }

    fn read(r: &mut Reader<'_>) -> Result<KnownPack, WireError> {
        Ok(KnownPack {
            namespace: r.string(MAX_STRING)?.to_owned(),
            id: r.string(MAX_STRING)?.to_owned(),
            version: r.string(MAX_STRING)?.to_owned(),
        })
    }
}

/// Which chat the player sees (`ChatVisiblity`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChatVisibility {
    Full = 0,
    System = 1,
    Hidden = 2,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MainHand {
    Left = 0,
    Right = 1,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParticleStatus {
    All = 0,
    Decreased = 1,
    Minimal = 2,
}

/// What the client tells the server about its options (`ClientInformation`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientInformation {
    pub language: String,
    pub view_distance: u8,
    pub chat_visibility: ChatVisibility,
    pub chat_colors: bool,
    /// Bits of the shown skin layers (`PlayerModelPart` masks).
    pub model_parts: u8,
    pub main_hand: MainHand,
    pub text_filtering: bool,
    pub allows_listing: bool,
    pub particles: ParticleStatus,
}

impl Default for ClientInformation {
    /// Vanilla's options as a new install has them: English, render distance 12, every
    /// skin layer shown, right-handed, listed in the server's player sample.
    fn default() -> ClientInformation {
        ClientInformation {
            language: "en_us".into(),
            view_distance: 12,
            chat_visibility: ChatVisibility::Full,
            chat_colors: true,
            model_parts: 0x7F,
            main_hand: MainHand::Right,
            text_filtering: false,
            allows_listing: true,
            particles: ParticleStatus::All,
        }
    }
}

impl ClientInformation {
    fn write(&self, out: &mut Vec<u8>) {
        wire::write_string(out, &self.language);
        out.push(self.view_distance);
        wire::write_varint(out, self.chat_visibility as i32);
        wire::write_bool(out, self.chat_colors);
        out.push(self.model_parts);
        wire::write_varint(out, self.main_hand as i32);
        wire::write_bool(out, self.text_filtering);
        wire::write_bool(out, self.allows_listing);
        wire::write_varint(out, self.particles as i32);
    }
}

/// A resource pack status (`ServerboundResourcePackPacket.Action`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PackAction {
    SuccessfullyLoaded = 0,
    Declined = 1,
    FailedDownload = 2,
    Accepted = 3,
    Downloaded = 4,
    InvalidUrl = 5,
    FailedReload = 6,
    Discarded = 7,
}

/// A server resource pack offer (`ClientboundResourcePackPushPacket`).
#[derive(Debug, Clone, PartialEq)]
pub struct PackPush {
    pub id: u128,
    pub url: String,
    pub hash: String,
    pub required: bool,
    pub prompt: Option<Tag>,
}

/// A text component in its NBT form (`ComponentSerialization.TRUSTED_CONTEXT_FREE_STREAM_CODEC`).
fn text(r: &mut Reader<'_>) -> Result<Tag, WireError> {
    r.nbt()?.ok_or(WireError::Invalid("text component"))
}

fn optional<T>(
    r: &mut Reader<'_>,
    f: impl FnOnce(&mut Reader<'_>) -> Result<T, WireError>,
) -> Result<Option<T>, WireError> {
    if r.bool()? { f(r).map(Some) } else { Ok(None) }
}

/// Packets every phase after login shares, decoded the same way in each.
#[derive(Debug, Clone, PartialEq)]
pub enum Common {
    CookieRequest {
        key: String,
    },
    /// A custom payload: its channel and data (a brand's data is its name).
    CustomPayload {
        channel: String,
        data: Vec<u8>,
    },
    Disconnect {
        reason: Tag,
    },
    KeepAlive {
        id: i64,
    },
    Ping {
        id: i32,
    },
    ResourcePackPop {
        id: Option<u128>,
    },
    ResourcePackPush(PackPush),
    StoreCookie {
        key: String,
        payload: Vec<u8>,
    },
    Transfer {
        host: String,
        port: i32,
    },
    CustomReportDetails {
        details: Vec<(String, String)>,
    },
}

impl Common {
    fn read_cookie_request(r: &mut Reader<'_>) -> Result<Common, WireError> {
        Ok(Common::CookieRequest {
            key: r.identifier()?,
        })
    }

    fn read_custom_payload(r: &mut Reader<'_>) -> Result<Common, WireError> {
        let channel = r.identifier()?;
        let data = r.rest();
        if data.len() > MAX_PAYLOAD {
            return Err(WireError::TooLong {
                len: data.len(),
                max: MAX_PAYLOAD,
            });
        }
        Ok(Common::CustomPayload {
            channel,
            data: data.to_vec(),
        })
    }

    fn read_pack_push(r: &mut Reader<'_>) -> Result<Common, WireError> {
        Ok(Common::ResourcePackPush(PackPush {
            id: r.uuid()?,
            url: r.string(MAX_STRING)?.to_owned(),
            hash: r.string(40)?.to_owned(),
            required: r.bool()?,
            prompt: optional(r, text)?,
        }))
    }

    fn read_store_cookie(r: &mut Reader<'_>) -> Result<Common, WireError> {
        Ok(Common::StoreCookie {
            key: r.identifier()?,
            payload: r.byte_array(MAX_COOKIE)?.to_vec(),
        })
    }

    fn read_transfer(r: &mut Reader<'_>) -> Result<Common, WireError> {
        Ok(Common::Transfer {
            host: r.string(MAX_STRING)?.to_owned(),
            port: r.varint()?,
        })
    }

    fn read_report_details(r: &mut Reader<'_>) -> Result<Common, WireError> {
        let n = r.count(32)?;
        let mut details = Vec::with_capacity(n);
        for _ in 0..n {
            details.push((r.string(128)?.to_owned(), r.string(4096)?.to_owned()));
        }
        Ok(Common::CustomReportDetails { details })
    }
}

/// The brand in a `minecraft:brand` payload (`BrandPayload`).
pub fn read_brand(data: &[u8]) -> Result<String, WireError> {
    let mut r = Reader::new(data);
    let s = r.string(MAX_STRING)?.to_owned();
    r.finish()?;
    Ok(s)
}

/// Login: the handshake's `intention` is here too, as it opens the login.
pub mod login {
    use super::*;

    /// The handshake's intent (`ClientIntent`).
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum Intent {
        Status = 1,
        Login = 2,
        Transfer = 3,
    }

    /// The server's encryption request (`ClientboundHelloPacket`).
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct Hello {
        pub server_id: String,
        /// X.509 SubjectPublicKeyInfo, DER.
        pub public_key: Vec<u8>,
        pub challenge: Vec<u8>,
        pub should_authenticate: bool,
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    pub enum Clientbound {
        /// The reason as JSON text (lenient, as vanilla reads it).
        Disconnect {
            reason: String,
        },
        Hello(Hello),
        Finished {
            profile: GameProfile,
            session_id: u128,
        },
        Compression {
            threshold: i32,
        },
        CustomQuery {
            transaction_id: i32,
            channel: String,
        },
        CookieRequest {
            key: String,
        },
    }

    impl Clientbound {
        pub fn decode(id: u32, body: &[u8]) -> Result<Clientbound, WireError> {
            let mut r = Reader::new(body);
            let p = match lg::Clientbound::from_id(id).ok_or(WireError::Invalid("packet id"))? {
                lg::Clientbound::LoginDisconnect => Clientbound::Disconnect {
                    reason: r.string(MAX_LOGIN_REASON)?.to_owned(),
                },
                lg::Clientbound::Hello => {
                    let server_id = r.string(20)?.to_owned();
                    let public_key = r.byte_array(usize::MAX)?.to_vec();
                    let challenge = r.byte_array(usize::MAX)?.to_vec();
                    Clientbound::Hello(Hello {
                        server_id,
                        public_key,
                        challenge,
                        should_authenticate: r.bool()?,
                    })
                }
                lg::Clientbound::LoginFinished => Clientbound::Finished {
                    profile: GameProfile::read(&mut r)?,
                    session_id: r.uuid()?,
                },
                lg::Clientbound::LoginCompression => Clientbound::Compression {
                    threshold: r.varint()?,
                },
                lg::Clientbound::CustomQuery => {
                    let transaction_id = r.varint()?;
                    let channel = r.identifier()?;
                    let rest = r.rest().len();
                    if rest > MAX_PAYLOAD {
                        return Err(WireError::TooLong {
                            len: rest,
                            max: MAX_PAYLOAD,
                        });
                    }
                    Clientbound::CustomQuery {
                        transaction_id,
                        channel,
                    }
                }
                lg::Clientbound::CookieRequest => Clientbound::CookieRequest {
                    key: r.identifier()?,
                },
            };
            r.finish()?;
            Ok(p)
        }
    }

    /// The handshake (`ClientIntentionPacket`): protocol, host and port, intent.
    pub fn intention(protocol: i32, host: &str, port: u16, intent: Intent) -> Vec<u8> {
        use ruthenium_data::v26_3::packets::handshake;
        wire::packet(handshake::Serverbound::Intention.id(), |o| {
            wire::write_varint(o, protocol);
            wire::write_string(o, host);
            wire::write_u16(o, port);
            wire::write_varint(o, intent as i32);
        })
    }

    pub fn hello(name: &str, profile_id: u128) -> Vec<u8> {
        wire::packet(lg::Serverbound::Hello.id(), |o| {
            wire::write_string(o, name);
            wire::write_uuid(o, profile_id);
        })
    }

    /// The shared secret and the challenge, each encrypted with the server's key.
    pub fn key(secret: &[u8], challenge: &[u8]) -> Vec<u8> {
        wire::packet(lg::Serverbound::Key.id(), |o| {
            wire::write_byte_array(o, secret);
            wire::write_byte_array(o, challenge);
        })
    }

    /// Vanilla answers every query with no payload.
    pub fn custom_query_answer(transaction_id: i32) -> Vec<u8> {
        wire::packet(lg::Serverbound::CustomQueryAnswer.id(), |o| {
            wire::write_varint(o, transaction_id);
            wire::write_bool(o, false);
        })
    }

    pub fn login_acknowledged() -> Vec<u8> {
        wire::packet(lg::Serverbound::LoginAcknowledged.id(), |_| {})
    }

    pub fn cookie_response(key: &str, payload: Option<&[u8]>) -> Vec<u8> {
        wire::packet(lg::Serverbound::CookieResponse.id(), |o| {
            write_cookie(o, key, payload)
        })
    }
}

fn write_cookie(o: &mut Vec<u8>, key: &str, payload: Option<&[u8]>) {
    wire::write_string(o, key);
    wire::write_bool(o, payload.is_some());
    if let Some(p) = payload {
        wire::write_byte_array(o, p);
    }
}

/// Configuration.
pub mod configuration {
    use super::*;

    /// A registry entry: its id and, unless a known pack has it, its data.
    #[derive(Debug, Clone, PartialEq)]
    pub struct RegistryEntry {
        pub id: String,
        pub data: Option<Tag>,
    }

    /// The tags of one registry: names and the network ids of their members.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct RegistryTags {
        pub registry: String,
        pub tags: Vec<(String, Vec<i32>)>,
    }

    #[derive(Debug, Clone, PartialEq)]
    pub enum Clientbound {
        Common(Common),
        Finish,
        ResetChat,
        RegistryData {
            registry: String,
            entries: Vec<RegistryEntry>,
        },
        PostEffects,
        UpdateEnabledFeatures {
            features: Vec<String>,
        },
        UpdateTags(Vec<RegistryTags>),
        SelectKnownPacks(Vec<KnownPack>),
        /// Not read yet: server links and dialogs.
        ServerLinks,
        ClearDialog,
        ShowDialog,
        CodeOfConduct(String),
    }

    /// The tags of every registry (`ClientboundUpdateTagsPacket`), shared with play.
    pub(crate) fn read_tags(r: &mut Reader<'_>) -> Result<Vec<RegistryTags>, WireError> {
        let n = r.count(usize::MAX)?;
        let mut out = Vec::with_capacity(n.min(256));
        for _ in 0..n {
            let registry = r.identifier()?;
            let m = r.count(usize::MAX)?;
            let mut tags = Vec::with_capacity(m.min(4096));
            for _ in 0..m {
                let name = r.identifier()?;
                let k = r.count(usize::MAX)?;
                let mut ids = Vec::with_capacity(k.min(4096));
                for _ in 0..k {
                    ids.push(r.varint()?);
                }
                tags.push((name, ids));
            }
            out.push(RegistryTags { registry, tags });
        }
        Ok(out)
    }

    impl Clientbound {
        pub fn decode(id: u32, body: &[u8]) -> Result<Clientbound, WireError> {
            use cfg::Clientbound as C;
            let mut r = Reader::new(body);
            let p = match C::from_id(id).ok_or(WireError::Invalid("packet id"))? {
                C::CookieRequest => Clientbound::Common(Common::read_cookie_request(&mut r)?),
                C::CustomPayload => Clientbound::Common(Common::read_custom_payload(&mut r)?),
                C::Disconnect => Clientbound::Common(Common::Disconnect {
                    reason: text(&mut r)?,
                }),
                C::FinishConfiguration => Clientbound::Finish,
                C::KeepAlive => Clientbound::Common(Common::KeepAlive { id: r.i64()? }),
                C::Ping => Clientbound::Common(Common::Ping { id: r.i32()? }),
                C::ResetChat => Clientbound::ResetChat,
                C::RegistryData => {
                    let registry = r.identifier()?;
                    let n = r.count(usize::MAX)?;
                    let mut entries = Vec::with_capacity(n.min(4096));
                    for _ in 0..n {
                        let id = r.identifier()?;
                        // `ByteBufCodecs.TAG` under `optional`: a present end tag is
                        // still "no data".
                        let data = optional(&mut r, |r| r.nbt())?.flatten();
                        entries.push(RegistryEntry { id, data });
                    }
                    Clientbound::RegistryData { registry, entries }
                }
                C::ResourcePackPop => Clientbound::Common(Common::ResourcePackPop {
                    id: optional(&mut r, |r| r.uuid())?,
                }),
                C::ResourcePackPush => Clientbound::Common(Common::read_pack_push(&mut r)?),
                C::PostEffects => {
                    r.rest();
                    Clientbound::PostEffects
                }
                C::StoreCookie => Clientbound::Common(Common::read_store_cookie(&mut r)?),
                C::Transfer => Clientbound::Common(Common::read_transfer(&mut r)?),
                C::UpdateEnabledFeatures => {
                    let n = r.count(usize::MAX)?;
                    let mut features = Vec::with_capacity(n.min(64));
                    for _ in 0..n {
                        features.push(r.identifier()?);
                    }
                    Clientbound::UpdateEnabledFeatures { features }
                }
                C::UpdateTags => Clientbound::UpdateTags(read_tags(&mut r)?),
                C::SelectKnownPacks => {
                    let n = r.count(usize::MAX)?;
                    let mut packs = Vec::with_capacity(n.min(64));
                    for _ in 0..n {
                        packs.push(KnownPack::read(&mut r)?);
                    }
                    Clientbound::SelectKnownPacks(packs)
                }
                C::CustomReportDetails => Clientbound::Common(Common::read_report_details(&mut r)?),
                C::ServerLinks => {
                    r.rest();
                    Clientbound::ServerLinks
                }
                C::ClearDialog => Clientbound::ClearDialog,
                C::ShowDialog => {
                    r.rest();
                    Clientbound::ShowDialog
                }
                C::CodeOfConduct => Clientbound::CodeOfConduct(r.string(MAX_STRING)?.to_owned()),
            };
            r.finish()?;
            Ok(p)
        }

        /// Whether the protocol switches after this packet (`Packet.isTerminal`).
        pub fn is_terminal(&self) -> bool {
            matches!(self, Clientbound::Finish)
        }
    }

    pub fn client_information(info: &ClientInformation) -> Vec<u8> {
        wire::packet(cfg::Serverbound::ClientInformation.id(), |o| info.write(o))
    }

    pub fn cookie_response(key: &str, payload: Option<&[u8]>) -> Vec<u8> {
        wire::packet(cfg::Serverbound::CookieResponse.id(), |o| {
            write_cookie(o, key, payload)
        })
    }

    /// The client's brand (`BrandPayload` on `minecraft:brand`).
    pub fn brand(brand: &str) -> Vec<u8> {
        wire::packet(cfg::Serverbound::CustomPayload.id(), |o| {
            wire::write_string(o, "minecraft:brand");
            wire::write_string(o, brand);
        })
    }

    pub fn finish_configuration() -> Vec<u8> {
        wire::packet(cfg::Serverbound::FinishConfiguration.id(), |_| {})
    }

    pub fn keep_alive(id: i64) -> Vec<u8> {
        wire::packet(cfg::Serverbound::KeepAlive.id(), |o| wire::write_i64(o, id))
    }

    pub fn pong(id: i32) -> Vec<u8> {
        wire::packet(cfg::Serverbound::Pong.id(), |o| wire::write_i32(o, id))
    }

    pub fn resource_pack(id: u128, action: PackAction) -> Vec<u8> {
        wire::packet(cfg::Serverbound::ResourcePack.id(), |o| {
            wire::write_uuid(o, id);
            wire::write_varint(o, action as i32);
        })
    }

    pub fn select_known_packs(packs: &[KnownPack]) -> Vec<u8> {
        wire::packet(cfg::Serverbound::SelectKnownPacks.id(), |o| {
            wire::write_varint(o, packs.len() as i32);
            for p in packs {
                wire::write_string(o, &p.namespace);
                wire::write_string(o, &p.id);
                wire::write_string(o, &p.version);
            }
        })
    }

    pub fn accept_code_of_conduct() -> Vec<u8> {
        wire::packet(cfg::Serverbound::AcceptCodeOfConduct.id(), |_| {})
    }
}

/// Play: the packets every phase shares, the protocol switch, and joining the world
/// (login, respawn, teleports, chunk batches, the tick rate) so far.
pub mod play {
    use super::*;

    /// Where and how the player spawns (`CommonPlayerSpawnInfo`).
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct SpawnInfo {
        /// Network id in the `dimension_type` registry.
        pub dimension_type: i32,
        pub dimension: String,
        pub seed: i64,
        pub game_type: i32,
        pub previous_game_type: Option<i32>,
        pub is_debug: bool,
        pub is_flat: bool,
        /// Dimension and packed block position.
        pub last_death: Option<(String, i64)>,
        pub portal_cooldown: i32,
        pub sea_level: i32,
    }

    impl SpawnInfo {
        fn read(r: &mut Reader<'_>) -> Result<SpawnInfo, WireError> {
            Ok(SpawnInfo {
                dimension_type: r.varint()?,
                dimension: r.identifier()?,
                seed: r.i64()?,
                game_type: r.varint()?,
                // `ByteBufCodecs.OPTIONAL_VAR_INT`: 0 is none, else the id plus one.
                previous_game_type: match r.varint()? {
                    0 => None,
                    n => Some(n - 1),
                },
                is_debug: r.bool()?,
                is_flat: r.bool()?,
                last_death: optional(r, |r| Ok((r.identifier()?, r.i64()?)))?,
                portal_cooldown: r.varint()?,
                sea_level: r.varint()?,
            })
        }
    }

    /// Game type ids (`GameType`).
    pub const SURVIVAL: i32 = 0;
    pub const CREATIVE: i32 = 1;
    pub const ADVENTURE: i32 = 2;
    pub const SPECTATOR: i32 = 3;

    /// `ClientboundPlayerAbilitiesPacket`'s flag bits (the serverbound packet carries
    /// only `ABILITY_FLYING`).
    pub const ABILITY_INVULNERABLE: u8 = 1;
    pub const ABILITY_FLYING: u8 = 1 << 1;
    pub const ABILITY_MAY_FLY: u8 = 1 << 2;
    pub const ABILITY_INSTABUILD: u8 = 1 << 3;

    /// `ClientboundUpdateMobEffectPacket`'s flag bits.
    pub const EFFECT_AMBIENT: u8 = 1;
    pub const EFFECT_VISIBLE: u8 = 1 << 1;
    pub const EFFECT_SHOW_ICON: u8 = 1 << 2;
    pub const EFFECT_BLEND: u8 = 1 << 3;

    /// Joining the world (`ClientboundLoginPacket`).
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct Login {
        pub player_id: i32,
        pub hardcore: bool,
        pub levels: Vec<String>,
        pub max_players: i32,
        pub chunk_radius: i32,
        pub simulation_distance: i32,
        pub reduced_debug_info: bool,
        pub show_death_screen: bool,
        pub limited_crafting: bool,
        pub spawn: SpawnInfo,
        pub online_mode: bool,
        pub enforces_secure_chat: bool,
    }

    /// A teleport (`ClientboundPlayerPositionPacket`): the values, each absolute or
    /// relative as `relatives` says (bit `i` for `Relative` ordinal `i`: x, y, z,
    /// y rotation, x rotation, delta x, y, z, rotate delta).
    #[derive(Debug, Clone, Copy, PartialEq)]
    pub struct PlayerPosition {
        pub id: i32,
        pub position: [f64; 3],
        pub delta: [f64; 3],
        pub y_rot: f32,
        pub x_rot: f32,
        pub relatives: u32,
    }

    /// An attribute's base and modifiers (`ClientboundUpdateAttributesPacket.AttributeSnapshot`).
    #[derive(Debug, Clone, PartialEq)]
    pub struct AttributeSnapshot {
        /// Network id in the attribute registry.
        pub attribute: i32,
        pub base: f64,
        /// (id, amount, operation).
        pub modifiers: Vec<(String, f64, u8)>,
    }

    /// `BlockPos.of`: x in the top 26 bits, z in the next 26, y in the low 12.
    pub fn unpack_block_pos(v: i64) -> [i32; 3] {
        [
            (v >> 38) as i32,
            (v << 52 >> 52) as i32,
            (v << 26 >> 38) as i32,
        ]
    }

    /// `BlockPos.asLong`, the inverse of [`unpack_block_pos`].
    pub fn pack_block_pos([x, y, z]: [i32; 3]) -> i64 {
        (x as i64 & 0x3FF_FFFF) << 38 | (z as i64 & 0x3FF_FFFF) << 12 | (y as i64 & 0xFFF)
    }

    /// A step of a [`PositionPath`]: a position reached `ticks` after the last one.
    #[derive(Debug, Clone, Copy, PartialEq)]
    pub struct PositionStep {
        pub position: [f64; 3],
        pub ticks: i32,
    }

    /// Where an entity moves (`PositionPath`): straight to a point, or through steps.
    /// A stepped path is never empty.
    #[derive(Debug, Clone, PartialEq)]
    pub enum PositionPath {
        Linear([f64; 3]),
        Stepped(Vec<PositionStep>),
    }

    impl PositionPath {
        pub fn end(&self) -> [f64; 3] {
            match self {
                PositionPath::Linear(p) => *p,
                PositionPath::Stepped(s) => s.last().map_or([0.0; 3], |s| s.position),
            }
        }

        fn read(r: &mut Reader<'_>) -> Result<PositionPath, WireError> {
            // `ByIdMap.continuous` with the zero fallback: any other id is linear.
            match r.varint()? {
                1 => {
                    let n = r.count(usize::MAX)?;
                    let mut steps = Vec::with_capacity(n.min(64));
                    for _ in 0..n {
                        steps.push(PositionStep {
                            position: vec3(r)?,
                            ticks: r.varint()?,
                        });
                    }
                    if steps.is_empty() {
                        return Err(WireError::Invalid("empty stepped path"));
                    }
                    Ok(PositionPath::Stepped(steps))
                }
                _ => Ok(PositionPath::Linear(vec3(r)?)),
            }
        }
    }

    /// A move relative to the entity's last sent position (`VecDelta`), in 1/4096ths of
    /// a block: one delta, or steps each with the ticks it takes.
    #[derive(Debug, Clone, PartialEq)]
    pub enum VecDelta {
        Linear([i16; 3]),
        Stepped(Vec<([i16; 3], i32)>),
    }

    impl VecDelta {
        fn read(r: &mut Reader<'_>, steps: i32) -> Result<VecDelta, WireError> {
            let delta = |r: &mut Reader<'_>| -> Result<[i16; 3], WireError> {
                Ok([r.u16()? as i16, r.u16()? as i16, r.u16()? as i16])
            };
            if steps <= 0 {
                return Ok(VecDelta::Linear(delta(r)?));
            }
            // Each step takes at least 7 bytes.
            if steps as usize > r.remaining() / 7 {
                return Err(WireError::Invalid("move steps"));
            }
            let mut out = Vec::with_capacity(steps as usize);
            for _ in 0..steps {
                let ticks = r.varint()?;
                out.push((delta(r)?, ticks));
            }
            Ok(VecDelta::Stepped(out))
        }
    }

    /// An entity's new position, motion and rotation (`PositionMoveRotation`), each
    /// absolute or relative as `relatives` says (the same bits as
    /// [`PlayerPosition`]).
    #[derive(Debug, Clone, Copy, PartialEq)]
    pub struct PositionChange {
        pub position: [f64; 3],
        pub delta: [f64; 3],
        pub y_rot: f32,
        pub x_rot: f32,
        pub relatives: u32,
    }

    /// A new entity (`ClientboundAddEntityPacket`).
    #[derive(Debug, Clone, PartialEq)]
    pub struct AddEntity {
        pub entity_id: i32,
        pub uuid: u128,
        /// Network id in the entity type registry.
        pub entity_type: i32,
        pub position: [f64; 3],
        pub movement: [f64; 3],
        /// Rotations as bytes, 256 steps a turn.
        pub x_rot: i8,
        pub y_rot: i8,
        pub y_head_rot: i8,
        pub data: i32,
    }

    /// A synched-data value (`SynchedEntityData.DataValue`), as far as the client reads
    /// them; registry holders and enums are their ids.
    #[derive(Debug, Clone, PartialEq)]
    pub enum DataValue {
        Byte(i8),
        Int(i32),
        Long(i64),
        Float(f32),
        String(String),
        /// A text component (skipped).
        Component,
        OptionalComponent(bool),
        Boolean(bool),
        Rotations([f32; 3]),
        BlockPos([i32; 3]),
        OptionalBlockPos(Option<[i32; 3]>),
        /// An entity's UUID.
        OptionalEntity(Option<u128>),
        /// A block state id; for the optional kind, 0 is none.
        BlockState(i32),
        /// `OptionalInt`: none, or the value.
        OptionalUnsignedInt(Option<i32>),
        /// `Pose` ordinal.
        Pose(i32),
        /// Any value that is one VarInt: a direction, a registry holder, an enum.
        Id(i32),
        /// Villager type, profession and level.
        VillagerData([i32; 3]),
        Vector3([f32; 3]),
        Quaternion([f32; 4]),
        OptionalGlobalPos(Option<(String, [i32; 3])>),
        /// Effect particles: how many (only the colour-only `entity_effect` kind is
        /// read).
        Particles(usize),
        /// An item stack (possibly empty): a dropped item, a firework, an item frame's
        /// item.
        Item(ItemStack),
    }

    /// The `minecraft:entity_effect` particle type, the one particle the client reads
    /// out of entity data (a colour, an int).
    const ENTITY_EFFECT_PARTICLE: i32 = 28;

    impl DataValue {
        /// The value for serializer `kind`, or `None` for a kind the client can't read
        /// yet (other particles, inline painting variants, profiles): everything after it
        /// is then lost.
        fn read(r: &mut Reader<'_>, kind: i32) -> Result<Option<DataValue>, WireError> {
            let block_pos = |r: &mut Reader<'_>| r.i64().map(unpack_block_pos);
            let v = match kind {
                0 => DataValue::Byte(r.u8()? as i8),
                1 => DataValue::Int(r.varint()?),
                2 => DataValue::Long(r.varlong()?),
                3 => DataValue::Float(r.f32()?),
                4 => DataValue::String(r.string(MAX_STRING)?.to_owned()),
                5 => {
                    text(r)?;
                    DataValue::Component
                }
                6 => {
                    let some = r.bool()?;
                    if some {
                        text(r)?;
                    }
                    DataValue::OptionalComponent(some)
                }
                7 => DataValue::Item(ItemStack::read_optional(r)?),
                8 => DataValue::Boolean(r.bool()?),
                9 => DataValue::Rotations([r.f32()?, r.f32()?, r.f32()?]),
                10 => DataValue::BlockPos(block_pos(r)?),
                11 => DataValue::OptionalBlockPos(optional(r, block_pos)?),
                13 => DataValue::OptionalEntity(optional(r, |r| r.uuid())?),
                14 | 15 => DataValue::BlockState(r.varint()?),
                17 => {
                    let n = r.count(usize::MAX)?;
                    for _ in 0..n {
                        if r.varint()? != ENTITY_EFFECT_PARTICLE {
                            return Ok(None);
                        }
                        r.i32()?;
                    }
                    DataValue::Particles(n)
                }
                18 => DataValue::VillagerData([r.varint()?, r.varint()?, r.varint()?]),
                19 => DataValue::OptionalUnsignedInt(match r.varint()? {
                    0 => None,
                    n => Some(n - 1),
                }),
                20 => DataValue::Pose(r.varint()?),
                // A direction, the variants (registry ids), then the states, the arm and
                // the colour (enum ids).
                12 | 21..=32 | 35..=38 | 42 | 43 => DataValue::Id(r.varint()?),
                33 => DataValue::OptionalGlobalPos(optional(r, |r| {
                    Ok((r.identifier()?, block_pos(r)?))
                })?),
                34 => {
                    // A painting variant: 0 means the variant follows inline.
                    match r.varint()? {
                        0 => return Ok(None),
                        n => DataValue::Id(n - 1),
                    }
                }
                39 => DataValue::Vector3([r.f32()?, r.f32()?, r.f32()?]),
                40 => DataValue::Quaternion([r.f32()?, r.f32()?, r.f32()?, r.f32()?]),
                // Particles (16) and profiles (41).
                _ => return Ok(None),
            };
            Ok(Some(v))
        }
    }

    /// A team's settings (`ClientboundSetPlayerTeamPacket.Parameters`); the texts are
    /// skipped.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct TeamParameters {
        pub name_tag_visibility: i32,
        /// `Team.CollisionRule` id: 0 always, 1 never, 2 push other teams, 3 push own
        /// team.
        pub collision_rule: i32,
        pub color: Option<i32>,
        pub options: u8,
    }

    /// One player's part of a `ClientboundPlayerInfoUpdatePacket`: what the actions
    /// carry that the client keeps so far.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct PlayerInfoEntry {
        pub uuid: u128,
        /// With `ADD_PLAYER`: the profile's name.
        pub name: Option<String>,
        /// With `UPDATE_GAME_MODE`.
        pub game_mode: Option<i32>,
    }

    /// `ClientboundPlayerInfoUpdatePacket.Action` bits.
    pub const PLAYER_INFO_ADD: u8 = 1;
    pub const PLAYER_INFO_GAME_MODE: u8 = 1 << 2;

    fn vec3(r: &mut Reader<'_>) -> Result<[f64; 3], WireError> {
        Ok([r.f64()?, r.f64()?, r.f64()?])
    }

    /// One entry of a player info update: the fields of each action in the set, in the
    /// actions' order.
    fn read_player_info(r: &mut Reader<'_>, actions: u8) -> Result<PlayerInfoEntry, WireError> {
        let mut e = PlayerInfoEntry {
            uuid: r.uuid()?,
            name: None,
            game_mode: None,
        };
        let has = |bit: u8| actions & 1 << bit != 0;
        if has(0) {
            e.name = Some(r.string(MAX_NAME)?.to_owned());
            let n = r.count(16)?;
            for _ in 0..n {
                r.string(64)?;
                r.string(MAX_STRING)?;
                optional(r, |r| r.string(1024).map(|_| ()))?;
            }
        }
        if has(1) {
            // The chat session: its id, the key's expiry, the key and its signature.
            optional(r, |r| {
                r.uuid()?;
                r.i64()?;
                r.byte_array(512)?;
                r.byte_array(4096)?;
                Ok(())
            })?;
        }
        if has(2) {
            e.game_mode = Some(r.varint()?);
        }
        if has(3) {
            r.bool()?;
        }
        if has(4) {
            r.varint()?;
        }
        if has(5) {
            optional(r, |r| text(r).map(|_| ()))?;
        }
        if has(6) {
            r.varint()?;
        }
        if has(7) {
            r.bool()?;
        }
        Ok(e)
    }

    #[derive(Debug, Clone, PartialEq)]
    pub enum Clientbound {
        Common(Common),
        StartConfiguration,
        Login(Box<Login>),
        Respawn {
            spawn: Box<SpawnInfo>,
            data_to_keep: u8,
        },
        PlayerPosition(PlayerPosition),
        /// `ClientboundGameEventPacket`: the event id and its parameter.
        GameEvent {
            event: u8,
            param: f32,
        },
        ChunkBatchStart,
        ChunkBatchFinished {
            batch_size: i32,
        },
        /// A chunk: its position and its sections' data (`ClientboundLevelChunkPacketData`'s
        /// buffer); heightmaps, block entities and light are skipped.
        LevelChunk {
            x: i32,
            z: i32,
            data: Vec<u8>,
        },
        BlockUpdate {
            pos: [i32; 3],
            state: u32,
        },
        /// `ClientboundChunksBiomesPacket`: chunks' new biomes, each as its sections'
        /// biome containers in order (`fillbiome`).
        ChunksBiomes {
            chunks: Vec<(i32, i32, Vec<u8>)>,
        },
        /// A block's event (`ClientboundBlockEventPacket`): a piston moving, a chest or
        /// shulker box opening, a note block playing. `block` is the block registry id
        /// the server sent it for (the client runs it for whatever block is there).
        BlockEvent {
            pos: [i32; 3],
            b0: u8,
            b1: u8,
            block: u32,
        },
        /// The server has handled the client's predicted block changes up to this
        /// sequence.
        BlockChangedAck {
            sequence: i32,
        },
        /// Changes in one section: the section and each (local position, state); local
        /// positions are `x << 8 | z << 4 | y`.
        SectionBlocksUpdate {
            section: [i32; 3],
            changes: Vec<(u16, u32)>,
        },
        UpdateTags(Vec<configuration::RegistryTags>),
        UpdateAttributes {
            entity_id: i32,
            attributes: Vec<AttributeSnapshot>,
        },
        ForgetLevelChunk {
            x: i32,
            z: i32,
        },
        SetHeldSlot {
            slot: i32,
        },
        /// The recipe property sets (`campfire_input`, `furnace_input`...): each set's
        /// key and item ids. The stonecutter's recipes after them aren't read.
        UpdateRecipes {
            item_sets: Vec<(String, Vec<i32>)>,
        },
        TickingState {
            tick_rate: f32,
            frozen: bool,
        },
        /// An entity's new velocity (knockback, for the local player).
        SetEntityMotion {
            entity_id: i32,
            movement: [f64; 3],
        },
        /// An explosion: only the push it gives the local player is read.
        Explode {
            player_knockback: Option<[f64; 3]>,
        },
        AddEntity(Box<AddEntity>),
        RemoveEntities {
            ids: Vec<i32>,
        },
        /// `ClientboundMoveEntityPacket`: a move from the last sent position, a new
        /// rotation (bytes), or both.
        MoveEntity {
            entity_id: i32,
            delta: Option<VecDelta>,
            rotation: Option<(i8, i8)>,
            on_ground: bool,
        },
        /// `ClientboundTeleportEntityPacket`.
        TeleportEntity {
            entity_id: i32,
            change: PositionChange,
            on_ground: bool,
        },
        /// `ClientboundEntityPositionSyncPacket`: the exact position, every so often.
        EntityPositionSync {
            entity_id: i32,
            path: PositionPath,
            y_rot: f32,
            x_rot: f32,
            on_ground: bool,
        },
        /// `ClientboundSetEntityDataPacket`: (field id, value) in the server's order;
        /// `complete` is false when a value couldn't be read and the rest was skipped.
        SetEntityData {
            entity_id: i32,
            values: Vec<(u8, DataValue)>,
            complete: bool,
        },
        SetPassengers {
            vehicle: i32,
            passengers: Vec<i32>,
        },
        /// `ClientboundSetPlayerTeamPacket`: method 0 adds the team, 1 removes it, 2
        /// changes it, 3 adds players (names, or UUIDs for other entities), 4 removes
        /// them.
        SetPlayerTeam {
            name: String,
            method: u8,
            parameters: Option<TeamParameters>,
            players: Vec<String>,
        },
        /// `ClientboundPlayerInfoUpdatePacket`: the action bits and the entries.
        PlayerInfoUpdate {
            actions: u8,
            entries: Vec<PlayerInfoEntry>,
        },
        PlayerInfoRemove {
            ids: Vec<u128>,
        },
        SetHealth {
            health: f32,
            food: i32,
            saturation: f32,
        },
        /// `ClientboundPlayerCombatKillPacket`: a player died (the local player, for
        /// this client), with the death message.
        PlayerCombatKill {
            player_id: i32,
            message: Tag,
        },
        /// `ClientboundPlayerAbilitiesPacket`: the flag bits (`ABILITY_*`) and the
        /// speeds.
        PlayerAbilities {
            flags: u8,
            flying_speed: f32,
            walking_speed: f32,
        },
        /// `ClientboundUpdateMobEffectPacket`: an effect (`minecraft:mob_effect` id)
        /// added to an entity or changed; the flag bits are `EFFECT_*`. A duration of
        /// -1 is infinite.
        UpdateMobEffect {
            entity_id: i32,
            effect: i32,
            amplifier: i32,
            duration: i32,
            flags: u8,
        },
        /// `ClientboundRemoveMobEffectPacket`.
        RemoveMobEffect {
            entity_id: i32,
            effect: i32,
        },
        /// `ClientboundContainerSetContentPacket`: every slot of a container (0 is the
        /// player's inventory menu) and the stack on the cursor.
        ContainerSetContent {
            container_id: i32,
            state_id: i32,
            items: Vec<ItemStack>,
            carried: ItemStack,
        },
        /// `ClientboundContainerSetSlotPacket`: one slot of a container.
        ContainerSetSlot {
            container_id: i32,
            state_id: i32,
            slot: i16,
            item: ItemStack,
        },
        /// `ClientboundSetPlayerInventoryPacket`: one slot of the player's inventory
        /// (`Inventory` numbering), whatever menu is open.
        SetPlayerInventory {
            slot: i32,
            item: ItemStack,
        },
        /// `ClientboundSetCursorItemPacket`.
        SetCursorItem {
            item: ItemStack,
        },
        /// `ClientboundCooldownPacket`: a cooldown group (an item id, or a
        /// `use_cooldown` group) and its length in ticks; 0 ends it.
        Cooldown {
            group: String,
            duration: i32,
        },
        /// `ClientboundEntityEventPacket`: an `EntityEvent` id for an entity.
        EntityEvent {
            entity_id: i32,
            event: u8,
        },
        /// `ClientboundMoveVehiclePacket`: where the server has the vehicle the player
        /// controls.
        MoveVehicle {
            position: [f64; 3],
            y_rot: f32,
            x_rot: f32,
        },
        /// `ClientboundSetEquipmentPacket`: an entity's equipment, by
        /// `EquipmentSlot` ordinal (`EQUIPMENT_*`).
        SetEquipment {
            entity_id: i32,
            slots: Vec<(u8, ItemStack)>,
        },
        /// `ClientboundSetTimePacket`: the level's game time (the clocks' updates that
        /// follow aren't read).
        SetTime {
            game_time: i64,
        },
        /// `ClientboundInitializeBorderPacket`: the whole world border.
        InitializeBorder {
            center: [f64; 2],
            old_size: f64,
            new_size: f64,
            lerp_time: i64,
            absolute_max_size: i32,
            warning_blocks: i32,
            warning_time: i32,
        },
        /// `ClientboundSetBorderCenterPacket`.
        SetBorderCenter {
            center: [f64; 2],
        },
        /// `ClientboundSetBorderLerpSizePacket`: from one size to another over
        /// `lerp_time` ticks.
        SetBorderLerpSize {
            old_size: f64,
            new_size: f64,
            lerp_time: i64,
        },
        /// `ClientboundSetBorderSizePacket`.
        SetBorderSize {
            size: f64,
        },
        /// `ClientboundSetBorderWarningDelayPacket`.
        SetBorderWarningDelay {
            warning_time: i32,
        },
        /// `ClientboundSetBorderWarningDistancePacket`.
        SetBorderWarningDistance {
            warning_blocks: i32,
        },
        /// Any other packet, not read yet.
        Other(pl::Clientbound),
    }

    impl Clientbound {
        pub fn decode(id: u32, body: &[u8]) -> Result<Clientbound, WireError> {
            use pl::Clientbound as C;
            let mut r = Reader::new(body);
            let p = match C::from_id(id).ok_or(WireError::Invalid("packet id"))? {
                C::CookieRequest => Clientbound::Common(Common::read_cookie_request(&mut r)?),
                C::CustomPayload => Clientbound::Common(Common::read_custom_payload(&mut r)?),
                C::Disconnect => Clientbound::Common(Common::Disconnect {
                    reason: text(&mut r)?,
                }),
                C::KeepAlive => Clientbound::Common(Common::KeepAlive { id: r.i64()? }),
                C::Ping => Clientbound::Common(Common::Ping { id: r.i32()? }),
                C::ResourcePackPop => Clientbound::Common(Common::ResourcePackPop {
                    id: optional(&mut r, |r| r.uuid())?,
                }),
                C::ResourcePackPush => Clientbound::Common(Common::read_pack_push(&mut r)?),
                C::StoreCookie => Clientbound::Common(Common::read_store_cookie(&mut r)?),
                C::Transfer => Clientbound::Common(Common::read_transfer(&mut r)?),
                C::CustomReportDetails => Clientbound::Common(Common::read_report_details(&mut r)?),
                C::StartConfiguration => Clientbound::StartConfiguration,
                C::Login => {
                    let player_id = r.i32()?;
                    let hardcore = r.bool()?;
                    let n = r.count(usize::MAX)?;
                    let mut levels = Vec::with_capacity(n.min(64));
                    for _ in 0..n {
                        levels.push(r.identifier()?);
                    }
                    Clientbound::Login(Box::new(Login {
                        player_id,
                        hardcore,
                        levels,
                        max_players: r.varint()?,
                        chunk_radius: r.varint()?,
                        simulation_distance: r.varint()?,
                        reduced_debug_info: r.bool()?,
                        show_death_screen: r.bool()?,
                        limited_crafting: r.bool()?,
                        spawn: SpawnInfo::read(&mut r)?,
                        online_mode: r.bool()?,
                        enforces_secure_chat: r.bool()?,
                    }))
                }
                C::Respawn => Clientbound::Respawn {
                    spawn: Box::new(SpawnInfo::read(&mut r)?),
                    data_to_keep: r.u8()?,
                },
                C::PlayerPosition => Clientbound::PlayerPosition(PlayerPosition {
                    id: r.varint()?,
                    position: [r.f64()?, r.f64()?, r.f64()?],
                    delta: [r.f64()?, r.f64()?, r.f64()?],
                    y_rot: r.f32()?,
                    x_rot: r.f32()?,
                    relatives: r.i32()? as u32,
                }),
                C::GameEvent => Clientbound::GameEvent {
                    event: r.u8()?,
                    param: r.f32()?,
                },
                C::ChunkBatchStart => Clientbound::ChunkBatchStart,
                C::ChunkBatchFinished => Clientbound::ChunkBatchFinished {
                    batch_size: r.varint()?,
                },
                C::LevelChunkWithLight => {
                    let (x, z) = (r.i32()?, r.i32()?);
                    // Heightmaps: (type, long array) pairs.
                    let n = r.count(16)?;
                    for _ in 0..n {
                        r.varint()?;
                        let longs = r.count(4096)?;
                        r.bytes(longs * 8)?;
                    }
                    let data = r.byte_array(2 << 20)?.to_vec();
                    r.rest();
                    Clientbound::LevelChunk { x, z, data }
                }
                C::ChunksBiomes => {
                    let n = r.count(1 << 16)?;
                    let mut chunks = Vec::with_capacity(n.min(1024));
                    for _ in 0..n {
                        // `ChunkPos.pack`: x in the low half, z in the high.
                        let packed = r.i64()?;
                        let data = r.byte_array(2 << 20)?.to_vec();
                        chunks.push((packed as i32, (packed >> 32) as i32, data));
                    }
                    Clientbound::ChunksBiomes { chunks }
                }
                C::BlockChangedAck => Clientbound::BlockChangedAck {
                    sequence: r.varint()?,
                },
                C::BlockUpdate => Clientbound::BlockUpdate {
                    pos: unpack_block_pos(r.i64()?),
                    state: r.varint()? as u32,
                },
                C::BlockEvent => Clientbound::BlockEvent {
                    pos: unpack_block_pos(r.i64()?),
                    b0: r.u8()?,
                    b1: r.u8()?,
                    block: r.varint()? as u32,
                },
                C::SectionBlocksUpdate => {
                    let s = r.i64()?;
                    let section = [
                        (s >> 42) as i32,
                        (s << 44 >> 44) as i32,
                        (s << 22 >> 42) as i32,
                    ];
                    let n = r.count(4096)?;
                    let mut changes = Vec::with_capacity(n);
                    for _ in 0..n {
                        let v = r.varlong()?;
                        changes.push(((v & 4095) as u16, (v as u64 >> 12) as u32));
                    }
                    Clientbound::SectionBlocksUpdate { section, changes }
                }
                C::UpdateTags => Clientbound::UpdateTags(configuration::read_tags(&mut r)?),
                C::UpdateAttributes => {
                    let entity_id = r.varint()?;
                    let n = r.count(128)?;
                    let mut attributes = Vec::with_capacity(n);
                    for _ in 0..n {
                        let attribute = r.varint()?;
                        let base = r.f64()?;
                        let m = r.count(usize::MAX)?;
                        let mut modifiers = Vec::with_capacity(m.min(64));
                        for _ in 0..m {
                            modifiers.push((r.identifier()?, r.f64()?, r.varint()? as u8));
                        }
                        attributes.push(AttributeSnapshot {
                            attribute,
                            base,
                            modifiers,
                        });
                    }
                    Clientbound::UpdateAttributes {
                        entity_id,
                        attributes,
                    }
                }
                C::ForgetLevelChunk => {
                    // `ChunkPos` as one long: x in the low half, z in the high half.
                    let packed = r.i64()?;
                    Clientbound::ForgetLevelChunk {
                        x: packed as i32,
                        z: (packed >> 32) as i32,
                    }
                }
                C::SetHeldSlot => Clientbound::SetHeldSlot { slot: r.varint()? },
                C::UpdateRecipes => {
                    let n = r.count(usize::MAX)?;
                    let mut item_sets = Vec::with_capacity(n.min(64));
                    for _ in 0..n {
                        let key = r.identifier()?;
                        let m = r.count(usize::MAX)?;
                        let mut items = Vec::with_capacity(m.min(4096));
                        for _ in 0..m {
                            items.push(r.varint()?);
                        }
                        item_sets.push((key, items));
                    }
                    r.rest();
                    Clientbound::UpdateRecipes { item_sets }
                }
                C::TickingState => Clientbound::TickingState {
                    tick_rate: r.f32()?,
                    frozen: r.bool()?,
                },
                C::SetEntityMotion => Clientbound::SetEntityMotion {
                    entity_id: r.varint()?,
                    movement: r.lp_vec3()?,
                },
                C::Explode => {
                    // Centre, radius, block count, then the player's knockback; the
                    // particles and sound after it aren't read.
                    for _ in 0..3 {
                        r.f64()?;
                    }
                    r.f32()?;
                    r.i32()?;
                    let player_knockback = if r.bool()? {
                        Some([r.f64()?, r.f64()?, r.f64()?])
                    } else {
                        None
                    };
                    r.rest();
                    Clientbound::Explode { player_knockback }
                }
                C::AddEntity => Clientbound::AddEntity(Box::new(AddEntity {
                    entity_id: r.varint()?,
                    uuid: r.uuid()?,
                    entity_type: r.varint()?,
                    position: vec3(&mut r)?,
                    movement: r.lp_vec3()?,
                    x_rot: r.u8()? as i8,
                    y_rot: r.u8()? as i8,
                    y_head_rot: r.u8()? as i8,
                    data: r.varint()?,
                })),
                C::RemoveEntities => {
                    let n = r.count(usize::MAX)?;
                    let mut ids = Vec::with_capacity(n.min(1024));
                    for _ in 0..n {
                        ids.push(r.varint()?);
                    }
                    Clientbound::RemoveEntities { ids }
                }
                kind @ (C::MoveEntityPos | C::MoveEntityPosRot) => {
                    let entity_id = r.varint()?;
                    // On the ground in bit 0, the number of steps above it.
                    let properties = r.varint()?;
                    let delta = VecDelta::read(&mut r, (properties as u32 >> 1) as i32)?;
                    let rotation = if matches!(kind, C::MoveEntityPosRot) {
                        Some((r.u8()? as i8, r.u8()? as i8))
                    } else {
                        None
                    };
                    Clientbound::MoveEntity {
                        entity_id,
                        delta: Some(delta),
                        rotation,
                        on_ground: properties & 1 != 0,
                    }
                }
                C::MoveEntityRot => {
                    let entity_id = r.varint()?;
                    let on_ground = r.bool()?;
                    Clientbound::MoveEntity {
                        entity_id,
                        delta: None,
                        rotation: Some((r.u8()? as i8, r.u8()? as i8)),
                        on_ground,
                    }
                }
                C::TeleportEntity => Clientbound::TeleportEntity {
                    entity_id: r.varint()?,
                    change: PositionChange {
                        position: vec3(&mut r)?,
                        delta: vec3(&mut r)?,
                        y_rot: r.f32()?,
                        x_rot: r.f32()?,
                        relatives: r.i32()? as u32,
                    },
                    on_ground: r.bool()?,
                },
                C::EntityPositionSync => Clientbound::EntityPositionSync {
                    entity_id: r.varint()?,
                    path: PositionPath::read(&mut r)?,
                    y_rot: r.f32()?,
                    x_rot: r.f32()?,
                    on_ground: r.bool()?,
                },
                C::SetEntityData => {
                    let entity_id = r.varint()?;
                    let mut values = Vec::new();
                    let mut complete = true;
                    loop {
                        let field = r.u8()?;
                        if field == 255 {
                            break;
                        }
                        let kind = r.varint()?;
                        match DataValue::read(&mut r, kind)? {
                            Some(v) => values.push((field, v)),
                            None => {
                                complete = false;
                                r.rest();
                                break;
                            }
                        }
                    }
                    Clientbound::SetEntityData {
                        entity_id,
                        values,
                        complete,
                    }
                }
                C::SetPassengers => {
                    let vehicle = r.varint()?;
                    let n = r.count(usize::MAX)?;
                    let mut passengers = Vec::with_capacity(n.min(64));
                    for _ in 0..n {
                        passengers.push(r.varint()?);
                    }
                    Clientbound::SetPassengers {
                        vehicle,
                        passengers,
                    }
                }
                C::SetPlayerTeam => {
                    let name = r.string(MAX_STRING)?.to_owned();
                    let method = r.u8()?;
                    let parameters = if method == 0 || method == 2 {
                        for _ in 0..3 {
                            text(&mut r)?;
                        }
                        Some(TeamParameters {
                            name_tag_visibility: r.varint()?,
                            collision_rule: r.varint()?,
                            color: optional(&mut r, |r| r.varint())?,
                            options: r.u8()?,
                        })
                    } else {
                        None
                    };
                    let mut players = Vec::new();
                    if matches!(method, 0 | 3 | 4) {
                        let n = r.count(usize::MAX)?;
                        for _ in 0..n {
                            players.push(r.string(MAX_STRING)?.to_owned());
                        }
                    }
                    Clientbound::SetPlayerTeam {
                        name,
                        method,
                        parameters,
                        players,
                    }
                }
                C::PlayerInfoUpdate => {
                    // An `EnumSet` of the eight actions: one byte.
                    let actions = r.u8()?;
                    let n = r.count(usize::MAX)?;
                    let mut entries = Vec::with_capacity(n.min(256));
                    for _ in 0..n {
                        entries.push(read_player_info(&mut r, actions)?);
                    }
                    Clientbound::PlayerInfoUpdate { actions, entries }
                }
                C::PlayerInfoRemove => {
                    let n = r.count(usize::MAX)?;
                    let mut ids = Vec::with_capacity(n.min(256));
                    for _ in 0..n {
                        ids.push(r.uuid()?);
                    }
                    Clientbound::PlayerInfoRemove { ids }
                }
                C::SetHealth => Clientbound::SetHealth {
                    health: r.f32()?,
                    food: r.varint()?,
                    saturation: r.f32()?,
                },
                C::PlayerCombatKill => Clientbound::PlayerCombatKill {
                    player_id: r.varint()?,
                    message: text(&mut r)?,
                },
                C::PlayerAbilities => Clientbound::PlayerAbilities {
                    flags: r.u8()?,
                    flying_speed: r.f32()?,
                    walking_speed: r.f32()?,
                },
                C::UpdateMobEffect => Clientbound::UpdateMobEffect {
                    entity_id: r.varint()?,
                    effect: r.varint()?,
                    amplifier: r.varint()?,
                    duration: r.varint()?,
                    flags: r.u8()?,
                },
                C::RemoveMobEffect => Clientbound::RemoveMobEffect {
                    entity_id: r.varint()?,
                    effect: r.varint()?,
                },
                C::ContainerSetContent => {
                    let container_id = r.varint()?;
                    let state_id = r.varint()?;
                    let n = r.count(usize::MAX)?;
                    let mut items = Vec::with_capacity(n.min(256));
                    for _ in 0..n {
                        items.push(ItemStack::read_optional(&mut r)?);
                    }
                    Clientbound::ContainerSetContent {
                        container_id,
                        state_id,
                        items,
                        carried: ItemStack::read_optional(&mut r)?,
                    }
                }
                C::ContainerSetSlot => Clientbound::ContainerSetSlot {
                    container_id: r.varint()?,
                    state_id: r.varint()?,
                    slot: r.u16()? as i16,
                    item: ItemStack::read_optional(&mut r)?,
                },
                C::SetPlayerInventory => Clientbound::SetPlayerInventory {
                    slot: r.varint()?,
                    item: ItemStack::read_optional(&mut r)?,
                },
                C::SetCursorItem => Clientbound::SetCursorItem {
                    item: ItemStack::read_optional(&mut r)?,
                },
                C::Cooldown => Clientbound::Cooldown {
                    group: r.identifier()?,
                    duration: r.varint()?,
                },
                C::EntityEvent => Clientbound::EntityEvent {
                    entity_id: r.i32()?,
                    event: r.u8()?,
                },
                C::MoveVehicle => Clientbound::MoveVehicle {
                    position: vec3(&mut r)?,
                    y_rot: r.f32()?,
                    x_rot: r.f32()?,
                },
                C::SetEquipment => {
                    let entity_id = r.varint()?;
                    let mut slots = Vec::new();
                    loop {
                        let slot = r.u8()?;
                        slots.push((slot & 127, ItemStack::read_optional(&mut r)?));
                        if slot & 128 == 0 {
                            break;
                        }
                    }
                    Clientbound::SetEquipment { entity_id, slots }
                }
                C::SetTime => {
                    let game_time = r.i64()?;
                    r.rest();
                    Clientbound::SetTime { game_time }
                }
                C::InitializeBorder => Clientbound::InitializeBorder {
                    center: [r.f64()?, r.f64()?],
                    old_size: r.f64()?,
                    new_size: r.f64()?,
                    lerp_time: r.varlong()?,
                    absolute_max_size: r.varint()?,
                    warning_blocks: r.varint()?,
                    warning_time: r.varint()?,
                },
                C::SetBorderCenter => Clientbound::SetBorderCenter {
                    center: [r.f64()?, r.f64()?],
                },
                C::SetBorderLerpSize => Clientbound::SetBorderLerpSize {
                    old_size: r.f64()?,
                    new_size: r.f64()?,
                    lerp_time: r.varlong()?,
                },
                C::SetBorderSize => Clientbound::SetBorderSize { size: r.f64()? },
                C::SetBorderWarningDelay => Clientbound::SetBorderWarningDelay {
                    warning_time: r.varint()?,
                },
                C::SetBorderWarningDistance => Clientbound::SetBorderWarningDistance {
                    warning_blocks: r.varint()?,
                },
                other => {
                    r.rest();
                    Clientbound::Other(other)
                }
            };
            r.finish()?;
            Ok(p)
        }

        pub fn is_terminal(&self) -> bool {
            matches!(self, Clientbound::StartConfiguration)
        }
    }

    pub fn keep_alive(id: i64) -> Vec<u8> {
        wire::packet(pl::Serverbound::KeepAlive.id(), |o| wire::write_i64(o, id))
    }

    pub fn pong(id: i32) -> Vec<u8> {
        wire::packet(pl::Serverbound::Pong.id(), |o| wire::write_i32(o, id))
    }

    /// The client's answer to a teleport: its id and where the player now is.
    pub fn accept_teleportation(id: i32, pos: [f64; 3], y_rot: f32, x_rot: f32) -> Vec<u8> {
        wire::packet(pl::Serverbound::AcceptTeleportation.id(), |o| {
            wire::write_varint(o, id);
            for v in pos {
                wire::write_f64(o, v);
            }
            wire::write_f32(o, y_rot);
            wire::write_f32(o, x_rot);
        })
    }

    pub fn chunk_batch_received(desired_chunks_per_tick: f32) -> Vec<u8> {
        wire::packet(pl::Serverbound::ChunkBatchReceived.id(), |o| {
            wire::write_f32(o, desired_chunks_per_tick)
        })
    }

    pub fn client_tick_end() -> Vec<u8> {
        wire::packet(pl::Serverbound::ClientTickEnd.id(), |_| {})
    }

    pub fn player_loaded() -> Vec<u8> {
        wire::packet(pl::Serverbound::PlayerLoaded.id(), |_| {})
    }

    /// `ServerboundClientCommandPacket.Action` ordinals.
    pub const CLIENT_COMMAND_PERFORM_RESPAWN: i32 = 0;
    pub const CLIENT_COMMAND_REQUEST_STATS: i32 = 1;
    pub const CLIENT_COMMAND_REQUEST_GAMERULE_VALUES: i32 = 2;

    /// `ServerboundClientCommandPacket`: the action as its ordinal.
    pub fn client_command(action: i32) -> Vec<u8> {
        wire::packet(pl::Serverbound::ClientCommand.id(), |o| {
            wire::write_varint(o, action)
        })
    }

    /// `ServerboundPlayerAbilitiesPacket`: only whether the player flies.
    pub fn player_abilities(flying: bool) -> Vec<u8> {
        wire::packet(pl::Serverbound::PlayerAbilities.id(), |o| {
            o.push(if flying { ABILITY_FLYING } else { 0 })
        })
    }

    pub fn set_carried_item(slot: i32) -> Vec<u8> {
        // `ServerboundSetCarriedItemPacket` writes the slot as a short.
        wire::packet(pl::Serverbound::SetCarriedItem.id(), |o| {
            o.extend_from_slice(&(slot as i16).to_be_bytes())
        })
    }

    pub fn configuration_acknowledged() -> Vec<u8> {
        wire::packet(pl::Serverbound::ConfigurationAcknowledged.id(), |_| {})
    }

    /// `InteractionHand` ids.
    pub const MAIN_HAND: i32 = 0;
    pub const OFF_HAND: i32 = 1;

    /// `Direction` ids (`get3DDataValue`).
    pub const DOWN: i32 = 0;
    pub const UP: i32 = 1;
    pub const NORTH: i32 = 2;
    pub const SOUTH: i32 = 3;
    pub const WEST: i32 = 4;
    pub const EAST: i32 = 5;

    /// `ServerboundPlayerActionPacket.Action` ordinals.
    pub const ACTION_START_DESTROY_BLOCK: i32 = 0;
    pub const ACTION_CHANGE_DESTROY_DIRECTION: i32 = 1;
    pub const ACTION_ABORT_DESTROY_BLOCK: i32 = 2;
    pub const ACTION_STOP_DESTROY_BLOCK: i32 = 3;
    pub const ACTION_DROP_ALL_ITEMS: i32 = 4;
    pub const ACTION_DROP_ITEM: i32 = 5;
    pub const ACTION_RELEASE_USE_ITEM: i32 = 6;
    pub const ACTION_SWAP_ITEM_WITH_OFFHAND: i32 = 7;
    pub const ACTION_STAB: i32 = 8;

    /// `ServerboundUseItemPacket`: using the item in `hand` (not on a block or an
    /// entity), with the player's rotation at the time.
    pub fn use_item(hand: i32, sequence: i32, y_rot: f32, x_rot: f32) -> Vec<u8> {
        wire::packet(pl::Serverbound::UseItem.id(), |o| {
            wire::write_varint(o, hand);
            wire::write_varint(o, sequence);
            wire::write_f32(o, y_rot);
            wire::write_f32(o, x_rot);
        })
    }

    /// `ServerboundUseItemOnPacket`: using the item in `hand` on a block's face;
    /// `cursor` is where the hit landed, relative to the block's corner.
    #[allow(clippy::too_many_arguments)]
    pub fn use_item_on(
        hand: i32,
        block_pos: [i32; 3],
        direction: i32,
        cursor: [f32; 3],
        inside: bool,
        world_border_hit: bool,
        sequence: i32,
    ) -> Vec<u8> {
        wire::packet(pl::Serverbound::UseItemOn.id(), |o| {
            wire::write_varint(o, hand);
            // `BlockHitResult.STREAM_CODEC`: the position, the face as a VarInt, the
            // cursor, inside, world border.
            wire::write_i64(o, pack_block_pos(block_pos));
            wire::write_varint(o, direction);
            for v in cursor {
                wire::write_f32(o, v);
            }
            wire::write_bool(o, inside);
            wire::write_bool(o, world_border_hit);
            wire::write_varint(o, sequence);
        })
    }

    /// `ServerboundPlayerActionPacket`: an `ACTION_*`, the block and face it concerns
    /// (zeros for the item actions), and the block-change sequence.
    pub fn player_action(
        action: i32,
        block_pos: [i32; 3],
        direction: i32,
        sequence: i32,
    ) -> Vec<u8> {
        wire::packet(pl::Serverbound::PlayerAction.id(), |o| {
            wire::write_varint(o, action);
            wire::write_i64(o, pack_block_pos(block_pos));
            // Unlike the use packet's, this face is one unsigned byte.
            o.push(direction as u8);
            wire::write_varint(o, sequence);
        })
    }

    /// `ServerboundPunchPacket`: the main-hand swing that ends each attack click, after
    /// the attack or digging packet, or alone on a miss (26.3 has no hand-choosing swing
    /// packet; the server swings the main hand).
    pub fn punch() -> Vec<u8> {
        wire::packet(pl::Serverbound::Punch.id(), |_| {})
    }

    /// `ServerboundSpectatorActionPacket`: a spectator's attack, on an entity to watch
    /// it (`Some(id)`) or on nothing (an optional varint: 0, or the id plus 1).
    pub fn spectator_action(entity_id: Option<i32>) -> Vec<u8> {
        wire::packet(pl::Serverbound::SpectatorAction.id(), |o| {
            wire::write_varint(o, entity_id.map_or(0, |id| id + 1));
        })
    }

    /// `ServerboundAttackPacket`: attacking an entity.
    pub fn attack(entity_id: i32) -> Vec<u8> {
        wire::packet(pl::Serverbound::Attack.id(), |o| {
            wire::write_varint(o, entity_id)
        })
    }

    /// `EquipmentSlot` ordinals.
    pub const EQUIPMENT_MAINHAND: u8 = 0;
    pub const EQUIPMENT_OFFHAND: u8 = 1;
    pub const EQUIPMENT_BODY: u8 = 6;
    pub const EQUIPMENT_SADDLE: u8 = 7;

    /// `ServerboundMoveVehiclePacket`: where the client has the vehicle it controls.
    pub fn move_vehicle(position: [f64; 3], y_rot: f32, x_rot: f32, on_ground: bool) -> Vec<u8> {
        wire::packet(pl::Serverbound::MoveVehicle.id(), |o| {
            for v in position {
                o.extend(v.to_be_bytes());
            }
            o.extend(y_rot.to_be_bytes());
            o.extend(x_rot.to_be_bytes());
            wire::write_bool(o, on_ground);
        })
    }

    /// `ServerboundPaddleBoatPacket`: which paddles the boat's rider works.
    pub fn paddle_boat(left: bool, right: bool) -> Vec<u8> {
        wire::packet(pl::Serverbound::PaddleBoat.id(), |o| {
            wire::write_bool(o, left);
            wire::write_bool(o, right);
        })
    }

    /// `ServerboundPlayerCommandPacket.Action` ordinals.
    pub const PLAYER_COMMAND_START_RIDING_JUMP: i32 = 3;
    pub const PLAYER_COMMAND_STOP_RIDING_JUMP: i32 = 4;
    pub const PLAYER_COMMAND_OPEN_INVENTORY: i32 = 5;

    /// `ServerboundPlayerCommandPacket`: an action of an entity (the player), with its
    /// data (a riding jump's strength).
    pub fn player_command(entity_id: i32, action: i32, data: i32) -> Vec<u8> {
        wire::packet(pl::Serverbound::PlayerCommand.id(), |o| {
            wire::write_varint(o, entity_id);
            wire::write_varint(o, action);
            wire::write_varint(o, data);
        })
    }

    /// `ServerboundInteractPacket`: using `hand` on an entity; `location` is where the
    /// hit landed relative to the entity's position (sent as a low-precision vector),
    /// and `using_secondary_action` whether the player sneaks.
    pub fn interact(
        entity_id: i32,
        hand: i32,
        location: [f64; 3],
        using_secondary_action: bool,
    ) -> Vec<u8> {
        wire::packet(pl::Serverbound::Interact.id(), |o| {
            wire::write_varint(o, entity_id);
            wire::write_varint(o, hand);
            wire::write_lp_vec3(o, location);
            wire::write_bool(o, using_secondary_action);
        })
    }

    pub fn cookie_response(key: &str, payload: Option<&[u8]>) -> Vec<u8> {
        wire::packet(pl::Serverbound::CookieResponse.id(), |o| {
            write_cookie(o, key, payload)
        })
    }

    pub fn resource_pack(id: u128, action: PackAction) -> Vec<u8> {
        wire::packet(pl::Serverbound::ResourcePack.id(), |o| {
            wire::write_uuid(o, id);
            wire::write_varint(o, action as i32);
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn body(p: &[u8]) -> (u32, &[u8]) {
        let mut r = Reader::new(p);
        let id = r.varint().unwrap() as u32;
        (id, r.rest())
    }

    #[test]
    fn login_finished_reads_a_profile() {
        let mut b = Vec::new();
        wire::write_uuid(&mut b, 7);
        wire::write_string(&mut b, "Steve");
        wire::write_varint(&mut b, 1);
        wire::write_string(&mut b, "textures");
        wire::write_string(&mut b, "e30=");
        wire::write_bool(&mut b, true);
        wire::write_string(&mut b, "sig");
        wire::write_uuid(&mut b, 9);
        let p = login::Clientbound::decode(2, &b).unwrap();
        let login::Clientbound::Finished {
            profile,
            session_id,
        } = p
        else {
            panic!("{p:?}")
        };
        assert_eq!(
            (profile.id, profile.name.as_str(), session_id),
            (7, "Steve", 9)
        );
        assert_eq!(profile.properties[0].signature.as_deref(), Some("sig"));
        // A byte too many is refused.
        b.push(0);
        assert_eq!(
            login::Clientbound::decode(2, &b),
            Err(WireError::TrailingBytes(1))
        );
    }

    #[test]
    fn serverbound_layouts() {
        let hello = login::hello("Dev", 0x0102);
        assert_eq!(hello[0], 0);
        assert_eq!(&hello[1..5], [3, b'D', b'e', b'v']);
        assert_eq!(hello.len(), 5 + 16);
        let info = configuration::client_information(&ClientInformation::default());
        assert_eq!(
            info,
            [
                0, 5, b'e', b'n', b'_', b'u', b's', 12, 0, 1, 0x7F, 1, 0, 1, 0
            ]
        );
        let q = login::custom_query_answer(300);
        assert_eq!(q, [2, 0xAC, 0x02, 0]);
        let brand = configuration::brand("ruthenium");
        let (id, rest) = body(&brand);
        assert_eq!(id, 2);
        let mut r = Reader::new(rest);
        assert_eq!(r.identifier().unwrap(), "minecraft:brand");
        assert_eq!(read_brand(r.rest()).unwrap(), "ruthenium");
        let packs = configuration::select_known_packs(&[KnownPack::vanilla("core", "26.3")]);
        assert_eq!(packs[..3], [7, 1, 9]);
    }

    #[test]
    fn registry_data_with_and_without_entries() {
        let mut b = Vec::new();
        wire::write_string(&mut b, "minecraft:dimension_type");
        wire::write_varint(&mut b, 2);
        wire::write_string(&mut b, "overworld");
        wire::write_bool(&mut b, false);
        wire::write_string(&mut b, "minecraft:the_end");
        wire::write_bool(&mut b, true);
        crate::nbt::write_network(&mut b, &Tag::Compound(crate::nbt::Compound::new())).unwrap();
        let p = configuration::Clientbound::decode(7, &b).unwrap();
        let configuration::Clientbound::RegistryData { registry, entries } = p else {
            panic!("{p:?}")
        };
        assert_eq!(registry, "minecraft:dimension_type");
        assert_eq!(entries[0].id, "minecraft:overworld");
        assert!(entries[0].data.is_none() && entries[1].data.is_some());
    }

    #[test]
    fn tags_and_known_packs() {
        let mut b = Vec::new();
        wire::write_varint(&mut b, 1);
        wire::write_string(&mut b, "minecraft:block");
        wire::write_varint(&mut b, 1);
        wire::write_string(&mut b, "minecraft:climbable");
        wire::write_varint(&mut b, 2);
        wire::write_varint(&mut b, 200);
        wire::write_varint(&mut b, 5);
        let p = configuration::Clientbound::decode(14, &b).unwrap();
        assert_eq!(
            p,
            configuration::Clientbound::UpdateTags(vec![configuration::RegistryTags {
                registry: "minecraft:block".into(),
                tags: vec![("minecraft:climbable".into(), vec![200, 5])],
            }])
        );
        let mut b = Vec::new();
        wire::write_varint(&mut b, 1);
        for s in ["minecraft", "core", "26.3"] {
            wire::write_string(&mut b, s);
        }
        assert_eq!(
            configuration::Clientbound::decode(15, &b).unwrap(),
            configuration::Clientbound::SelectKnownPacks(vec![KnownPack::vanilla("core", "26.3")])
        );
    }

    #[test]
    fn play_reads_the_shared_packets_and_skips_the_rest() {
        let ka = 0x1122_3344_5566_7788_i64.to_be_bytes();
        assert_eq!(
            play::Clientbound::decode(45, &ka).unwrap(),
            play::Clientbound::Common(Common::KeepAlive {
                id: 0x1122_3344_5566_7788
            })
        );
        let p = play::Clientbound::decode(10, &[1, 2, 3]).unwrap();
        assert_eq!(
            p,
            play::Clientbound::Other(pl::Clientbound::ChangeDifficulty)
        );
        assert!(play::Clientbound::decode(120, &[]).unwrap().is_terminal());
        // A teleport: id 7, position, motion, rotation, relative x and y rotation.
        let mut b = Vec::new();
        wire::write_varint(&mut b, 7);
        for v in [1.5, -60.0, 9.5, 0.0, 0.0, 0.0] {
            wire::write_f64(&mut b, v);
        }
        wire::write_f32(&mut b, 90.0);
        wire::write_f32(&mut b, 0.0);
        wire::write_i32(&mut b, 0b1001);
        let p = play::Clientbound::decode(73, &b).unwrap();
        assert!(matches!(p, play::Clientbound::PlayerPosition(t)
            if t.id == 7 && t.position == [1.5, -60.0, 9.5] && t.relatives == 9));
        let a = play::accept_teleportation(7, [1.5, -60.0, 9.5], 90.0, 0.0);
        assert_eq!((a[0], a[1], a.len()), (0, 7, 2 + 24 + 8));
        assert_eq!(play::set_carried_item(3), [54, 0, 3]);
        // A chunk position is one long: x low, z high.
        let packed = ((-2i64) << 32) | (5i64 & 0xFFFF_FFFF);
        assert_eq!(
            play::Clientbound::decode(38, &packed.to_be_bytes()).unwrap(),
            play::Clientbound::ForgetLevelChunk { x: 5, z: -2 }
        );
        assert_eq!(play::keep_alive(1)[0], 28);
    }

    #[test]
    fn play_reads_abilities_and_effects() {
        let decode =
            |kind: pl::Clientbound, b: &[u8]| play::Clientbound::decode(kind.id(), b).unwrap();
        // Creative: invulnerable, may fly, instabuild; vanilla's speeds.
        let mut b = vec![0b1101];
        wire::write_f32(&mut b, 0.05);
        wire::write_f32(&mut b, 0.1);
        assert_eq!(
            decode(pl::Clientbound::PlayerAbilities, &b),
            play::Clientbound::PlayerAbilities {
                flags: play::ABILITY_INVULNERABLE
                    | play::ABILITY_MAY_FLY
                    | play::ABILITY_INSTABUILD,
                flying_speed: 0.05,
                walking_speed: 0.1,
            }
        );
        // Jump boost II on entity 300 for 200 ticks, visible with an icon.
        let mut b = Vec::new();
        wire::write_varint(&mut b, 300);
        wire::write_varint(&mut b, 7);
        wire::write_varint(&mut b, 1);
        wire::write_varint(&mut b, 200);
        b.push(play::EFFECT_VISIBLE | play::EFFECT_SHOW_ICON);
        assert_eq!(
            decode(pl::Clientbound::UpdateMobEffect, &b),
            play::Clientbound::UpdateMobEffect {
                entity_id: 300,
                effect: 7,
                amplifier: 1,
                duration: 200,
                flags: 6,
            }
        );
        assert_eq!(
            decode(pl::Clientbound::RemoveMobEffect, &[0x2a, 27]),
            play::Clientbound::RemoveMobEffect {
                entity_id: 42,
                effect: 27,
            }
        );
        let id = pl::Serverbound::PlayerAbilities.id() as u8;
        assert_eq!(play::player_abilities(true), [id, 2]);
        assert_eq!(play::player_abilities(false), [id, 0]);
    }

    #[test]
    fn play_reads_a_death_and_writes_the_respawn_request() {
        let mut b = Vec::new();
        wire::write_varint(&mut b, 300);
        crate::nbt::write_network(&mut b, &Tag::String("Steve fell".into())).unwrap();
        assert_eq!(
            play::Clientbound::decode(pl::Clientbound::PlayerCombatKill.id(), &b).unwrap(),
            play::Clientbound::PlayerCombatKill {
                player_id: 300,
                message: Tag::String("Steve fell".into()),
            }
        );
        let id = pl::Serverbound::ClientCommand.id() as u8;
        assert_eq!(
            play::client_command(play::CLIENT_COMMAND_PERFORM_RESPAWN),
            [id, 0]
        );
    }

    #[test]
    fn play_reads_entity_packets() {
        let decode =
            |kind: pl::Clientbound, b: &[u8]| play::Clientbound::decode(kind.id(), b).unwrap();
        // A zombie: id, UUID, type, position, no motion, rotations, data.
        let mut b = Vec::new();
        wire::write_varint(&mut b, 9);
        wire::write_uuid(&mut b, 0x1234);
        wire::write_varint(&mut b, 150);
        for v in [0.5, -60.0, 1.25] {
            wire::write_f64(&mut b, v);
        }
        wire::write_lp_vec3(&mut b, [0.0; 3]);
        b.extend([0, 64, 0x80]);
        wire::write_varint(&mut b, 0);
        let play::Clientbound::AddEntity(a) = decode(pl::Clientbound::AddEntity, &b) else {
            panic!()
        };
        assert_eq!((a.entity_id, a.uuid, a.entity_type), (9, 0x1234, 150));
        assert_eq!(
            (a.position, a.y_rot, a.y_head_rot),
            ([0.5, -60.0, 1.25], 64, -128)
        );

        // Two steps, on the ground: the step count sits above the ground bit.
        let mut b = Vec::new();
        wire::write_varint(&mut b, 9);
        wire::write_varint(&mut b, 2 << 1 | 1);
        for (ticks, d) in [(1, [100i16, 0, -5]), (2, [0, 0, 7])] {
            wire::write_varint(&mut b, ticks);
            for v in d {
                b.extend(v.to_be_bytes());
            }
        }
        assert_eq!(
            decode(pl::Clientbound::MoveEntityPos, &b),
            play::Clientbound::MoveEntity {
                entity_id: 9,
                delta: Some(play::VecDelta::Stepped(vec![
                    ([100, 0, -5], 1),
                    ([0, 0, 7], 2)
                ])),
                rotation: None,
                on_ground: true,
            }
        );
        assert_eq!(
            decode(pl::Clientbound::MoveEntityRot, &[9, 0, 32, 0xF0]),
            play::Clientbound::MoveEntity {
                entity_id: 9,
                delta: None,
                rotation: Some((32, -16)),
                on_ground: false,
            }
        );

        // Entity data: a pose, then a particle, which isn't read: the rest is skipped.
        let mut b = Vec::new();
        wire::write_varint(&mut b, 9);
        b.extend([6, 20, 5, 8, 16, 1, 2, 3, 255]);
        assert_eq!(
            decode(pl::Clientbound::SetEntityData, &b),
            play::Clientbound::SetEntityData {
                entity_id: 9,
                values: vec![(6, play::DataValue::Pose(5))],
                complete: false,
            }
        );
        let mut b = Vec::new();
        wire::write_varint(&mut b, 9);
        b.extend([16, 8, 1, 9, 3]);
        b.extend(20.0f32.to_be_bytes());
        b.push(255);
        let play::Clientbound::SetEntityData {
            values, complete, ..
        } = decode(pl::Clientbound::SetEntityData, &b)
        else {
            panic!()
        };
        assert!(complete);
        assert_eq!(
            values,
            [
                (16, play::DataValue::Boolean(true)),
                (9, play::DataValue::Float(20.0))
            ]
        );

        // A new team that never collides, with one member.
        let mut b = Vec::new();
        wire::write_string(&mut b, "nopush");
        b.push(0);
        for _ in 0..3 {
            crate::nbt::write_network(&mut b, &Tag::String(String::new())).unwrap();
        }
        wire::write_varint(&mut b, 0);
        wire::write_varint(&mut b, 1);
        wire::write_bool(&mut b, false);
        b.push(0);
        wire::write_varint(&mut b, 1);
        wire::write_string(&mut b, "Developer");
        let play::Clientbound::SetPlayerTeam {
            name,
            method,
            parameters,
            players,
        } = decode(pl::Clientbound::SetPlayerTeam, &b)
        else {
            panic!()
        };
        assert_eq!((name.as_str(), method), ("nopush", 0));
        assert_eq!(parameters.map(|p| p.collision_rule), Some(1));
        assert_eq!(players, ["Developer"]);

        // A player joining: its name (no properties) and game mode.
        let mut b = vec![play::PLAYER_INFO_ADD | play::PLAYER_INFO_GAME_MODE];
        wire::write_varint(&mut b, 1);
        wire::write_uuid(&mut b, 77);
        wire::write_string(&mut b, "Alex");
        wire::write_varint(&mut b, 0);
        wire::write_varint(&mut b, 3);
        assert_eq!(
            decode(pl::Clientbound::PlayerInfoUpdate, &b),
            play::Clientbound::PlayerInfoUpdate {
                actions: 5,
                entries: vec![play::PlayerInfoEntry {
                    uuid: 77,
                    name: Some("Alex".into()),
                    game_mode: Some(3),
                }],
            }
        );
    }

    /// A stack of `count` of `item` with a damage component, as the server writes it.
    fn damaged_stack(b: &mut Vec<u8>, item: i32, count: i32, damage: i32) {
        wire::write_varint(b, count);
        wire::write_varint(b, item);
        b.extend([1, 0, 3]);
        wire::write_varint(b, damage);
    }

    #[test]
    fn play_reads_item_packets() {
        use crate::item::{Component, ItemStack};
        let decode =
            |kind: pl::Clientbound, b: &[u8]| play::Clientbound::decode(kind.id(), b).unwrap();

        // The inventory menu: three slots (the middle one empty), and an empty cursor.
        let mut b = Vec::new();
        wire::write_varint(&mut b, 0);
        wire::write_varint(&mut b, 41);
        wire::write_varint(&mut b, 3);
        damaged_stack(&mut b, 850, 1, 7);
        b.push(0);
        wire::write_varint(&mut b, 64);
        wire::write_varint(&mut b, 35);
        b.extend([0, 0]);
        b.push(0);
        let play::Clientbound::ContainerSetContent {
            container_id,
            state_id,
            items,
            carried,
        } = decode(pl::Clientbound::ContainerSetContent, &b)
        else {
            panic!()
        };
        assert_eq!((container_id, state_id, items.len()), (0, 41, 3));
        assert_eq!(
            items[0].get(ruthenium_data::v26_3::registries::DataComponentType::Damage),
            Some(&Component::Damage(7))
        );
        assert!(items[1].is_empty() && carried.is_empty());
        assert_eq!((items[2].item, items[2].count), (35, 64));

        // One slot: a short, which may be negative (-1 is the cursor in old menus).
        let mut b = Vec::new();
        wire::write_varint(&mut b, 2);
        wire::write_varint(&mut b, 300);
        b.extend((-1i16).to_be_bytes());
        damaged_stack(&mut b, 850, 1, 0);
        let play::Clientbound::ContainerSetSlot {
            container_id,
            state_id,
            slot,
            item,
        } = decode(pl::Clientbound::ContainerSetSlot, &b)
        else {
            panic!()
        };
        assert_eq!((container_id, state_id, slot, item.item), (2, 300, -1, 850));

        let mut b = Vec::new();
        wire::write_varint(&mut b, 40);
        b.extend([0]);
        assert_eq!(
            decode(pl::Clientbound::SetPlayerInventory, &b),
            play::Clientbound::SetPlayerInventory {
                slot: 40,
                item: ItemStack::EMPTY
            }
        );
        let mut b = Vec::new();
        damaged_stack(&mut b, 1, 3, 2);
        assert!(matches!(decode(pl::Clientbound::SetCursorItem, &b),
            play::Clientbound::SetCursorItem { item } if item.count == 3));

        let mut b = Vec::new();
        wire::write_string(&mut b, "ender_pearl");
        wire::write_varint(&mut b, 20);
        assert_eq!(
            decode(pl::Clientbound::Cooldown, &b),
            play::Clientbound::Cooldown {
                group: "minecraft:ender_pearl".into(),
                duration: 20
            }
        );
        // The entity id is a full int here, not a VarInt.
        assert_eq!(
            decode(pl::Clientbound::EntityEvent, &[0, 0, 1, 0, 35]),
            play::Clientbound::EntityEvent {
                entity_id: 256,
                event: 35
            }
        );

        // A dropped item's entity data: the stack, then a boolean.
        let mut b = Vec::new();
        wire::write_varint(&mut b, 9);
        b.extend([8, 7]);
        damaged_stack(&mut b, 850, 1, 4);
        b.extend([9, 8, 1, 255]);
        let play::Clientbound::SetEntityData {
            values, complete, ..
        } = decode(pl::Clientbound::SetEntityData, &b)
        else {
            panic!()
        };
        assert!(complete);
        let play::DataValue::Item(stack) = &values[0].1 else {
            panic!("{values:?}")
        };
        assert_eq!(
            (stack.item, values[1].1.clone()),
            (850, play::DataValue::Boolean(true))
        );
    }

    #[test]
    fn play_item_serverbound_layouts() {
        let id = |p: pl::Serverbound| p.id() as u8;
        let mut want = vec![id(pl::Serverbound::UseItem), 1, 5];
        want.extend(90.0f32.to_be_bytes());
        want.extend((-10.0f32).to_be_bytes());
        assert_eq!(play::use_item(play::OFF_HAND, 5, 90.0, -10.0), want);

        let pos = [-3, 64, 1_000_000];
        assert_eq!(play::unpack_block_pos(play::pack_block_pos(pos)), pos);
        assert_eq!(
            play::unpack_block_pos(play::pack_block_pos([5, -64, -7])),
            [5, -64, -7]
        );

        let p = play::use_item_on(
            play::MAIN_HAND,
            pos,
            play::UP,
            [0.5, 1.0, 0.25],
            false,
            false,
            9,
        );
        let (pid, rest) = body(&p);
        assert_eq!(pid, pl::Serverbound::UseItemOn.id());
        let mut r = Reader::new(rest);
        assert_eq!(r.varint().unwrap(), 0);
        assert_eq!(play::unpack_block_pos(r.i64().unwrap()), pos);
        assert_eq!(r.varint().unwrap(), play::UP);
        assert_eq!(
            [r.f32().unwrap(), r.f32().unwrap(), r.f32().unwrap()],
            [0.5, 1.0, 0.25]
        );
        assert_eq!(
            (r.bool().unwrap(), r.bool().unwrap(), r.varint().unwrap()),
            (false, false, 9)
        );
        r.finish().unwrap();

        let p = play::player_action(play::ACTION_START_DESTROY_BLOCK, [1, 2, 3], play::EAST, 300);
        let (pid, rest) = body(&p);
        assert_eq!(pid, pl::Serverbound::PlayerAction.id());
        assert_eq!(rest[0], 0);
        assert_eq!(
            play::unpack_block_pos(i64::from_be_bytes(rest[1..9].try_into().unwrap())),
            [1, 2, 3]
        );
        assert_eq!(rest[9..], [5, 0xAC, 0x02]);
        let p = play::player_action(play::ACTION_SWAP_ITEM_WITH_OFFHAND, [0; 3], play::DOWN, 0);
        assert_eq!(p[1..], [7, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);

        assert_eq!(play::punch(), [id(pl::Serverbound::Punch)]);
        assert_eq!(play::attack(300), [id(pl::Serverbound::Attack), 0xAC, 0x02]);
        assert_eq!(
            play::spectator_action(None),
            [id(pl::Serverbound::SpectatorAction), 0]
        );
        assert_eq!(
            play::spectator_action(Some(4)),
            [id(pl::Serverbound::SpectatorAction), 5]
        );
        // Interacting at the entity's feet: a zero vector is one byte.
        assert_eq!(
            play::interact(7, play::MAIN_HAND, [0.0; 3], true),
            [id(pl::Serverbound::Interact), 7, 0, 0, 1]
        );
        let p = play::interact(7, play::OFF_HAND, [0.25, 1.5, -0.25], false);
        let mut r = Reader::new(&p[1..]);
        assert_eq!((r.varint().unwrap(), r.varint().unwrap()), (7, 1));
        let v = r.lp_vec3().unwrap();
        assert!((v[1] - 1.5).abs() < 1e-3 && (v[2] + 0.25).abs() < 1e-3);
        assert!(!r.bool().unwrap());
        r.finish().unwrap();
    }
}

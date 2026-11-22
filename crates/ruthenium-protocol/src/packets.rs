//! Packets of the login and configuration phases, and the few play packets every phase
//! shares (keep-alive, ping, disconnect, cookies, resource packs, transfer), with 26.3's
//! layouts. Decoding is exact: a packet with bytes left after its fields is refused, as
//! vanilla's `PacketDecoder` does.
//!
//! Serverbound packets are built as their bytes before framing: the id as a VarInt, then
//! the fields ([`wire::packet`]).

use ruthenium_data::v26_3::packets::{configuration as cfg, login as lg, play as pl};

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

    /// The game type id of spectator mode (`GameType.SPECTATOR`).
    pub const SPECTATOR: i32 = 3;

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
        /// A chunk arrived; only its position is read so far.
        LevelChunk {
            x: i32,
            z: i32,
        },
        ForgetLevelChunk {
            x: i32,
            z: i32,
        },
        SetHeldSlot {
            slot: i32,
        },
        TickingState {
            tick_rate: f32,
            frozen: bool,
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
                    r.rest();
                    Clientbound::LevelChunk { x, z }
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
                C::TickingState => Clientbound::TickingState {
                    tick_rate: r.f32()?,
                    frozen: r.bool()?,
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

    pub fn set_carried_item(slot: i32) -> Vec<u8> {
        // `ServerboundSetCarriedItemPacket` writes the slot as a short.
        wire::packet(pl::Serverbound::SetCarriedItem.id(), |o| {
            o.extend_from_slice(&(slot as i16).to_be_bytes())
        })
    }

    pub fn configuration_acknowledged() -> Vec<u8> {
        wire::packet(pl::Serverbound::ConfigurationAcknowledged.id(), |_| {})
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
}

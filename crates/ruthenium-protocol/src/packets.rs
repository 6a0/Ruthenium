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
use crate::profile::ResolvableProfile;
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
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
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
        let properties = Property::read_list(r)?;
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

    /// A sound event on the wire (`SoundEvent.STREAM_CODEC`): an id in the
    /// `sound_event` registry, or an event written out, with the fixed range the server
    /// used to choose who hears it (the client doesn't use it).
    #[derive(Debug, Clone, PartialEq)]
    pub enum SoundEventRef {
        Registry(i32),
        Direct { id: String, range: Option<f32> },
    }

    /// `ByteBufCodecs.holder`: 0 for a value written out, else the registry id + 1.
    pub fn read_sound_event(r: &mut Reader<'_>) -> Result<SoundEventRef, WireError> {
        Ok(match r.varint()? {
            0 => SoundEventRef::Direct {
                id: r.identifier()?,
                range: optional(r, |r| r.f32())?,
            },
            n => SoundEventRef::Registry(n - 1),
        })
    }

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
        /// A text component, in its NBT form.
        Component(Tag),
        OptionalComponent(Option<Tag>),
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
        /// A mannequin's profile.
        Profile(Box<ResolvableProfile>),
    }

    /// The `minecraft:entity_effect` particle type, the one particle the client reads
    /// out of entity data (a colour, an int).
    const ENTITY_EFFECT_PARTICLE: i32 = 28;

    impl DataValue {
        /// The value for serializer `kind`, or `None` for a kind the client can't read
        /// yet (other particles, inline painting variants): everything after it
        /// is then lost.
        fn read(r: &mut Reader<'_>, kind: i32) -> Result<Option<DataValue>, WireError> {
            let block_pos = |r: &mut Reader<'_>| r.i64().map(unpack_block_pos);
            let v = match kind {
                0 => DataValue::Byte(r.u8()? as i8),
                1 => DataValue::Int(r.varint()?),
                2 => DataValue::Long(r.varlong()?),
                3 => DataValue::Float(r.f32()?),
                4 => DataValue::String(r.string(MAX_STRING)?.to_owned()),
                5 => DataValue::Component(text(r)?),
                6 => DataValue::OptionalComponent(optional(r, text)?),
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
                41 => DataValue::Profile(Box::new(ResolvableProfile::read(r)?)),
                // Particles (16).
                _ => return Ok(None),
            };
            Ok(Some(v))
        }
    }

    /// A team's settings (`ClientboundSetPlayerTeamPacket.Parameters`); the texts are
    /// components in their NBT form.
    #[derive(Debug, Clone, PartialEq)]
    pub struct TeamParameters {
        pub display_name: Tag,
        /// What stands before and after its members' names.
        pub prefix: Tag,
        pub suffix: Tag,
        /// `Team.Visibility` id: 0 always, 1 never, 2 hidden from other teams, 3 hidden
        /// from the own team.
        pub name_tag_visibility: i32,
        /// `Team.CollisionRule` id: 0 always, 1 never, 2 push other teams, 3 push own
        /// team.
        pub collision_rule: i32,
        /// `ChatFormatting` ordinal: the sixteen colours, then obfuscated, bold,
        /// strikethrough, underline, italic and reset.
        pub color: Option<i32>,
        pub options: u8,
    }

    /// How a score is written (`NumberFormat`).
    #[derive(Debug, Clone, PartialEq)]
    pub enum NumberFormat {
        /// Not at all.
        Blank,
        /// As a number in this style (a style's fields in a compound).
        Styled(Tag),
        /// As this text, whatever the number.
        Fixed(Tag),
    }

    /// `DisplaySlot.BELOW_NAME`.
    pub const DISPLAY_SLOT_BELOW_NAME: i32 = 2;

    fn number_format(r: &mut Reader<'_>) -> Result<Option<NumberFormat>, WireError> {
        optional(r, |r| {
            Ok(match r.varint()? {
                0 => NumberFormat::Blank,
                1 => NumberFormat::Styled(text(r)?),
                2 => NumberFormat::Fixed(text(r)?),
                _ => return Err(WireError::Invalid("number format")),
            })
        })
    }

    /// `DisplaySlot.LIST` and `SIDEBAR`; a team colour's own sidebar is 3 plus the
    /// colour.
    pub const DISPLAY_SLOT_LIST: i32 = 0;
    pub const DISPLAY_SLOT_SIDEBAR: i32 = 1;
    pub const DISPLAY_SLOT_TEAM: i32 = 3;

    /// Bytes of a chat message's signature.
    pub const SIGNATURE_BYTES: usize = 256;

    /// A chat message's signature, whole or as its place in the signatures both sides
    /// keep (`MessageSignature.Packed`).
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub enum PackedSignature {
        Cached(usize),
        Full(Box<[u8; SIGNATURE_BYTES]>),
    }

    fn signature(r: &mut Reader<'_>) -> Result<Box<[u8; SIGNATURE_BYTES]>, WireError> {
        let bytes: [u8; SIGNATURE_BYTES] = r
            .bytes(SIGNATURE_BYTES)?
            .try_into()
            .map_err(|_| WireError::Invalid("signature"))?;
        Ok(Box::new(bytes))
    }

    fn packed_signature(r: &mut Reader<'_>) -> Result<PackedSignature, WireError> {
        Ok(match r.varint()? {
            0 => PackedSignature::Full(signature(r)?),
            n => PackedSignature::Cached(
                usize::try_from(n - 1).map_err(|_| WireError::Invalid("signature id"))?,
            ),
        })
    }

    /// How a kind of chat message is written (`ChatTypeDecoration`): a translation, what
    /// it is filled in with, and its style (a style's fields in a compound).
    #[derive(Debug, Clone, PartialEq)]
    pub struct ChatDecoration {
        pub translation_key: String,
        pub parameters: Vec<ChatParameter>,
        pub style: Option<Tag>,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum ChatParameter {
        Sender,
        Target,
        Content,
    }

    fn chat_decoration(r: &mut Reader<'_>) -> Result<ChatDecoration, WireError> {
        let translation_key = r.string(MAX_STRING)?.to_owned();
        let n = r.count(usize::MAX)?;
        let mut parameters = Vec::with_capacity(n.min(16));
        for _ in 0..n {
            parameters.push(match r.varint()? {
                1 => ChatParameter::Target,
                2 => ChatParameter::Content,
                // (Unknown ones read as the first.)
                _ => ChatParameter::Sender,
            });
        }
        Ok(ChatDecoration {
            translation_key,
            parameters,
            style: r.nbt()?,
        })
    }

    /// A chat message's kind and who it is from and to (`ChatType.Bound`).
    #[derive(Debug, Clone, PartialEq)]
    pub struct BoundChatType {
        pub kind: ChatTypeRef,
        pub name: Tag,
        pub target: Option<Tag>,
    }

    /// A `chat_type` of the registry, by its id, or one written out.
    #[derive(Debug, Clone, PartialEq)]
    pub enum ChatTypeRef {
        Id(usize),
        Direct(Box<ChatDecoration>),
    }

    fn bound_chat_type(r: &mut Reader<'_>) -> Result<BoundChatType, WireError> {
        let kind = match r.varint()? {
            0 => {
                let chat = chat_decoration(r)?;
                // How it is read out.
                chat_decoration(r)?;
                ChatTypeRef::Direct(Box::new(chat))
            }
            n => ChatTypeRef::Id(
                usize::try_from(n - 1).map_err(|_| WireError::Invalid("chat type"))?,
            ),
        };
        Ok(BoundChatType {
            kind,
            name: text(r)?,
            target: optional(r, text)?,
        })
    }

    /// Which characters of a chat message a server's filter hides (`FilterMask`).
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub enum FilterMask {
        PassThrough,
        FullyFiltered,
        /// Bit `i % 8` of byte `i / 8` is the message's UTF-16 unit `i`.
        Partly(Vec<u8>),
    }

    /// A player's chat message (`ClientboundPlayerChatPacket`).
    #[derive(Debug, Clone, PartialEq)]
    pub struct PlayerChat {
        /// How many of them the server had sent before this one.
        pub global_index: i32,
        pub sender: u128,
        pub signature: Option<Box<[u8; SIGNATURE_BYTES]>>,
        /// What the player wrote, and when, in milliseconds of the epoch.
        pub content: String,
        pub time: i64,
        /// The messages the sender had last seen.
        pub last_seen: Vec<PackedSignature>,
        /// What the server shows in its place.
        pub unsigned_content: Option<Tag>,
        pub filter: FilterMask,
        pub kind: BoundChatType,
    }

    fn player_chat(r: &mut Reader<'_>) -> Result<PlayerChat, WireError> {
        let global_index = r.varint()?;
        let sender = r.uuid()?;
        // How many messages of the sender's session came before.
        r.varint()?;
        let signature = optional(r, signature)?;
        let content = r.string(256)?.to_owned();
        let time = r.i64()?;
        // The salt.
        r.i64()?;
        let n = r.count(20)?;
        let mut last_seen = Vec::with_capacity(n);
        for _ in 0..n {
            last_seen.push(packed_signature(r)?);
        }
        let unsigned_content = optional(r, text)?;
        let filter = match r.varint()? {
            1 => FilterMask::FullyFiltered,
            2 => FilterMask::Partly(r.byte_array(1 << 16)?.to_vec()),
            _ => FilterMask::PassThrough,
        };
        Ok(PlayerChat {
            global_index,
            sender,
            signature,
            content,
            time,
            last_seen,
            unsigned_content,
            filter,
            kind: bound_chat_type(r)?,
        })
    }

    /// What a `ClientboundBossEventPacket` does to the bar it names. Colours are
    /// `BossBarColor` ordinals (pink, blue, red, green, yellow, purple, white), overlays
    /// `BossBarOverlay`'s (none, then 6, 10, 12 and 20 notches).
    #[derive(Debug, Clone, PartialEq)]
    pub enum BossEvent {
        Add {
            name: Tag,
            progress: f32,
            color: u8,
            overlay: u8,
            /// Bit 0 darkens the screen, 1 plays the boss music, 2 makes fog.
            flags: u8,
        },
        Remove,
        Progress(f32),
        Name(Tag),
        Style {
            color: u8,
            overlay: u8,
        },
        Properties {
            flags: u8,
        },
    }

    fn boss_event(r: &mut Reader<'_>) -> Result<BossEvent, WireError> {
        fn ordinal(r: &mut Reader<'_>, count: u8) -> Result<u8, WireError> {
            u8::try_from(r.varint()?)
                .ok()
                .filter(|v| *v < count)
                .ok_or(WireError::Invalid("boss bar style"))
        }
        Ok(match r.varint()? {
            0 => BossEvent::Add {
                name: text(r)?,
                progress: r.f32()?,
                color: ordinal(r, 7)?,
                overlay: ordinal(r, 5)?,
                flags: r.u8()?,
            },
            1 => BossEvent::Remove,
            2 => BossEvent::Progress(r.f32()?),
            3 => BossEvent::Name(text(r)?),
            4 => BossEvent::Style {
                color: ordinal(r, 7)?,
                overlay: ordinal(r, 5)?,
            },
            5 => BossEvent::Properties { flags: r.u8()? },
            _ => return Err(WireError::Invalid("boss event")),
        })
    }

    /// One player's part of a `ClientboundPlayerInfoUpdatePacket`: what the actions
    /// carry that the client keeps so far.
    #[derive(Debug, Clone, Default, PartialEq)]
    pub struct PlayerInfoEntry {
        pub uuid: u128,
        /// With `ADD_PLAYER`: the profile's name and properties.
        pub name: Option<String>,
        pub properties: Vec<Property>,
        /// With `UPDATE_GAME_MODE`.
        pub game_mode: Option<i32>,
        /// With `UPDATE_LISTED`, `UPDATE_LATENCY`, `UPDATE_DISPLAY_NAME` (which may be
        /// taken away), `UPDATE_LIST_ORDER` and `UPDATE_HAT`.
        pub listed: Option<bool>,
        pub latency: Option<i32>,
        pub display_name: Option<Option<Tag>>,
        pub list_order: Option<i32>,
        pub show_hat: Option<bool>,
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
            ..PlayerInfoEntry::default()
        };
        let has = |bit: u8| actions & 1 << bit != 0;
        if has(0) {
            e.name = Some(r.string(MAX_NAME)?.to_owned());
            e.properties = Property::read_list(r)?;
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
            e.listed = Some(r.bool()?);
        }
        if has(4) {
            e.latency = Some(r.varint()?);
        }
        if has(5) {
            e.display_name = Some(optional(r, text)?);
        }
        if has(6) {
            e.list_order = Some(r.varint()?);
        }
        if has(7) {
            e.show_hat = Some(r.bool()?);
        }
        Ok(e)
    }

    /// Bytes of one section's light: a nibble per block, x fastest, then z, then y
    /// (`DataLayer`).
    pub const LIGHT_BYTES: usize = 2048;

    /// The most bytes a light mask may have: enough for the tallest level's light
    /// sections (4064 blocks: 256 of them) several times over.
    const MAX_MASK_BYTES: usize = 128;

    /// The light of some of a chunk's sections (`ClientboundLightUpdatePacketData`).
    /// Light sections start one below the level's lowest section and end one above its
    /// highest; bit `i` of a mask is light section `i`. A section in a mask has an array
    /// in the matching list, in order; a section in an empty mask is all dark; a section
    /// in neither is left as it was.
    #[derive(Debug, Clone, PartialEq, Default)]
    pub struct LightData {
        pub sky_mask: Vec<u64>,
        pub block_mask: Vec<u64>,
        pub empty_sky_mask: Vec<u64>,
        pub empty_block_mask: Vec<u64>,
        pub sky: Vec<std::sync::Arc<[u8; LIGHT_BYTES]>>,
        pub block: Vec<std::sync::Arc<[u8; LIGHT_BYTES]>>,
    }

    impl LightData {
        fn read(r: &mut Reader<'_>) -> Result<LightData, WireError> {
            // Each mask is a `BitSet` as its bytes (`BitSet.toByteArray`): bit `i` is
            // bit `i % 8` of byte `i / 8`.
            let mut masks: [Vec<u64>; 4] = Default::default();
            for mask in &mut masks {
                for (i, b) in r.byte_array(MAX_MASK_BYTES)?.iter().enumerate() {
                    if i % 8 == 0 {
                        mask.push(0);
                    }
                    mask[i / 8] |= u64::from(*b) << (i % 8 * 8);
                }
            }
            let arrays = |r: &mut Reader<'_>| -> Result<_, WireError> {
                let n = r.count(4096)?;
                let mut out = Vec::with_capacity(n.min(64));
                for _ in 0..n {
                    let bytes: [u8; LIGHT_BYTES] = r
                        .byte_array(LIGHT_BYTES)?
                        .try_into()
                        .map_err(|_| WireError::Invalid("light section size"))?;
                    out.push(std::sync::Arc::new(bytes));
                }
                Ok(out)
            };
            let sky = arrays(r)?;
            let block = arrays(r)?;
            let [sky_mask, block_mask, empty_sky_mask, empty_block_mask] = masks;
            Ok(LightData {
                sky_mask,
                block_mask,
                empty_sky_mask,
                empty_block_mask,
                sky,
                block,
            })
        }

        /// Whether bit `i` of a mask is set (`BitSet.get`).
        pub fn bit(mask: &[u64], i: usize) -> bool {
            mask.get(i / 64).is_some_and(|w| w >> (i % 64) & 1 == 1)
        }
    }

    /// A block entity a chunk packet lists (`BlockEntityInfo`).
    #[derive(Debug, Clone, PartialEq)]
    pub struct ChunkBlockEntity {
        /// The position in the chunk: x in the high nibble, z in the low.
        pub xz: u8,
        pub y: i16,
        /// The block entity type's registry id.
        pub kind: i32,
        /// What the type sends clients (`getUpdateTag`), if anything.
        pub tag: Option<crate::nbt::Tag>,
    }

    /// A world clock as the server sends it (`ClockNetworkState`).
    #[derive(Debug, Clone, Copy, PartialEq)]
    pub struct ClockState {
        pub total_ticks: i64,
        pub partial_tick: f32,
        /// Clock ticks per game tick; 0 while the clock is stopped.
        pub rate: f32,
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
        /// `ClientboundPlayerRotationPacket` (`/rotate`): the player's new rotation, each
        /// part added to the current one when it is relative.
        PlayerRotation {
            y_rot: f32,
            relative_y: bool,
            x_rot: f32,
            relative_x: bool,
        },
        /// `ClientboundGameEventPacket`: the event id and its parameter.
        GameEvent {
            event: u8,
            param: f32,
        },
        ChunkBatchStart,
        ChunkBatchFinished {
            batch_size: i32,
        },
        /// A chunk (`ClientboundLevelChunkWithLightPacket`): its position, the heightmaps
        /// the server sends clients (each a type id and its packed heights), its
        /// sections' data (`ClientboundLevelChunkPacketData`'s buffer), the block
        /// entities in it and its light.
        LevelChunk {
            x: i32,
            z: i32,
            heightmaps: Vec<(i32, Vec<i64>)>,
            data: Vec<u8>,
            block_entities: Vec<ChunkBlockEntity>,
            light: LightData,
        },
        /// `ClientboundLightUpdatePacket`: new light for some of a chunk's sections.
        LightUpdate {
            x: i32,
            z: i32,
            light: LightData,
        },
        /// `ClientboundBlockEntityDataPacket`: a block entity's data as its type sends
        /// it to clients (a sign's text, a banner's patterns, a skull's profile).
        BlockEntityData {
            pos: [i32; 3],
            /// The block entity type's registry id.
            kind: i32,
            tag: Option<crate::nbt::Tag>,
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
        /// key and item ids; then the stonecutter's recipes.
        UpdateRecipes {
            item_sets: Vec<(String, Vec<i32>)>,
            stonecutter: Vec<crate::recipe::StonecutterEntry>,
        },
        /// `ClientboundMerchantOffersPacket`: a merchant's offers for its open menu, its
        /// level and experience, whether the screen shows the bar, and whether it restocks.
        MerchantOffers {
            container_id: i32,
            offers: Vec<crate::trade::MerchantOffer>,
            villager_level: i32,
            villager_xp: i32,
            show_progress: bool,
            can_restock: bool,
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
        /// `ClientboundSetObjectivePacket`: method 0 adds the objective, 1 removes it,
        /// 2 changes it; the name it shows and how its scores are written come with 0
        /// and 2.
        SetObjective {
            name: String,
            method: u8,
            display_name: Option<Tag>,
            /// The player list shows it as hearts.
            hearts: bool,
            number_format: Option<NumberFormat>,
        },
        /// `ClientboundSetDisplayObjectivePacket`: the objective a `DisplaySlot` shows;
        /// none for an empty name.
        SetDisplayObjective {
            slot: i32,
            objective: String,
        },
        /// `ClientboundSetScorePacket`.
        SetScore {
            owner: String,
            objective: String,
            score: i32,
            display: Option<Tag>,
            number_format: Option<NumberFormat>,
        },
        /// `ClientboundResetScorePacket`: one score of an owner, or all of them.
        ResetScore {
            owner: String,
            objective: Option<String>,
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
        /// `ClientboundSetExperiencePacket`: how far the bar is filled (0 to 1), the
        /// level, and all the points so far.
        SetExperience {
            progress: f32,
            level: i32,
            total: i32,
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
        /// `ClientboundOpenScreenPacket`: a menu opens; its type is a `minecraft:menu`
        /// registry id, its title a text component.
        OpenScreen {
            container_id: i32,
            menu_type: i32,
            title: Tag,
        },
        /// `ClientboundContainerClosePacket`.
        ContainerClose {
            container_id: i32,
        },
        /// `ClientboundContainerSetDataPacket`: one of a menu's data slots (a furnace's
        /// fire and arrow).
        ContainerSetData {
            container_id: i32,
            id: i16,
            value: i16,
        },
        /// `ClientboundMountScreenOpenPacket`: a horse's (or nautilus's) inventory.
        MountScreenOpen {
            container_id: i32,
            inventory_columns: i32,
            entity_id: i32,
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
        /// `ClientboundSetSimulationDistancePacket`, in chunks.
        SetSimulationDistance {
            distance: i32,
        },
        /// `ClientboundRotateHeadPacket`: the way an entity's head turns to.
        RotateHead {
            entity_id: i32,
            y_head_rot: i8,
        },
        /// `ClientboundDamageEventPacket`: an entity was hurt, with the kind of damage (its
        /// id in the `damage_type` registry; who and what caused it are not read).
        DamageEvent {
            entity_id: i32,
            source_type: i32,
        },
        /// `ClientboundHurtAnimationPacket`: an entity flinches; `yaw` is where what
        /// hurt it came from, in degrees from the way it faces.
        HurtAnimation {
            entity_id: i32,
            yaw: f32,
        },
        /// `ClientboundBlockDestructionPacket`: how far the entity `id` has broken the
        /// block at `pos`: a stage from 0 to 9, anything else for no longer.
        BlockDestruction {
            id: i32,
            pos: [i32; 3],
            progress: u8,
        },
        /// `ClientboundSwingAnimationPacket`: an entity swings a hand. `animation` is
        /// none (0), a whack (1) or a stab (2).
        SwingAnimation {
            entity_id: i32,
            off_hand: bool,
            animation: i32,
            duration: i32,
        },
        /// `ClientboundSetTimePacket`: the level's game time and the world clocks that
        /// changed, each by its id in the `world_clock` registry.
        SetTime {
            game_time: i64,
            clocks: Vec<(i32, ClockState)>,
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
        /// `ClientboundSystemChatPacket`: a message of the server's, for the chat or (an
        /// overlay) over the hotbar.
        SystemChat {
            content: Tag,
            overlay: bool,
        },
        PlayerChat(Box<PlayerChat>),
        /// `ClientboundDisguisedChatPacket`: a message shown as a player's that no player
        /// wrote.
        DisguisedChat {
            message: Tag,
            kind: BoundChatType,
        },
        DeleteChat {
            signature: PackedSignature,
        },
        SetActionBarText {
            text: Tag,
        },
        SetTitleText {
            text: Tag,
        },
        SetSubtitleText {
            text: Tag,
        },
        /// `ClientboundSetTitlesAnimationPacket`, in ticks; below 0 leaves one as it is.
        SetTitlesAnimation {
            fade_in: i32,
            stay: i32,
            fade_out: i32,
        },
        ClearTitles {
            reset_times: bool,
        },
        BossEvent {
            id: u128,
            event: BossEvent,
        },
        /// `ClientboundTabListPacket`: what stands over and under the player list.
        TabList {
            header: Tag,
            footer: Tag,
        },
        /// `ClientboundLevelEventPacket`.
        LevelEvent {
            event: i32,
            pos: [i32; 3],
            data: i32,
            global: bool,
        },
        /// `ClientboundSoundPacket`: a sound at a place, given in eighths of a block;
        /// `source` is the `SoundSource` ordinal and `seed` picks the sound.
        Sound {
            event: SoundEventRef,
            source: i32,
            position: [i32; 3],
            volume: f32,
            pitch: f32,
            seed: i64,
        },
        /// `ClientboundSoundEntityPacket`: a sound that follows an entity.
        SoundEntity {
            event: SoundEventRef,
            source: i32,
            entity_id: i32,
            volume: f32,
            pitch: f32,
            seed: i64,
        },
        /// `ClientboundStopSoundPacket`: stop sounds of a category, of an event, or all.
        StopSound {
            source: Option<i32>,
            name: Option<String>,
        },
        /// `ClientboundTakeItemEntityPacket`: an entity picked an item or orb up.
        TakeItemEntity {
            item_id: i32,
            player_id: i32,
            amount: i32,
        },
        /// `ClientboundCommandsPacket`: the commands the player may use.
        Commands(Box<crate::commands::CommandTree>),
        /// `ClientboundBundleDelimiterPacket`: opens or closes a bundle. Never reaches the
        /// game thread: the connection groups the packets between two into one event, as
        /// vanilla's `PacketBundlePacker` makes a `ClientboundBundlePacket`.
        BundleDelimiter,
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
                C::BundleDelimiter => Clientbound::BundleDelimiter,
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
                C::PlayerRotation => Clientbound::PlayerRotation {
                    y_rot: r.f32()?,
                    relative_y: r.bool()?,
                    x_rot: r.f32()?,
                    relative_x: r.bool()?,
                },
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
                    let mut heightmaps = Vec::with_capacity(n);
                    for _ in 0..n {
                        let kind = r.varint()?;
                        let longs = r.count(4096)?;
                        let mut heights = Vec::with_capacity(longs);
                        for _ in 0..longs {
                            heights.push(r.i64()?);
                        }
                        heightmaps.push((kind, heights));
                    }
                    let data = r.byte_array(2 << 20)?.to_vec();
                    let n = r.count(1 << 16)?;
                    let mut block_entities = Vec::with_capacity(n.min(1024));
                    for _ in 0..n {
                        block_entities.push(ChunkBlockEntity {
                            xz: r.u8()?,
                            y: r.u16()? as i16,
                            kind: r.varint()?,
                            tag: r.nbt()?,
                        });
                    }
                    let light = LightData::read(&mut r)?;
                    Clientbound::LevelChunk {
                        x,
                        z,
                        heightmaps,
                        data,
                        block_entities,
                        light,
                    }
                }
                C::LightUpdate => Clientbound::LightUpdate {
                    x: r.varint()?,
                    z: r.varint()?,
                    light: LightData::read(&mut r)?,
                },
                C::BlockEntityData => Clientbound::BlockEntityData {
                    pos: unpack_block_pos(r.i64()?),
                    kind: r.varint()?,
                    tag: r.nbt()?,
                },
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
                    let stonecutter = crate::recipe::read_stonecutter_recipes(&mut r)?;
                    Clientbound::UpdateRecipes {
                        item_sets,
                        stonecutter,
                    }
                }
                C::MerchantOffers => Clientbound::MerchantOffers {
                    container_id: r.varint()?,
                    offers: crate::trade::read_offers(&mut r)?,
                    villager_level: r.varint()?,
                    villager_xp: r.varint()?,
                    show_progress: r.bool()?,
                    can_restock: r.bool()?,
                },
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
                        Some(TeamParameters {
                            display_name: text(&mut r)?,
                            prefix: text(&mut r)?,
                            suffix: text(&mut r)?,
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
                C::SetObjective => {
                    let name = r.string(MAX_STRING)?.to_owned();
                    let method = r.u8()?;
                    let (display_name, hearts, number_format) = if method == 0 || method == 2 {
                        let display_name = text(&mut r)?;
                        let hearts = r.varint()? == 1;
                        (Some(display_name), hearts, number_format(&mut r)?)
                    } else {
                        (None, false, None)
                    };
                    Clientbound::SetObjective {
                        name,
                        method,
                        display_name,
                        hearts,
                        number_format,
                    }
                }
                C::SetDisplayObjective => Clientbound::SetDisplayObjective {
                    slot: r.varint()?,
                    objective: r.string(MAX_STRING)?.to_owned(),
                },
                C::SetScore => Clientbound::SetScore {
                    owner: r.string(MAX_STRING)?.to_owned(),
                    objective: r.string(MAX_STRING)?.to_owned(),
                    score: r.varint()?,
                    display: optional(&mut r, text)?,
                    number_format: number_format(&mut r)?,
                },
                C::ResetScore => Clientbound::ResetScore {
                    owner: r.string(MAX_STRING)?.to_owned(),
                    objective: optional(&mut r, |r| Ok(r.string(MAX_STRING)?.to_owned()))?,
                },
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
                C::SetExperience => Clientbound::SetExperience {
                    progress: r.f32()?,
                    level: r.varint()?,
                    total: r.varint()?,
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
                C::OpenScreen => Clientbound::OpenScreen {
                    container_id: r.varint()?,
                    menu_type: r.varint()?,
                    title: text(&mut r)?,
                },
                C::ContainerClose => Clientbound::ContainerClose {
                    container_id: r.varint()?,
                },
                C::ContainerSetData => Clientbound::ContainerSetData {
                    container_id: r.varint()?,
                    id: r.u16()? as i16,
                    value: r.u16()? as i16,
                },
                C::MountScreenOpen => Clientbound::MountScreenOpen {
                    container_id: r.varint()?,
                    inventory_columns: r.varint()?,
                    entity_id: r.i32()?,
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
                C::SetSimulationDistance => Clientbound::SetSimulationDistance {
                    distance: r.varint()?,
                },
                C::RotateHead => Clientbound::RotateHead {
                    entity_id: r.varint()?,
                    y_head_rot: r.u8()? as i8,
                },
                C::DamageEvent => {
                    let entity_id = r.varint()?;
                    let source_type = r.varint()?;
                    // Who and what caused it (one above their ids), and where it came
                    // from.
                    for _ in 0..2 {
                        r.varint()?;
                    }
                    optional(&mut r, vec3)?;
                    Clientbound::DamageEvent {
                        entity_id,
                        source_type,
                    }
                }
                C::HurtAnimation => Clientbound::HurtAnimation {
                    entity_id: r.varint()?,
                    yaw: r.f32()?,
                },
                C::BlockDestruction => Clientbound::BlockDestruction {
                    id: r.varint()?,
                    pos: unpack_block_pos(r.i64()?),
                    progress: r.u8()?,
                },
                C::SwingAnimation => Clientbound::SwingAnimation {
                    entity_id: r.varint()?,
                    off_hand: r.varint()? == 1,
                    animation: r.varint()?,
                    duration: r.varint()?,
                },
                C::SetTime => {
                    let game_time = r.i64()?;
                    let n = r.count(1024)?;
                    let mut clocks = Vec::with_capacity(n);
                    for _ in 0..n {
                        clocks.push((
                            r.varint()?,
                            ClockState {
                                total_ticks: r.varlong()?,
                                partial_tick: r.f32()?,
                                rate: r.f32()?,
                            },
                        ));
                    }
                    Clientbound::SetTime { game_time, clocks }
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
                C::SystemChat => Clientbound::SystemChat {
                    content: text(&mut r)?,
                    overlay: r.bool()?,
                },
                C::PlayerChat => Clientbound::PlayerChat(Box::new(player_chat(&mut r)?)),
                C::DisguisedChat => Clientbound::DisguisedChat {
                    message: text(&mut r)?,
                    kind: bound_chat_type(&mut r)?,
                },
                C::DeleteChat => Clientbound::DeleteChat {
                    signature: packed_signature(&mut r)?,
                },
                C::SetActionBarText => Clientbound::SetActionBarText {
                    text: text(&mut r)?,
                },
                C::SetTitleText => Clientbound::SetTitleText {
                    text: text(&mut r)?,
                },
                C::SetSubtitleText => Clientbound::SetSubtitleText {
                    text: text(&mut r)?,
                },
                C::SetTitlesAnimation => Clientbound::SetTitlesAnimation {
                    fade_in: r.i32()?,
                    stay: r.i32()?,
                    fade_out: r.i32()?,
                },
                C::ClearTitles => Clientbound::ClearTitles {
                    reset_times: r.bool()?,
                },
                C::BossEvent => Clientbound::BossEvent {
                    id: r.uuid()?,
                    event: boss_event(&mut r)?,
                },
                C::TabList => Clientbound::TabList {
                    header: text(&mut r)?,
                    footer: text(&mut r)?,
                },
                C::LevelEvent => Clientbound::LevelEvent {
                    event: r.i32()?,
                    pos: unpack_block_pos(r.i64()?),
                    data: r.i32()?,
                    global: r.bool()?,
                },
                C::Sound => Clientbound::Sound {
                    event: read_sound_event(&mut r)?,
                    source: r.varint()?,
                    position: [r.i32()?, r.i32()?, r.i32()?],
                    volume: r.f32()?,
                    pitch: r.f32()?,
                    seed: r.i64()?,
                },
                C::SoundEntity => Clientbound::SoundEntity {
                    event: read_sound_event(&mut r)?,
                    source: r.varint()?,
                    entity_id: r.varint()?,
                    volume: r.f32()?,
                    pitch: r.f32()?,
                    seed: r.i64()?,
                },
                C::StopSound => {
                    let flags = r.u8()?;
                    let source = if flags & 1 != 0 {
                        Some(r.varint()?)
                    } else {
                        None
                    };
                    let name = if flags & 2 != 0 {
                        Some(r.identifier()?)
                    } else {
                        None
                    };
                    Clientbound::StopSound { source, name }
                }
                C::Commands => {
                    Clientbound::Commands(Box::new(crate::commands::CommandTree::read(&mut r)?))
                }
                C::TakeItemEntity => Clientbound::TakeItemEntity {
                    item_id: r.varint()?,
                    player_id: r.varint()?,
                    amount: r.varint()?,
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

    /// How many signed chat messages were taken in since the last time
    /// (`ServerboundChatAckPacket`).
    pub fn chat_ack(offset: i32) -> Vec<u8> {
        wire::packet(pl::Serverbound::ChatAck.id(), |o| {
            wire::write_varint(o, offset);
        })
    }

    /// The messages a chat message or a signed command says its sender had seen
    /// (`LastSeenMessages.Update`): how many were taken in since the last update, which
    /// of the 20 last seen are acknowledged (bit `i` of byte `i / 8`, a `BitSet`), and a
    /// checksum of their signatures (0: not checked).
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
    pub struct LastSeenUpdate {
        pub offset: i32,
        pub acknowledged: [u8; 3],
        pub checksum: u8,
    }

    impl LastSeenUpdate {
        fn write(&self, o: &mut Vec<u8>) {
            wire::write_varint(o, self.offset);
            o.extend_from_slice(&self.acknowledged);
            o.push(self.checksum);
        }
    }

    /// `ServerboundChatPacket`: a chat message, when it was sent (milliseconds of the
    /// epoch), its salt and its signature, if the player has a chat session.
    pub fn chat(
        message: &str,
        time: i64,
        salt: i64,
        signature: Option<&[u8; SIGNATURE_BYTES]>,
        last_seen: &LastSeenUpdate,
    ) -> Vec<u8> {
        wire::packet(pl::Serverbound::Chat.id(), |o| {
            wire::write_string(o, message);
            wire::write_i64(o, time);
            wire::write_i64(o, salt);
            wire::write_bool(o, signature.is_some());
            if let Some(s) = signature {
                o.extend_from_slice(s);
            }
            last_seen.write(o);
        })
    }

    /// `ServerboundChatCommandPacket`: a command (without its slash) with nothing in it
    /// to sign.
    pub fn chat_command(command: &str) -> Vec<u8> {
        wire::packet(pl::Serverbound::ChatCommand.id(), |o| {
            wire::write_string(o, command)
        })
    }

    /// `ServerboundChatCommandSignedPacket`: a command with arguments the server takes as
    /// chat, each signed by name (none without a chat session).
    pub fn chat_command_signed(
        command: &str,
        time: i64,
        salt: i64,
        arguments: &[(String, Box<[u8; SIGNATURE_BYTES]>)],
        last_seen: &LastSeenUpdate,
    ) -> Vec<u8> {
        wire::packet(pl::Serverbound::ChatCommandSigned.id(), |o| {
            wire::write_string(o, command);
            wire::write_i64(o, time);
            wire::write_i64(o, salt);
            wire::write_varint(o, arguments.len() as i32);
            for (name, signature) in arguments {
                wire::write_string(o, name);
                o.extend_from_slice(&signature[..]);
            }
            last_seen.write(o);
        })
    }

    /// `ServerboundChatSessionUpdatePacket`: the player's chat session, its public key
    /// (X.509 DER), when the key expires (milliseconds of the epoch) and Mojang's
    /// signature over it.
    pub fn chat_session_update(
        session_id: u128,
        expires_at: i64,
        public_key: &[u8],
        key_signature: &[u8],
    ) -> Vec<u8> {
        wire::packet(pl::Serverbound::ChatSessionUpdate.id(), |o| {
            wire::write_uuid(o, session_id);
            wire::write_i64(o, expires_at);
            wire::write_byte_array(o, public_key);
            wire::write_byte_array(o, key_signature);
        })
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

    /// `ServerboundSetCreativeModeSlotPacket`: the inventory menu's slot as a short (-1:
    /// dropped), then the stack (`ItemStack.OPTIONAL_UNTRUSTED_STREAM_CODEC`). A stack the
    /// patch can't be written for goes out without its patch's values (none of the
    /// client's own).
    pub fn set_creative_mode_slot(slot: i16, stack: &crate::item::ItemStack) -> Vec<u8> {
        wire::packet(pl::Serverbound::SetCreativeModeSlot.id(), |o| {
            o.extend_from_slice(&slot.to_be_bytes());
            let mut body = Vec::new();
            if stack.write_untrusted(&mut body).is_err() {
                body.clear();
                crate::item::ItemStack {
                    patch: crate::item::ComponentPatch::default(),
                    ..stack.clone()
                }
                .write_untrusted(&mut body)
                .ok();
            }
            o.extend_from_slice(&body);
        })
    }

    /// `ServerboundContainerClosePacket`.
    pub fn container_close(container_id: i32) -> Vec<u8> {
        wire::packet(pl::Serverbound::ContainerClose.id(), |o| {
            wire::write_varint(o, container_id)
        })
    }

    /// `ServerboundContainerSlotStateChangedPacket`: a crafter's slot switched on or off
    /// (the slot first, then the container).
    pub fn container_slot_state_changed(slot: i32, container_id: i32, enabled: bool) -> Vec<u8> {
        wire::packet(pl::Serverbound::ContainerSlotStateChanged.id(), |o| {
            wire::write_varint(o, slot);
            wire::write_varint(o, container_id);
            wire::write_bool(o, enabled);
        })
    }

    /// `ServerboundSelectTradePacket`: the merchant's offer chosen, by its place in the
    /// list.
    pub fn select_trade(offer: i32) -> Vec<u8> {
        wire::packet(pl::Serverbound::SelectTrade.id(), |o| {
            wire::write_varint(o, offer)
        })
    }

    /// `ServerboundRenameItemPacket`: the name typed into an anvil's box.
    pub fn rename_item(name: &str) -> Vec<u8> {
        wire::packet(pl::Serverbound::RenameItem.id(), |o| {
            wire::write_string(o, name)
        })
    }

    /// `ServerboundContainerButtonClickPacket`: a menu's button (a stonecutter's recipe,
    /// a loom's pattern, an enchanting table's offer...), by the menu's own numbering.
    pub fn container_button_click(container_id: i32, button_id: i32) -> Vec<u8> {
        wire::packet(pl::Serverbound::ContainerButtonClick.id(), |o| {
            wire::write_varint(o, container_id);
            wire::write_varint(o, button_id);
        })
    }

    /// `ServerboundContainerClickPacket`: the menu, its state id, the slot (a short),
    /// the button (a byte), the `ContainerInput` id, the slots the click changed with
    /// their new stacks hashed, and the stack on the cursor hashed. `changed` is in slot
    /// order, as `handleContainerInput` puts them into its `Int2ObjectOpenHashMap`; they
    /// are written in the order that map iterates them ([`int_map_order`]).
    pub fn container_click(
        container_id: i32,
        state_id: i32,
        slot: i16,
        button: i8,
        input: i32,
        changed: &[(i16, Option<crate::hashed::HashedStack>)],
        carried: Option<&crate::hashed::HashedStack>,
    ) -> Vec<u8> {
        use crate::hashed::HashedStack;
        wire::packet(pl::Serverbound::ContainerClick.id(), |o| {
            wire::write_varint(o, container_id);
            wire::write_varint(o, state_id);
            o.extend_from_slice(&slot.to_be_bytes());
            o.push(button as u8);
            wire::write_varint(o, input);
            let keys: Vec<i32> = changed.iter().map(|(k, _)| i32::from(*k)).collect();
            wire::write_varint(o, changed.len() as i32);
            for i in int_map_order(&keys) {
                let (k, stack) = &changed[i];
                o.extend_from_slice(&k.to_be_bytes());
                HashedStack::write(stack.as_ref(), o);
            }
            HashedStack::write(carried, o);
        })
    }

    /// `ServerboundContainerButtonClickPacket`: a menu's button (an enchanting table's
    /// offer, a lectern's page buttons...), by the id the menu's `clickMenuButton` reads.
    pub fn container_button_click(container_id: i32, button_id: i32) -> Vec<u8> {
        wire::packet(pl::Serverbound::ContainerButtonClick.id(), |o| {
            wire::write_varint(o, container_id);
            wire::write_varint(o, button_id);
        })
    }

    /// `ServerboundSetBeaconPacket`: the beacon's primary and secondary effects, each a
    /// `minecraft:mob_effect` registry id if chosen (`MobEffect.STREAM_CODEC`, optional).
    pub fn set_beacon(primary: Option<i32>, secondary: Option<i32>) -> Vec<u8> {
        wire::packet(pl::Serverbound::SetBeacon.id(), |o| {
            for effect in [primary, secondary] {
                o.push(u8::from(effect.is_some()));
                if let Some(id) = effect {
                    wire::write_varint(o, id);
                }
            }
        })
    }

    /// The order fastutil's `Int2ObjectOpenHashMap` (default size 16, load factor 0.75)
    /// iterates distinct keys put into it in the given order: the key 0 first (kept
    /// apart), then the table from its last position down. A key's place is
    /// `HashCommon.mix(key) & mask`, probing upwards; the table doubles (`rehash`,
    /// which moves the keys from the top down) once it holds more than three quarters.
    /// Returns indices into `keys`.
    pub fn int_map_order(keys: &[i32]) -> Vec<usize> {
        fn mix(x: i32) -> u32 {
            let h = (x as u32).wrapping_mul(0x9E37_79B9);
            h ^ (h >> 16)
        }
        fn array_size(expected: usize) -> usize {
            // `HashCommon.arraySize(expected, 0.75f)`: the next power of two of
            // ceil(expected / 0.75), at least 2.
            let s = (expected as f64 / 0.75).ceil() as usize;
            s.next_power_of_two().max(2)
        }
        fn max_fill(n: usize) -> usize {
            ((n as f64 * 0.75).ceil() as usize).min(n - 1)
        }
        fn place(table: &mut [Option<usize>], mask: usize, key: i32, index: usize) {
            let mut pos = mix(key) as usize & mask;
            while table[pos].is_some() {
                pos = (pos + 1) & mask;
            }
            table[pos] = Some(index);
        }
        let mut n = array_size(16);
        let mut table: Vec<Option<usize>> = vec![None; n];
        let mut null_key = None;
        // (Each key is new: the map holds as many as went in before.)
        for (index, &key) in keys.iter().enumerate() {
            let size = index;
            if key == 0 {
                null_key = Some(index);
            } else {
                place(&mut table, n - 1, key, index);
            }
            if size >= max_fill(n) {
                let new_n = array_size(size + 2);
                let mut new_table = vec![None; new_n];
                for &i in table.iter().rev().flatten() {
                    place(&mut new_table, new_n - 1, keys[i], i);
                }
                n = new_n;
                table = new_table;
            }
        }
        null_key
            .into_iter()
            .chain(table.iter().rev().flatten().copied())
            .collect()
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
    pub const EQUIPMENT_FEET: u8 = 2;
    pub const EQUIPMENT_LEGS: u8 = 3;
    pub const EQUIPMENT_CHEST: u8 = 4;
    pub const EQUIPMENT_HEAD: u8 = 5;
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

    /// `ServerboundMovePlayerPacket.Rot`: the player's rotation, with its ground and
    /// horizontal collision flags.
    pub fn move_player_rot(
        y_rot: f32,
        x_rot: f32,
        on_ground: bool,
        horizontal_collision: bool,
    ) -> Vec<u8> {
        wire::packet(pl::Serverbound::MovePlayerRot.id(), |o| {
            wire::write_f32(o, y_rot);
            wire::write_f32(o, x_rot);
            o.push(u8::from(on_ground) | u8::from(horizontal_collision) << 1);
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

    fn nbt_text(o: &mut Vec<u8>, s: &str) -> Tag {
        let tag = Tag::String(s.to_owned());
        crate::nbt::write_network(o, &tag).unwrap();
        tag
    }

    #[test]
    fn play_reads_chat() {
        use play::{ChatParameter, ChatTypeRef, Clientbound, FilterMask, PackedSignature};
        let id = |k: pl::Clientbound| k.id();
        let mut b = Vec::new();
        let hello = nbt_text(&mut b, "hello");
        b.push(1);
        assert_eq!(
            Clientbound::decode(id(pl::Clientbound::SystemChat), &b).unwrap(),
            Clientbound::SystemChat {
                content: hello.clone(),
                overlay: true
            }
        );
        // A player's, with everything there can be.
        let mut b = Vec::new();
        wire::write_varint(&mut b, 5);
        wire::write_uuid(&mut b, 77);
        wire::write_varint(&mut b, 2);
        b.push(1);
        b.extend([9; 256]);
        wire::write_string(&mut b, "hi there");
        wire::write_i64(&mut b, 1234);
        wire::write_i64(&mut b, -1);
        wire::write_varint(&mut b, 2);
        wire::write_varint(&mut b, 4);
        wire::write_varint(&mut b, 0);
        b.extend([3; 256]);
        b.push(1);
        nbt_text(&mut b, "hi *****");
        wire::write_varint(&mut b, 2);
        wire::write_byte_array(&mut b, &[0b1110_0000]);
        // The registry's first type, and a name without a target.
        wire::write_varint(&mut b, 1);
        let name = nbt_text(&mut b, "Kai");
        b.push(0);
        let Clientbound::PlayerChat(chat) =
            Clientbound::decode(id(pl::Clientbound::PlayerChat), &b).unwrap()
        else {
            panic!()
        };
        assert_eq!((chat.global_index, chat.sender, chat.time), (5, 77, 1234));
        assert_eq!(chat.signature, Some(Box::new([9; 256])));
        assert_eq!(chat.content, "hi there");
        assert_eq!(
            chat.last_seen,
            [
                PackedSignature::Cached(3),
                PackedSignature::Full(Box::new([3; 256]))
            ]
        );
        assert_eq!(chat.unsigned_content, Some(Tag::String("hi *****".into())));
        assert_eq!(chat.filter, FilterMask::Partly(vec![0b1110_0000]));
        assert_eq!(chat.kind.kind, ChatTypeRef::Id(0));
        assert_eq!((&chat.kind.name, &chat.kind.target), (&name, &None));
        // A message no player wrote, of a kind that is written out, with a target.
        let mut b = Vec::new();
        nbt_text(&mut b, "hello");
        wire::write_varint(&mut b, 0);
        for key in ["chat.type.team.text", "chat.type.text.narrate"] {
            wire::write_string(&mut b, key);
            wire::write_varint(&mut b, 3);
            for p in [1, 0, 2] {
                wire::write_varint(&mut b, p);
            }
            crate::nbt::write_network(&mut b, &Tag::Compound(crate::nbt::Compound::new())).unwrap();
        }
        nbt_text(&mut b, "Kai");
        b.push(1);
        let team = nbt_text(&mut b, "Red");
        let Clientbound::DisguisedChat { message, kind } =
            Clientbound::decode(id(pl::Clientbound::DisguisedChat), &b).unwrap()
        else {
            panic!()
        };
        assert_eq!((message, kind.target), (hello, Some(team)));
        let ChatTypeRef::Direct(d) = kind.kind else {
            panic!()
        };
        assert_eq!(d.translation_key, "chat.type.team.text");
        assert_eq!(
            d.parameters,
            [
                ChatParameter::Target,
                ChatParameter::Sender,
                ChatParameter::Content
            ]
        );
        let mut b = Vec::new();
        wire::write_varint(&mut b, 128);
        assert_eq!(
            Clientbound::decode(id(pl::Clientbound::DeleteChat), &b).unwrap(),
            Clientbound::DeleteChat {
                signature: PackedSignature::Cached(127)
            }
        );
        assert_eq!(body(&play::chat_ack(65)), (6, &[65][..]));
    }

    /// What the game's own codecs write.
    #[test]
    fn play_reads_what_the_game_writes_of_the_chat() {
        use play::{ChatParameter, ChatTypeRef, Clientbound, FilterMask, PackedSignature};
        fn bytes(hex: &str) -> Vec<u8> {
            hex.as_bytes()
                .chunks(2)
                .map(|c| u8::from_str_radix(std::str::from_utf8(c).unwrap(), 16).unwrap())
                .collect()
        }
        let read =
            |kind: pl::Clientbound, hex: &str| Clientbound::decode(kind.id(), &bytes(hex)).unwrap();
        let (nines, sevens) = ("09".repeat(256), "07".repeat(256));
        let hex = format!(
            "ac0200000000000000010000000000000002040\
             1{nines}0b68656c6c6f207468657265000000000012d687000000000000002a\
             020600{sevens}010800026869\
             02020202\
             020800034b61690108000352\
             6564"
        );
        let Clientbound::PlayerChat(chat) = read(pl::Clientbound::PlayerChat, &hex) else {
            panic!()
        };
        assert_eq!((chat.global_index, chat.sender), (300, 1 << 64 | 2));
        assert_eq!(chat.signature, Some(Box::new([9; 256])));
        assert_eq!(
            (chat.content.as_str(), chat.time),
            ("hello there", 1_234_567)
        );
        assert_eq!(
            chat.last_seen,
            [
                PackedSignature::Cached(5),
                PackedSignature::Full(Box::new([7; 256]))
            ]
        );
        assert_eq!(chat.unsigned_content, Some(Tag::String("hi".into())));
        // The second and the tenth character.
        assert_eq!(chat.filter, FilterMask::Partly(vec![2, 2]));
        assert_eq!(chat.kind.kind, ChatTypeRef::Id(1));
        assert_eq!(chat.kind.name, Tag::String("Kai".into()));
        assert_eq!(chat.kind.target, Some(Tag::String("Red".into())));
        let Clientbound::PlayerChat(chat) = read(
            pl::Clientbound::PlayerChat,
            "00000000000000000000000000000000030000017800000000000000050000000000000000\
             0000010003782e790201020a0100066974616c696301000f636861742e747970652e656d6f\
             74650200020a000800034b616900",
        ) else {
            panic!()
        };
        assert_eq!(
            (chat.signature, chat.filter),
            (None, FilterMask::FullyFiltered)
        );
        let ChatTypeRef::Direct(d) = chat.kind.kind else {
            panic!()
        };
        assert_eq!(d.translation_key, "x.y");
        assert_eq!(
            d.parameters,
            [ChatParameter::Target, ChatParameter::Content]
        );
        let style = d.style.unwrap();
        assert_eq!(style.as_compound().unwrap().boolean("italic"), Some(true));
        assert_eq!(
            read(pl::Clientbound::DeleteChat, "12"),
            Clientbound::DeleteChat {
                signature: PackedSignature::Cached(17)
            }
        );
        assert_eq!(
            read(pl::Clientbound::SystemChat, "0800017301"),
            Clientbound::SystemChat {
                content: Tag::String("s".into()),
                overlay: true
            }
        );
        assert_eq!(
            read(
                pl::Clientbound::LevelEvent,
                "000003f20000004000003ffe0000000701"
            ),
            Clientbound::LevelEvent {
                event: 1010,
                pos: [1, -2, 3],
                data: 7,
                global: true
            }
        );
        assert_eq!(
            read(pl::Clientbound::TabList, "080003746f70080000"),
            Clientbound::TabList {
                header: Tag::String("top".into()),
                footer: Tag::String(String::new())
            }
        );
        assert_eq!(
            read(
                pl::Clientbound::SetTitlesAnimation,
                "00000005ffffffff00000014"
            ),
            Clientbound::SetTitlesAnimation {
                fade_in: 5,
                stay: -1,
                fade_out: 20
            }
        );
    }

    #[test]
    fn play_reads_titles_boss_bars_and_the_tab_list() {
        use play::{BossEvent, Clientbound};
        let mut b = Vec::new();
        for v in [5, -1, 20] {
            wire::write_i32(&mut b, v);
        }
        assert_eq!(
            Clientbound::decode(pl::Clientbound::SetTitlesAnimation.id(), &b).unwrap(),
            Clientbound::SetTitlesAnimation {
                fade_in: 5,
                stay: -1,
                fade_out: 20
            }
        );
        let boss = |write: &dyn Fn(&mut Vec<u8>)| {
            let mut b = Vec::new();
            wire::write_uuid(&mut b, 3);
            write(&mut b);
            match Clientbound::decode(pl::Clientbound::BossEvent.id(), &b) {
                Ok(Clientbound::BossEvent { id: 3, event }) => Ok(event),
                Ok(other) => panic!("{other:?}"),
                Err(e) => Err(e),
            }
        };
        assert_eq!(
            boss(&|b| {
                wire::write_varint(b, 0);
                nbt_text(b, "Raid");
                wire::write_f32(b, 0.5);
                wire::write_varint(b, 2);
                wire::write_varint(b, 4);
                b.push(5);
            }),
            Ok(BossEvent::Add {
                name: Tag::String("Raid".into()),
                progress: 0.5,
                color: 2,
                overlay: 4,
                flags: 5
            })
        );
        assert_eq!(boss(&|b| b.push(1)), Ok(BossEvent::Remove));
        assert_eq!(
            boss(&|b| {
                b.push(2);
                wire::write_f32(b, 0.25);
            }),
            Ok(BossEvent::Progress(0.25))
        );
        assert_eq!(
            boss(&|b| {
                b.push(3);
                nbt_text(b, "x");
            }),
            Ok(BossEvent::Name(Tag::String("x".into())))
        );
        assert_eq!(
            boss(&|b| b.extend([4, 6, 1])),
            Ok(BossEvent::Style {
                color: 6,
                overlay: 1
            })
        );
        assert_eq!(
            boss(&|b| b.extend([5, 3])),
            Ok(BossEvent::Properties { flags: 3 })
        );
        assert!(boss(&|b| b.extend([4, 7, 0])).is_err());
        assert!(boss(&|b| b.extend([4, 0, 5])).is_err());
        let mut b = Vec::new();
        let (header, footer) = (nbt_text(&mut b, "top"), nbt_text(&mut b, ""));
        assert_eq!(
            Clientbound::decode(pl::Clientbound::TabList.id(), &b).unwrap(),
            Clientbound::TabList { header, footer }
        );
        // A level event: where, what with, and whether it is heard everywhere.
        let mut b = Vec::new();
        wire::write_i32(&mut b, 1010);
        wire::write_i64(&mut b, 0);
        wire::write_i32(&mut b, 4);
        b.push(0);
        assert_eq!(
            Clientbound::decode(pl::Clientbound::LevelEvent.id(), &b).unwrap(),
            Clientbound::LevelEvent {
                event: 1010,
                pos: [0; 3],
                data: 4,
                global: false
            }
        );
    }

    #[test]
    fn play_reads_what_the_player_list_shows() {
        let mut b = vec![0b1111_1000];
        wire::write_varint(&mut b, 2);
        for (uuid, named) in [(1, true), (2, false)] {
            wire::write_uuid(&mut b, uuid);
            wire::write_bool(&mut b, named);
            wire::write_varint(&mut b, 250);
            wire::write_bool(&mut b, named);
            if named {
                nbt_text(&mut b, "Boss");
            }
            wire::write_varint(&mut b, 7);
            wire::write_bool(&mut b, !named);
        }
        let play::Clientbound::PlayerInfoUpdate { entries, .. } =
            play::Clientbound::decode(pl::Clientbound::PlayerInfoUpdate.id(), &b).unwrap()
        else {
            panic!()
        };
        let e = &entries[0];
        assert_eq!((e.listed, e.latency), (Some(true), Some(250)));
        assert_eq!(e.display_name, Some(Some(Tag::String("Boss".into()))));
        assert_eq!((e.list_order, e.show_hat), (Some(7), Some(false)));
        let e = &entries[1];
        assert_eq!((e.listed, &e.display_name), (Some(false), &Some(None)));
        assert_eq!((e.name.as_ref(), e.game_mode), (None, None));
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
    fn chunks_keep_heightmaps_block_entities_and_light() {
        let mut sign = crate::nbt::Compound::new();
        sign.put("is_waxed", crate::nbt::Tag::Byte(1));
        let sign = crate::nbt::Tag::Compound(sign);
        let mut b = Vec::new();
        wire::write_i32(&mut b, 3);
        wire::write_i32(&mut b, -7);
        // One heightmap (motion blocking) of two longs.
        wire::write_varint(&mut b, 1);
        wire::write_varint(&mut b, 4);
        wire::write_varint(&mut b, 2);
        wire::write_i64(&mut b, 0x0102);
        wire::write_i64(&mut b, -1);
        wire::write_byte_array(&mut b, &[9, 8, 7]);
        // Two block entities: one with data, one without (the end tag).
        wire::write_varint(&mut b, 2);
        b.extend([0xA5]);
        wire::write_u16(&mut b, (-12_i16) as u16);
        wire::write_varint(&mut b, 7);
        crate::nbt::write_network(&mut b, &sign).unwrap();
        b.extend([0x10]);
        wire::write_u16(&mut b, 70);
        wire::write_varint(&mut b, 1);
        b.push(0);
        // Light: sky sections 1 and 3, block section 2, sky section 0 empty.
        // (Each mask as the bytes of a `BitSet`; an empty one has none.)
        let mut light = Vec::new();
        for mask in [&[0b1010_u8][..], &[0b100], &[0b1], &[]] {
            wire::write_byte_array(&mut light, mask);
        }
        wire::write_varint(&mut light, 2);
        wire::write_byte_array(&mut light, &[0x11; play::LIGHT_BYTES]);
        wire::write_byte_array(&mut light, &[0x22; play::LIGHT_BYTES]);
        wire::write_varint(&mut light, 1);
        wire::write_byte_array(&mut light, &[0x33; play::LIGHT_BYTES]);
        b.extend(&light);
        let play::Clientbound::LevelChunk {
            x,
            z,
            heightmaps,
            data,
            block_entities,
            light: l,
        } = play::Clientbound::decode(46, &b).unwrap()
        else {
            panic!("not a chunk");
        };
        assert_eq!((x, z), (3, -7));
        assert_eq!(heightmaps, vec![(4, vec![0x0102, -1])]);
        assert_eq!(data, [9, 8, 7]);
        assert_eq!(
            block_entities,
            vec![
                play::ChunkBlockEntity {
                    xz: 0xA5,
                    y: -12,
                    kind: 7,
                    tag: Some(sign.clone()),
                },
                play::ChunkBlockEntity {
                    xz: 0x10,
                    y: 70,
                    kind: 1,
                    tag: None,
                },
            ]
        );
        assert!(play::LightData::bit(&l.sky_mask, 1) && play::LightData::bit(&l.sky_mask, 3));
        assert!(!play::LightData::bit(&l.sky_mask, 2) && !play::LightData::bit(&l.sky_mask, 64));
        assert!(play::LightData::bit(&l.empty_sky_mask, 0));
        assert!(l.empty_block_mask.is_empty());
        // A mask longer than a byte, as a tall level's: bit 9 is in the second byte,
        // bit 70 in the ninth.
        let mut tall = Vec::new();
        wire::write_varint(&mut tall, 0);
        wire::write_varint(&mut tall, 0);
        wire::write_byte_array(&mut tall, &[0, 0b10, 0, 0, 0, 0, 0, 0, 0b0100_0000]);
        for _ in 0..3 {
            wire::write_byte_array(&mut tall, &[]);
        }
        wire::write_varint(&mut tall, 2);
        wire::write_byte_array(&mut tall, &[0; play::LIGHT_BYTES]);
        wire::write_byte_array(&mut tall, &[0; play::LIGHT_BYTES]);
        wire::write_varint(&mut tall, 0);
        let play::Clientbound::LightUpdate { light: t, .. } =
            play::Clientbound::decode(49, &tall).unwrap()
        else {
            panic!("not a light update");
        };
        assert!(play::LightData::bit(&t.sky_mask, 9) && play::LightData::bit(&t.sky_mask, 70));
        assert_eq!(t.sky_mask.iter().map(|w| w.count_ones()).sum::<u32>(), 2);
        assert_eq!((l.sky.len(), l.block.len()), (2, 1));
        assert_eq!((l.sky[1][0], l.block[0][2047]), (0x22, 0x33));

        // The same light as an update of its own.
        let mut u = Vec::new();
        wire::write_varint(&mut u, 3);
        wire::write_varint(&mut u, -7);
        u.extend(&light);
        assert_eq!(
            play::Clientbound::decode(49, &u).unwrap(),
            play::Clientbound::LightUpdate {
                x: 3,
                z: -7,
                light: l,
            }
        );
        // A light array of the wrong size is refused.
        let mut bad = Vec::new();
        wire::write_varint(&mut bad, 0);
        wire::write_varint(&mut bad, 0);
        for _ in 0..4 {
            wire::write_varint(&mut bad, 0);
        }
        wire::write_varint(&mut bad, 1);
        wire::write_byte_array(&mut bad, &[0; 100]);
        wire::write_varint(&mut bad, 0);
        assert!(play::Clientbound::decode(49, &bad).is_err());

        // A block entity's data, and the time with one clock.
        let mut e = Vec::new();
        let (px, py, pz) = (-3_i64, 64_i64, 100_i64);
        wire::write_i64(
            &mut e,
            (px & 0x3FF_FFFF) << 38 | (pz & 0x3FF_FFFF) << 12 | (py & 0xFFF),
        );
        wire::write_varint(&mut e, 7);
        crate::nbt::write_network(&mut e, &sign).unwrap();
        assert_eq!(
            play::Clientbound::decode(6, &e).unwrap(),
            play::Clientbound::BlockEntityData {
                pos: [-3, 64, 100],
                kind: 7,
                tag: Some(sign),
            }
        );
        let mut t = Vec::new();
        wire::write_i64(&mut t, 123_456);
        wire::write_varint(&mut t, 1);
        wire::write_varint(&mut t, 0);
        wire::write_varint(&mut t, 255); // the clock's ticks, a varlong
        wire::write_f32(&mut t, 0.25);
        wire::write_f32(&mut t, 1.0);
        assert_eq!(
            play::Clientbound::decode(115, &t).unwrap(),
            play::Clientbound::SetTime {
                game_time: 123_456,
                clocks: vec![(
                    0,
                    play::ClockState {
                        total_ticks: 255,
                        partial_tick: 0.25,
                        rate: 1.0,
                    }
                )],
            }
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
    fn play_reads_the_scoreboard() {
        use play::{Clientbound as C, NumberFormat};
        let text = |b: &mut Vec<u8>, s: &str| {
            crate::nbt::write_network(b, &Tag::String(s.into())).unwrap();
        };
        let said = |s: &str| Tag::String(s.into());
        // An objective: its name and the method; one that is made or changed says what
        // it is called, how a list shows it, and how its numbers are written.
        let objective = |kind: i32| {
            let mut b = Vec::new();
            wire::write_string(&mut b, "hp");
            b.push(2);
            text(&mut b, "Health");
            wire::write_varint(&mut b, 1);
            wire::write_bool(&mut b, true);
            wire::write_varint(&mut b, kind);
            if kind != 0 {
                text(&mut b, "s");
            }
            C::decode(108, &b)
        };
        let formats = [
            NumberFormat::Blank,
            NumberFormat::Styled(said("s")),
            NumberFormat::Fixed(said("s")),
        ];
        for (kind, format) in formats.into_iter().enumerate() {
            assert_eq!(
                objective(kind as i32).unwrap(),
                C::SetObjective {
                    name: "hp".into(),
                    method: 2,
                    display_name: Some(said("Health")),
                    hearts: true,
                    number_format: Some(format),
                }
            );
        }
        assert!(objective(3).is_err());
        let mut b = Vec::new();
        wire::write_string(&mut b, "hp");
        b.push(1);
        assert_eq!(
            C::decode(108, &b).unwrap(),
            C::SetObjective {
                name: "hp".into(),
                method: 1,
                display_name: None,
                hearts: false,
                number_format: None,
            }
        );
        // The slot below names is the third.
        let mut b = Vec::new();
        wire::write_varint(&mut b, 2);
        wire::write_string(&mut b, "hp");
        assert_eq!(
            C::decode(100, &b).unwrap(),
            C::SetDisplayObjective {
                slot: play::DISPLAY_SLOT_BELOW_NAME,
                objective: "hp".into(),
            }
        );
        // A score, with what it is shown as, and no way of writing of its own.
        let mut b = Vec::new();
        wire::write_string(&mut b, "Kai");
        wire::write_string(&mut b, "hp");
        wire::write_varint(&mut b, 17);
        wire::write_bool(&mut b, true);
        text(&mut b, "seventeen");
        wire::write_bool(&mut b, false);
        assert_eq!(
            C::decode(112, &b).unwrap(),
            C::SetScore {
                owner: "Kai".into(),
                objective: "hp".into(),
                score: 17,
                display: Some(said("seventeen")),
                number_format: None,
            }
        );
        // One score reset, and all of an owner's.
        let reset = |objective: Option<&str>| {
            let mut b = Vec::new();
            wire::write_string(&mut b, "Kai");
            wire::write_bool(&mut b, objective.is_some());
            if let Some(o) = objective {
                wire::write_string(&mut b, o);
            }
            C::decode(80, &b).unwrap()
        };
        for objective in [Some("hp"), None] {
            assert_eq!(
                reset(objective),
                C::ResetScore {
                    owner: "Kai".into(),
                    objective: objective.map(str::to_owned),
                }
            );
        }
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
    fn play_reads_health_and_experience() {
        // Health as a float, the food as a number, its saturation as a float.
        let mut b = 13.5f32.to_be_bytes().to_vec();
        wire::write_varint(&mut b, 17);
        b.extend(2.25f32.to_be_bytes());
        assert_eq!(
            play::Clientbound::decode(pl::Clientbound::SetHealth.id(), &b).unwrap(),
            play::Clientbound::SetHealth {
                health: 13.5,
                food: 17,
                saturation: 2.25,
            }
        );
        // How far the bar is filled, then the level, then all the points: not in the
        // order the packet's constructor takes them.
        let mut b = 0.25f32.to_be_bytes().to_vec();
        wire::write_varint(&mut b, 7);
        wire::write_varint(&mut b, 300);
        assert_eq!(b, [0x3E, 0x80, 0, 0, 7, 0xAC, 2]);
        assert_eq!(
            play::Clientbound::decode(pl::Clientbound::SetExperience.id(), &b).unwrap(),
            play::Clientbound::SetExperience {
                progress: 0.25,
                level: 7,
                total: 300,
            }
        );
        assert!(play::Clientbound::decode(pl::Clientbound::SetExperience.id(), &b[..5]).is_err());
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
                    ..Default::default()
                }],
            }
        );
    }

    #[test]
    fn play_reads_how_entities_move_their_parts() {
        let decode =
            |kind: pl::Clientbound, b: &[u8]| play::Clientbound::decode(kind.id(), b).unwrap();
        assert_eq!(
            decode(pl::Clientbound::RotateHead, &[5, 0xC0]),
            play::Clientbound::RotateHead {
                entity_id: 5,
                y_head_rot: -64
            }
        );
        assert_eq!(
            decode(pl::Clientbound::SwingAnimation, &[5, 1, 2, 13]),
            play::Clientbound::SwingAnimation {
                entity_id: 5,
                off_hand: true,
                animation: 2,
                duration: 13
            }
        );
        let hurt = play::Clientbound::DamageEvent {
            entity_id: 5,
            source_type: 1,
        };
        assert_eq!(decode(pl::Clientbound::DamageEvent, &[5, 1, 8, 0, 0]), hurt);
        let mut from = vec![5, 1, 0, 0, 1];
        for c in [1.0, 2.0, 3.0] {
            wire::write_f64(&mut from, c);
        }
        assert_eq!(decode(pl::Clientbound::DamageEvent, &from), hurt);
        let mut b = vec![5];
        wire::write_f32(&mut b, 90.0);
        assert_eq!(
            decode(pl::Clientbound::HurtAnimation, &b),
            play::Clientbound::HurtAnimation {
                entity_id: 5,
                yaw: 90.0
            }
        );
        // A block being broken: who breaks it, where, and how far.
        let mut b = vec![7];
        wire::write_i64(&mut b, play::pack_block_pos([3, -60, -8]));
        b.push(9);
        assert_eq!(
            decode(pl::Clientbound::BlockDestruction, &b),
            play::Clientbound::BlockDestruction {
                id: 7,
                pos: [3, -60, -8],
                progress: 9
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

    #[test]
    fn changed_slots_go_in_fastutils_order() {
        // `Int2ObjectOpenHashMap`'s iteration of keys put in in order, from vanilla's
        // own fastutil (the 26.3 libraries), across its first two rehashes.
        let cases: [(Vec<i32>, &[i32]); 7] = [
            (vec![0, 1, 2, 3], &[0, 2, 1, 3]),
            (
                (0..46).collect(),
                &[
                    0, 33, 29, 25, 37, 4, 12, 13, 8, 9, 30, 28, 24, 26, 18, 16, 17, 19, 23, 20, 22,
                    21, 35, 39, 2, 27, 6, 15, 14, 31, 11, 10, 32, 1, 3, 34, 7, 38, 36, 5, 45, 44,
                    42, 41, 40, 43,
                ],
            ),
            (
                (5..63).collect(),
                &[
                    33, 58, 25, 49, 50, 8, 9, 57, 24, 59, 26, 16, 17, 23, 18, 35, 60, 39, 56, 27,
                    47, 48, 51, 55, 31, 52, 32, 34, 36, 38, 46, 40, 41, 45, 44, 42, 43, 29, 37, 54,
                    12, 13, 53, 61, 30, 28, 19, 62, 21, 22, 20, 6, 15, 14, 11, 10, 7, 5,
                ],
            ),
            (vec![44, 3, 17, 0, 9], &[0, 9, 3, 17, 44]),
            (
                (1..25).collect(),
                &[
                    2, 6, 4, 15, 14, 12, 13, 8, 9, 11, 10, 1, 3, 24, 7, 23, 5, 16, 17, 19, 18, 22,
                    21, 20,
                ],
            ),
            (
                (0..24).collect(),
                &[
                    0, 2, 6, 4, 15, 14, 12, 13, 8, 9, 11, 10, 1, 3, 7, 23, 5, 16, 17, 19, 18, 22,
                    21, 20,
                ],
            ),
            (
                (0..25).collect(),
                &[
                    0, 4, 12, 13, 8, 9, 24, 18, 16, 17, 19, 23, 20, 22, 21, 2, 6, 15, 14, 11, 10,
                    1, 3, 7, 5,
                ],
            ),
        ];
        for (keys, want) in cases {
            let got: Vec<i32> = play::int_map_order(&keys)
                .into_iter()
                .map(|i| keys[i])
                .collect();
            assert_eq!(got, want, "{keys:?}");
        }
    }

    #[test]
    fn writes_the_menu_buttons_and_the_beacons_effects() {
        // An enchanting table's third offer in menu 2; a lectern's page 7 (`100 + page`).
        assert_eq!(
            play::container_button_click(2, 2),
            [pl::Serverbound::ContainerButtonClick.id() as u8, 2, 2]
        );
        assert_eq!(play::container_button_click(1, 107)[1..], [1, 107]);
        // Haste (registry id 2) and no secondary; then both, the second id over a byte.
        assert_eq!(
            play::set_beacon(Some(2), None),
            [pl::Serverbound::SetBeacon.id() as u8, 1, 2, 0]
        );
        assert_eq!(
            play::set_beacon(Some(9), Some(200))[1..],
            [1, 9, 1, 0xC8, 1]
        );
        assert_eq!(play::set_beacon(None, None)[1..], [0, 0]);
    }

    #[test]
    fn reads_the_menus_packets() {
        let decode =
            |kind: pl::Clientbound, b: &[u8]| play::Clientbound::decode(kind.id(), b).unwrap();
        // open_screen: container 3, generic_9x3 (2), the title "Chest" as a string tag.
        let mut b = vec![3, 2, 8, 0, 5];
        b.extend_from_slice(b"Chest");
        let play::Clientbound::OpenScreen {
            container_id,
            menu_type,
            title,
        } = decode(pl::Clientbound::OpenScreen, &b)
        else {
            panic!()
        };
        assert_eq!((container_id, menu_type), (3, 2));
        assert_eq!(title, Tag::String("Chest".into()));
        assert_eq!(
            decode(pl::Clientbound::ContainerClose, &[7]),
            play::Clientbound::ContainerClose { container_id: 7 }
        );
        // set_data: shorts, which may be negative.
        assert_eq!(
            decode(pl::Clientbound::ContainerSetData, &[1, 0, 2, 0xFF, 0xFE]),
            play::Clientbound::ContainerSetData {
                container_id: 1,
                id: 2,
                value: -2
            }
        );
        assert_eq!(
            decode(pl::Clientbound::MountScreenOpen, &[4, 3, 0, 0, 1, 0]),
            play::Clientbound::MountScreenOpen {
                container_id: 4,
                inventory_columns: 3,
                entity_id: 256
            }
        );
        // A merchant's offers, then its level, xp, the bar and restocking.
        let mut b = vec![2, 1, 5, 12, 0, 3, 9, 0, 0, 0, 1];
        for v in [12i32, 12, 2, 0] {
            b.extend(v.to_be_bytes());
        }
        b.extend(0.2f32.to_be_bytes());
        b.extend(0i32.to_be_bytes());
        b.extend([3, 70, 1, 0]);
        let play::Clientbound::MerchantOffers {
            container_id,
            offers,
            villager_level,
            villager_xp,
            show_progress,
            can_restock,
        } = decode(pl::Clientbound::MerchantOffers, &b)
        else {
            panic!("not merchant offers");
        };
        assert_eq!((container_id, offers.len()), (2, 1));
        assert!(offers[0].out_of_stock && offers[0].cost_b.is_none());
        assert_eq!((villager_level, villager_xp), (3, 70));
        assert!(show_progress && !can_restock);
        // The recipe sets, then the stonecutter's recipes (one: items 1 and 2 to item 9).
        let mut b = vec![1];
        crate::wire::write_string(&mut b, "minecraft:furnace_input");
        b.extend([1, 4]);
        b.extend([1, 3, 1, 2, 4, 9]);
        let play::Clientbound::UpdateRecipes {
            item_sets,
            stonecutter,
        } = decode(pl::Clientbound::UpdateRecipes, &b)
        else {
            panic!("not update_recipes");
        };
        assert_eq!(item_sets, [("minecraft:furnace_input".to_owned(), vec![4])]);
        assert_eq!(stonecutter.len(), 1);
        assert_eq!(stonecutter[0].display, crate::recipe::SlotDisplay::Item(9));
        // container_close and container_click as vanilla writes them.
        assert_eq!(
            play::container_close(5),
            [pl::Serverbound::ContainerClose.id() as u8, 5]
        );
        assert_eq!(
            play::container_slot_state_changed(4, 2, false),
            [
                pl::Serverbound::ContainerSlotStateChanged.id() as u8,
                4,
                2,
                0
            ]
        );
        assert_eq!(
            play::select_trade(300),
            [pl::Serverbound::SelectTrade.id() as u8, 0xAC, 2]
        );
        assert_eq!(
            play::rename_item("Az"),
            [pl::Serverbound::RenameItem.id() as u8, 2, b'A', b'z']
        );
        assert_eq!(
            play::container_button_click(3, 17),
            [pl::Serverbound::ContainerButtonClick.id() as u8, 3, 17]
        );
        let stone = crate::hashed::HashedStack {
            item: 1,
            count: 3,
            added: Vec::new(),
            removed: Vec::new(),
        };
        let click = play::container_click(
            1,
            41,
            -999,
            1,
            0,
            &[(0, None), (36, Some(stone.clone()))],
            Some(&stone),
        );
        assert_eq!(
            click[1..],
            [
                1, 41, 0xFC, 0x19, 1, 0, 2, 0, 0, 0, 0, 36, 1, 1, 3, 0, 0, 1, 1, 3, 0, 0
            ]
        );
    }
    #[test]
    fn the_servers_rotation_and_the_rotation_packet() {
        let mut b = Vec::new();
        wire::write_f32(&mut b, 45.0);
        wire::write_bool(&mut b, false);
        wire::write_f32(&mut b, -10.0);
        wire::write_bool(&mut b, true);
        assert_eq!(
            play::Clientbound::decode(pl::Clientbound::PlayerRotation.id(), &b).unwrap(),
            play::Clientbound::PlayerRotation {
                y_rot: 45.0,
                relative_y: false,
                x_rot: -10.0,
                relative_x: true,
            }
        );
        let mut want = vec![pl::Serverbound::MovePlayerRot.id() as u8];
        want.extend(45.0f32.to_be_bytes());
        want.extend((-10.0f32).to_be_bytes());
        want.push(0b10);
        assert_eq!(play::move_player_rot(45.0, -10.0, false, true), want);
    }
}

#[cfg(test)]
mod creative_slot_tests {
    use super::play::set_creative_mode_slot;
    use crate::item::ItemStack;
    use crate::wire::Reader;

    #[test]
    fn a_creative_slot_carries_a_delimited_patch() {
        // A stack of 5 with one component (max_stack_size 3: type 1, value 3).
        let bytes = [1u8, 0, 1, 3];
        let mut body = vec![5, 7];
        body.extend_from_slice(&bytes);
        let stack = ItemStack::read_optional(&mut Reader::new(&body)).expect("reads");
        let p = set_creative_mode_slot(36, &stack);
        // Packet id, the slot, count 5, item 7, one added and none removed, type 1 with
        // one byte.
        assert_eq!(&p[1..], &[0, 36, 5, 7, 1, 0, 1, 1, 3]);
        assert_eq!(
            &set_creative_mode_slot(-1, &ItemStack::EMPTY)[1..],
            &[0xFF, 0xFF, 0]
        );
    }
}

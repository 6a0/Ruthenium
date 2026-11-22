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

/// Play: only the packets every phase shares, and the protocol switch, so far.
pub mod play {
    use super::*;

    #[derive(Debug, Clone, PartialEq)]
    pub enum Clientbound {
        Common(Common),
        StartConfiguration,
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
        let p = play::Clientbound::decode(50, &[1, 2, 3]).unwrap();
        assert_eq!(p, play::Clientbound::Other(pl::Clientbound::Login));
        assert!(play::Clientbound::decode(120, &[]).unwrap().is_terminal());
        assert_eq!(play::keep_alive(1)[0], 28);
    }
}

//! What the player sends to the chat (`ChatScreen.handleChatInput`,
//! `ClientPacketListener.sendChat` and `sendCommand`): the message made tidy, the
//! messages the player has seen (`LastSeenMessagesTracker`), and, with a chat session,
//! the signatures (`LocalChatSession`, `SignedMessageChain`, `SignedMessageBody`).
//!
//! A signature is RSA with SHA-256 over what `PlayerChatMessage.updateSignature` lays
//! out: the version (1), the link (sender, session, index), then the body (salt, the
//! time in whole seconds, the content's UTF-8 with its length, and the signatures last
//! seen with their count). Big-endian throughout.

use std::sync::Arc;

use ruthenium_protocol::packets::play::{LastSeenUpdate, SIGNATURE_BYTES};

/// A chat message's signature.
pub type Signature = Box<[u8; SIGNATURE_BYTES]>;

/// The longest message, in UTF-16 units (`StringUtil.trimChatMessage`).
pub const MAX_MESSAGE: usize = 256;
/// Messages the last-seen list holds (`LastSeenMessages.LAST_SEEN_MESSAGES_MAX_LENGTH`).
pub const LAST_SEEN: usize = 20;

/// `StringUtil.isAllowedChatCharacter`: not `§`, no control character.
pub fn allowed_character(c: char) -> bool {
    c != '§' && c >= ' ' && c != '\u{7F}'
}

/// Java's `Character.isWhitespace`: the space separators but the non-breaking ones, the
/// line and paragraph separators, and the controls tab to carriage return and the
/// file to unit separators.
fn java_whitespace(c: char) -> bool {
    matches!(c,
        '\t'..='\r'
        | '\u{1C}'..='\u{20}'
        | '\u{1680}'
        | '\u{2000}'..='\u{2006}'
        | '\u{2008}'..='\u{200A}'
        | '\u{2028}'
        | '\u{2029}'
        | '\u{205F}'
        | '\u{3000}')
}

/// `ChatScreen.normalizeChatMessage`: trimmed (`String.trim`), every run of whitespace
/// one space and non-breaking spaces plain ones (`StringUtils.normalizeSpace`), then
/// cut to [`MAX_MESSAGE`] UTF-16 units (half a surrogate pair cut off becomes `?`, as
/// Java writes it).
pub fn normalize(message: &str) -> String {
    let trimmed = message.trim_matches(|c| c <= ' ');
    let mut out = String::with_capacity(trimmed.len());
    let mut in_space = false;
    for c in trimmed.chars() {
        if java_whitespace(c) {
            in_space = true;
            continue;
        }
        if in_space && !out.is_empty() {
            out.push(' ');
        }
        in_space = false;
        out.push(if c == '\u{A0}' { ' ' } else { c });
    }
    // (`normalizeSpace` trims once more: a non-breaking space at either end goes.)
    let out = out.trim_matches(|c| c <= ' ');
    let mut units = 0;
    let mut cut = String::with_capacity(out.len());
    for c in out.chars() {
        units += c.len_utf16();
        if units > MAX_MESSAGE {
            if units - c.len_utf16() < MAX_MESSAGE {
                cut.push('?');
            }
            break;
        }
        cut.push(c);
    }
    cut
}

/// `LastSeenMessages.computeChecksum`: `31 * h + Arrays.hashCode(signature)` over the
/// signatures from 1, its low byte, 1 instead of 0.
pub fn checksum(signatures: &[Signature]) -> u8 {
    let mut h: i32 = 1;
    for s in signatures {
        let mut sh: i32 = 1;
        for &b in s.iter() {
            sh = sh.wrapping_mul(31).wrapping_add(i32::from(b as i8));
        }
        h = h.wrapping_mul(31).wrapping_add(sh);
    }
    match h as u8 {
        0 => 1,
        b => b,
    }
}

/// The signed messages the player has seen (`LastSeenMessagesTracker` of 20): a ring of
/// the last 20, each shown (and still to be acknowledged, or acknowledged) or not.
#[derive(Debug, Clone)]
pub struct LastSeenTracker {
    /// `(signature, pending)`; `None`: a message that was not shown.
    entries: [Option<(Signature, bool)>; LAST_SEEN],
    tail: usize,
    offset: i32,
    last: Option<Signature>,
}

impl Default for LastSeenTracker {
    fn default() -> LastSeenTracker {
        LastSeenTracker {
            entries: std::array::from_fn(|_| None),
            tail: 0,
            offset: 0,
            last: None,
        }
    }
}

impl LastSeenTracker {
    /// `addPending`: a signed message came in, shown or not; false if it is the one
    /// before again.
    pub fn add_pending(&mut self, signature: &Signature, shown: bool) -> bool {
        if self.last.as_ref() == Some(signature) {
            return false;
        }
        self.last = Some(signature.clone());
        self.entries[self.tail] = shown.then(|| (signature.clone(), true));
        self.tail = (self.tail + 1) % LAST_SEEN;
        self.offset += 1;
        true
    }

    /// `ignorePending`: a message the server took back is not acknowledged.
    pub fn ignore_pending(&mut self, signature: &Signature) {
        if let Some(e) = self.entries.iter_mut().find(|e| {
            e.as_ref()
                .is_some_and(|(s, pending)| *pending && s == signature)
        }) {
            *e = None;
        }
    }

    /// Messages taken in since the server was last told.
    pub fn offset(&self) -> i32 {
        self.offset
    }

    /// `getAndClearOffset`.
    pub fn take_offset(&mut self) -> i32 {
        std::mem::take(&mut self.offset)
    }

    /// `generateAndApplyUpdate`: the shown messages oldest first, now acknowledged, and
    /// the update that tells the server of them.
    pub fn update(&mut self) -> (Vec<Signature>, LastSeenUpdate) {
        let offset = self.take_offset();
        let mut acknowledged = [0u8; 3];
        let mut seen = Vec::with_capacity(LAST_SEEN);
        for i in 0..LAST_SEEN {
            if let Some((s, pending)) = &mut self.entries[(self.tail + i) % LAST_SEEN] {
                acknowledged[i / 8] |= 1 << (i % 8);
                seen.push(s.clone());
                *pending = false;
            }
        }
        let checksum = checksum(&seen);
        (
            seen,
            LastSeenUpdate {
                offset,
                acknowledged,
                checksum,
            },
        )
    }
}

/// The player's chat signing key pair (`ProfileKeyPair`): the private key (PKCS#8
/// DER), the public key (X.509 DER) with when it expires and Mojang's signature over it,
/// and when a new pair should be fetched (milliseconds of the epoch).
#[derive(Clone, PartialEq, Eq)]
pub struct ProfileKeys {
    pub private_key: Vec<u8>,
    pub public_key: Vec<u8>,
    pub key_signature: Vec<u8>,
    pub expires_at: i64,
    pub refresh_after: i64,
}

impl std::fmt::Debug for ProfileKeys {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProfileKeys")
            .field("expires_at", &self.expires_at)
            .field("refresh_after", &self.refresh_after)
            .finish_non_exhaustive()
    }
}

/// Where the session's key pair comes from (`ProfileKeyPairManager`): fetched off the
/// game thread, kept in memory.
pub trait ProfileKeySource: Send + Sync + std::fmt::Debug {
    /// `prepareKeyPair`: starts getting the key pair (the one kept, or a new one when it
    /// is due); [`ProfileKeySource::poll`] has it when it is there.
    fn prepare(&self);
    /// Whether the last [`ProfileKeySource::prepare`] is done, and with what key pair.
    fn poll(&self) -> KeyPoll;
    /// `shouldRefreshKeyPair`: an hour since the last prepare, and the pair kept is due
    /// for a new one (or there is none).
    fn should_refresh(&self) -> bool;
}

/// How getting the key pair is going.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeyPoll {
    Pending,
    /// Done: the key pair, or none (it couldn't be had, and none was kept).
    Ready(Option<Arc<ProfileKeys>>),
}

/// What a signature is over (`PlayerChatMessage.updateSignature` of a link and a
/// `SignedMessageBody`). `time` is in milliseconds; whole seconds are signed.
pub fn signed_payload(
    sender: u128,
    session: u128,
    index: i32,
    content: &str,
    time: i64,
    salt: i64,
    last_seen: &[Signature],
) -> Vec<u8> {
    let mut o = Vec::with_capacity(64 + content.len() + last_seen.len() * SIGNATURE_BYTES);
    o.extend_from_slice(&1i32.to_be_bytes());
    o.extend_from_slice(&sender.to_be_bytes());
    o.extend_from_slice(&session.to_be_bytes());
    o.extend_from_slice(&index.to_be_bytes());
    o.extend_from_slice(&salt.to_be_bytes());
    o.extend_from_slice(&time.div_euclid(1000).to_be_bytes());
    o.extend_from_slice(&(content.len() as i32).to_be_bytes());
    o.extend_from_slice(content.as_bytes());
    o.extend_from_slice(&(last_seen.len() as i32).to_be_bytes());
    for s in last_seen {
        o.extend_from_slice(&s[..]);
    }
    o
}

/// The player's chat session (`LocalChatSession` and its `SignedMessageChain.Encoder`):
/// a random id, the key pair, and the index of the next message.
pub struct LocalChatSession {
    pub id: u128,
    pub keys: Arc<ProfileKeys>,
    key: rsa::PrivateKey,
    sender: u128,
    /// `None` once the chain is used up.
    next: Option<i32>,
}

impl std::fmt::Debug for LocalChatSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LocalChatSession")
            .field("id", &self.id)
            .field("next", &self.next)
            .finish_non_exhaustive()
    }
}

impl LocalChatSession {
    /// A session with the id `id` for the player `sender` (`LocalChatSession.create`
    /// takes a random one: [`random_uuid`]).
    pub fn new(id: u128, keys: Arc<ProfileKeys>, sender: u128) -> Result<LocalChatSession, String> {
        use rsa::pkcs8::DecodePrivateKey;
        let key = rsa::PrivateKey::from_pkcs8_der(&keys.private_key)
            .map_err(|e| format!("the chat key can't be read: {e}"))?;
        Ok(LocalChatSession {
            id,
            keys,
            key,
            sender,
            next: Some(0),
        })
    }

    /// Signs the next message of the chain (`SignedMessageChain.encoder`).
    pub fn sign(
        &mut self,
        content: &str,
        time: i64,
        salt: i64,
        last_seen: &[Signature],
    ) -> Option<Signature> {
        let index = self.next?;
        self.next = index.checked_add(1);
        let payload = signed_payload(self.sender, self.id, index, content, time, salt, last_seen);
        let signature = self
            .key
            .pkcs1v15_sign::<rsa::sha2::Sha256>(&payload)
            .inspect_err(|e| eprintln!("[chat] a message couldn't be signed: {e}"))
            .ok()?;
        let bytes = <[u8; SIGNATURE_BYTES]>::try_from(signature.as_slice()).ok()?;
        Some(Box::new(bytes))
    }

    /// `ServerboundChatSessionUpdatePacket` of this session.
    pub fn update_packet(&self) -> Vec<u8> {
        ruthenium_protocol::packets::play::chat_session_update(
            self.id,
            self.keys.expires_at,
            &self.keys.public_key,
            &self.keys.key_signature,
        )
    }
}

/// A random (version 4) UUID (`UUID.randomUUID`).
pub fn random_uuid() -> Option<u128> {
    let mut b = ruthenium_net::crypt::secret().ok()?;
    b[6] = b[6] & 0x0F | 0x40;
    b[8] = b[8] & 0x3F | 0x80;
    Some(u128::from_be_bytes(b))
}

/// A message's salt (`Crypt.SaltSupplier.getLong`).
pub fn salt() -> i64 {
    ruthenium_net::crypt::secret().map_or(0, |b| {
        i64::from_be_bytes(b[..8].try_into().unwrap_or_default())
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use ruthenium_protocol::commands::CommandTree;
    use ruthenium_protocol::packets::play::{self, SIGNATURE_BYTES};
    use ruthenium_protocol::wire::Reader;

    /// One step of the tracker's golden sequence: a message (seed, shown, whether it
    /// counted), a taken-back one, the offset taken, or an update (offset, bits,
    /// checksum, the seeds of the signatures seen).
    enum Track {
        Add(u32, bool, bool),
        Ignore(u32),
        Offset(i32),
        Update(i32, [u8; 3], u8, &'static [u32]),
    }

    /// The arguments a command signs: names and what they say.
    type Signed = &'static [(&'static str, &'static str)];

    include!("chat_golden.rs");

    fn hex(s: &str) -> Vec<u8> {
        (0..s.len() / 2)
            .map(|i| u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).unwrap())
            .collect()
    }

    /// A made-up signature, as the generator makes them.
    fn sig(seed: u32) -> Signature {
        let mut s = i64::from(seed).wrapping_mul(2_654_435_761).wrapping_add(1);
        let mut b = [0u8; SIGNATURE_BYTES];
        for v in &mut b {
            s = s
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            *v = (s as u64 >> 56) as u8;
        }
        Box::new(b)
    }

    /// A key pair of the golden's key, its public half's signature made up.
    pub(crate) fn keys() -> Arc<ProfileKeys> {
        use rsa::pkcs8::{DecodePrivateKey, EncodePublicKey};
        let private_key = hex(KEY);
        let public_key = rsa::PrivateKey::from_pkcs8_der(&private_key)
            .unwrap()
            .to_public_key()
            .to_public_key_der()
            .unwrap()
            .as_bytes()
            .to_vec();
        Arc::new(ProfileKeys {
            private_key,
            public_key,
            key_signature: sig(3).to_vec(),
            expires_at: 1_764_244_800_456,
            refresh_after: 0,
        })
    }

    /// The tree of the golden's commands.
    pub(crate) fn tree() -> CommandTree {
        CommandTree::read(&mut Reader::new(&hex(TREE))).unwrap()
    }

    /// The packet's body after its id.
    fn body(p: &[u8]) -> Vec<u8> {
        let mut r = Reader::new(p);
        r.varint().unwrap();
        r.rest().to_vec()
    }

    #[test]
    fn signatures_are_vanillas_down_a_chain() {
        let mut session = LocalChatSession::new(SESSION, keys(), SENDER).unwrap();
        for (i, &(content, time, salt, seen, expected)) in SIGNED.iter().enumerate() {
            let seen: Vec<Signature> = seen.iter().map(|&s| sig(s)).collect();
            let s = session.sign(content, time, salt, &seen).unwrap();
            assert_eq!(s.to_vec(), hex(expected), "message {i}: {content:?}");
        }
        let payload = signed_payload(SENDER, SESSION, 3, "hi", 1_764_158_400_999, 42, &[sig(7)]);
        assert_eq!(payload, hex(PAYLOAD));
        // A time before the epoch signs its seconds rounded down, as `getEpochSecond`.
        let early = signed_payload(SENDER, SESSION, 0, "", -1, 0, &[]);
        assert_eq!(early[48..56], (-1i64).to_be_bytes());
    }

    #[test]
    fn the_last_seen_tracker_is_vanillas() {
        let mut t = LastSeenTracker::default();
        for (i, step) in TRACKER.iter().enumerate() {
            match *step {
                Track::Add(seed, shown, counted) => {
                    assert_eq!(t.add_pending(&sig(seed), shown), counted, "step {i}");
                }
                Track::Ignore(seed) => t.ignore_pending(&sig(seed)),
                Track::Offset(offset) => assert_eq!(t.take_offset(), offset, "step {i}"),
                Track::Update(offset, acknowledged, checksum, seen) => {
                    let (signatures, update) = t.update();
                    let expected: Vec<Signature> = seen.iter().map(|&s| sig(s)).collect();
                    assert_eq!(signatures, expected, "step {i}");
                    assert_eq!(
                        update,
                        play::LastSeenUpdate {
                            offset,
                            acknowledged,
                            checksum
                        },
                        "step {i}"
                    );
                }
            }
        }
    }

    #[test]
    fn the_chat_packets_are_laid_out_as_vanillas() {
        let update = play::LastSeenUpdate {
            offset: 3,
            acknowledged: [0x81, 0x40, 0x08],
            checksum: 0x9c,
        };
        let s1 = sig(1);
        let chat = play::chat("héllo", 1_764_158_400_123, -5, Some(&s1), &update);
        assert_eq!(body(&chat), hex(CHAT_PACKET));
        let ones = play::LastSeenUpdate {
            checksum: 1,
            ..play::LastSeenUpdate::default()
        };
        assert_eq!(
            body(&play::chat("x", 1, 0, None, &ones)),
            hex(CHAT_UNSIGNED_PACKET)
        );
        assert_eq!(
            body(&play::chat_command("tp @s ~ ~1 ~")),
            hex(COMMAND_PACKET)
        );
        let arguments = [
            ("message".to_owned(), sig(1)),
            ("action".to_owned(), sig(2)),
        ];
        let signed =
            play::chat_command_signed("msg Bob hi", 1_764_158_400_123, 77, &arguments, &update);
        assert_eq!(body(&signed), hex(SIGNED_COMMAND_PACKET));
        let session = LocalChatSession::new(SESSION, keys(), SENDER).unwrap();
        assert_eq!(body(&session.update_packet()), hex(SESSION_PACKET));
    }

    #[test]
    fn messages_are_made_tidy_as_vanillas() {
        for &(text, expected) in NORMALIZED {
            assert_eq!(normalize(text), expected, "{text:?}");
        }
    }

    #[test]
    fn the_arguments_signed_are_those_vanilla_finds() {
        let tree = CommandTree::read(&mut Reader::new(&hex(TREE))).unwrap();
        for &(command, selectors, expected) in COMMANDS {
            let found =
                crate::command::signable_arguments(&tree, command, if selectors { 2 } else { 0 });
            let expected: Vec<(String, String)> = expected
                .iter()
                .map(|&(n, v)| (n.to_owned(), v.to_owned()))
                .collect();
            assert_eq!(found, expected, "{command:?}, selectors {selectors}");
        }
    }

    #[test]
    fn session_ids_are_random_version_4_uuids() {
        let a = random_uuid().unwrap();
        let b = random_uuid().unwrap();
        assert_ne!(a, b);
        for id in [a, b] {
            assert_eq!(id >> 76 & 0xF, 4, "{id:032x}");
            assert_eq!(id >> 62 & 0b11, 0b10, "{id:032x}");
        }
        assert_ne!(salt(), salt());
    }

    #[test]
    fn checksums_are_never_zero() {
        // (0 tells the server not to check.)
        assert_eq!(checksum(&[]), 1);
        let low_byte = |s: &Signature| {
            let mut h: i32 = 1;
            for &b in s.iter() {
                h = h.wrapping_mul(31).wrapping_add(i32::from(b as i8));
            }
            31i32.wrapping_add(h) as u8
        };
        let zero = (0..10_000).map(sig).find(|s| low_byte(s) == 0).unwrap();
        assert_eq!(checksum(std::slice::from_ref(&zero)), 1);
        let other = (0..10_000).map(sig).find(|s| low_byte(s) > 1).unwrap();
        assert_eq!(checksum(std::slice::from_ref(&other)), low_byte(&other));
    }
}

//! What the HUD says in words: the selected item's name, the message over the hotbar, the
//! titles, the chat, the boss bars, the sidebar and the player list (`Hud`, `ChatListener`,
//! `ChatComponent`, `BossHealthOverlay`, `PlayerTabOverlay`), as the packets and the HUD's
//! ticks leave them. Laying them out and drawing them is done elsewhere, on what a frame
//! carries of them.
//!
//! Texts are components in their NBT form: what draws them has the language. A text's
//! `Arc` stays the same one for as long as the text does, so that what was made of it can
//! be kept.
//!
//! Not yet: the signatures of chat messages are not checked (a message with one counts as
//! its sender's); blocked players; the chat options (its delay, hiding, only secure).

use std::sync::Arc;
use std::time::Instant;

use ruthenium_protocol::nbt::{Compound, Tag};
use ruthenium_protocol::packets::configuration::RegistryEntry;
use ruthenium_protocol::packets::play::{
    self, BossEvent, BoundChatType, ChatDecoration, ChatParameter, ChatTypeRef, FilterMask,
    PackedSignature, PlayerChat,
};
use ruthenium_protocol::text::StyledText;

use crate::avatar::SkinSource;
use crate::entity::{PlayerInfos, Teams};
use crate::item::Stack;
use crate::item_name::{styled_name, translation};
use crate::scoreboard::{Scoreboard, Sidebar, ignoring_case};

/// Messages kept (`ChatComponent.MAX_CHAT_HISTORY`).
const MESSAGES: usize = 100;
/// A message shows at least this long before it is taken away, in ticks.
const DELETION_DELAY: i32 = 60;
/// Signatures both sides keep (`MessageSignatureCache`).
const SIGNATURES: usize = 128;
/// Signed messages taken in before the server is told.
const ACK_AFTER: i32 = 64;
/// After this long a message's signature does not count, in milliseconds.
const EXPIRES: i64 = 7 * 60 * 1000;
/// How long a message shows over the hotbar, in ticks.
const OVERLAY_TICKS: i32 = 60;
/// The titles' usual times, in ticks: fading in, staying, fading out.
const TITLE_TIMES: [i32; 3] = [10, 70, 20];
/// Players the list shows at most.
const LISTED: usize = 80;

/// What a chat message's edge is marked with (`GuiMessageTag`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChatTag {
    /// 0xRRGGBB.
    pub colour: u32,
    /// The icon of a changed message takes room at the line's end.
    pub icon: bool,
}

impl ChatTag {
    const SYSTEM: ChatTag = ChatTag {
        colour: 0xD0_D0D0,
        icon: false,
    };
    const NOT_SECURE: ChatTag = ChatTag::SYSTEM;
    const MODIFIED: ChatTag = ChatTag {
        colour: 0x60_6060,
        icon: true,
    };
    const ERROR: ChatTag = ChatTag {
        colour: 0xFF_5555,
        icon: false,
    };
}

use crate::chat::{LastSeenTracker, Signature};

/// A message in the chat.
#[derive(Debug, Clone, PartialEq)]
pub struct ChatMessage {
    /// The HUD's tick it came in.
    pub added: i32,
    pub content: Arc<Tag>,
    pub tag: Option<ChatTag>,
    signature: Option<Signature>,
}

/// The message over the hotbar.
#[derive(Debug, Clone, PartialEq)]
pub struct Overlay {
    pub text: Arc<Tag>,
    /// How long it still shows, in ticks.
    pub time: i32,
    /// Its colour goes round the rainbow.
    pub animated: bool,
}

/// The title, and the subtitle under it.
#[derive(Debug, Clone, PartialEq)]
pub struct Title {
    pub title: Arc<Tag>,
    pub subtitle: Option<Arc<Tag>>,
    /// How long it still shows, in ticks, and how it does.
    pub time: i32,
    pub fade_in: i32,
    pub stay: i32,
    pub fade_out: i32,
}

/// A boss bar (`LerpingBossEvent`).
#[derive(Debug, Clone, PartialEq)]
pub struct BossBar {
    id: u128,
    pub name: Arc<Tag>,
    /// How full it was when it was last set, and how full it is to be a tenth of a second
    /// after.
    from: f32,
    target: f32,
    set: Instant,
    /// Ordinals of `BossBarColor` and `BossBarOverlay`.
    pub color: u8,
    pub overlay: u8,
    /// Bit 0 darkens the screen, 1 plays the boss music, 2 makes fog.
    pub flags: u8,
}

impl BossBar {
    /// A bar that has been as full as this for a while.
    pub fn new(id: u128, name: Arc<Tag>, progress: f32, color: u8, overlay: u8) -> BossBar {
        BossBar {
            id,
            name,
            from: progress,
            target: progress,
            set: Instant::now(),
            color,
            overlay,
            flags: 0,
        }
    }

    /// How full it is at `now`.
    pub fn progress(&self, now: Instant) -> f32 {
        let since = now.saturating_duration_since(self.set).as_millis() as f32;
        let t = (since / 100.0).clamp(0.0, 1.0);
        self.from + t * (self.target - self.from)
    }
}

/// How the player list shows scores.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ListScores {
    None,
    Numbers,
    Hearts,
}

/// A player in the list.
#[derive(Debug, Clone, PartialEq)]
pub struct ListedPlayer {
    pub id: u128,
    pub name: Arc<Tag>,
    pub spectator: bool,
    /// In milliseconds.
    pub latency: i32,
    /// The score, and how it is written; nothing for a player without one.
    pub score: Option<(i32, Arc<Tag>)>,
    pub skin: Arc<SkinSource>,
    pub show_hat: bool,
    /// Its face is upside down.
    pub flipped: bool,
}

/// What the player list shows (`PlayerTabOverlay`).
#[derive(Debug, Clone, PartialEq)]
pub struct PlayerList {
    pub header: Option<Arc<Tag>>,
    pub footer: Option<Arc<Tag>>,
    pub players: Vec<ListedPlayer>,
    pub scores: ListScores,
    /// The players' faces show.
    pub faces: bool,
}

/// A frame's texts.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct HudTexts {
    /// The selected item's name, and how long it still shows, in ticks.
    pub selected: Option<(Arc<Tag>, i32)>,
    pub overlay: Option<Overlay>,
    pub title: Option<Title>,
    /// The newest first.
    pub chat: Arc<Vec<Arc<ChatMessage>>>,
    pub bosses: Arc<Vec<BossBar>>,
    pub sidebar: Option<Arc<Sidebar>>,
    /// While it is asked for.
    pub players: Option<Arc<PlayerList>>,
}

/// How kinds of chat message are written: the `chat_type` registry.
#[derive(Debug, Clone, Default)]
pub struct ChatTypes(Vec<ChatDecoration>);

impl ChatTypes {
    /// From the registry's entries; those the server left to the client are vanilla's.
    pub fn from_registry(entries: &[RegistryEntry]) -> ChatTypes {
        let read = |e: &RegistryEntry| {
            let c = e.data.as_ref()?.as_compound()?.get("chat")?.as_compound()?;
            Some(ChatDecoration {
                translation_key: c.string("translation_key")?.to_owned(),
                parameters: c
                    .list("parameters")
                    .iter()
                    .filter_map(|p| match p {
                        Tag::String(s) => match s.as_str() {
                            "sender" => Some(ChatParameter::Sender),
                            "target" => Some(ChatParameter::Target),
                            "content" => Some(ChatParameter::Content),
                            _ => None,
                        },
                        _ => None,
                    })
                    .collect(),
                style: c.get("style").cloned(),
            })
        };
        ChatTypes(
            entries
                .iter()
                .map(|e| read(e).unwrap_or_else(|| ChatTypes::vanilla(&e.id)))
                .collect(),
        )
    }

    /// Vanilla's own (`ChatType.bootstrap`).
    fn vanilla(id: &str) -> ChatDecoration {
        use ChatParameter::{Content, Sender, Target};
        let whisper = || {
            let mut style = Compound::new();
            style.put("color", Tag::String("gray".into()));
            style.put("italic", Tag::Byte(1));
            Some(Tag::Compound(style))
        };
        let (key, parameters, style) = match id.trim_start_matches("minecraft:") {
            "say_command" => ("chat.type.announcement", vec![Sender, Content], None),
            "msg_command_incoming" => (
                "commands.message.display.incoming",
                vec![Sender, Content],
                whisper(),
            ),
            "msg_command_outgoing" => (
                "commands.message.display.outgoing",
                vec![Target, Content],
                whisper(),
            ),
            "team_msg_command_incoming" => {
                ("chat.type.team.text", vec![Target, Sender, Content], None)
            }
            "team_msg_command_outgoing" => {
                ("chat.type.team.sent", vec![Target, Sender, Content], None)
            }
            "emote_command" => ("chat.type.emote", vec![Sender, Content], None),
            _ => ("chat.type.text", vec![Sender, Content], None),
        };
        ChatDecoration {
            translation_key: key.to_owned(),
            parameters,
            style,
        }
    }

    /// `ChatType.Bound.decorate`.
    fn decorate(&self, kind: &BoundChatType, content: &Tag) -> Result<Tag, String> {
        let decoration = match &kind.kind {
            ChatTypeRef::Direct(d) => d,
            ChatTypeRef::Id(id) => self.0.get(*id).ok_or("a chat message of no known kind")?,
        };
        let with = decoration
            .parameters
            .iter()
            .map(|p| match p {
                ChatParameter::Sender => kind.name.clone(),
                ChatParameter::Target => kind
                    .target
                    .clone()
                    .unwrap_or_else(|| Tag::String(String::new())),
                ChatParameter::Content => content.clone(),
            })
            .collect();
        let mut text = translation(&decoration.translation_key, with);
        if let (Some(Tag::Compound(style)), Tag::Compound(c)) = (&decoration.style, &mut text) {
            for (k, v) in style.iter() {
                c.put(k, v.clone());
            }
        }
        Ok(text)
    }
}

/// What is left of a message a filter hides parts of (`FilterMask.applyWithFormatting`).
fn filtered(content: &str, mask: &[u8]) -> Tag {
    let hidden = |i: usize| mask.get(i / 8).is_some_and(|b| b >> (i % 8) & 1 != 0);
    let units: Vec<u16> = content.encode_utf16().collect();
    let mut parts = Vec::new();
    let mut start = 0;
    while start < units.len() {
        let hide = hidden(start);
        let end = (start..units.len())
            .find(|i| hidden(*i) != hide)
            .unwrap_or(units.len());
        parts.push(if hide {
            let mut c = Compound::new();
            c.put("text", Tag::String("#".repeat(end - start)));
            c.put("color", Tag::String("dark_gray".into()));
            Tag::Compound(c)
        } else {
            Tag::String(String::from_utf16_lossy(&units[start..end]))
        });
        start = end;
    }
    let mut c = Compound::new();
    c.put("text", Tag::String(String::new()));
    if !parts.is_empty() {
        c.put("extra", Tag::List(parts));
    }
    Tag::Compound(c)
}

/// Whether any part of a component is in a font of its own.
fn has_font(tag: &Tag) -> bool {
    match tag {
        Tag::List(items) => items.iter().any(has_font),
        Tag::Compound(c) => {
            c.string("font").is_some_and(|f| f != "minecraft:default")
                || c.list("extra").iter().any(has_font)
                || c.list("with").iter().any(has_font)
        }
        _ => false,
    }
}

/// What a play connection keeps of the chat (`ClientPacketListener`).
#[derive(Debug, Clone, Default)]
struct ChatSession {
    next_index: i32,
    signatures: Vec<Signature>,
    /// The signed messages seen, and how many the server has not been told of.
    seen: LastSeenTracker,
}

impl ChatSession {
    /// `MessageSignatureCache.push`: the newest first, each once.
    fn push(&mut self, seen: Vec<Signature>, own: Option<&Signature>) {
        let mut queue: Vec<Signature> = seen;
        queue.extend(own.cloned());
        queue.reverse();
        for s in std::mem::take(&mut self.signatures) {
            if !queue.contains(&s) {
                queue.push(s);
            }
        }
        queue.truncate(SIGNATURES);
        self.signatures = queue;
    }

    fn unpack(&self, s: &PackedSignature) -> Option<Signature> {
        match s {
            PackedSignature::Full(s) => Some(s.clone()),
            PackedSignature::Cached(i) => self.signatures.get(*i).cloned(),
        }
    }
}

/// The HUD's texts as they stand (see the module docs).
#[derive(Debug, Clone)]
pub struct HudTextState {
    selected: Option<Arc<Tag>>,
    overlay: Option<Overlay>,
    title: Option<Arc<Tag>>,
    subtitle: Option<Arc<Tag>>,
    title_time: i32,
    title_times: [i32; 3],
    chat: Arc<Vec<Arc<ChatMessage>>>,
    /// Messages to be taken away, and when.
    deletions: Vec<(Signature, i32)>,
    session: ChatSession,
    bosses: Arc<Vec<BossBar>>,
    header: Option<Arc<Tag>>,
    footer: Option<Arc<Tag>>,
    /// The sidebar and the player list as they were last made, and of what.
    sidebar: Option<([u64; 2], Option<Arc<Sidebar>>)>,
    players: Option<([u64; 3], Arc<PlayerList>)>,
}

impl Default for HudTextState {
    fn default() -> HudTextState {
        HudTextState {
            selected: None,
            overlay: None,
            title: None,
            subtitle: None,
            title_time: 0,
            title_times: TITLE_TIMES,
            chat: Arc::default(),
            deletions: Vec::new(),
            session: ChatSession::default(),
            bosses: Arc::default(),
            header: None,
            footer: None,
            sidebar: None,
            players: None,
        }
    }
}

impl HudTextState {
    /// The part of `Hud.tick` that is the texts', at the HUD's tick `ticks`.
    pub fn tick(&mut self, ticks: i32) {
        if let Some(overlay) = &mut self.overlay {
            overlay.time -= 1;
            if overlay.time <= 0 {
                self.overlay = None;
            }
        }
        if self.title_time > 0 {
            self.title_time -= 1;
            if self.title_time <= 0 {
                self.title = None;
                self.subtitle = None;
            }
        }
        let due: Vec<Signature> = {
            let (due, later) = std::mem::take(&mut self.deletions)
                .into_iter()
                .partition(|(_, at)| ticks >= *at);
            self.deletions = later;
            let due: Vec<(Signature, i32)> = due;
            due.into_iter().map(|(s, _)| s).collect()
        };
        for signature in due {
            self.delete_message(&signature, ticks);
        }
    }

    /// The selected item is now this one.
    pub fn select(&mut self, stack: &Stack) {
        self.selected = (!stack.is_empty()).then(|| Arc::new(styled_name(stack)));
    }

    /// `setOverlayMessage`.
    pub fn set_overlay(&mut self, text: Tag, animated: bool) {
        self.overlay = Some(Overlay {
            text: Arc::new(text),
            time: OVERLAY_TICKS,
            animated,
        });
    }

    /// `setNowPlaying`.
    pub fn now_playing(&mut self, song: Tag) {
        self.set_overlay(translation("record.nowPlaying", vec![song]), true);
    }

    pub fn set_title(&mut self, text: Tag) {
        self.title = Some(Arc::new(text));
        self.title_time = self.title_times.iter().sum();
    }

    pub fn set_subtitle(&mut self, text: Tag) {
        self.subtitle = Some(Arc::new(text));
    }

    /// `setTimes`: those that are not below 0.
    pub fn set_title_times(&mut self, times: [i32; 3]) {
        for (kept, new) in self.title_times.iter_mut().zip(times) {
            if new >= 0 {
                *kept = new;
            }
        }
        if self.title_time > 0 {
            self.title_time = self.title_times.iter().sum();
        }
    }

    /// `clearTitles`, and `resetTitleTimes` if it is to.
    pub fn clear_titles(&mut self, reset_times: bool) {
        self.title = None;
        self.subtitle = None;
        self.title_time = 0;
        if reset_times {
            self.title_times = TITLE_TIMES;
        }
    }

    fn add(&mut self, ticks: i32, text: Tag, signature: Option<Signature>, tag: Option<ChatTag>) {
        let chat = Arc::make_mut(&mut self.chat);
        chat.insert(
            0,
            Arc::new(ChatMessage {
                added: ticks,
                content: Arc::new(text),
                tag,
                signature,
            }),
        );
        chat.truncate(MESSAGES);
    }

    /// `handleSystemMessage`.
    pub fn system_message(&mut self, ticks: i32, text: Tag) {
        self.add(ticks, text, None, Some(ChatTag::SYSTEM));
    }

    /// `handleDisguisedChatMessage`.
    pub fn disguised(
        &mut self,
        ticks: i32,
        message: &Tag,
        kind: &BoundChatType,
        types: &ChatTypes,
    ) -> Result<(), String> {
        let text = types.decorate(kind, message)?;
        self.add(ticks, text, None, Some(ChatTag::SYSTEM));
        Ok(())
    }

    /// `handlePlayerChat` at `now` (milliseconds of the epoch), of a sender the player
    /// list has or not. Returns how many signed messages to tell the server were taken
    /// in, when it is time to; an error ends the connection.
    pub fn player_chat(
        &mut self,
        ticks: i32,
        now: i64,
        chat: &PlayerChat,
        known: bool,
        types: &ChatTypes,
    ) -> Result<Option<i32>, String> {
        let expected = self.session.next_index;
        self.session.next_index = expected.wrapping_add(1);
        if chat.global_index != expected {
            return Err("a chat message is missing or out of order".into());
        }
        let seen = chat
            .last_seen
            .iter()
            .map(|s| self.session.unpack(s))
            .collect::<Option<Vec<_>>>()
            .ok_or("a chat message refers to one that is not known")?;
        self.session.push(seen, chat.signature.as_ref());
        if known {
            // `decoratedContent`, and how far it is to be trusted.
            let written = Tag::String(chat.content.clone());
            let shown = chat.unsigned_content.as_ref().unwrap_or(&written);
            let changed = chat.unsigned_content.as_ref().is_some_and(|c| {
                !StyledText::from_nbt_in(c, &())
                    .plain()
                    .contains(&chat.content)
                    || has_font(c)
            });
            let tag = if chat.signature.is_none() || now > chat.time.saturating_add(EXPIRES) {
                Some(ChatTag::NOT_SECURE)
            } else if changed {
                Some(ChatTag::MODIFIED)
            } else {
                None
            };
            let content = match &chat.filter {
                FilterMask::PassThrough => Some(shown.clone()),
                FilterMask::FullyFiltered => None,
                FilterMask::Partly(mask) => Some(filtered(&chat.content, mask)),
            };
            if let Some(content) = content {
                let text = types.decorate(&chat.kind, &content)?;
                self.add(ticks, text, chat.signature.clone(), tag);
            }
        } else {
            let mut error = Compound::new();
            error.put("translate", Tag::String("chat.validation_error".into()));
            error.put("color", Tag::String("red".into()));
            error.put("italic", Tag::Byte(1));
            let text = types.decorate(&chat.kind, &Tag::Compound(error))?;
            self.add(ticks, text, None, Some(ChatTag::ERROR));
        }
        // `markMessageAsProcessed`.
        // (Shown: from a player on the list, not wholly filtered.)
        let shown = known && chat.filter != FilterMask::FullyFiltered;
        let seen = &mut self.session.seen;
        if let Some(signature) = &chat.signature
            && seen.add_pending(signature, shown)
            && seen.offset() > ACK_AFTER
        {
            return Ok(self.take_acknowledgement());
        }
        Ok(None)
    }

    /// The signed messages the server is to be told of now, if any.
    pub fn take_acknowledgement(&mut self) -> Option<i32> {
        let offset = self.session.seen.take_offset();
        (offset > 0).then_some(offset)
    }

    /// What a message or a signed command sent now says the player has seen
    /// (`LastSeenMessagesTracker.generateAndApplyUpdate`): the signatures to sign with,
    /// and the update to send.
    pub fn last_seen_update(&mut self) -> (Vec<Signature>, play::LastSeenUpdate) {
        self.session.seen.update()
    }

    /// `handleDeleteChat`.
    pub fn delete_chat(&mut self, ticks: i32, packed: &PackedSignature) -> Result<(), String> {
        let signature = self
            .session
            .unpack(packed)
            .ok_or("the server deletes a chat message that is not known")?;
        self.session.seen.ignore_pending(&signature);
        self.delete_message(&signature, ticks);
        Ok(())
    }

    /// `deleteMessageOrDelay`.
    fn delete_message(&mut self, signature: &Signature, ticks: i32) {
        let found = self
            .chat
            .iter()
            .position(|m| m.signature.as_ref() == Some(signature));
        let Some(i) = found else {
            return;
        };
        let added = self.chat[i].added;
        if ticks < added + DELETION_DELAY {
            self.deletions
                .push((signature.clone(), added + DELETION_DELAY));
            return;
        }
        let mut marker = Compound::new();
        marker.put("translate", Tag::String("chat.deleted_marker".into()));
        marker.put("color", Tag::String("gray".into()));
        marker.put("italic", Tag::Byte(1));
        Arc::make_mut(&mut self.chat)[i] = Arc::new(ChatMessage {
            added,
            content: Arc::new(Tag::Compound(marker)),
            tag: Some(ChatTag::SYSTEM),
            signature: None,
        });
    }

    /// `BossHealthOverlay.update` at `now`. A change to a bar that is not there does
    /// nothing.
    pub fn boss_event(&mut self, id: u128, event: BossEvent, now: Instant) {
        let bosses = Arc::make_mut(&mut self.bosses);
        let at = bosses.iter().position(|b| b.id == id);
        match event {
            BossEvent::Add {
                name,
                progress,
                color,
                overlay,
                flags,
            } => {
                let bar = BossBar {
                    id,
                    name: Arc::new(name),
                    from: progress,
                    target: progress,
                    set: now,
                    color,
                    overlay,
                    flags,
                };
                // One that is there already keeps its place.
                match at {
                    Some(i) => bosses[i] = bar,
                    None => bosses.push(bar),
                }
            }
            BossEvent::Remove => bosses.retain(|b| b.id != id),
            change => {
                let Some(bar) = at.map(|i| &mut bosses[i]) else {
                    return;
                };
                match change {
                    BossEvent::Progress(progress) => {
                        bar.from = bar.progress(now);
                        bar.target = progress;
                        bar.set = now;
                    }
                    BossEvent::Name(name) => bar.name = Arc::new(name),
                    BossEvent::Style { color, overlay } => {
                        bar.color = color;
                        bar.overlay = overlay;
                    }
                    BossEvent::Properties { flags } => bar.flags = flags,
                    BossEvent::Add { .. } | BossEvent::Remove => {}
                }
            }
        }
    }

    /// `handleTabListCustomisation`: an empty text is none.
    pub fn set_tab_list(&mut self, header: Tag, footer: Tag) {
        let some = |t: Tag| {
            // (A translation is taken not to be empty.)
            let empty = StyledText::from_nbt_in(&t, &()).is_empty();
            (!empty).then(|| Arc::new(t))
        };
        self.header = some(header);
        self.footer = some(footer);
        self.players = None;
    }

    /// The connection goes back to configuration (`clearClientLevel`): the chat stays,
    /// what belongs to the play connection goes.
    pub fn reconfigure(&mut self) {
        *self = HudTextState {
            selected: self.selected.take(),
            overlay: self.overlay.take(),
            chat: std::mem::take(&mut self.chat),
            deletions: std::mem::take(&mut self.deletions),
            ..HudTextState::default()
        };
    }

    /// Takes the frame's texts into its record. `highlight` is how long the selected
    /// item's name still shows; the player list is made only when it is `wanted`.
    #[allow(clippy::too_many_arguments)]
    pub fn fill(
        &mut self,
        out: &mut HudTexts,
        highlight: i32,
        own_name: &str,
        wanted: bool,
        faces: bool,
        players: &PlayerInfos,
        teams: &Teams,
        scores: &Scoreboard,
    ) {
        out.selected = self
            .selected
            .clone()
            .filter(|_| highlight > 0)
            .map(|name| (name, highlight));
        out.overlay.clone_from(&self.overlay);
        out.title = self
            .title
            .clone()
            .filter(|_| self.title_time > 0)
            .map(|title| {
                let [fade_in, stay, fade_out] = self.title_times;
                Title {
                    title,
                    subtitle: self.subtitle.clone(),
                    time: self.title_time,
                    fade_in,
                    stay,
                    fade_out,
                }
            });
        out.chat.clone_from(&self.chat);
        out.bosses.clone_from(&self.bosses);
        let of = [scores.rev(), teams.rev()];
        if self.sidebar.as_ref().is_none_or(|(was, _)| *was != of) {
            let sidebar = scores.sidebar(own_name, teams).map(Arc::new);
            self.sidebar = Some((of, sidebar));
        }
        out.sidebar = self.sidebar.as_ref().and_then(|(_, s)| s.clone());
        out.players = None;
        if wanted {
            let of = [players.rev(), scores.rev(), teams.rev()];
            if self.players.as_ref().is_none_or(|(was, _)| *was != of) {
                let list = self.player_list(faces, players, teams, scores);
                self.players = Some((of, Arc::new(list)));
            }
            out.players = self.players.as_ref().map(|(_, p)| p.clone());
        }
    }

    /// `PlayerTabOverlay`'s players, in its order, and how they show.
    fn player_list(
        &self,
        faces: bool,
        players: &PlayerInfos,
        teams: &Teams,
        scores: &Scoreboard,
    ) -> PlayerList {
        let mut listed: Vec<_> = players.iter().filter(|(_, p)| p.listed).collect();
        let team = |name: &str| teams.team_of(name).map_or("", |t| t.name.as_str());
        listed.sort_by(|(a_id, a), (b_id, b)| {
            let spectator = |mode: i32| mode == play::SPECTATOR;
            b.list_order
                .cmp(&a.list_order)
                .then(spectator(a.game_mode).cmp(&spectator(b.game_mode)))
                .then_with(|| {
                    // `String.compareTo`: by UTF-16 units.
                    team(&a.name)
                        .encode_utf16()
                        .cmp(team(&b.name).encode_utf16())
                })
                .then_with(|| ignoring_case(&a.name, &b.name))
                .then(a_id.cmp(b_id))
        });
        listed.truncate(LISTED);
        let players = listed
            .into_iter()
            .map(|(id, p)| {
                let spectator = p.game_mode == play::SPECTATOR;
                let name = match &p.display_name {
                    Some(name) => name.clone(),
                    None => teams.decorated(&p.name, Tag::String(p.name.clone())),
                };
                let name = if spectator {
                    let mut c = Compound::new();
                    c.put("text", Tag::String(String::new()));
                    c.put("italic", Tag::Byte(1));
                    c.put("extra", Tag::List(vec![name]));
                    Tag::Compound(c)
                } else {
                    name
                };
                ListedPlayer {
                    id,
                    name: Arc::new(name),
                    spectator,
                    latency: p.latency,
                    score: scores.list_score(&p.name).map(|(v, t)| (v, Arc::new(t))),
                    skin: Arc::new(SkinSource::Player {
                        id,
                        textures: p.textures.clone(),
                    }),
                    show_hat: p.show_hat,
                    flipped: matches!(p.name.as_str(), "Dinnerbone" | "Grumm"),
                }
            })
            .collect();
        PlayerList {
            header: self.header.clone(),
            footer: self.footer.clone(),
            players,
            scores: match scores.list_hearts() {
                None => ListScores::None,
                Some(false) => ListScores::Numbers,
                Some(true) => ListScores::Hearts,
            },
            faces,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    fn text(s: &str) -> Tag {
        Tag::String(s.to_owned())
    }

    /// The frame's texts, with nothing but `state` to take them from.
    fn shown(state: &mut HudTextState) -> HudTexts {
        let mut out = HudTexts::default();
        state.fill(
            &mut out,
            0,
            "Kai",
            false,
            false,
            &PlayerInfos::default(),
            &Teams::default(),
            &Scoreboard::default(),
        );
        out
    }

    fn plain(tag: &Tag) -> String {
        StyledText::from_nbt_in(tag, &()).plain()
    }

    #[test]
    fn titles_fade_by_the_times_they_were_given() {
        let mut s = HudTextState::default();
        // A subtitle alone shows nothing, and waits for its title.
        s.set_subtitle(text("under"));
        assert_eq!(shown(&mut s).title, None);
        s.set_title(text("Title"));
        let t = shown(&mut s).title.unwrap();
        assert_eq!((t.time, t.fade_in, t.stay, t.fade_out), (100, 10, 70, 20));
        assert_eq!(
            (&*t.title, t.subtitle.as_deref()),
            (&text("Title"), Some(&text("under")))
        );
        s.tick(1);
        assert_eq!(shown(&mut s).title.unwrap().time, 99);
        // New times start a title that shows again; those below 0 stay.
        s.set_title_times([5, -1, 0]);
        let t = shown(&mut s).title.unwrap();
        assert_eq!((t.time, t.fade_in, t.stay, t.fade_out), (75, 5, 70, 0));
        for tick in 2..76 {
            s.tick(tick);
        }
        assert_eq!(shown(&mut s).title.unwrap().time, 1);
        s.tick(76);
        assert_eq!(shown(&mut s).title, None);
        // The subtitle went with it; the times did not.
        s.set_title(text("Again"));
        let t = shown(&mut s).title.unwrap();
        assert_eq!((t.time, t.subtitle), (75, None));
        s.clear_titles(false);
        assert_eq!(shown(&mut s).title, None);
        s.set_title(text("Again"));
        assert_eq!(shown(&mut s).title.unwrap().time, 75);
        s.clear_titles(true);
        s.set_title(text("Again"));
        assert_eq!(shown(&mut s).title.unwrap().time, 100);
        // Times set while nothing shows start nothing.
        s.clear_titles(true);
        s.set_title_times([1, 1, 1]);
        assert_eq!(shown(&mut s).title, None);
    }

    #[test]
    fn a_message_over_the_hotbar_shows_for_three_seconds() {
        let mut s = HudTextState::default();
        s.set_overlay(text("Low"), false);
        let o = shown(&mut s).overlay.unwrap();
        assert_eq!((&*o.text, o.time, o.animated), (&text("Low"), 60, false));
        for tick in 0..59 {
            s.tick(tick);
        }
        assert_eq!(shown(&mut s).overlay.unwrap().time, 1);
        s.tick(59);
        assert_eq!(shown(&mut s).overlay, None);
        s.now_playing(text("cat"));
        let o = shown(&mut s).overlay.unwrap();
        assert!(o.animated);
        let c = o.text.as_compound().unwrap();
        assert_eq!(c.string("translate"), Some("record.nowPlaying"));
        assert_eq!(c.list("with"), [text("cat")]);
    }

    fn kind(id: usize) -> BoundChatType {
        BoundChatType {
            kind: ChatTypeRef::Id(id),
            name: text("Kai"),
            target: None,
        }
    }

    fn types() -> ChatTypes {
        let entry = |id: &str, data| RegistryEntry {
            id: id.to_owned(),
            data,
        };
        let mut chat = Compound::new();
        chat.put("translation_key", text("%s>%s"));
        chat.put(
            "parameters",
            Tag::List(vec![text("content"), text("target")]),
        );
        let mut style = Compound::new();
        style.put("color", text("red"));
        chat.put("style", Tag::Compound(style));
        let mut own = Compound::new();
        own.put("chat", Tag::Compound(chat));
        ChatTypes::from_registry(&[
            entry("minecraft:chat", None),
            entry("minecraft:msg_command_outgoing", None),
            entry("example:own", Some(Tag::Compound(own))),
        ])
    }

    fn signature(n: u8) -> Signature {
        Box::new([n; play::SIGNATURE_BYTES])
    }

    fn said(index: i32, content: &str, signed: Option<u8>) -> PlayerChat {
        PlayerChat {
            global_index: index,
            sender: 1,
            signature: signed.map(signature),
            content: content.to_owned(),
            time: 1000,
            last_seen: Vec::new(),
            unsigned_content: None,
            filter: FilterMask::PassThrough,
            kind: kind(0),
        }
    }

    #[test]
    fn only_messages_shown_and_not_taken_back_are_acknowledged() {
        use ruthenium_protocol::packets::play::{FilterMask, PackedSignature};
        let t = types();
        let mut s = HudTextState::default();
        let mut hidden = said(0, "hidden", Some(1));
        hidden.filter = FilterMask::FullyFiltered;
        s.player_chat(0, 0, &hidden, true, &t).unwrap();
        s.player_chat(0, 0, &said(1, "shown", Some(2)), true, &t)
            .unwrap();
        s.player_chat(0, 0, &said(2, "unknown", Some(3)), false, &t)
            .unwrap();
        s.player_chat(0, 0, &said(3, "taken back", Some(4)), true, &t)
            .unwrap();
        s.delete_chat(0, &PackedSignature::Full(signature(4)))
            .unwrap();
        let (seen, update) = s.last_seen_update();
        assert_eq!(seen, [signature(2)]);
        assert_eq!(update.offset, 4);
    }

    #[test]
    fn kinds_of_message_are_written_as_the_registry_says() {
        let t = types();
        let content = text("hello");
        let d = t.decorate(&kind(0), &content).unwrap();
        let c = d.as_compound().unwrap();
        assert_eq!(c.string("translate"), Some("chat.type.text"));
        assert_eq!(c.list("with"), [text("Kai"), text("hello")]);
        assert_eq!(c.string("color"), None);
        let to = BoundChatType {
            target: Some(text("Efe")),
            ..kind(1)
        };
        let d = t.decorate(&to, &content).unwrap();
        let c = d.as_compound().unwrap();
        assert_eq!(
            c.string("translate"),
            Some("commands.message.display.outgoing")
        );
        assert_eq!(c.list("with"), [text("Efe"), text("hello")]);
        assert_eq!(
            (c.string("color"), c.boolean("italic")),
            (Some("gray"), Some(true))
        );
        let d = t.decorate(&kind(2), &content).unwrap();
        let c = d.as_compound().unwrap();
        assert_eq!(c.list("with"), [text("hello"), text("")]);
        assert_eq!(c.string("color"), Some("red"));
        assert!(t.decorate(&kind(3), &content).is_err());
        let direct = BoundChatType {
            kind: ChatTypeRef::Direct(Box::new(ChatTypes::vanilla("emote_command"))),
            ..kind(0)
        };
        let d = t.decorate(&direct, &content).unwrap();
        assert_eq!(
            d.as_compound().unwrap().string("translate"),
            Some("chat.type.emote")
        );
    }

    #[test]
    fn messages_are_marked_by_how_far_they_can_be_trusted() {
        let mut s = HudTextState::default();
        let t = types();
        let mut tag = |chat: &PlayerChat, now: i64, known: bool| {
            s.session.next_index = chat.global_index;
            s.player_chat(5, now, chat, known, &t).unwrap();
            let m = shown(&mut s).chat[0].clone();
            assert_eq!(m.added, 5);
            m.tag
        };
        assert_eq!(tag(&said(0, "hi", Some(1)), 1000, true), None);
        assert_eq!(
            tag(&said(1, "hi", None), 1000, true),
            Some(ChatTag::NOT_SECURE)
        );
        // A signature counts for seven minutes.
        assert_eq!(tag(&said(2, "hi", Some(2)), 421_000, true), None);
        assert_eq!(
            tag(&said(3, "hi", Some(3)), 421_001, true),
            Some(ChatTag::NOT_SECURE)
        );
        let shows = |index, unsigned: Tag, n| PlayerChat {
            unsigned_content: Some(unsigned),
            ..said(index, "hi", Some(n))
        };
        assert_eq!(tag(&shows(4, text("oh hi!"), 4), 1000, true), None);
        assert_eq!(
            tag(&shows(5, text("no"), 5), 1000, true),
            Some(ChatTag::MODIFIED)
        );
        let mut fancy = Compound::new();
        fancy.put("text", text("hi"));
        fancy.put("font", text("minecraft:alt"));
        assert_eq!(
            tag(&shows(6, Tag::Compound(fancy), 6), 1000, true),
            Some(ChatTag::MODIFIED)
        );
        assert_eq!(
            tag(&said(7, "hi", Some(7)), 1000, false),
            Some(ChatTag::ERROR)
        );
        let m = shown(&mut s).chat[0].clone();
        assert!(plain(&m.content).contains("chat.type.text"));
        let with = m.content.as_compound().unwrap().list("with").to_vec();
        assert_eq!(plain(&with[1]), "chat.validation_error");
        // The server's own.
        s.system_message(9, text("Saved"));
        s.disguised(9, &text("psst"), &kind(0), &t).unwrap();
        let chat = shown(&mut s).chat;
        assert_eq!(chat[1].content, Arc::new(text("Saved")));
        assert_eq!([chat[0].tag, chat[1].tag], [Some(ChatTag::SYSTEM); 2]);
        assert_eq!(chat.len(), 10);
    }

    #[test]
    fn filters_hide_parts_of_a_message_or_all_of_it() {
        let mut s = HudTextState::default();
        let t = types();
        let mut chat = said(0, "so bad 😀 x", None);
        // Units 3, 4 and 5, and the second half of the face.
        chat.filter = FilterMask::Partly(vec![0b0011_1000, 0b0000_0001]);
        s.player_chat(0, 0, &chat, true, &t).unwrap();
        let with = |s: &mut HudTextState| {
            let m = shown(s).chat[0].clone();
            m.content.as_compound().unwrap().list("with")[1].clone()
        };
        let Tag::Compound(c) = with(&mut s) else {
            panic!()
        };
        let parts = c.list("extra");
        assert_eq!(parts[0], text("so "));
        let hidden = parts[1].as_compound().unwrap();
        assert_eq!(
            (hidden.string("text"), hidden.string("color")),
            (Some("###"), Some("dark_gray"))
        );
        assert!(plain(&parts[3]) == "#" && plain(&parts[4]) == " x");
        // Nothing of it: no message.
        let mut chat = said(1, "bad", None);
        chat.filter = FilterMask::FullyFiltered;
        s.player_chat(0, 0, &chat, true, &t).unwrap();
        assert_eq!(shown(&mut s).chat.len(), 1);
        assert_eq!(
            filtered("ab", &[0xFF, 0xFF])
                .as_compound()
                .unwrap()
                .list("extra")
                .len(),
            1
        );
    }

    #[test]
    fn the_server_is_told_of_every_65_signed_messages() {
        let mut s = HudTextState::default();
        let t = types();
        let mut index = 0;
        let mut say = |s: &mut HudTextState, signed| {
            index += 1;
            s.player_chat(0, 0, &said(index - 1, "x", signed), true, &t)
                .unwrap()
        };
        for n in 0..64 {
            assert_eq!(say(&mut s, Some(n)), None);
        }
        // Unsigned ones do not count, nor the same one again.
        assert_eq!(say(&mut s, None), None);
        assert_eq!(say(&mut s, Some(63)), None);
        assert_eq!(say(&mut s, Some(64)), Some(65));
        assert_eq!(s.take_acknowledgement(), None);
        assert_eq!(say(&mut s, Some(65)), None);
        assert_eq!(s.take_acknowledgement(), Some(1));
        // A message that does not follow the last ends the connection.
        assert!(s.player_chat(0, 0, &said(3, "x", None), true, &t).is_err());
    }

    #[test]
    fn signatures_are_kept_for_messages_to_refer_to() {
        let mut s = HudTextState::default();
        let t = types();
        let mut a = said(0, "a", Some(1));
        s.player_chat(0, 0, &a, true, &t).unwrap();
        // The second saw the first, and one that only it knows.
        a = said(1, "b", Some(2));
        a.last_seen = vec![
            PackedSignature::Full(signature(9)),
            PackedSignature::Cached(0),
        ];
        s.player_chat(0, 0, &a, true, &t).unwrap();
        // The newest first: its own, then what it saw, the last seen first.
        assert_eq!(
            s.session.signatures,
            [signature(2), signature(1), signature(9)]
        );
        a = said(2, "c", Some(3));
        a.last_seen = vec![PackedSignature::Cached(3)];
        assert!(s.player_chat(0, 0, &a, true, &t).is_err());
        for n in 10..150u8 {
            s.session.push(Vec::new(), Some(&signature(n)));
        }
        assert_eq!(s.session.signatures.len(), 128);
        assert_eq!(s.session.signatures[0], signature(149));
    }

    #[test]
    fn deleted_messages_show_for_three_seconds_first() {
        let mut s = HudTextState::default();
        let t = types();
        s.player_chat(100, 0, &said(0, "bad", Some(1)), true, &t)
            .unwrap();
        s.player_chat(100, 0, &said(1, "worse", Some(2)), true, &t)
            .unwrap();
        let cached = |i| PackedSignature::Cached(i);
        // The older one long after, at once.
        s.delete_chat(160, &cached(1)).unwrap();
        let chat = shown(&mut s).chat;
        assert_eq!(plain(&chat[1].content), "chat.deleted_marker");
        assert_eq!((chat[1].added, chat[1].tag), (100, Some(ChatTag::SYSTEM)));
        assert!(plain(&chat[0].content).contains("chat.type.text"));
        // A new one when it has been seen.
        s.player_chat(200, 0, &said(2, "new", Some(3)), true, &t)
            .unwrap();
        s.delete_chat(210, &cached(0)).unwrap();
        s.tick(259);
        let is_gone =
            |s: &mut HudTextState| plain(&shown(s).chat[0].content) == "chat.deleted_marker";
        assert!(!is_gone(&mut s));
        s.tick(260);
        assert!(is_gone(&mut s));
        assert!(s.deletions.is_empty());
        // One that is not known ends the connection.
        assert!(s.delete_chat(300, &cached(3)).is_err());
    }

    #[test]
    fn the_chat_keeps_a_hundred_messages_through_a_reconfiguration() {
        let mut s = HudTextState::default();
        for i in 0..120 {
            s.system_message(i, text(&i.to_string()));
        }
        let chat = shown(&mut s).chat;
        assert_eq!((chat.len(), chat[0].added, chat[99].added), (100, 119, 20));
        // What has been published stays as it was.
        s.system_message(120, text("new"));
        assert_eq!((chat[0].added, shown(&mut s).chat[0].added), (119, 120));
        s.set_title(text("t"));
        s.set_overlay(text("o"), false);
        s.set_tab_list(text("h"), text(""));
        s.session.next_index = 7;
        s.boss_event(1, boss("b", 1.0), Instant::now());
        s.reconfigure();
        let after = shown(&mut s);
        assert!(after.chat.len() == 100 && after.overlay.is_some());
        assert!(after.title.is_none() && after.bosses.is_empty());
        assert_eq!((s.session.next_index, s.header), (0, None));
    }

    fn boss(name: &str, progress: f32) -> BossEvent {
        BossEvent::Add {
            name: text(name),
            progress,
            color: 1,
            overlay: 2,
            flags: 0,
        }
    }

    #[test]
    fn boss_bars_stay_in_order_and_fill_over_a_tenth_of_a_second() {
        let mut s = HudTextState::default();
        let start = Instant::now();
        let at = |ms: u64| start + Duration::from_millis(ms);
        s.boss_event(1, boss("a", 0.5), at(0));
        s.boss_event(2, boss("b", 1.0), at(0));
        s.boss_event(1, BossEvent::Progress(1.0), at(1000));
        let bars = shown(&mut s).bosses;
        let a = &bars[0];
        let progress = [999, 1000, 1050, 1100, 5000].map(|ms| a.progress(at(ms)));
        assert_eq!(progress, [0.5, 0.5, 0.75, 1.0, 1.0]);
        // Set again on the way, it goes on from where it is.
        s.boss_event(1, BossEvent::Progress(0.0), at(1050));
        let a = shown(&mut s).bosses[0].clone();
        assert_eq!((a.progress(at(1050)), a.progress(at(1150))), (0.75, 0.0));
        s.boss_event(2, BossEvent::Name(text("c")), at(2000));
        s.boss_event(
            2,
            BossEvent::Style {
                color: 6,
                overlay: 0,
            },
            at(2000),
        );
        s.boss_event(2, BossEvent::Properties { flags: 5 }, at(2000));
        let b = shown(&mut s).bosses[1].clone();
        assert_eq!(
            (&*b.name, b.color, b.overlay, b.flags),
            (&text("c"), 6, 0, 5)
        );
        // Added again it keeps its place; what is not there is not changed.
        s.boss_event(1, boss("again", 0.25), at(3000));
        s.boss_event(9, BossEvent::Progress(0.5), at(3000));
        let names: Vec<Tag> = shown(&mut s)
            .bosses
            .iter()
            .map(|b| (*b.name).clone())
            .collect();
        assert_eq!(names, [text("again"), text("c")]);
        s.boss_event(1, BossEvent::Remove, at(4000));
        assert_eq!(shown(&mut s).bosses.len(), 1);
    }

    #[test]
    fn the_player_list_is_in_order_and_only_there_when_wanted() {
        let mut players = PlayerInfos::default();
        let entry = |uuid, name: &str| play::PlayerInfoEntry {
            uuid,
            name: Some(name.into()),
            listed: Some(true),
            latency: Some(20),
            game_mode: Some(0),
            ..Default::default()
        };
        let mut entries = vec![
            entry(1, "zed"),
            entry(2, "Ann"),
            entry(3, "bob"),
            entry(4, "Spec"),
            entry(5, "first"),
            entry(6, "hidden"),
        ];
        entries[3].game_mode = Some(play::SPECTATOR);
        entries[4].list_order = Some(1);
        entries[5].listed = Some(false);
        entries[2].display_name = Some(Some(text("Bobby")));
        entries[2].show_hat = Some(false);
        players.update(play::PLAYER_INFO_ADD, &entries);
        let mut teams = Teams::default();
        let params = play::TeamParameters {
            display_name: text(""),
            prefix: text("<"),
            suffix: text(">"),
            name_tag_visibility: 0,
            collision_rule: 0,
            color: None,
            options: 0,
        };
        teams.apply("a", 0, Some(&params), &["zed".to_owned()]);
        let mut scores = Scoreboard::default();
        scores.set_objective("o", 0, Some(text("")), true, None);
        scores.set_display(play::DISPLAY_SLOT_LIST, "o");
        scores.set_score("Ann", "o", 13, None, None);
        let mut s = HudTextState::default();
        let mut out = HudTexts::default();
        let mut fill = |s: &mut HudTextState, wanted, scores: &Scoreboard| {
            s.fill(&mut out, 0, "Kai", wanted, true, &players, &teams, scores);
            out.players.clone()
        };
        assert_eq!(fill(&mut s, false, &scores), None);
        let list = fill(&mut s, true, &scores).unwrap();
        let ids: Vec<u128> = list.players.iter().map(|p| p.id).collect();
        // By order, spectators last, then by team (none first), then by name.
        assert_eq!(ids, [5, 2, 3, 1, 4]);
        let of = |id| list.players.iter().find(|p| p.id == id).unwrap();
        assert_eq!(plain(&of(1).name), "<zed>");
        assert_eq!(*of(3).name, text("Bobby"));
        assert!(!of(3).show_hat && of(2).show_hat);
        assert_eq!(
            of(4).name.as_compound().unwrap().boolean("italic"),
            Some(true)
        );
        assert!(of(4).spectator && !of(2).spectator);
        assert_eq!(of(2).score.as_ref().unwrap().0, 13);
        assert_eq!((of(1).score.as_ref(), of(1).latency), (None, 20));
        assert_eq!((list.scores, list.faces), (ListScores::Hearts, true));
        // The same one until something changes.
        let again = fill(&mut s, true, &scores).unwrap();
        assert!(Arc::ptr_eq(&list, &again));
        s.set_tab_list(text("Top"), text(""));
        let list = fill(&mut s, true, &scores).unwrap();
        assert_eq!(
            (list.header.as_deref(), &list.footer),
            (Some(&text("Top")), &None)
        );
        scores.set_score("Ann", "o", 14, None, None);
        let list = fill(&mut s, true, &scores).unwrap();
        assert_eq!(list.players[1].score.as_ref().unwrap().0, 14);
    }

    #[test]
    fn the_sidebar_is_made_again_only_when_something_changed() {
        let mut scores = Scoreboard::default();
        scores.set_objective("o", 0, Some(text("O")), false, None);
        scores.set_display(play::DISPLAY_SLOT_SIDEBAR, "o");
        let mut s = HudTextState::default();
        let fill = |s: &mut HudTextState, scores: &Scoreboard| {
            let mut out = HudTexts::default();
            let (players, teams) = (PlayerInfos::default(), Teams::default());
            s.fill(&mut out, 0, "Kai", false, false, &players, &teams, scores);
            out.sidebar.unwrap()
        };
        let first = fill(&mut s, &scores);
        assert!(Arc::ptr_eq(&first, &fill(&mut s, &scores)));
        scores.set_score("Kai", "o", 1, None, None);
        assert_eq!(fill(&mut s, &scores).lines.len(), 1);
    }

    #[test]
    fn the_selected_items_name_shows_while_it_is_highlighted() {
        let mut s = HudTextState::default();
        let stack = Stack {
            identity: 1,
            item: 1,
            count: 1,
            c: Arc::new(crate::item::Components {
                custom_name: Some(Arc::new(text("Mine"))),
                ..Default::default()
            }),
            ..Stack::empty()
        };
        s.select(&stack);
        let mut out = HudTexts::default();
        let mut fill = |s: &mut HudTextState, highlight| {
            s.fill(
                &mut out,
                highlight,
                "Kai",
                false,
                false,
                &PlayerInfos::default(),
                &Teams::default(),
                &Scoreboard::default(),
            );
            out.selected.clone().map(|(t, left)| (plain(&t), left))
        };
        assert_eq!(fill(&mut s, 40), Some(("Mine".to_owned(), 40)));
        assert_eq!(fill(&mut s, 0), None);
        s.select(&Stack::empty());
        assert_eq!(fill(&mut s, 40), None);
    }
}

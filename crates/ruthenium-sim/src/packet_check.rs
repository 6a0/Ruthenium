//! The packet-order checker (research 02 §12, PLAN §2.1): it watches every packet the
//! game thread sends, with the tick boundaries, the packets the game thread handled and
//! a little of the simulation's state, and flags any break of the invariants that can be
//! judged from that stream. Each [`Violation`] names the invariant (`INV-T1`...), the
//! packet and the tick.
//!
//! Where it hooks in: [`Outbound`] wraps the session's connection, and every packet the
//! game thread sends goes through its `send`: that is the one choke point, since the
//! session, its containers and its resource packs all send through `Game.connection`.
//! The frames and ticks are marked around `Game::tick`, the handled packets at the top
//! of `Game::play`, and the simulation tells it the selected slot, whether the player
//! rides and whether a screen is open where vanilla reads them. The connection's own
//! thread answers keep-alives and chunk batches, as vanilla's network thread does
//! (`handleKeepAlive` and `handleChunkBatchFinished` don't hop to the main thread), so
//! those are not seen here (INV-R2, INV-R3).
//!
//! Vanilla is the rule. The checker runs over the outbound stream of every golden trace
//! recorded from vanilla's client (`tests/packet_check.rs`), so a rule vanilla breaks is
//! a wrong rule. Where research 02 §12 says more than vanilla does, the rule here follows
//! vanilla:
//! - INV-T4: `ABORT_DESTROY_BLOCK` is also sent outside a tick, by `pauseGame`
//!   (`Minecraft.java:1586`) and while handling a teleport
//!   (`ClientPacketListener.java:798`); the bed screen's Leave Bed sends `STOP_SLEEPING`
//!   at frame time (`InBedChatScreen.java:31`).
//! - INV-O1: `startDestroyBlock` doesn't send the held slot first
//!   (`MultiPlayerGameMode.java:153`), so a block hit right after a hotbar key goes out
//!   with the server's old slot; only the actions that call `ensureHasSentCarriedItem`
//!   are held to it. Two different held slots in one tick are fine (PLAN §1.5).
//! - INV-O3: a punch also comes alone (a miss, or `continueDestroyBlock` counting down
//!   its delay, `Minecraft.java:1604`); a spectator's attack has none
//!   (`Minecraft.java:1640`).
//! - INV-C5/C6: `ABORT_DESTROY_BLOCK` and `CHANGE_DESTROY_DIRECTION` carry sequence 0
//!   (they aren't predictions), and the abort of an old target carries the new face
//!   (`MultiPlayerGameMode.java:180`).
//!
//! Not judged here, as they need more than the stream: INV-S1..S3 (the pick, when the
//! rotation changes, the physics: the golden replays compare those bit for bit), INV-C4
//! (the collision flags), the raycast face of INV-C6, INV-C8's hit location, INV-C11's
//! "no attack or use while using an item or with busy hands", INV-C12's "never while
//! gliding" (only one start per tick is), the clock of INV-T3 (only the 10-tick cap and
//! packets handled inside a tick are), INV-R2 and INV-R3 (the connection thread's).
//!
//! It is on in debug builds and tests (a violation panics in this crate's tests and is
//! logged loudly otherwise), and off in release builds unless `--check-packets on`
//! ([`set_enabled`]); off, sending costs one check of an `Option`.

use std::cell::RefCell;
use std::fmt;
use std::sync::atomic::{AtomicU8, Ordering};

use ruthenium_data::v26_3::packets::play::{Clientbound as C, Serverbound as S};
use ruthenium_net::connection::{Connection, Protocol};
use ruthenium_protocol::packets::{Common, play};
use ruthenium_protocol::wire::{Reader, WireError};

/// Whether sessions check their packets: unset (the build's default), on or off.
static ENABLED: AtomicU8 = AtomicU8::new(0);

/// Turns the checker on or off for the sessions started from now on (`--check-packets`).
pub fn set_enabled(on: bool) {
    ENABLED.store(if on { 1 } else { 2 }, Ordering::Relaxed);
}

/// Whether new sessions check their packets: on in debug builds unless turned off.
pub fn enabled() -> bool {
    match ENABLED.load(Ordering::Relaxed) {
        1 => true,
        2 => false,
        _ => cfg!(debug_assertions),
    }
}

/// What a violation does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OnViolation {
    /// Panic (tests).
    Panic,
    /// Print it loudly to stderr, and keep the first thousand.
    Log,
    /// Only keep it (tests that look at them).
    Collect,
}

/// When a violation happened: during a tick, after one (handling packets, the screens'
/// buttons), or before the first.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum At {
    Tick(u64),
    After(u64),
    Start,
}

impl fmt::Display for At {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            At::Tick(n) => write!(f, "tick {n}"),
            At::After(n) => write!(f, "after tick {n}"),
            At::Start => write!(f, "before the first tick"),
        }
    }
}

/// A broken invariant.
#[derive(Debug, Clone, PartialEq)]
pub struct Violation {
    /// The invariant's id in research 02 §12 (`INV-T1`...), or `MALFORMED` for a packet
    /// whose fields don't read.
    pub invariant: &'static str,
    /// The packet's name (`minecraft:...`): the one sent, or the handled one left
    /// unanswered.
    pub packet: &'static str,
    pub at: At,
    pub detail: String,
}

impl fmt::Display for Violation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} at {} ({}): {}",
            self.invariant, self.at, self.packet, self.detail
        )
    }
}

/// Where in a tick a packet belongs (`Minecraft.tick` and `handleKeybinds`, in order).
mod rank {
    /// `gameMode.tick` (the held slot), the listener's tick (`player_loaded`, the chat
    /// session), `Gui.tick` (a respawn request, a screen's close).
    pub const START: u8 = 0;
    /// The hotbar keys (a spectator's teleport) and the inventory key.
    pub const HOTBAR: u8 = 1;
    pub const SWAP: u8 = 2;
    pub const DROP: u8 = 3;
    /// The release of a used item, or the attack clicks.
    pub const ATTACK: u8 = 4;
    pub const USE: u8 = 5;
    pub const PICK: u8 = 6;
    /// The use key held (`rightClickDelay` run out).
    pub const HELD_USE: u8 = 7;
    /// `continueAttack`.
    pub const CONTINUE: u8 = 8;
    /// `level.tickEntities`: the player's `aiStep` and its vehicle's tick.
    pub const ENTITY: u8 = 9;
    /// `sendChanges`: the input, the sprint, the movement; riding: the input, the
    /// rotation, the vehicle, the sprint.
    pub const INPUT: u8 = 10;
    pub const SPRINT: u8 = 11;
    pub const MOVE: u8 = 12;
    pub const VEHICLE: u8 = 13;
    pub const VEHICLE_SPRINT: u8 = 14;
}

/// `ServerboundPlayerActionPacket.Action` ordinals.
const ACTION_START: i32 = play::ACTION_START_DESTROY_BLOCK;
const ACTION_ABORT: i32 = play::ACTION_ABORT_DESTROY_BLOCK;
const ACTION_STOP: i32 = play::ACTION_STOP_DESTROY_BLOCK;
const ACTION_CHANGE: i32 = play::ACTION_CHANGE_DESTROY_DIRECTION;
const ACTION_DROP_ALL: i32 = play::ACTION_DROP_ALL_ITEMS;
const ACTION_DROP: i32 = play::ACTION_DROP_ITEM;
const ACTION_RELEASE: i32 = play::ACTION_RELEASE_USE_ITEM;
const ACTION_SWAP: i32 = play::ACTION_SWAP_ITEM_WITH_OFFHAND;
const ACTION_STAB: i32 = play::ACTION_STAB;
/// `ServerboundPlayerCommandPacket.Action` ordinals.
const STOP_SLEEPING: i32 = 0;
const START_SPRINTING: i32 = 1;
const STOP_SPRINTING: i32 = 2;
const START_RIDING_JUMP: i32 = play::PLAYER_COMMAND_START_RIDING_JUMP;
const STOP_RIDING_JUMP: i32 = play::PLAYER_COMMAND_STOP_RIDING_JUMP;
const OPEN_INVENTORY: i32 = play::PLAYER_COMMAND_OPEN_INVENTORY;
const START_FALL_FLYING: i32 = 6;

/// A packet that must be the next one sent.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Expect {
    /// `handleMovePlayer`'s accept (`ClientPacketListener.java:794`): the teleport's
    /// id, and the player's place and rotation after it where the teleport set them
    /// outright and the player doesn't ride.
    Accept {
        id: i32,
        position: Option<[f64; 3]>,
        y_rot: Option<f32>,
        x_rot: Option<f32>,
    },
    /// `handleRotatePlayer`'s `Rot(yRot, xRot, false, false)`
    /// (`ClientPacketListener.java:836`).
    Rot,
    /// `handlePing`'s pong (`ClientCommonPacketListenerImpl.java:153`).
    Pong(i32),
}

impl Expect {
    fn invariant(&self) -> &'static str {
        match self {
            Expect::Accept { .. } => "INV-C9",
            Expect::Rot => "INV-T2",
            Expect::Pong(_) => "INV-R1",
        }
    }

    fn answering(&self) -> &'static str {
        match self {
            Expect::Accept { .. } => C::PlayerPosition.name(),
            Expect::Rot => C::PlayerRotation.name(),
            Expect::Pong(_) => C::Ping.name(),
        }
    }
}

/// The simulation's state the checker is told of, for the rules the stream alone can't
/// judge (`None`: not known, nothing judged on it).
#[derive(Debug, Clone, Copy, Default)]
struct Context {
    /// The selected hotbar slot as the key binds' packets go out.
    selected: Option<i32>,
    /// The player rides something as it sends its changes.
    riding: Option<bool>,
    /// A screen is open over the world where the key binds would run (so they don't).
    screen: Option<bool>,
}

/// The checker: a state machine over the outbound stream (research 02 §13.2's
/// "Grim-lite" linter).
#[derive(Debug)]
pub struct PacketChecker {
    on_violation: OnViolation,
    violations: Vec<Violation>,
    count: u64,
    /// The stream is in the play protocol (nothing else is checked).
    play: bool,
    /// The tick under way, if one is, and the last one.
    tick: Option<u64>,
    last_tick: Option<u64>,
    /// A level exists: its ticks end with `client_tick_end`.
    level: bool,
    frame_ticks: u32,
    // This tick's packets so far.
    rank: u8,
    previous: Option<(S, i32)>,
    ended: bool,
    moves: u32,
    sprints: u32,
    inputs: u32,
    elytra: u32,
    use_item_rots: Vec<(f32, f32)>,
    tick_rot: Option<(f32, f32)>,
    /// Position-bearing movement packets since the last `client_tick_end`.
    positions: u32,
    /// The answer owed to the packet just handled.
    expect: Option<Expect>,
    /// A vehicle move or teleport was just handled: a `move_vehicle` may answer it.
    vehicle_answer: bool,
    /// The passengers changed since the last tick (whether the player rides is stale).
    passengers_changed: bool,
    // What the client last told the server (`None`: not known).
    /// `MultiPlayerGameMode.carriedIndex`.
    carried: Option<i32>,
    /// The slot the server holds selected.
    server_slot: Option<i32>,
    /// `LocalPlayer.wasSprinting`.
    sprinting: Option<bool>,
    /// `BlockStatePredictionHandler.currentSequenceNr`.
    sequence: Option<i32>,
    /// `LocalPlayer.lastSentInput`.
    input: Option<u8>,
    /// `xLast`/`yLast`/`zLast` and the tick it was sent in, `yRotLast`/`xRotLast`,
    /// `lastOnGround`/`lastHorizontalCollision`.
    last_position: Option<([f64; 3], u64)>,
    last_rot: Option<(f32, f32)>,
    last_flags: Option<u8>,
    /// The player has loaded into the level (`player_loaded` sent).
    loaded: Option<bool>,
    dimension: Option<String>,
    /// The open menu's id (0: the inventory's) and state id, the inventory menu's state id.
    container: Option<i32>,
    container_state: Option<i32>,
    inventory_state: Option<i32>,
    context: Context,
}

impl PacketChecker {
    /// A checker for a session from its start. It takes the stream as play once the
    /// connection switches to play or a login is handled.
    pub fn new(on_violation: OnViolation) -> PacketChecker {
        PacketChecker {
            on_violation,
            violations: Vec::new(),
            count: 0,
            play: false,
            tick: None,
            last_tick: None,
            level: false,
            frame_ticks: 0,
            rank: rank::START,
            previous: None,
            ended: false,
            moves: 0,
            sprints: 0,
            inputs: 0,
            elytra: 0,
            use_item_rots: Vec::new(),
            tick_rot: None,
            positions: 0,
            expect: None,
            vehicle_answer: false,
            passengers_changed: false,
            carried: None,
            server_slot: None,
            sprinting: None,
            sequence: None,
            input: None,
            last_position: None,
            last_rot: None,
            last_flags: None,
            loaded: None,
            dimension: None,
            container: None,
            container_state: None,
            inventory_state: None,
            context: Context::default(),
        }
    }

    /// A checker for a stream already in play and in a level, with nothing known of what
    /// was sent before (a golden trace: vanilla's client had joined before it began).
    pub fn in_play(on_violation: OnViolation) -> PacketChecker {
        let mut c = PacketChecker::new(on_violation);
        c.play = true;
        c.level = true;
        c
    }

    /// The violations kept so far.
    pub fn violations(&self) -> &[Violation] {
        &self.violations
    }

    /// How many violations there have been.
    pub fn count(&self) -> u64 {
        self.count
    }

    fn at(&self) -> At {
        match (self.tick, self.last_tick) {
            (Some(n), _) => At::Tick(n),
            (None, Some(n)) => At::After(n),
            (None, None) => At::Start,
        }
    }

    fn flag(&mut self, invariant: &'static str, packet: &'static str, detail: String) {
        let v = Violation {
            invariant,
            packet,
            at: self.at(),
            detail,
        };
        self.count += 1;
        match self.on_violation {
            OnViolation::Panic => panic!("packet-order checker: {v}"),
            OnViolation::Log => {
                eprintln!("[packet-check] !!! VIOLATION {v}");
                // The first ones only: a broken rule can repeat every tick.
                if self.violations.len() < 1000 {
                    self.violations.push(v);
                }
            }
            OnViolation::Collect => self.violations.push(v),
        }
    }

    /// The answer owed to a handled packet didn't come next.
    fn missing_answer(&mut self, e: Expect, instead: &str) {
        let detail = format!("not answered at once ({instead} came first)");
        self.flag(e.invariant(), e.answering(), detail);
    }

    /// A frame begins (`Minecraft.runTick`).
    pub fn frame(&mut self) {
        self.frame_ticks = 0;
    }

    /// Client tick `n` begins, with or without a level.
    pub fn begin_tick(&mut self, n: u64, level: bool) {
        if let Some(e) = self.expect.take() {
            self.missing_answer(e, "a tick");
        }
        self.vehicle_answer = false;
        self.frame_ticks += 1;
        self.tick = Some(n);
        if self.play {
            self.level = level;
        }
        if self.frame_ticks > 10 {
            let detail = format!(
                "{} ticks in one frame (vanilla runs 10 at most)",
                self.frame_ticks
            );
            self.flag("INV-T3", "", detail);
        }
        self.rank = rank::START;
        self.previous = None;
        self.ended = false;
        self.moves = 0;
        self.sprints = 0;
        self.inputs = 0;
        self.elytra = 0;
        self.use_item_rots.clear();
        self.tick_rot = None;
        self.context.selected = None;
        self.context.screen = None;
        // Whether the player rides holds from the last tick unless the passengers changed.
        if std::mem::take(&mut self.passengers_changed) {
            self.context.riding = None;
        }
    }

    /// The tick ends.
    pub fn end_tick(&mut self) {
        let Some(n) = self.tick else { return };
        if self.play && self.level && !self.ended {
            let detail = "a tick in a level didn't end with client_tick_end".into();
            self.flag("INV-T1", S::ClientTickEnd.name(), detail);
        }
        // INV-C7: a use carries the tick's rotation, which this tick's movement packet
        // carries if it turned; else it is the one last sent.
        let used = std::mem::take(&mut self.use_item_rots);
        if self.context.riding == Some(false)
            && let Some(rot) = self.tick_rot.or(self.last_rot)
        {
            for u in used {
                if !same_rot(u, rot) {
                    let detail = format!("rotation {u:?}, the tick's is {rot:?}");
                    self.flag("INV-C7", S::UseItem.name(), detail);
                }
            }
        }
        self.last_tick = Some(n);
        self.tick = None;
    }

    /// The simulation's selected hotbar slot, as the key binds' packets go out.
    pub fn set_selected(&mut self, slot: i32) {
        self.context.selected = Some(slot);
    }

    /// Whether the player rides something, as its changes go out.
    pub fn set_riding(&mut self, riding: bool) {
        self.context.riding = Some(riding);
    }

    /// Whether a screen is open over the world where the key binds would run.
    pub fn set_screen(&mut self, open: bool) {
        self.context.screen = Some(open);
    }

    /// The connection switched protocol (after its acknowledgement, which was sent).
    pub fn switched(&mut self, to: Protocol) {
        self.play = to == Protocol::Play;
        if !self.play {
            self.level = false;
        }
    }

    /// A clientbound play packet is handled, in order.
    pub fn handled(&mut self, p: &play::Clientbound) {
        if let play::Clientbound::Login(_) = p {
            self.play = true;
        }
        if !self.handling() {
            return;
        }
        match p {
            play::Clientbound::Common(Common::Ping { id }) => self.expect = Some(Expect::Pong(*id)),
            play::Clientbound::Login(login) => {
                // `handleLogin`: a new game mode (held slot 0), level (sequence 0) and
                // player, which has sent nothing and waits for the level to load.
                self.level = true;
                self.carried = Some(0);
                self.server_slot = None;
                self.sprinting = Some(false);
                self.sequence = Some(0);
                self.input = Some(0);
                self.new_player();
                self.dimension = Some(login.spawn.dimension.clone());
            }
            play::Clientbound::Respawn {
                spawn,
                data_to_keep,
            } => {
                // `handleRespawn` (`ClientPacketListener.java:1251`): a new level only for
                // another dimension; a new player, which keeps the input it last sent and
                // takes the old one's sprint as sent when the entity data is kept.
                if self.dimension.as_deref() != Some(spawn.dimension.as_str()) {
                    self.sequence = Some(0);
                }
                self.dimension = Some(spawn.dimension.clone());
                if data_to_keep & 2 != 0 {
                    self.sprinting = None;
                    self.input = None;
                } else {
                    self.sprinting = Some(false);
                    self.input = Some(0);
                }
                self.new_player();
            }
            play::Clientbound::StartConfiguration => self.level = false,
            play::Clientbound::PlayerPosition(t) => {
                // A passenger stays put (`handleMovePlayer`), so only a player known not to
                // ride is held to where the teleport put it.
                let riding = self.context.riding != Some(false) || self.passengers_changed;
                let outright = |bits: u32| t.relatives & bits == 0 && !riding;
                self.expect = Some(Expect::Accept {
                    id: t.id,
                    position: outright(0b111).then_some(t.position),
                    // `PositionMoveRotation.calculateAbsolute` clamps the pitch.
                    y_rot: outright(1 << 3).then_some(0.0 + t.y_rot),
                    x_rot: outright(1 << 4).then_some(t.x_rot.clamp(-90.0, 90.0)),
                });
            }
            play::Clientbound::Other(C::PlayerRotation) => self.expect = Some(Expect::Rot),
            play::Clientbound::SetHeldSlot { slot } => {
                if (0..9).contains(slot) {
                    self.server_slot = Some(*slot);
                }
            }
            play::Clientbound::OpenScreen { container_id, .. }
            | play::Clientbound::MountScreenOpen { container_id, .. } => {
                self.container = Some(*container_id);
                self.container_state = Some(0);
            }
            play::Clientbound::ContainerClose { .. } => {
                self.container = Some(0);
                self.container_state = None;
            }
            play::Clientbound::ContainerSetContent {
                container_id,
                state_id,
                ..
            }
            | play::Clientbound::ContainerSetSlot {
                container_id,
                state_id,
                ..
            } => {
                // `handleContainerContent`, `handleContainerSetSlot`: id 0 is the
                // inventory menu's whatever is open, the open menu takes its own.
                if *container_id == 0 {
                    self.inventory_state = Some(*state_id);
                } else if Some(*container_id) == self.container {
                    self.container_state = Some(*state_id);
                }
            }
            play::Clientbound::MoveVehicle { .. } | play::Clientbound::TeleportEntity { .. } => {
                self.vehicle_answer = true;
            }
            play::Clientbound::SetPassengers { .. } | play::Clientbound::RemoveEntities { .. } => {
                self.passengers_changed = true;
            }
            _ => {}
        }
    }

    /// A clientbound play packet is handled whose fields aren't known (a golden trace
    /// records only some packets' bytes): what it changes becomes unknown.
    pub fn handled_unread(&mut self, name: &str) {
        if !self.handling() {
            return;
        }
        match name {
            "minecraft:open_screen" | "minecraft:mount_screen_open" => {
                self.container = None;
                self.container_state = None;
            }
            "minecraft:login" | "minecraft:respawn" => {
                *self = PacketChecker {
                    violations: std::mem::take(&mut self.violations),
                    last_tick: self.last_tick,
                    count: self.count,
                    ..PacketChecker::in_play(self.on_violation)
                };
            }
            "minecraft:start_configuration" => self.level = false,
            "minecraft:set_passengers" | "minecraft:remove_entities" => {
                self.passengers_changed = true;
            }
            _ => {}
        }
    }

    /// What every handled packet does: the answer owed to the one before must have been
    /// sent, and no tick is under way. False outside play.
    fn handling(&mut self) -> bool {
        if !self.play {
            return false;
        }
        if let Some(e) = self.expect.take() {
            self.missing_answer(e, "another packet handled");
        }
        self.vehicle_answer = false;
        if self.tick.is_some() {
            self.flag("INV-T3", "", "a packet handled inside a tick".into());
        }
        true
    }

    /// A new local player (a login or a respawn) has sent nothing yet.
    fn new_player(&mut self) {
        self.last_position = None;
        self.last_rot = None;
        self.last_flags = None;
        self.loaded = Some(false);
        self.container = Some(0);
        self.container_state = None;
        self.inventory_state = Some(0);
        self.context.riding = Some(false);
        self.passengers_changed = false;
    }

    /// A packet the game thread sends (its id and fields).
    pub fn sent(&mut self, packet: &[u8]) {
        if !self.play {
            return;
        }
        let mut r = Reader::new(packet);
        let Some(id) = r.varint().ok().and_then(|id| S::from_id(id as u32)) else {
            self.flag("MALFORMED", "", format!("no play packet id: {packet:02x?}"));
            return;
        };
        let mut sub = -1;
        if let Err(e) = self.check(id, &mut r, &mut sub) {
            self.flag("MALFORMED", id.name(), format!("{e}: {packet:02x?}"));
        }
        if self.tick.is_some() {
            self.previous = Some((id, sub));
        }
    }

    /// Checks one packet; `sub` is set to its action (a player action's or command's).
    fn check(&mut self, id: S, r: &mut Reader<'_>, sub: &mut i32) -> Result<(), WireError> {
        let name = id.name();
        let in_tick = self.tick.is_some();
        let vehicle_answer = std::mem::take(&mut self.vehicle_answer);
        // An answer owed comes first.
        if let Some(e) = self.expect.take() {
            if self.answer(e, id, r)? {
                if in_tick {
                    self.flag(e.invariant(), name, "an answer sent inside a tick".into());
                }
                return Ok(());
            }
            self.missing_answer(e, name);
        }
        match id {
            S::PlayerAction => *sub = r.varint()?,
            S::PlayerCommand => {
                let _entity = r.varint()?;
                *sub = r.varint()?;
            }
            _ => {}
        }
        if !in_tick {
            return self.outside_tick(id, *sub, vehicle_answer, r);
        }
        if self.ended && id != S::KeepAlive && id != S::ClientTickEnd {
            self.flag(
                "INV-T1",
                name,
                "sent after the tick's client_tick_end".into(),
            );
        }
        match id {
            S::ClientTickEnd => {
                if !self.level {
                    self.flag(
                        "INV-T1",
                        name,
                        "a tick without a level ended with it".into(),
                    );
                }
                if self.ended {
                    self.flag("INV-T1", name, "a second one in the tick".into());
                }
                self.ended = true;
                self.positions = 0;
                return Ok(());
            }
            S::KeepAlive => return Ok(()),
            _ => {}
        }
        self.order(id, *sub);
        match id {
            S::SetCarriedItem => {
                let b = r.bytes(2)?;
                let slot = i32::from(i16::from_be_bytes([b[0], b[1]]));
                if !(0..9).contains(&slot) {
                    self.flag("INV-C1", name, format!("slot {slot} is not a hotbar slot"));
                }
                if self.carried == Some(slot) {
                    self.flag(
                        "INV-C1",
                        name,
                        format!("slot {slot} again (the last one sent)"),
                    );
                }
                self.carried = Some(slot);
                self.server_slot = Some(slot);
            }
            S::Attack => self.held_item(name),
            S::Interact => {
                self.held_item(name);
                let _entity = r.varint()?;
                let _hand = r.varint()?;
                let _location = r.lp_vec3()?;
                let sneaking = r.bool()?;
                // `isShiftKeyDown` reads the input of the last tick, which was sent then.
                if let Some(input) = self.input
                    && sneaking != (input & 1 << 5 != 0)
                {
                    let detail =
                        format!("sneaking {sneaking}, the input last sent is {input:#04x}");
                    self.flag("INV-C8", name, detail);
                }
            }
            S::UseItem => {
                self.held_item(name);
                let _hand = r.varint()?;
                let seq = r.varint()?;
                self.predicted(name, seq);
                self.use_item_rots.push((r.f32()?, r.f32()?));
            }
            S::UseItemOn => {
                self.held_item(name);
                let _hand = r.varint()?;
                let _pos = r.i64()?;
                let _face = r.varint()?;
                let _cursor = (r.f32()?, r.f32()?, r.f32()?);
                let _inside = r.bool()?;
                let _border = r.bool()?;
                let seq = r.varint()?;
                self.predicted(name, seq);
            }
            S::PlayerAction => {
                let pos = r.i64()?;
                let face = r.u8()?;
                let seq = r.varint()?;
                self.player_action(*sub, pos, face, seq);
            }
            S::PlayerCommand => self.player_command(*sub),
            S::PlayerInput => {
                let bits = r.u8()?;
                self.inputs += 1;
                if self.inputs > 1 {
                    self.flag("INV-O5", name, "a second input in one tick".into());
                }
                if self.input == Some(bits) {
                    let detail = format!("input {bits:#04x} sent again unchanged");
                    self.flag("INV-O5", name, detail);
                }
                if self.context.screen == Some(true) && bits != 0 {
                    let detail =
                        format!("input {bits:#04x} with a screen open (it lets go of the keys)");
                    self.flag("INV-C11", name, detail);
                }
                self.input = Some(bits);
                self.before_loaded(name);
            }
            S::MovePlayerPos | S::MovePlayerPosRot | S::MovePlayerRot | S::MovePlayerStatusOnly => {
                self.movement(id, r)?;
            }
            S::MoveVehicle => {
                self.vehicle_move(name, r)?;
                self.before_loaded(name);
            }
            S::PlayerLoaded => {
                if self.loaded == Some(true) {
                    self.flag("INV-R4", name, "sent twice for one level".into());
                }
                self.loaded = Some(true);
            }
            S::ContainerClick => self.container_click(r)?,
            S::ContainerClose => self.container_close(r)?,
            _ => {}
        }
        Ok(())
    }

    /// Whether `id` is the answer `e` is owed, checking it.
    fn answer(&mut self, e: Expect, id: S, r: &mut Reader<'_>) -> Result<bool, WireError> {
        let name = id.name();
        match (e, id) {
            (Expect::Pong(want), S::Pong) => {
                let got = r.i32()?;
                if got != want {
                    self.flag("INV-R1", name, format!("pong {got}, the ping was {want}"));
                }
            }
            (Expect::Rot, S::MovePlayerRot) => {
                let (y, x) = (r.f32()?, r.f32()?);
                let flags = r.u8()?;
                if flags != 0 {
                    let detail = format!("flags {flags} (vanilla answers with false, false)");
                    self.flag("INV-T2", name, detail);
                }
                self.rotation_values(name, y, x);
            }
            (
                Expect::Accept {
                    id: want,
                    position,
                    y_rot,
                    x_rot,
                },
                S::AcceptTeleportation,
            ) => {
                let got = r.varint()?;
                let pos = [r.f64()?, r.f64()?, r.f64()?];
                let (y, x) = (r.f32()?, r.f32()?);
                if got != want {
                    self.flag(
                        "INV-C9",
                        name,
                        format!("teleport {got}, the server's was {want}"),
                    );
                }
                if let Some(p) = position
                    && p != pos
                {
                    let detail = format!("accepted at {pos:?}, the teleport put it at {p:?}");
                    self.flag("INV-C9", name, detail);
                }
                if y_rot.is_some_and(|w| !same_f32(w, y)) || x_rot.is_some_and(|w| !same_f32(w, x))
                {
                    let detail = format!(
                        "accepted with rotation ({y}, {x}), the teleport set ({y_rot:?}, {x_rot:?})"
                    );
                    self.flag("INV-C9", name, detail);
                }
            }
            _ => return Ok(false),
        }
        Ok(true)
    }

    /// A packet sent outside a tick: what vanilla sends while handling packets and from
    /// its screens.
    fn outside_tick(
        &mut self,
        id: S,
        sub: i32,
        vehicle_answer: bool,
        r: &mut Reader<'_>,
    ) -> Result<(), WireError> {
        let name = id.name();
        match id {
            S::ClientTickEnd => self.flag("INV-T1", name, "sent outside a tick".into()),
            S::MovePlayerPos | S::MovePlayerPosRot | S::MovePlayerRot | S::MovePlayerStatusOnly => {
                let detail = "a movement packet outside a tick that answers no rotation".into();
                self.flag("INV-T2", name, detail);
            }
            // `handleMoveVehicle` and `handleTeleportEntity` answer with the vehicle's
            // place (`ClientPacketListener.java:687, 2173`).
            S::MoveVehicle => {
                if !vehicle_answer {
                    let detail = "outside a tick, answering no vehicle move or teleport".into();
                    self.flag("INV-T4", name, detail);
                }
                self.vehicle_move(name, r)?;
            }
            S::PlayerAction if sub == ACTION_ABORT => {
                let _pos = r.i64()?;
                let _face = r.u8()?;
                let seq = r.varint()?;
                if seq != 0 {
                    self.flag("INV-C5", name, format!("an abort with sequence {seq}"));
                }
            }
            S::PlayerCommand if sub == STOP_SLEEPING => {}
            S::SetCarriedItem
            | S::Attack
            | S::Interact
            | S::UseItem
            | S::UseItemOn
            | S::PlayerAction
            | S::Punch
            | S::PickItemFromBlock
            | S::PickItemFromEntity
            | S::PlayerCommand
            | S::PlayerAbilities
            | S::PlayerInput
            | S::SpectatorAction
            | S::PaddleBoat
            | S::PlayerLoaded => self.flag("INV-T4", name, "sent outside a tick".into()),
            S::Pong => self.flag("INV-R1", name, "a pong answering no ping".into()),
            S::AcceptTeleportation => {
                self.flag("INV-C9", name, "an accept answering no teleport".into());
            }
            S::ContainerClick => self.container_click(r)?,
            S::ContainerClose => self.container_close(r)?,
            _ => {}
        }
        Ok(())
    }

    /// INV-T1/O1/O2/O4/O5: the packet's place in the tick.
    fn order(&mut self, id: S, sub: i32) {
        use rank::*;
        let name = id.name();
        let previous = self.previous;
        // INV-O3: an attack is followed by its punch; a jab has none.
        if previous.is_some_and(|(p, _)| p == S::Attack) && id != S::Punch {
            self.flag(
                "INV-O3",
                S::Attack.name(),
                format!("followed by {name}, not a punch"),
            );
        }
        if id == S::Punch && previous == Some((S::PlayerAction, ACTION_STAB)) {
            self.flag("INV-O3", name, "a punch after a spear's jab".into());
        }
        let ranks: &[u8] = match id {
            S::SetCarriedItem => {
                // `ensureHasSentCarriedItem`: as the tick begins and before the actions
                // that need it, never once the entities have ticked.
                if self.rank > CONTINUE {
                    let detail = "the held slot after the entities ticked".into();
                    self.flag("INV-O1", name, detail);
                }
                return;
            }
            S::PlayerLoaded | S::ClientCommand | S::ChatSessionUpdate => &[START],
            S::TeleportToEntity => &[HOTBAR],
            S::PlayerCommand => match sub {
                OPEN_INVENTORY => &[HOTBAR],
                START_SPRINTING | STOP_SPRINTING => &[SPRINT, VEHICLE_SPRINT],
                START_FALL_FLYING | START_RIDING_JUMP | STOP_RIDING_JUMP => &[ENTITY],
                _ => &[],
            },
            S::PlayerAction => match sub {
                ACTION_SWAP => &[SWAP],
                ACTION_DROP_ALL | ACTION_DROP => &[DROP],
                ACTION_RELEASE | ACTION_STAB => &[ATTACK],
                ACTION_START | ACTION_ABORT => &[ATTACK, CONTINUE],
                ACTION_STOP | ACTION_CHANGE => &[CONTINUE],
                _ => &[],
            },
            S::Attack | S::SpectatorAction => &[ATTACK],
            S::Punch => &[ATTACK, CONTINUE],
            S::Interact | S::UseItemOn | S::UseItem => &[USE, HELD_USE],
            S::PickItemFromBlock | S::PickItemFromEntity => &[PICK],
            S::PlayerAbilities | S::PaddleBoat => &[ENTITY],
            S::PlayerInput => &[INPUT],
            S::MovePlayerPos | S::MovePlayerPosRot | S::MovePlayerRot | S::MovePlayerStatusOnly => {
                &[MOVE]
            }
            S::MoveVehicle => &[VEHICLE],
            _ => &[],
        };
        if ranks.is_empty() {
            // Not placed in the tick, but nothing comes between `sendChanges`'s packets
            // and the tick's end (`Minecraft.java:1813-1852`).
            if self.rank >= INPUT {
                let detail = "sent between the player's changes and client_tick_end".into();
                self.flag("INV-T1", name, detail);
            }
            return;
        }
        let next = match ranks.iter().copied().find(|&r| r >= self.rank) {
            // A rider's sprint comes right after its vehicle's move.
            Some(VEHICLE_SPRINT) if previous.is_none_or(|(p, _)| p != S::MoveVehicle) => None,
            n => n,
        };
        match next {
            Some(r) => self.rank = r,
            None => {
                let invariant = match (id, sub) {
                    (S::PlayerInput, _)
                    | (S::MovePlayerPos, _)
                    | (S::MovePlayerPosRot, _)
                    | (S::MovePlayerRot, _)
                    | (S::MovePlayerStatusOnly, _)
                    | (S::MoveVehicle, _)
                    | (S::PlayerCommand, START_SPRINTING | STOP_SPRINTING) => "INV-O5",
                    (S::PlayerCommand, _) | (S::PlayerAbilities, _) | (S::PaddleBoat, _) => {
                        "INV-O4"
                    }
                    _ => "INV-O2",
                };
                let detail = format!(
                    "out of vanilla's order in the tick (after step {})",
                    self.rank
                );
                self.flag(invariant, name, detail);
            }
        }
    }

    /// INV-O1: an action `ensureHasSentCarriedItem` precedes goes out with the server
    /// holding the slot the player has selected.
    fn held_item(&mut self, name: &'static str) {
        if let (Some(selected), Some(server)) = (self.context.selected, self.server_slot)
            && selected != server
        {
            let detail = format!("slot {selected} selected, the server holds {server}");
            self.flag("INV-O1", name, detail);
        }
    }

    /// INV-C5: a prediction's sequence is one more than the last.
    fn predicted(&mut self, name: &'static str, seq: i32) {
        if let Some(last) = self.sequence
            && seq != last.wrapping_add(1)
        {
            self.flag(
                "INV-C5",
                name,
                format!("sequence {seq}, the last was {last}"),
            );
        }
        self.sequence = Some(seq);
    }

    fn player_action(&mut self, action: i32, pos: i64, face: u8, seq: i32) {
        let name = S::PlayerAction.name();
        match action {
            ACTION_DROP_ALL | ACTION_DROP | ACTION_RELEASE | ACTION_SWAP | ACTION_STAB => {
                if matches!(
                    action,
                    ACTION_DROP_ALL | ACTION_DROP | ACTION_RELEASE | ACTION_STAB
                ) {
                    self.held_item(name);
                }
                if pos != 0 || face != 0 || seq != 0 {
                    let detail = format!(
                        "action {action} with block {:?}, face {face}, sequence {seq} (vanilla: zero, down, 0)",
                        play::unpack_block_pos(pos)
                    );
                    self.flag("INV-C6", name, detail);
                }
            }
            ACTION_START | ACTION_STOP => self.predicted(name, seq),
            ACTION_ABORT | ACTION_CHANGE => {
                if seq != 0 {
                    self.flag(
                        "INV-C5",
                        name,
                        format!("action {action} with sequence {seq}"),
                    );
                }
            }
            _ => self.flag("INV-C6", name, format!("no such action {action}")),
        }
    }

    fn player_command(&mut self, command: i32) {
        let name = S::PlayerCommand.name();
        match command {
            START_SPRINTING | STOP_SPRINTING => {
                let start = command == START_SPRINTING;
                self.sprints += 1;
                if self.sprints > 1 {
                    self.flag("INV-C2", name, "a second sprint command in one tick".into());
                }
                if self.sprinting == Some(start) {
                    let what = if start {
                        "START_SPRINTING"
                    } else {
                        "STOP_SPRINTING"
                    };
                    self.flag("INV-C2", name, format!("{what} twice in a row"));
                }
                self.sprinting = Some(start);
                self.before_loaded(name);
            }
            START_FALL_FLYING => {
                self.elytra += 1;
                if self.elytra > 1 {
                    let detail = "a second START_FALL_FLYING in one tick".into();
                    self.flag("INV-C12", name, detail);
                }
            }
            _ => {}
        }
    }

    /// INV-R4: no player tick's packets before `player_loaded`.
    fn before_loaded(&mut self, name: &'static str) {
        if self.loaded == Some(false) {
            self.flag(
                "INV-R4",
                name,
                "the player's changes before player_loaded".into(),
            );
        }
    }

    /// INV-T2, INV-C3: a movement packet in a tick (`LocalPlayer.sendPosition`, or a
    /// rider's rotation in `sendChanges`).
    fn movement(&mut self, id: S, r: &mut Reader<'_>) -> Result<(), WireError> {
        let name = id.name();
        let has_pos = matches!(id, S::MovePlayerPos | S::MovePlayerPosRot);
        let has_rot = matches!(id, S::MovePlayerPosRot | S::MovePlayerRot);
        let pos = if has_pos {
            Some([r.f64()?, r.f64()?, r.f64()?])
        } else {
            None
        };
        let rot = if has_rot {
            Some((r.f32()?, r.f32()?))
        } else {
            None
        };
        let flags = r.u8()?;
        self.moves += 1;
        if self.moves > 1 {
            self.flag(
                "INV-T2",
                name,
                "a second movement packet in one tick".into(),
            );
        }
        if let Some(p) = pos {
            self.positions += 1;
            if self.positions > 1 {
                let detail = "a second position before client_tick_end (the server kicks)".into();
                self.flag("INV-T2", name, detail);
            }
            if p.iter().any(|v| v.is_nan()) {
                self.flag("INV-C3", name, format!("position {p:?}"));
            }
        }
        if let Some((y, x)) = rot {
            self.rotation_values(name, y, x);
        }
        if flags & !0b11 != 0 {
            self.flag("INV-C3", name, format!("flags {flags:#04x}"));
        }
        self.before_loaded(name);
        let Some(n) = self.tick else { return Ok(()) };
        match self.context.riding {
            // A rider sends its rotation, which `sendPosition`'s last values don't take.
            Some(true) => {
                if id != S::MovePlayerRot {
                    self.flag("INV-O5", name, "a rider sends only its rotation".into());
                }
            }
            Some(false) => {
                if let Some(p) = pos {
                    // Moved more than 2e-4, or the 20-tick reminder (at most as many
                    // `sendPosition`s as ticks have passed).
                    if let Some((last, at)) = self.last_position {
                        let d = [p[0] - last[0], p[1] - last[1], p[2] - last[2]];
                        let length = d[0] * d[0] + d[1] * d[1] + d[2] * d[2];
                        if length <= 2.0E-4 * 2.0E-4 && n.saturating_sub(at) < 20 {
                            let detail = format!(
                                "moved {:e} after {} ticks: neither a move nor the reminder",
                                length.sqrt(),
                                n - at
                            );
                            self.flag("INV-C3", name, detail);
                        }
                    }
                    self.last_position = Some((p, n));
                }
                if let Some(rot) = rot {
                    if let Some(last) = self.last_rot
                        && same_rot(rot, last)
                    {
                        self.flag("INV-C3", name, format!("rotation {rot:?} unchanged"));
                    }
                    self.last_rot = Some(rot);
                    self.tick_rot = Some(rot);
                }
                if id == S::MovePlayerStatusOnly && self.last_flags == Some(flags) {
                    self.flag("INV-C3", name, "the flags unchanged".into());
                }
                self.last_flags = Some(flags);
            }
            None => {
                self.last_position = None;
                self.last_rot = None;
                self.last_flags = None;
            }
        }
        Ok(())
    }

    /// INV-C3: a rotation the server takes (finite; a pitch within 90 degrees).
    fn rotation_values(&mut self, name: &'static str, y: f32, x: f32) {
        if !y.is_finite() || !x.is_finite() || !(-90.0..=90.0).contains(&x) {
            self.flag("INV-C3", name, format!("rotation ({y}, {x})"));
        }
    }

    fn vehicle_move(&mut self, name: &'static str, r: &mut Reader<'_>) -> Result<(), WireError> {
        let pos = [r.f64()?, r.f64()?, r.f64()?];
        let (y, x) = (r.f32()?, r.f32()?);
        let _on_ground = r.bool()?;
        if pos.iter().any(|v| v.is_nan()) || !y.is_finite() || !x.is_finite() {
            self.flag("INV-C3", name, format!("vehicle at {pos:?} ({y}, {x})"));
        }
        Ok(())
    }

    /// INV-C10: a click in the open menu, with its state id, and a button its kind of
    /// click takes (`AbstractContainerScreen`'s `slotClicked` calls).
    fn container_click(&mut self, r: &mut Reader<'_>) -> Result<(), WireError> {
        let name = S::ContainerClick.name();
        let container = r.varint()?;
        let state = r.varint()?;
        let _slot = r.bytes(2)?;
        let button = r.u8()? as i8;
        let input = r.varint()?;
        if let Some(open) = self.container {
            if container != open {
                let detail = format!("a click in menu {container}, menu {open} is open");
                self.flag("INV-C10", name, detail);
            } else if let Some(want) = if open == 0 {
                self.inventory_state
            } else {
                self.container_state
            } && state != want
            {
                let detail = format!("state id {state}, the server's is {want}");
                self.flag("INV-C10", name, detail);
            }
        }
        let fits = match input {
            // PICKUP, QUICK_MOVE, THROW (ctrl), PICKUP_ALL: the left or right button.
            0 | 1 | 4 | 6 => (0..=1).contains(&button),
            // SWAP: a hotbar key, or 40 for the off hand.
            2 => (0..9).contains(&button) || button == 40,
            // CLONE: the mouse button the pick key is on, or 0 from the keyboard.
            3 => (0..8).contains(&button),
            // QUICK_CRAFT: `getQuickcraftMask(header, type)`, both 0 to 2.
            5 => button & 3 < 3 && button >> 2 < 3 && button >= 0,
            _ => false,
        };
        if !fits {
            self.flag(
                "INV-C10",
                name,
                format!("button {button} with click kind {input}"),
            );
        }
        Ok(())
    }

    /// INV-C10: closing the menu that is open (`LocalPlayer.closeContainer`).
    fn container_close(&mut self, r: &mut Reader<'_>) -> Result<(), WireError> {
        let name = S::ContainerClose.name();
        let container = r.varint()?;
        if let Some(open) = self.container
            && container != open
        {
            let detail = format!("closing menu {container}, menu {open} is open");
            self.flag("INV-C10", name, detail);
        }
        self.container = Some(0);
        self.container_state = None;
        Ok(())
    }

    /// The session ended: a line about what was found.
    pub fn finish(&self) {
        if self.on_violation == OnViolation::Log {
            if self.count == 0 {
                eprintln!("[packet-check] no violations");
            } else {
                eprintln!("[packet-check] !!! {} violations this session", self.count);
            }
        }
    }
}

/// Whether two rotations are the same as `sendPosition` sees them (`delta != 0.0` in
/// doubles).
fn same_rot(a: (f32, f32), b: (f32, f32)) -> bool {
    same_f32(a.0, b.0) && same_f32(a.1, b.1)
}

fn same_f32(a: f32, b: f32) -> bool {
    f64::from(a) - f64::from(b) == 0.0
}

/// The session's connection, with the checker watching what goes out (`None`: off).
pub(crate) struct Outbound {
    connection: Connection,
    checker: Option<RefCell<PacketChecker>>,
}

impl Outbound {
    /// The checker is on as [`enabled`] says; in this crate's tests a violation panics.
    pub(crate) fn new(connection: Connection) -> Outbound {
        let on_violation = if cfg!(test) {
            OnViolation::Panic
        } else {
            OnViolation::Log
        };
        Outbound {
            connection,
            checker: enabled().then(|| RefCell::new(PacketChecker::new(on_violation))),
        }
    }

    /// Sends a packet in the current protocol (`Connection::send`), past the checker.
    pub(crate) fn send(&self, packet: Vec<u8>) {
        if let Some(c) = &self.checker {
            c.borrow_mut().sent(&packet);
        }
        self.connection.send(packet);
    }

    /// `Connection::switch`, with the checker told.
    pub(crate) fn switch(&self, to: Protocol, ack: Vec<u8>) {
        if let Some(c) = &self.checker {
            let mut c = c.borrow_mut();
            c.sent(&ack);
            c.switched(to);
        }
        self.connection.switch(to, ack);
    }

    /// Tells the checker something, if it is on.
    pub(crate) fn check(&self, f: impl FnOnce(&mut PacketChecker)) {
        if let Some(c) = &self.checker {
            f(&mut c.borrow_mut());
        }
    }
}

impl std::ops::Deref for Outbound {
    type Target = Connection;

    fn deref(&self) -> &Connection {
        &self.connection
    }
}

impl fmt::Debug for Outbound {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Outbound")
            .field("checked", &self.checker.is_some())
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ruthenium_protocol::wire;

    fn spawn(dimension: &str) -> play::SpawnInfo {
        play::SpawnInfo {
            dimension_type: 0,
            dimension: dimension.into(),
            seed: 0,
            game_type: play::SURVIVAL,
            previous_game_type: None,
            is_debug: false,
            is_flat: true,
            last_death: None,
            portal_cooldown: 0,
            sea_level: 0,
        }
    }

    fn login() -> play::Clientbound {
        play::Clientbound::Login(Box::new(play::Login {
            player_id: 7,
            hardcore: false,
            levels: vec!["minecraft:overworld".into()],
            max_players: 20,
            chunk_radius: 2,
            simulation_distance: 2,
            reduced_debug_info: false,
            show_death_screen: true,
            limited_crafting: false,
            spawn: spawn("minecraft:overworld"),
            online_mode: false,
            enforces_secure_chat: false,
        }))
    }

    fn teleport(id: i32, position: [f64; 3], relatives: u32) -> play::Clientbound {
        play::Clientbound::PlayerPosition(play::PlayerPosition {
            id,
            position,
            delta: [0.0; 3],
            y_rot: 10.0,
            x_rot: 20.0,
            relatives,
        })
    }

    fn pos(p: [f64; 3]) -> Vec<u8> {
        wire::packet(S::MovePlayerPos.id(), |o| {
            p.iter().for_each(|v| wire::write_f64(o, *v));
            o.push(1);
        })
    }

    fn pos_rot(p: [f64; 3], y: f32, x: f32) -> Vec<u8> {
        wire::packet(S::MovePlayerPosRot.id(), |o| {
            p.iter().for_each(|v| wire::write_f64(o, *v));
            wire::write_f32(o, y);
            wire::write_f32(o, x);
            o.push(1);
        })
    }

    fn rot(y: f32, x: f32, flags: u8) -> Vec<u8> {
        wire::packet(S::MovePlayerRot.id(), |o| {
            wire::write_f32(o, y);
            wire::write_f32(o, x);
            o.push(flags);
        })
    }

    fn status(flags: u8) -> Vec<u8> {
        wire::packet(S::MovePlayerStatusOnly.id(), |o| o.push(flags))
    }

    fn input(bits: u8) -> Vec<u8> {
        wire::packet(S::PlayerInput.id(), |o| o.push(bits))
    }

    fn sprint(start: bool) -> Vec<u8> {
        play::player_command(
            7,
            if start {
                START_SPRINTING
            } else {
                STOP_SPRINTING
            },
            0,
        )
    }

    fn action(a: i32, seq: i32) -> Vec<u8> {
        play::player_action(a, [0; 3], 0, seq)
    }

    fn use_item(seq: i32, y: f32, x: f32) -> Vec<u8> {
        play::use_item(play::MAIN_HAND, seq, y, x)
    }

    /// A checker in play: logged in, and the player loaded in the first tick.
    fn joined() -> PacketChecker {
        let mut c = PacketChecker::new(OnViolation::Collect);
        c.handled(&login());
        tick(&mut c, 0, &[play::player_loaded()]);
        c
    }

    /// Tick `n` sending `packets`, then its end, without a rider or a screen.
    fn tick(c: &mut PacketChecker, n: u64, packets: &[Vec<u8>]) {
        c.begin_tick(n, true);
        c.set_riding(false);
        for p in packets {
            c.sent(p);
        }
        c.sent(&play::client_tick_end());
        c.end_tick();
    }

    fn broken(c: &PacketChecker) -> Vec<&'static str> {
        c.violations().iter().map(|v| v.invariant).collect()
    }

    #[test]
    fn a_tick_in_vanilla_order_passes() {
        let mut c = joined();
        c.handled(&play::Clientbound::SetHeldSlot { slot: 0 });
        c.frame();
        c.begin_tick(1, true);
        c.set_screen(false);
        for p in [
            play::set_carried_item(1),
            action(ACTION_SWAP, 0),
            play::attack(3),
            play::punch(),
            use_item(1, 0.0, 0.0),
            play::player_action(ACTION_START, [1, 2, 3], 1, 2),
            play::punch(),
        ] {
            c.set_selected(1);
            c.sent(&p);
        }
        c.set_riding(false);
        for p in [
            play::player_abilities(false),
            input(1),
            sprint(true),
            pos_rot([0.0, 1.0, 0.0], 0.0, 0.0),
            play::client_tick_end(),
        ] {
            c.sent(&p);
        }
        c.end_tick();
        assert_eq!(broken(&c), Vec::<&str>::new());
        // Riding: the input, the rotation, the vehicle, the sprint.
        c.begin_tick(2, true);
        c.set_riding(true);
        for p in [
            input(0),
            rot(5.0, 5.0, 0),
            play::move_vehicle([0.0; 3], 0.0, 0.0, true),
            sprint(false),
            play::client_tick_end(),
        ] {
            c.sent(&p);
        }
        c.end_tick();
        assert_eq!(broken(&c), Vec::<&str>::new());
    }

    #[test]
    fn a_tick_ends_with_its_tick_end_and_only_then() {
        let mut c = joined();
        c.begin_tick(1, true);
        c.end_tick();
        assert_eq!(broken(&c), ["INV-T1"]);
        let mut c = joined();
        tick(&mut c, 1, &[play::client_tick_end()]);
        assert_eq!(broken(&c), ["INV-T1"]);
        let mut c = joined();
        c.sent(&play::client_tick_end());
        assert_eq!(broken(&c), ["INV-T1"]);
        // Nothing between the player's changes and the end.
        let mut c = joined();
        tick(&mut c, 1, &[input(1), play::chat_ack(1)]);
        assert_eq!(broken(&c), ["INV-T1"]);
        // A tick without a level sends no end.
        let mut c = joined();
        c.begin_tick(1, false);
        c.sent(&play::client_tick_end());
        c.end_tick();
        assert_eq!(broken(&c), ["INV-T1"]);
    }

    #[test]
    fn one_movement_packet_a_tick_and_one_position_before_the_tick_end() {
        let mut c = joined();
        tick(&mut c, 1, &[pos([1.0; 3]), rot(1.0, 1.0, 1)]);
        assert_eq!(broken(&c), ["INV-T2"]);
        let mut c = joined();
        c.begin_tick(1, true);
        c.set_riding(false);
        c.sent(&pos([1.0; 3]));
        c.end_tick();
        tick(&mut c, 2, &[pos([2.0; 3])]);
        assert_eq!(broken(&c), ["INV-T1", "INV-T2"]);
        // Outside a tick, only the answer to the server's rotation.
        let mut c = joined();
        c.sent(&rot(1.0, 2.0, 0));
        assert_eq!(broken(&c), ["INV-T2"]);
        let mut c = joined();
        c.handled(&play::Clientbound::Other(C::PlayerRotation));
        c.sent(&rot(1.0, 2.0, 0));
        assert_eq!(broken(&c), Vec::<&str>::new());
        c.handled(&play::Clientbound::Other(C::PlayerRotation));
        c.sent(&rot(1.0, 2.0, 1));
        assert_eq!(broken(&c), ["INV-T2"]);
        let mut c = joined();
        c.handled(&play::Clientbound::Other(C::PlayerRotation));
        tick(&mut c, 1, &[]);
        assert_eq!(broken(&c), ["INV-T2"]);
    }

    #[test]
    fn ten_ticks_a_frame_and_no_packets_handled_in_one() {
        let mut c = joined();
        c.frame();
        for n in 1..=10 {
            tick(&mut c, n, &[]);
        }
        assert_eq!(broken(&c), Vec::<&str>::new());
        tick(&mut c, 11, &[]);
        assert_eq!(broken(&c), ["INV-T3"]);
        let mut c = joined();
        c.begin_tick(1, true);
        c.handled(&play::Clientbound::SetHeldSlot { slot: 1 });
        assert_eq!(broken(&c), ["INV-T3"]);
    }

    #[test]
    fn tick_packets_only_in_ticks_but_vanillas_exceptions() {
        for p in [
            play::attack(3),
            play::punch(),
            play::set_carried_item(2),
            use_item(1, 0.0, 0.0),
            input(1),
            sprint(true),
            play::player_abilities(true),
            play::player_loaded(),
            action(ACTION_DROP, 0),
        ] {
            let mut c = joined();
            c.sent(&p);
            assert_eq!(broken(&c), ["INV-T4"], "{p:02x?}");
        }
        let mut c = joined();
        c.sent(&action(ACTION_ABORT, 0));
        c.sent(&play::player_command(7, STOP_SLEEPING, 0));
        c.sent(&play::client_command(play::CLIENT_COMMAND_PERFORM_RESPAWN));
        assert_eq!(broken(&c), Vec::<&str>::new());
        // A vehicle's place outside a tick only answers the server's.
        c.sent(&play::move_vehicle([0.0; 3], 0.0, 0.0, false));
        assert_eq!(broken(&c), ["INV-T4"]);
        let mut c = joined();
        c.handled(&play::Clientbound::MoveVehicle {
            position: [0.0; 3],
            y_rot: 0.0,
            x_rot: 0.0,
        });
        c.sent(&play::move_vehicle([0.0; 3], 0.0, 0.0, false));
        assert_eq!(broken(&c), Vec::<&str>::new());
    }

    #[test]
    fn the_held_slot_comes_before_what_needs_it() {
        let mut c = joined();
        tick(&mut c, 1, &[input(1), play::set_carried_item(2)]);
        assert_eq!(broken(&c), ["INV-O1"]);
        // An attack while the server holds another slot.
        let mut c = joined();
        c.handled(&play::Clientbound::SetHeldSlot { slot: 0 });
        c.begin_tick(1, true);
        c.set_selected(3);
        c.sent(&play::attack(3));
        c.sent(&play::punch());
        c.sent(&play::client_tick_end());
        c.end_tick();
        assert_eq!(broken(&c), ["INV-O1"]);
        // A block hit goes out first (`startDestroyBlock` doesn't send it), then
        // `continueDestroyBlock` sends it.
        let mut c = joined();
        c.handled(&play::Clientbound::SetHeldSlot { slot: 0 });
        c.begin_tick(1, true);
        c.set_selected(3);
        for p in [
            play::player_action(ACTION_START, [1, 2, 3], 1, 1),
            play::punch(),
            play::set_carried_item(3),
            play::punch(),
            play::client_tick_end(),
        ] {
            c.sent(&p);
        }
        c.end_tick();
        assert_eq!(broken(&c), Vec::<&str>::new());
    }

    #[test]
    fn key_actions_in_vanillas_order() {
        let mut c = joined();
        tick(
            &mut c,
            1,
            &[use_item(1, 0.0, 0.0), play::attack(3), play::punch()],
        );
        assert_eq!(broken(&c), ["INV-O2"]);
        let mut c = joined();
        tick(
            &mut c,
            1,
            &[play::attack(3), play::punch(), action(ACTION_SWAP, 0)],
        );
        assert_eq!(broken(&c), ["INV-O2"]);
        // `continueAttack` comes last.
        let mut c = joined();
        let finish = play::player_action(ACTION_STOP, [1, 2, 3], 1, 1);
        tick(&mut c, 1, &[finish, use_item(2, 0.0, 0.0)]);
        assert_eq!(broken(&c), ["INV-O2"]);
        // The held use after the picks.
        let mut c = joined();
        let pick = wire::packet(S::PickItemFromBlock.id(), |o| {
            wire::write_i64(o, 0);
            wire::write_bool(o, false);
        });
        tick(
            &mut c,
            1,
            &[use_item(1, 0.0, 0.0), pick, use_item(2, 0.0, 0.0)],
        );
        assert_eq!(broken(&c), Vec::<&str>::new());
    }

    #[test]
    fn an_attack_is_followed_by_its_punch_and_a_jab_by_none() {
        let mut c = joined();
        tick(&mut c, 1, &[play::attack(3), use_item(1, 0.0, 0.0)]);
        assert_eq!(broken(&c), ["INV-O3"]);
        let mut c = joined();
        tick(&mut c, 1, &[action(ACTION_STAB, 0), play::punch()]);
        assert_eq!(broken(&c), ["INV-O3"]);
    }

    #[test]
    fn entity_commands_between_the_keys_and_the_input() {
        let mut c = joined();
        let glide = play::player_command(7, START_FALL_FLYING, 0);
        tick(&mut c, 1, &[input(16), glide]);
        assert_eq!(broken(&c), ["INV-O4"]);
        let mut c = joined();
        let glide = play::player_command(7, START_FALL_FLYING, 0);
        tick(&mut c, 1, &[glide.clone(), play::attack(1), play::punch()]);
        assert_eq!(broken(&c), ["INV-O2", "INV-O2"]);
        let mut c = joined();
        tick(&mut c, 1, &[glide.clone(), glide]);
        assert_eq!(broken(&c), ["INV-C12"]);
    }

    #[test]
    fn the_changes_go_input_sprint_movement() {
        let mut c = joined();
        tick(&mut c, 1, &[pos([1.0; 3]), input(1)]);
        assert_eq!(broken(&c), ["INV-O5"]);
        let mut c = joined();
        tick(&mut c, 1, &[pos([1.0; 3]), sprint(true)]);
        assert_eq!(broken(&c), ["INV-O5"]);
        let mut c = joined();
        tick(&mut c, 1, &[input(1), input(2)]);
        assert_eq!(broken(&c), ["INV-O5"]);
        let mut c = joined();
        tick(&mut c, 1, &[input(1)]);
        tick(&mut c, 2, &[input(1)]);
        assert_eq!(broken(&c), ["INV-O5"]);
        // A rider sends its rotation only.
        let mut c = joined();
        c.begin_tick(1, true);
        c.set_riding(true);
        c.sent(&pos([1.0; 3]));
        c.sent(&play::client_tick_end());
        c.end_tick();
        assert_eq!(broken(&c), ["INV-O5"]);
    }

    #[test]
    fn a_held_slot_is_a_hotbar_slot_not_sent_twice() {
        let mut c = joined();
        tick(&mut c, 1, &[play::set_carried_item(9)]);
        assert_eq!(broken(&c), ["INV-C1"]);
        // The login's game mode starts at 0.
        let mut c = joined();
        tick(&mut c, 1, &[play::set_carried_item(0)]);
        assert_eq!(broken(&c), ["INV-C1"]);
        // The server's slot doesn't change what was sent: the client sends it back.
        let mut c = joined();
        tick(&mut c, 1, &[play::set_carried_item(4)]);
        c.handled(&play::Clientbound::SetHeldSlot { slot: 2 });
        tick(&mut c, 2, &[play::set_carried_item(2)]);
        assert_eq!(broken(&c), Vec::<&str>::new());
    }

    #[test]
    fn sprint_commands_alternate_once_a_tick() {
        let mut c = joined();
        tick(&mut c, 1, &[sprint(false)]);
        assert_eq!(broken(&c), ["INV-C2"]);
        let mut c = joined();
        tick(&mut c, 1, &[sprint(true)]);
        tick(&mut c, 2, &[sprint(true)]);
        assert_eq!(broken(&c), ["INV-C2"]);
        // A respawn keeping the entity data keeps what was sent unknown; without, the
        // new player starts not sprinting.
        let mut c = joined();
        tick(&mut c, 1, &[sprint(true)]);
        c.handled(&play::Clientbound::Respawn {
            spawn: Box::new(spawn("minecraft:overworld")),
            data_to_keep: 0,
        });
        tick(&mut c, 2, &[play::player_loaded(), sprint(true)]);
        assert_eq!(broken(&c), Vec::<&str>::new());
    }

    #[test]
    fn movement_packets_as_send_position_chooses_them() {
        let mut c = joined();
        tick(&mut c, 1, &[rot(0.0, 91.0, 1)]);
        assert_eq!(broken(&c), ["INV-C3"]);
        let mut c = joined();
        tick(&mut c, 1, &[pos([f64::NAN, 0.0, 0.0])]);
        assert_eq!(broken(&c), ["INV-C3"]);
        // Not moved: only every 20 ticks.
        let mut c = joined();
        tick(&mut c, 1, &[pos([1.0; 3])]);
        tick(&mut c, 2, &[pos([1.0, 1.0001, 1.0])]);
        assert_eq!(broken(&c), ["INV-C3"]);
        let mut c = joined();
        tick(&mut c, 1, &[pos([1.0; 3])]);
        tick(&mut c, 21, &[pos([1.0; 3])]);
        tick(&mut c, 22, &[pos([1.0, 1.0003, 1.0])]);
        assert_eq!(broken(&c), Vec::<&str>::new());
        // Not turned, the same flags.
        let mut c = joined();
        tick(&mut c, 1, &[rot(1.0, 2.0, 1)]);
        tick(&mut c, 2, &[rot(1.0, 2.0, 1)]);
        assert_eq!(broken(&c), ["INV-C3"]);
        let mut c = joined();
        tick(&mut c, 1, &[status(1)]);
        tick(&mut c, 2, &[status(1)]);
        assert_eq!(broken(&c), ["INV-C3"]);
        let mut c = joined();
        tick(&mut c, 1, &[status(1)]);
        tick(&mut c, 2, &[status(0)]);
        assert_eq!(broken(&c), Vec::<&str>::new());
    }

    #[test]
    fn predictions_count_up_and_the_rest_carry_zero() {
        let mut c = joined();
        tick(&mut c, 1, &[use_item(1, 0.0, 0.0)]);
        tick(&mut c, 2, &[use_item(3, 0.0, 0.0)]);
        assert_eq!(broken(&c), ["INV-C5"]);
        let mut c = joined();
        tick(&mut c, 1, &[action(ACTION_ABORT, 4)]);
        assert_eq!(broken(&c), ["INV-C5"]);
        let mut c = joined();
        tick(&mut c, 1, &[action(ACTION_DROP, 1)]);
        assert_eq!(broken(&c), ["INV-C6"]);
        let mut c = joined();
        tick(
            &mut c,
            1,
            &[play::player_action(ACTION_RELEASE, [0, 1, 0], 0, 0)],
        );
        assert_eq!(broken(&c), ["INV-C6"]);
        // Another dimension's level starts again.
        let mut c = joined();
        tick(&mut c, 1, &[use_item(1, 0.0, 0.0)]);
        c.handled(&play::Clientbound::Respawn {
            spawn: Box::new(spawn("minecraft:the_nether")),
            data_to_keep: 3,
        });
        tick(&mut c, 2, &[play::player_loaded(), use_item(1, 0.0, 0.0)]);
        assert_eq!(broken(&c), Vec::<&str>::new());
    }

    #[test]
    fn a_use_carries_the_ticks_rotation() {
        let mut c = joined();
        tick(&mut c, 1, &[use_item(1, 5.0, 6.0), rot(5.0, 7.0, 1)]);
        assert_eq!(broken(&c), ["INV-C7"]);
        // No turn this tick: the one last sent.
        tick(&mut c, 2, &[use_item(2, 5.0, 7.0)]);
        assert_eq!(broken(&c), ["INV-C7"]);
        tick(&mut c, 3, &[use_item(3, 5.0, 6.0)]);
        assert_eq!(broken(&c), ["INV-C7", "INV-C7"]);
    }

    #[test]
    fn an_interaction_sneaks_as_the_last_input_sent() {
        let mut c = joined();
        tick(&mut c, 1, &[play::interact(3, 0, [0.0; 3], true)]);
        assert_eq!(broken(&c), ["INV-C8"]);
        let mut c = joined();
        tick(&mut c, 1, &[input(1 << 5)]);
        tick(&mut c, 2, &[play::interact(3, 0, [0.0; 3], true)]);
        assert_eq!(broken(&c), Vec::<&str>::new());
    }

    #[test]
    fn a_teleport_is_accepted_at_once_where_it_put_the_player() {
        let accept = |id, p, y, x| play::accept_teleportation(id, p, y, x);
        let mut c = joined();
        c.handled(&teleport(5, [1.0, 2.0, 3.0], 0));
        c.sent(&accept(5, [1.0, 2.0, 3.0], 10.0, 20.0));
        assert_eq!(broken(&c), Vec::<&str>::new());
        c.handled(&teleport(6, [1.0, 2.0, 3.0], 0));
        c.sent(&accept(7, [1.0, 2.0, 3.0], 10.0, 20.0));
        assert_eq!(broken(&c), ["INV-C9"]);
        let mut c = joined();
        c.handled(&teleport(6, [1.0, 2.0, 3.0], 0));
        c.sent(&accept(6, [1.0, 2.5, 3.0], 10.0, 21.0));
        assert_eq!(broken(&c), ["INV-C9", "INV-C9"]);
        // Relative: wherever the player was.
        let mut c = joined();
        c.handled(&teleport(6, [1.0, 2.0, 3.0], 0b11111));
        c.sent(&accept(6, [9.0, 9.0, 9.0], 1.0, 2.0));
        assert_eq!(broken(&c), Vec::<&str>::new());
        // Not answered first.
        let mut c = joined();
        c.handled(&teleport(6, [1.0, 2.0, 3.0], 0));
        c.sent(&play::pong(1));
        assert!(broken(&c).contains(&"INV-C9"));
        let mut c = joined();
        c.handled(&teleport(6, [1.0, 2.0, 3.0], 0));
        c.handled(&play::Clientbound::SetHeldSlot { slot: 1 });
        assert_eq!(broken(&c), ["INV-C9"]);
        let mut c = joined();
        c.sent(&accept(6, [1.0, 2.0, 3.0], 10.0, 20.0));
        assert_eq!(broken(&c), ["INV-C9"]);
    }

    #[test]
    fn clicks_in_the_open_menu_with_its_state() {
        let click = |id, state, button, input| {
            play::container_click(id, state, 0, button, input, &[], None)
        };
        let mut c = joined();
        c.handled(&play::Clientbound::OpenScreen {
            container_id: 3,
            menu_type: 2,
            title: ruthenium_protocol::nbt::Tag::String(String::new()),
        });
        c.handled(&play::Clientbound::ContainerSetContent {
            container_id: 3,
            state_id: 8,
            items: Vec::new(),
            carried: Default::default(),
        });
        c.sent(&click(3, 8, 0, 0));
        c.sent(&click(3, 8, 40, 2));
        c.sent(&click(3, 8, 9, 5));
        assert_eq!(broken(&c), Vec::<&str>::new());
        c.sent(&click(0, 8, 0, 0));
        c.sent(&click(3, 7, 0, 0));
        c.sent(&click(3, 8, 2, 0));
        c.sent(&click(3, 8, 3, 5));
        c.sent(&play::container_close(4));
        assert_eq!(broken(&c), ["INV-C10"; 5]);
        // Closed: the inventory's.
        c.sent(&click(0, 0, 1, 0));
        assert_eq!(broken(&c).len(), 5);
    }

    #[test]
    fn no_keys_held_under_a_screen() {
        let mut c = joined();
        c.begin_tick(1, true);
        c.set_screen(true);
        c.set_riding(false);
        c.sent(&input(1));
        c.sent(&play::client_tick_end());
        c.end_tick();
        assert_eq!(broken(&c), ["INV-C11"]);
    }

    #[test]
    fn pings_are_answered_at_once_and_in_order() {
        let ping = |id| play::Clientbound::Common(Common::Ping { id });
        let mut c = joined();
        c.handled(&ping(4));
        c.sent(&play::pong(4));
        assert_eq!(broken(&c), Vec::<&str>::new());
        c.handled(&ping(5));
        c.sent(&play::pong(4));
        assert_eq!(broken(&c), ["INV-R1"]);
        let mut c = joined();
        c.handled(&ping(5));
        c.handled(&ping(6));
        assert_eq!(broken(&c), ["INV-R1"]);
        let mut c = joined();
        c.handled(&ping(5));
        tick(&mut c, 1, &[]);
        assert_eq!(broken(&c), ["INV-R1"]);
        let mut c = joined();
        c.sent(&play::pong(1));
        assert_eq!(broken(&c), ["INV-R1"]);
    }

    #[test]
    fn the_player_is_loaded_once_before_it_moves() {
        let mut c = PacketChecker::new(OnViolation::Collect);
        c.handled(&login());
        tick(&mut c, 0, &[input(1), pos([0.0; 3])]);
        assert_eq!(broken(&c), ["INV-R4", "INV-R4"]);
        let mut c = joined();
        tick(&mut c, 1, &[play::player_loaded()]);
        assert_eq!(broken(&c), ["INV-R4"]);
    }

    #[test]
    fn packets_that_dont_read_are_malformed() {
        let mut c = joined();
        tick(&mut c, 1, &[vec![S::MovePlayerPos.id() as u8, 1, 2]]);
        assert_eq!(broken(&c), ["MALFORMED"]);
    }

    #[test]
    fn nothing_is_checked_outside_play() {
        let mut c = PacketChecker::new(OnViolation::Collect);
        c.sent(&play::client_tick_end());
        c.switched(Protocol::Play);
        c.sent(&play::client_tick_end());
        assert_eq!(broken(&c), ["INV-T1"]);
        c.switched(Protocol::Configuration);
        c.sent(&play::attack(1));
        assert_eq!(broken(&c), ["INV-T1"]);
    }

    #[test]
    #[should_panic(expected = "INV-T4")]
    fn a_violation_panics_in_tests() {
        let mut c = PacketChecker::new(OnViolation::Panic);
        c.handled(&login());
        c.sent(&play::attack(1));
    }
}

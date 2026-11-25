//! One session with a server, on the game thread (`ClientConfigurationPacketListenerImpl`,
//! `ClientCommonPacketListenerImpl` and, for now, the switch parts of
//! `ClientPacketListener`).
//!
//! The connection's thread does the login and answers what vanilla answers on its network
//! thread (keep-alives, chunk batches); this thread takes every other packet in order and
//! answers the ones vanilla answers on its main thread: pings (the "transactions"
//! anticheats time), known packs, cookies, resource packs. The menus hear about it through
//! [`SessionEvent`]s and answer the code of conduct.
//!
//! Frames: this thread runs in step with the main thread's frames, as vanilla's single
//! main thread does. The main thread sends each frame with its input
//! ([`Session::frame`]), and the frame runs in `Minecraft.runTick`'s order: the ticks
//! due are counted, every packet that has arrived is handled
//! (`PacketProcessor.processQueuedPackets`), the frame's input is taken in, the ticks
//! run, and the mouse turns the camera (`handleAccumulatedMovement`). So packets are
//! answered when vanilla would answer them, once per frame, never the moment they arrive.
//!
//! Known packs: the client answers with the packs it has of those the server lists, in
//! the server's order (`KnownPacksManager.trySelectingPacks`); the caller passes the
//! packs of the imported game ([`SessionOptions::known_packs`]), or none without it, in
//! which case the server sends every registry entry in full.
//!
//! Not yet: server resource packs (every offer is declined; a server that requires its
//! pack will refuse the player), transfers (the session ends with a message), dialogs
//! and server links (ignored), and anything in play beyond pings, cookies and the switch
//! back to configuration.

use std::collections::{HashMap, VecDeque};
use std::sync::mpsc::{Receiver, Sender, TryRecvError, channel};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use ruthenium_net::connection::{
    Closed, ConnectOptions, Connection, Event, Local, Protocol, Reason, Stage,
};
use ruthenium_protocol::packets::configuration::{self as cfg, RegistryEntry, RegistryTags};
use ruthenium_protocol::packets::{
    ClientInformation, Common, KnownPack, MainHand, PackAction, play, read_brand,
};

use crate::avatar::{SkinSource, Swing};
use crate::biome::Climate;
use crate::blocks::{BlockTags, Blocks};
use crate::clock::{self, TickClock};
use crate::effect::Effect;
use crate::entity::{Entities, EntityTypes, Local as LocalInfo, PlayerInfos, Teams, TickContext};
use crate::environment::{Clocks, Environment, LocalRegistries};
use crate::first_person::{self, Destruction};
use crate::handoff::{Handoff, HandoffStats, Item, Limits, Take, Taken};
use crate::input::next_scroll_selection;
use crate::interact::{self, GameMode, UseKeys};
use crate::inventory::Hand;
use crate::item::Items;
use crate::level_load::LevelLoad;
use crate::light::LightTable;
use crate::pick;
use crate::player::{Env, Keys, LocalPlayer, Others, Pose};
use crate::predict::Predictions;
use crate::scene::Target;
use crate::scene::{
    DEFAULT_DIRTY_CAPACITY, Dims, EntityRecord, Frame, MAX_ENTITIES, Scene, ScenePublisher,
};
use crate::scoreboard::Scoreboard;
use crate::weather::Weather;
use crate::world::{BiomeInfo, ChunkPacket, Level, global_bits};
use ruthenium_protocol::profile::ResolvableProfile;

#[derive(Debug)]
pub struct SessionOptions {
    pub connect: ConnectOptions,
    /// The data packs the client has (the imported game's), offered back when the server
    /// lists them.
    pub known_packs: Vec<KnownPack>,
    /// What the local player's movement needs from the prepared game; without it the
    /// player doesn't move or send movement.
    pub physics: Option<Physics>,
}

/// The prepared game's data movement uses.
#[derive(Debug, Clone)]
pub struct Physics {
    pub blocks: std::sync::Arc<Blocks>,
    /// Entity types: sizes, update intervals, synched-data fields.
    pub entity_types: std::sync::Arc<EntityTypes>,
    /// Vanilla's dimension types by name, for servers that send them as known-pack
    /// entries without data.
    pub dimension_types: HashMap<String, DimensionType>,
    /// Items: their classes and default components.
    pub items: std::sync::Arc<Items>,
    /// Vanilla's block transformers by name, for servers that send them as known-pack
    /// entries without data.
    pub block_transformers: std::sync::Arc<crate::use_on::Transformers>,
    /// Vanilla's enchantments by name, for servers that send them as known-pack
    /// entries without data.
    pub enchantments: std::sync::Arc<crate::enchantment::Enchantments>,
    /// Vanilla's biomes' climates by name, for servers that send them as known-pack
    /// entries without data.
    pub biomes: HashMap<String, crate::biome::Climate>,
    /// Vanilla's dimension types, timelines, world clocks and biomes as their files
    /// define them, for the environment of a server that names them without data.
    pub registries: Arc<LocalRegistries>,
}

/// What movement needs from a dimension type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DimensionType {
    pub min_y: i32,
    pub height: i32,
    /// The `minecraft:gameplay/fast_lava` environment attribute.
    pub fast_lava: bool,
    /// `has_skylight` and `has_ceiling`: whether it can rain there.
    pub has_skylight: bool,
    pub has_ceiling: bool,
}

impl DimensionType {
    /// The environment attribute that speeds lava up.
    pub const FAST_LAVA: &str = "minecraft:gameplay/fast_lava";
}

/// What the menus hear.
#[derive(Debug, Clone, PartialEq)]
pub enum SessionEvent {
    Stage(Stage),
    LoggedIn {
        name: String,
    },
    /// The server's code of conduct: answer with [`Session::answer_code_of_conduct`].
    CodeOfConduct(String),
    /// The configuration is done and play has begun (again, after a reconfiguration).
    Playing,
    /// The player is in the world: the client told the server it has loaded.
    Loaded,
    /// The server went back to configuration (a server switch behind a proxy, a data pack
    /// reload).
    Reconfiguring,
    /// The local player died and the death screen opened (`DeathScreen`): why (`None`
    /// when the client found the player dead with no screen open), whether the world is
    /// hardcore (Spectate World instead of Respawn) and the score. Its buttons wait for
    /// [`SessionEvent::RespawnReady`]; Respawn is [`Session::respawn`].
    Died {
        cause: Option<ruthenium_protocol::nbt::Tag>,
        hardcore: bool,
        score: i32,
    },
    /// The death screen's buttons work now, 20 ticks after it opened.
    RespawnReady,
    /// The player lies in a bed and the bed screen opened (`InBedChatScreen`): Leave
    /// Bed is [`Session::leave_bed`].
    InBed,
    /// The player is out of bed and the bed screen closed.
    WokeUp,
    /// The server brought the player back (`handleRespawn`): the death screen closes and
    /// the level loads; [`SessionEvent::Loaded`] follows.
    Respawned,
    /// The last event.
    Closed(Closed),
}

/// What the menus ask of the session, taken in order with the frames.
#[derive(Debug)]
enum Command {
    CodeOfConduct(bool),
    /// The death screen's Respawn (or Spectate World) button.
    Respawn,
    /// The bed screen's Leave Bed button.
    LeaveBed,
}

/// The local camera for drawing: the latest frame, and where the player looks once
/// the turns the game thread hasn't taken in yet are applied to it.
#[derive(Debug, Clone)]
pub struct Camera {
    pub frame: Arc<Frame>,
    pub y_rot: f32,
    pub x_rot: f32,
    /// Turns applied on top of the frame's rotation.
    pub turns_ahead: usize,
}

/// What the player did during one frame on the main thread.
#[derive(Debug, Default)]
struct FrameInput {
    /// The movement keys held as the frame's events left them (none while a screen is
    /// open).
    keys: Keys,
    /// Hotbar key presses, by slot (`KeyMapping.click`).
    hotbar_clicks: [u32; 9],
    /// The use and attack keys held, and presses of the use, swap-hands and attack keys.
    use_down: bool,
    use_clicks: u32,
    swap_clicks: u32,
    attack_down: bool,
    attack_clicks: u32,
    /// A screen is open over the world (keys and clicks aren't read).
    screen_open: bool,
    /// The game was paused this frame (`pauseGame`): breaking stops.
    paused: bool,
    /// Scroll-wheel notches, in order.
    scrolls: Vec<i32>,
    /// The frame's turn of the camera (`Entity.turn`'s amounts), after its ticks.
    turns: Vec<(f64, f64)>,
}

/// The local player as of the last tick, for the screen.
#[derive(Debug, Clone, PartialEq)]
pub struct PlayerView {
    pub dimension: String,
    pub position: [f64; 3],
    pub y_rot: f32,
    pub x_rot: f32,
    pub on_ground: bool,
    pub sprinting: bool,
    pub pose: Pose,
    pub flying: bool,
    pub game_type: i32,
    /// The effects on the player: (registry id, amplifier, ticks left or -1).
    pub effects: Vec<(i32, i32, i32)>,
    pub health: f32,
    pub food: i32,
    pub selected_slot: i32,
    /// The stacks in the hands (`id*count`, empty as "").
    pub main_hand: String,
    pub off_hand: String,
    /// The item being used: the hand, the item and the ticks left.
    pub using: Option<(&'static str, String, i32)>,
    /// The attack's charge (`getAttackStrengthScale(0)`, what the attack indicator
    /// shows), 0 to 1.
    pub attack_strength: f32,
    /// The block being broken and how far (`destroyProgress`, 0 to 1).
    pub breaking: Option<([i32; 3], f32)>,
    pub chunks: usize,
    /// The frames and the client ticks the game thread has run since the session began,
    /// for tools that key input by tick. The view is set last in a frame: when it shows
    /// a frame count, what that frame published ([`Session::scene`]) is there too.
    pub frames: u64,
    pub ticks: u64,
    /// Other entities the client tracks.
    pub entities: usize,
    /// Whether the player has loaded into the level.
    pub loaded: bool,
}

/// A session on the game thread. Dropping it disconnects.
pub struct Session {
    /// The frames' input and the controls, numbered, on their way to the game thread.
    handoff: Arc<Handoff<FrameInput, Command>>,
    events: Receiver<SessionEvent>,
    /// The input of the frame under way, sent with [`Session::frame`].
    input: Mutex<FrameInput>,
    view: Arc<Mutex<Option<PlayerView>>>,
    /// What the game thread publishes for drawing.
    scene: Scene,
    /// The turns handed over, each with its frame's number, until a published frame
    /// has them in its rotation.
    turns: Mutex<VecDeque<(u64, f64, f64)>>,
    thread: Option<JoinHandle<()>>,
}

/// Turns kept for [`Session::camera`] at most (the oldest go first).
const MAX_PENDING_TURNS: usize = 8192;

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

impl std::fmt::Debug for Session {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Session").finish_non_exhaustive()
    }
}

impl Session {
    pub fn start(options: SessionOptions) -> std::io::Result<Session> {
        let information = options.connect.information.clone();
        let connection = Connection::open(options.connect)?;
        Session::with_connection(
            connection,
            options.known_packs,
            options.physics,
            information,
        )
    }

    /// A session on a connection that is already open.
    fn with_connection(
        connection: Connection,
        known_packs: Vec<KnownPack>,
        physics: Option<Physics>,
        information: ClientInformation,
    ) -> std::io::Result<Session> {
        let handoff = Arc::new(Handoff::new(Limits::default()));
        let (publisher, scene) = Scene::new(DEFAULT_DIRTY_CAPACITY);
        let (tx, events) = channel();
        if physics.is_none() {
            eprintln!("[session] no prepared game: the player won't move or send movement");
        }
        let view = Arc::new(Mutex::new(None));
        let game_view = view.clone();
        let game_handoff = handoff.clone();
        let thread = std::thread::Builder::new()
            .name("game".into())
            .spawn(move || {
                let mut game =
                    Game::new(connection, known_packs, physics, tx, publisher, information);
                game.view = game_view;
                game.run(&game_handoff);
            })?;
        Ok(Session {
            handoff,
            events,
            input: Mutex::default(),
            view,
            scene,
            turns: Mutex::default(),
            thread: Some(thread),
        })
    }

    /// The events since the last call, oldest first.
    pub fn poll(&self) -> impl Iterator<Item = SessionEvent> + '_ {
        self.events.try_iter()
    }

    pub fn answer_code_of_conduct(&self, accept: bool) {
        self.handoff.push_control(Command::CodeOfConduct(accept));
    }

    /// Disconnects; a [`SessionEvent::Closed`] follows. It doesn't wait its turn behind
    /// queued frames.
    pub fn disconnect(&self) {
        self.handoff.close();
    }

    /// Asks the server to bring the dead player back (`LocalPlayer.respawn`: the death
    /// screen's Respawn button); [`SessionEvent::Respawned`] follows.
    pub fn respawn(&self) {
        self.handoff.push_control(Command::Respawn);
    }

    /// Asks the server to wake the player (the bed screen's Leave Bed);
    /// [`SessionEvent::WokeUp`] follows once it does.
    pub fn leave_bed(&self) {
        self.handoff.push_control(Command::LeaveBed);
    }

    /// Ends the main thread's frame: the game thread runs it (`Minecraft.runTick`) with
    /// the input given since the last one. Call it once every frame the client runs,
    /// also while the window is hidden or no screen of the session shows; packets wait
    /// for it. Returns the frame's number in the handoff: a published
    /// [`Frame::input_sequence`] at or past it has this input in it.
    ///
    /// The game thread takes frames in order, each as its own, and at most
    /// [`Limits::frames`] wait: past that this call waits for room
    /// ([`Session::handoff_stats`] counts it).
    pub fn frame(&self) -> u64 {
        let mut input = lock(&self.input);
        let (keys, use_down, attack_down, screen_open) = (
            input.keys,
            input.use_down,
            input.attack_down,
            input.screen_open,
        );
        let frame = std::mem::take(&mut *input);
        input.keys = keys;
        input.use_down = use_down;
        input.attack_down = attack_down;
        input.screen_open = screen_open;
        drop(input);
        let events = frame.scrolls.len() + frame.turns.len();
        let turns = frame.turns.clone();
        let sequence = self.handoff.push_frame(frame, events);
        let mut log = lock(&self.turns);
        // Without a published frame nothing reads the log.
        match self.scene.frame() {
            Some(f) => {
                while log.front().is_some_and(|t| t.0 <= f.input_sequence) {
                    log.pop_front();
                }
            }
            None => log.clear(),
        }
        log.extend(turns.into_iter().map(|(xo, yo)| (sequence, xo, yo)));
        while log.len() > MAX_PENDING_TURNS {
            log.pop_front();
        }
        sequence
    }

    /// What the game thread publishes for drawing: the terrain, the frames and the
    /// dirty queue.
    pub fn scene(&self) -> &Scene {
        &self.scene
    }

    /// The latest frame with the camera turned as far as the mouse has: the frame's
    /// rotation, then every turn the game thread had not taken in when it published the
    /// frame, in the order they were made (those of the frame under way last). Movement
    /// and everything else come from the frame alone.
    pub fn camera(&self) -> Option<Camera> {
        let frame = self.scene.frame()?;
        let (mut y_rot, mut x_rot) = (frame.player.y_rot, frame.player.x_rot);
        let mut turns_ahead = 0;
        {
            let log = lock(&self.turns);
            for &(_, xo, yo) in log.iter().filter(|t| t.0 > frame.input_sequence) {
                (y_rot, x_rot) = crate::player::turned(y_rot, x_rot, xo, yo);
                turns_ahead += 1;
            }
        }
        for &(xo, yo) in &lock(&self.input).turns {
            (y_rot, x_rot) = crate::player::turned(y_rot, x_rot, xo, yo);
            turns_ahead += 1;
        }
        Some(Camera {
            frame,
            y_rot,
            x_rot,
            turns_ahead,
        })
    }

    /// Frames handed over that the game thread has not taken yet.
    pub fn frames_waiting(&self) -> usize {
        self.handoff.waiting()
    }

    /// How the handoff to the game thread has fared: the most frames that waited, the
    /// oldest input it took, and the times this thread had to wait for room.
    pub fn handoff_stats(&self) -> HandoffStats {
        self.handoff.stats()
    }

    /// The movement keys held from now on (`Keys::default()` while a screen is open).
    pub fn set_keys(&self, keys: Keys) {
        lock(&self.input).keys = keys;
    }

    /// The frame's turn of the camera (`Entity.turn`'s amounts, from
    /// [`crate::input::turn_amount`]), applied after the frame's ticks.
    pub fn turn(&self, xo: f64, yo: f64) {
        lock(&self.input).turns.push((xo, yo));
    }

    /// A press of the hotbar key for `slot` (0 to 8), read by the next tick.
    pub fn hotbar_key(&self, slot: u8) {
        if let Some(c) = lock(&self.input).hotbar_clicks.get_mut(usize::from(slot)) {
            *c += 1;
        }
    }

    /// The use key (the right mouse button) pressed or let go: a press counts as a click
    /// (`KeyMapping.click`) the next tick reads.
    pub fn set_use(&self, down: bool) {
        let mut input = lock(&self.input);
        if down && !input.use_down {
            input.use_clicks += 1;
        }
        input.use_down = down;
    }

    /// A press of the swap-hands key (F).
    pub fn swap_hands_key(&self) {
        lock(&self.input).swap_clicks += 1;
    }

    /// The attack key (the left mouse button) pressed or let go: a press counts as a
    /// click the next tick reads.
    pub fn set_attack(&self, down: bool) {
        let mut input = lock(&self.input);
        if down && !input.attack_down {
            input.attack_clicks += 1;
        }
        input.attack_down = down;
    }

    /// Whether a screen is open over the world this frame (`gui.screen() != null`):
    /// the ticks then read no keys, and attacks wait for the attack key to be let go.
    pub fn set_screen_open(&self, open: bool) {
        lock(&self.input).screen_open = open;
    }

    /// The game is paused (`pauseGame`: the Game Menu opens): breaking a block stops.
    pub fn pause(&self) {
        lock(&self.input).paused = true;
    }

    /// Scroll-wheel notches (from [`crate::input::ScrollWheel`]): the selected slot moves
    /// before the frame's ticks.
    pub fn scroll(&self, wheel: i32) {
        lock(&self.input).scrolls.push(wheel);
    }

    /// The local player as of the last tick (`None` outside a level).
    pub fn view(&self) -> Option<PlayerView> {
        lock(&self.view).clone()
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        self.handoff.drop_session();
        drop(self.thread.take());
    }
}

/// What the configuration gathered: the registries' entries in their network order, the
/// tags and the enabled features.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Configured {
    pub registries: Vec<(String, Vec<RegistryEntry>)>,
    pub tags: Vec<RegistryTags>,
    pub features: Vec<String>,
}

impl Configured {
    /// A registry's entries replace any earlier ones of the same registry
    /// (`RegistryDataCollector.appendContents`, on a reconfiguration).
    fn add_registry(&mut self, registry: String, entries: Vec<RegistryEntry>) {
        match self.registries.iter_mut().find(|(r, _)| *r == registry) {
            Some((_, e)) => *e = entries,
            None => self.registries.push((registry, entries)),
        }
    }
}

/// Which phase the server is in, as the game thread sees it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    Login,
    Configuration,
    Play,
}

struct Game {
    information: ClientInformation,
    connection: Connection,
    known_packs: Vec<KnownPack>,
    events: Sender<SessionEvent>,
    phase: Phase,
    configured: Configured,
    /// Collected during a configuration, then frozen into `configured` at its end.
    collecting: Configured,
    cookies: HashMap<String, Vec<u8>>,
    server_brand: Option<String>,
    report_details: Vec<(String, String)>,
    seen_code_of_conduct: bool,
    waiting_for_code_of_conduct: bool,
    /// Play packets since play began, by kind, for the log.
    play_packets: u64,
    /// The level, from the play login packet until the session goes back to configuration.
    world: Option<World>,
    physics: Option<Physics>,
    tags: BlockTags,
    /// The item tags the server sent.
    item_tags: crate::item::ItemTags,
    /// The recipe property sets the server sent.
    recipe_sets: crate::item::RecipeSets,
    /// The block transformers of the configuration's registry.
    transformers: crate::use_on::Transformers,
    clock: TickClock,
    /// The frames and the ticks run so far (see [`PlayerView`]).
    frames: u64,
    ticks: u64,
    started: Instant,
    /// Tests set the millisecond clock by hand.
    manual_ms: Option<i64>,
    view: Arc<Mutex<Option<PlayerView>>>,
    /// Where the level and each frame are published for drawing.
    publisher: ScenePublisher,
    /// The number of the last frame input taken in, and when it was handed over.
    input_sequence: u64,
    input_at: Option<Instant>,
    /// Which block states count in the motion-blocking heightmap, by the tags the
    /// server sent.
    motion_blocking: Option<Arc<Vec<u64>>>,
    /// What each block state does to light, from the prepared game: with it the
    /// level computes light after block changes.
    light_table: Option<Arc<LightTable>>,
    /// The keys the next ticks read.
    keys: Keys,
    /// Hotbar key presses not yet read (`KeyMapping.clickCount`).
    hotbar_clicks: [u32; 9],
    /// The use-related keys not yet read.
    use_keys: UseKeys,
    /// A screen is open over the world.
    screen_open: bool,
    /// The death screen, while it is open: the ticks since it opened (`delayTicker`).
    /// The session opens it itself, as vanilla's packet listener and `Gui.tick` do,
    /// so it counts as a screen from that tick on.
    death_screen: Option<u32>,
    /// The bed screen is open (`InBedChatScreen`).
    bed_screen: bool,
    /// The movement keys the frame's input held, whatever screen took them.
    held_keys: Keys,
    /// The world is hardcore (the login says).
    hardcore: bool,
    /// The HUD's own counters (`Hud.tick`).
    hud: crate::hud::HudState,
    /// The profile name the server knows the player by (its scoreboard name), and its
    /// UUID (its player list entry).
    name: String,
    uuid: u128,
    /// The player list and the scoreboard's teams (the play listener's, gone with it
    /// on a reconfiguration).
    players: PlayerInfos,
    teams: Teams,
    scoreboard: Scoreboard,
    /// How many chunks from the player the server ticks.
    simulation_distance: i32,
    /// Whose skin the player's own arm wears: its entry of the player list, signed or
    /// not.
    own_skin: Arc<SkinSource>,
    /// The outline last published, with the block state, the place and whether the
    /// held item filled it that it was made for.
    outline: Option<(Target, bool)>,
}

/// The feature flag that switches minecarts to their new movement, which doesn't
/// interpolate.
const MINECART_IMPROVEMENTS: &str = "minecraft:minecart_improvements";

/// `ClientboundGameEventPacket` events the session acts on.
const GAME_EVENT_CHANGE_GAME_MODE: u8 = 3;
const GAME_EVENT_IMMEDIATE_RESPAWN: u8 = 11;
const GAME_EVENT_LEVEL_CHUNKS_LOAD_START: u8 = 13;

/// `EntityEvent.USE_ITEM_COMPLETE`.
const ENTITY_EVENT_USE_ITEM_COMPLETE: u8 = 9;

/// What the game thread knows of the level it is in.
#[derive(Debug)]
struct World {
    dimension: String,
    player: LocalPlayer,
    level: Level,
    /// The other entities in the level.
    entities: Entities,
    /// Bits of the biome palette (from the biome registry's size).
    biome_bits: u32,
    /// Until the player counts as loaded.
    load: Option<LevelLoad>,
    loaded: bool,
    level_started: i64,
    /// `MultiPlayerGameMode` and `Minecraft`'s item use: the last hotbar slot sent (0 for
    /// a new game mode, so a server's slot other than 0 is sent back once), the use
    /// delay, the block being broken. The selected slot is the player's inventory's.
    game_mode: GameMode,
    /// The level's predicted blocks and sequence (new with each level).
    predictions: Predictions,
    /// The fluid shapes item use's raycasts have met (`FlowingFluid`'s cache).
    fluid_shapes: pick::FluidShapes,
    tick_rate: f32,
    frozen: bool,
    /// The level's game time: the server's (`set_time`), counted on by the ticks
    /// (`tickTime`). Pistons tell ticks apart by it; camels time sitting and getting up.
    game_time: i64,
    /// The world clocks (`ClientClockManager`): the play listener's, so they stay
    /// through a respawn.
    clocks: Clocks,
    /// What the dimension looks like, for drawing.
    environment: Arc<Environment>,
    /// The dimension's name again, shared with the published frames.
    dimension_name: Arc<str>,
    /// Client ticks run in this level.
    ticks: u64,
    /// Times the player was put somewhere (a teleport, a respawn).
    placements: u64,
    /// The blocks being broken, by the player and by others.
    destruction: Destruction,
}

impl World {
    /// The end of a client tick: unless the server froze the game, the border moves
    /// and the game time and the world clocks go on by one (`tickTime`).
    fn tick_time(&mut self) {
        if !self.frozen {
            self.level.border.tick();
            self.game_time += 1;
            self.clocks.tick(self.game_time);
        }
        self.destruction.tick(self.game_time);
        self.ticks += 1;
    }

    fn new(
        spawn: &play::SpawnInfo,
        now: i64,
        level: Level,
        biome_bits: u32,
        entities: Entities,
        environment: Arc<Environment>,
    ) -> World {
        // `handleLogin`: the game mode's abilities.
        let mut player = LocalPlayer::new();
        player.set_game_type(spawn.game_type);
        World {
            dimension: spawn.dimension.clone(),
            player,
            level,
            entities,
            biome_bits,
            load: Some(LevelLoad::start(now)),
            loaded: false,
            level_started: now,
            game_mode: GameMode::default(),
            predictions: Predictions::new(),
            fluid_shapes: pick::FluidShapes::new(),
            tick_rate: 20.0,
            frozen: false,
            game_time: 0,
            clocks: Clocks::default(),
            environment,
            dimension_name: Arc::from(spawn.dimension.as_str()),
            ticks: 0,
            placements: 0,
            destruction: Destruction::default(),
        }
    }
}

impl Game {
    fn new(
        connection: Connection,
        known_packs: Vec<KnownPack>,
        physics: Option<Physics>,
        events: Sender<SessionEvent>,
        publisher: ScenePublisher,
        information: ClientInformation,
    ) -> Game {
        let light_table = physics
            .as_ref()
            .map(|p| Arc::new(LightTable::of(&p.blocks)));
        Game {
            information,
            connection,
            known_packs,
            events,
            phase: Phase::Login,
            configured: Configured::default(),
            collecting: Configured::default(),
            cookies: HashMap::new(),
            server_brand: None,
            report_details: Vec::new(),
            seen_code_of_conduct: false,
            waiting_for_code_of_conduct: false,
            play_packets: 0,
            world: None,
            physics,
            tags: BlockTags::new(),
            item_tags: crate::item::ItemTags::new(),
            recipe_sets: crate::item::RecipeSets::new(),
            transformers: crate::use_on::Transformers::default(),
            clock: TickClock::new(0),
            frames: 0,
            ticks: 0,
            started: Instant::now(),
            manual_ms: None,
            view: Arc::default(),
            publisher,
            input_sequence: 0,
            input_at: None,
            motion_blocking: None,
            light_table,
            keys: Keys::default(),
            hotbar_clicks: [0; 9],
            use_keys: UseKeys::default(),
            screen_open: false,
            death_screen: None,
            bed_screen: false,
            held_keys: Keys::default(),
            hardcore: false,
            hud: crate::hud::HudState::default(),
            name: String::new(),
            uuid: 0,
            players: PlayerInfos::default(),
            teams: Teams::default(),
            scoreboard: Scoreboard::default(),
            simulation_distance: i32::MAX,
            own_skin: Arc::new(SkinSource::Profile(Default::default())),
            outline: None,
        }
    }

    fn report(&self, e: SessionEvent) {
        let _ = self.events.send(e);
    }

    fn run(&mut self, handoff: &Handoff<FrameInput, Command>) {
        while self.step(handoff.take()) {}
        // A main thread waiting for room in the queue goes on.
        handoff.end();
        self.publisher.end_level();
    }

    /// One thing from the handoff, in its order; false once the session has ended.
    fn step(&mut self, take: Take<FrameInput, Command>) -> bool {
        match take {
            // The session was dropped: no more frames.
            Take::Dropped => {
                self.connection.close();
                false
            }
            Take::Closing => {
                self.connection.close();
                true
            }
            Take::Item(Taken { sequence, at, item }) => match item {
                Item::Frame(input) => {
                    self.input_sequence = sequence;
                    self.input_at = Some(at);
                    self.frame(input)
                }
                Item::Control(c) => {
                    self.control(c);
                    true
                }
            },
        }
    }

    fn control(&mut self, c: Command) {
        match c {
            Command::Respawn => {
                if self.world.is_some() {
                    self.connection
                        .send(play::client_command(play::CLIENT_COMMAND_PERFORM_RESPAWN));
                }
            }
            // `InBedChatScreen.sendWakeUp`: only while the screen is open; it stays
            // open until the server wakes the player.
            Command::LeaveBed => {
                if self.bed_screen
                    && let Some(w) = &self.world
                {
                    self.connection.send(w.player.stop_sleeping_packet());
                }
            }
            Command::CodeOfConduct(accept) if self.waiting_for_code_of_conduct => {
                self.waiting_for_code_of_conduct = false;
                if accept {
                    self.connection.send(cfg::accept_code_of_conduct());
                } else {
                    // Vanilla disconnects with "multiplayer.disconnect.code_of_conduct".
                    self.connection.close();
                }
            }
            Command::CodeOfConduct(_) => {}
        }
    }

    /// One frame (`Minecraft.runTick`): the ticks due counted first, then every packet
    /// that has arrived, the frame's input, the ticks, and the camera's turn. False once
    /// the session has ended.
    fn frame(&mut self, input: FrameInput) -> bool {
        let ticks = self.clock.advance(self.millis(), self.ms_per_tick());
        loop {
            match self.connection.events().try_recv() {
                Ok(e) => {
                    if !self.handle(e) {
                        return false;
                    }
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    self.report(SessionEvent::Closed(Closed {
                        during_login: self.phase == Phase::Login,
                        reason: Reason::Local(Local::Io("the connection thread stopped".into())),
                    }));
                    return false;
                }
            }
        }
        self.take_input(&input);
        for _ in 0..ticks {
            self.tick();
        }
        self.frames += 1;
        self.ticks += u64::from(ticks);
        // `handleAccumulatedMovement`, without a player nothing turns.
        if let Some(w) = &mut self.world {
            for &(xo, yo) in &input.turns {
                w.player.turn(xo, yo);
            }
        }
        self.publish();
        self.update_view();
        true
    }

    /// Publishes the level's changes and the frame for drawing (vanilla's extract, on
    /// the game thread: the cost follows what changed and how many entities there are).
    fn publish(&mut self) {
        let Some(w) = &mut self.world else {
            return;
        };
        self.publisher.publish_level(&mut w.level);
        w.entities.refresh_names(&self.teams, &self.scoreboard);
        let now = Instant::now();
        let ms_per_tick = clock::ms_per_tick(w.tick_rate, !w.frozen);
        // The tick under way began this long before the clock's last reading.
        let into_tick = Duration::from_secs_f64(
            f64::from(self.clock.partial_tick()) * f64::from(ms_per_tick) / 1000.0,
        );
        let read = self.started + Duration::from_millis(self.clock.last_ms().max(0) as u64);
        let tick_start = read.checked_sub(into_tick).unwrap_or(read);
        let (input_sequence, input_at) = (self.input_sequence, self.input_at);
        let w = &*w;
        // The player's own skin: what its entry of the player list says.
        let textures = self
            .players
            .get(self.uuid)
            .and_then(|i| i.textures.as_ref());
        let same = matches!(&*self.own_skin, SkinSource::Profile(p)
            if p.id == Some(self.uuid) && p.properties.first() == textures);
        if !same {
            self.own_skin = Arc::new(SkinSource::Profile(ResolvableProfile {
                id: Some(self.uuid),
                name: Some(self.name.clone()),
                properties: textures.cloned().into_iter().collect(),
                ..ResolvableProfile::default()
            }));
        }
        // What the crosshair is on now (`Minecraft.pick` with the frame's partial
        // tick), where its outline is drawn (`shouldRenderBlockOutline`,
        // `extractBlockOutline`).
        let hit = self
            .physics
            .as_ref()
            .filter(|_| w.loaded && w.player.is_alive())
            .map(|physics| {
                let env = Env {
                    level: &w.level,
                    blocks: &physics.blocks,
                    tags: &self.tags,
                    colliders: &[],
                    others: None,
                };
                let partial = self.clock.partial_tick();
                frame_hit(&env, &w.entities, &self.players, &w.player, partial)
            });
        // `Hud.extractCrosshair`'s part of the full attack indicator: the crosshair is
        // on something that lives, within what the held item reaches.
        let target_alive = match hit {
            Some(pick::HitResult::Entity { id, location }) => {
                let p = &w.player;
                w.entities
                    .get(id)
                    .is_some_and(|e| e.info.living && !e.removed && e.health > 0.0)
                    && p.active_item().c.attack_range.is_none_or(|range| {
                        range.is_in_range(p.eye_position(), location, p.creative)
                    })
            }
            _ => false,
        };
        let target = self.physics.as_ref().and_then(|physics| {
            let p = &w.player;
            let pos = match hit? {
                pick::HitResult::Block(b) if !b.miss => b.pos,
                _ => return None,
            };
            let state = w.level.block(pos[0], pos[1], pos[2]);
            if physics.blocks.is_air(state) || !w.level.border.block_within_bounds(pos) {
                return None;
            }
            if !p.abilities.may_build {
                let visible = if p.game_type() == play::SPECTATOR {
                    crate::block::spectator_outline(
                        &physics.blocks,
                        &w.level,
                        &w.entities,
                        state,
                        pos,
                    )
                } else {
                    let held = p.hand(Hand::Main);
                    !held.is_empty()
                        && (crate::mining::can_break_in_adventure(held, state, &self.tags)
                            || crate::mining::can_place_on_in_adventure(held, state, &self.tags))
                };
                if !visible {
                    return None;
                }
            }
            let held = p.hand(Hand::Main).c.name;
            let filled = held == physics.blocks.name(state);
            if let Some((target, was_filled)) = &self.outline
                && (target.state, target.pos, *was_filled) == (state, pos, filled)
            {
                return Some(target.clone());
            }
            let placed = physics.blocks.outline(state, pos, held)?;
            let offset = placed.offset.unwrap_or([0.0; 3]);
            let edges = placed
                .shape
                .edges()
                .into_iter()
                .map(|ends| ends.map(|p| std::array::from_fn(|i| (p[i] + offset[i]) as f32)))
                .collect();
            let target = Target { pos, state, edges };
            self.outline = Some((target.clone(), filled));
            Some(target)
        });
        let own_skin = &self.own_skin;
        let (hud, hardcore) = (&self.hud, self.hardcore);
        self.publisher.publish_frame(
            || Frame::new(now, w.environment.clone(), w.dimension_name.clone()),
            |f| {
                f.input_sequence = input_sequence;
                f.input_at = input_at;
                f.published = now;
                f.ticks = w.ticks;
                f.tick_start = tick_start;
                f.ms_per_tick = ms_per_tick;
                f.frozen = w.frozen;
                f.game_time = w.game_time;
                w.clocks.samples(&mut f.clocks);
                f.rain = w.level.weather.rain;
                f.thunder = w.level.weather.thunder;
                if !Arc::ptr_eq(&f.dimension, &w.dimension_name) {
                    f.dimension = w.dimension_name.clone();
                }
                if !Arc::ptr_eq(&f.environment, &w.environment) {
                    f.environment = w.environment.clone();
                }
                f.terrain_stamp = w.level.stamp();
                let p = &w.player;
                let s = &mut f.player;
                s.entity_id = p.entity_id;
                s.position = p.position;
                s.old_position = p.old_position;
                s.y_rot = p.y_rot;
                s.x_rot = p.x_rot;
                s.old_rot = p.old_rot;
                s.eye_height = (p.eye_position()[1] - p.position[1]) as f32;
                s.eye_in_water = p.is_eye_in_water();
                s.water_vision = p.water_vision();
                s.pose = p.pose;
                s.on_ground = p.on_ground;
                s.sprinting = p.sprinting;
                s.flying = p.abilities.flying;
                s.game_type = p.game_type();
                s.health = p.health;
                s.food = p.food;
                s.selected_slot = p.inventory.selected;
                s.attack_strength = p.attack_strength_scale(0.0);
                s.breaking = w
                    .game_mode
                    .is_destroying
                    .then_some((w.game_mode.destroy_pos, w.game_mode.destroy_progress));
                s.effects.clear();
                s.effects
                    .extend(p.effects.iter().map(|e| (e.id, e.amplifier, e.duration)));
                s.vehicle = p.riding.as_ref().map(|r| r.vehicle);
                s.placements = w.placements;
                s.target = target;
                let r = &mut s.first_person;
                p.first_person.record(r);
                if !Arc::ptr_eq(&r.skin, own_skin) {
                    r.skin = own_skin.clone();
                }
                r.parts = self.information.model_parts;
                r.left_handed = self.information.main_hand == MainHand::Left;
                f.hud.fill(hud, p, hardcore, r.left_handed, target_alive);
                r.invisible = p.data.flags & crate::player::flag::INVISIBLE != 0;
                r.on_fire = p.data.flags & crate::player::flag::ON_FIRE != 0;
                r.sleeping = p.is_sleeping();
                r.spin_attack = p.is_auto_spin_attack();
                r.ticks_since_kinetic_hit = p
                    .first_person
                    .last_kinetic_hit
                    .and_then(|time| i32::try_from(w.game_time.saturating_sub(time)).ok())
                    .filter(|ticks| (0..10).contains(ticks));
                r.scoping = p.is_scoping();
                r.dead = p.is_dead_or_dying();
                r.death_time = p.death_time;
                r.using = p.is_using_item().then(|| p.used_hand() == Hand::Off);
                r.use_remaining = p.use_remaining();
                r.hands = first_person::hands_drawn(p.hand(Hand::Main), p.hand(Hand::Off), r.using);
                for hand in 0..2 {
                    let c = &r.held[hand].c;
                    r.use_duration[hand] = c.use_duration;
                    r.charge_duration[hand] = c.crossbow_charge_ticks;
                }
                w.destruction.stages(&mut f.breaking);
                w.level.block_entities.lids(&mut f.lids);
                f.entities.clear();
                f.entities_dropped = 0;
                for e in w.entities.iter() {
                    if f.entities.len() == MAX_ENTITIES {
                        f.entities_dropped += 1;
                        continue;
                    }
                    f.entities.push(EntityRecord {
                        id: e.id,
                        uuid: e.uuid,
                        entity_type: e.entity_type,
                        position: e.position,
                        old_position: e.old_position,
                        y_rot: e.y_rot,
                        x_rot: e.x_rot,
                        on_ground: e.on_ground,
                        pose: e.pose,
                        baby: e.baby,
                        vehicle: e.vehicle,
                        avatar: w
                            .entities
                            .avatar_record(e, &self.teams, p.spectator, &self.name),
                        name: w
                            .entities
                            .name_record(e, &self.teams, p.spectator, &self.name),
                    });
                }
            },
        );
    }

    /// The frame's events (`runAllTasks`, after the packets): keys and scrolls apply
    /// now, hotbar presses wait for the ticks.
    fn take_input(&mut self, input: &FrameInput) {
        self.keys = input.keys;
        self.held_keys = input.keys;
        self.use_keys.use_down = input.use_down;
        self.use_keys.use_clicks += input.use_clicks;
        self.use_keys.swap_clicks += input.swap_clicks;
        self.use_keys.attack_down = input.attack_down;
        self.use_keys.attack_clicks += input.attack_clicks;
        self.screen_open = input.screen_open;
        // The death screen or the bed screen took the keys (`KeyMapping.releaseAll`)
        // and keeps them.
        if self.death_screen.is_some() || self.bed_screen {
            self.release_keys();
        }
        // Without a player (`minecraft.player == null`) the input goes nowhere.
        let Some(w) = &mut self.world else {
            return;
        };
        if input.paused {
            let mut out = Vec::new();
            w.game_mode.stop_destroy_block(&mut w.player, &mut out);
            for p in out {
                self.connection.send(p);
            }
        }
        if !w.player.spectator {
            for &wheel in &input.scrolls {
                let inv = &mut w.player.inventory;
                inv.selected = next_scroll_selection(f64::from(wheel), inv.selected, 9);
            }
        }
        for (pending, new) in self.hotbar_clicks.iter_mut().zip(input.hotbar_clicks) {
            *pending += new;
        }
    }

    /// `KeyMapping.releaseAll`: a screen opened.
    fn release_keys(&mut self) {
        self.keys = Keys::default();
        self.use_keys.use_down = false;
        self.use_keys.attack_down = false;
    }

    /// Opens the death screen (`new DeathScreen`): the keys are let go.
    fn open_death_screen(&mut self, cause: Option<ruthenium_protocol::nbt::Tag>) {
        self.death_screen = Some(0);
        // It takes the bed screen's place.
        self.bed_screen = false;
        self.release_keys();
        let score = self.world.as_ref().map_or(0, |w| w.player.data.score);
        self.report(SessionEvent::Died {
            cause,
            hardcore: self.hardcore,
            score,
        });
    }

    /// Publishes the player's state for the screen.
    fn update_view(&self) {
        let view = self.world.as_ref().map(|w| PlayerView {
            dimension: w.dimension.clone(),
            position: w.player.position,
            y_rot: w.player.y_rot,
            x_rot: w.player.x_rot,
            on_ground: w.player.on_ground,
            sprinting: w.player.sprinting,
            pose: w.player.pose,
            flying: w.player.abilities.flying,
            game_type: w.player.game_type(),
            effects: w
                .player
                .effects
                .iter()
                .map(|e| (e.id, e.amplifier, e.duration))
                .collect(),
            health: w.player.health,
            food: w.player.food,
            selected_slot: w.player.inventory.selected,
            main_hand: w.player.hand(Hand::Main).describe(),
            off_hand: w.player.hand(Hand::Off).describe(),
            using: w.player.is_using_item().then(|| {
                (
                    w.player.used_hand().name(),
                    w.player.use_item().c.name.to_owned(),
                    w.player.use_remaining(),
                )
            }),
            attack_strength: w.player.attack_strength_scale(0.0),
            breaking: w
                .game_mode
                .is_destroying
                .then_some((w.game_mode.destroy_pos, w.game_mode.destroy_progress)),
            chunks: w.level.chunk_count(),
            frames: self.frames,
            ticks: self.ticks,
            entities: w.entities.len(),
            loaded: w.loaded,
        });
        *lock(&self.view) = view;
    }

    /// Vanilla's millisecond clock (`Util.getMillis`).
    fn millis(&self) -> i64 {
        self.manual_ms
            .unwrap_or_else(|| self.started.elapsed().as_millis() as i64)
    }

    fn ms_per_tick(&self) -> f32 {
        match &self.world {
            Some(w) => clock::ms_per_tick(w.tick_rate, !w.frozen),
            None => clock::DEFAULT_MS_PER_TICK,
        }
    }

    /// One event from the connection; false once it has closed.
    fn handle(&mut self, event: Event) -> bool {
        match event {
            Event::Stage(s) => self.report(SessionEvent::Stage(s)),
            Event::LoggedIn { profile } => {
                self.phase = Phase::Configuration;
                eprintln!("[session] logged in as {}", profile.name);
                self.name = profile.name.clone();
                self.uuid = profile.id;
                self.report(SessionEvent::LoggedIn { name: profile.name });
            }
            Event::Configuration(p) => {
                if let Err(e) = self.configuration(p) {
                    eprintln!("[session] {e}; disconnecting");
                    self.connection.close();
                }
            }
            Event::Play(p) => {
                if let Err(e) = self.play(p) {
                    eprintln!("[session] {e}; disconnecting");
                    self.connection.close();
                }
            }
            Event::Closed(c) => {
                log_closed(&c);
                self.report(SessionEvent::Closed(c));
                return false;
            }
        }
        true
    }

    /// A level for a dimension: its height from the dimension type the server named
    /// (the data it sent, or vanilla's for a known-pack entry), and the biome palette's
    /// width from the biome registry.
    fn new_level(&self, spawn: &play::SpawnInfo) -> (Level, u32) {
        let registry = |name: &str| {
            self.configured
                .registries
                .iter()
                .find(|(r, _)| r == name)
                .map(|(_, e)| e.as_slice())
                .unwrap_or(&[])
        };
        let types = registry("minecraft:dimension_type");
        let entry = usize::try_from(spawn.dimension_type)
            .ok()
            .and_then(|i| types.get(i));
        let from_data = entry.and_then(|e| {
            let c = e.data.as_ref()?.as_compound()?;
            let fast_lava = c
                .get("attributes")
                .and_then(|a| a.as_compound()?.boolean(DimensionType::FAST_LAVA))
                .unwrap_or(false);
            Some(DimensionType {
                min_y: c.int("min_y")?,
                height: c.int("height")?,
                fast_lava,
                has_skylight: c.boolean("has_skylight")?,
                has_ceiling: c.boolean("has_ceiling")?,
            })
        });
        let vanilla =
            entry.and_then(|e| self.physics.as_ref()?.dimension_types.get(&e.id).copied());
        let t = from_data.or(vanilla).unwrap_or_else(|| {
            eprintln!(
                "[session] dimension type {} unknown; assuming the overworld's height",
                spawn.dimension_type
            );
            DimensionType {
                min_y: -64,
                height: 384,
                fast_lava: false,
                has_skylight: true,
                has_ceiling: false,
            }
        });
        // The biomes' climates by network id: the data the server sent, else vanilla's
        // of that name.
        let biomes = registry("minecraft:worldgen/biome");
        let climates = biomes
            .iter()
            .map(|e| {
                let sent = e.data.as_ref().and_then(|d| {
                    let c = d.as_compound()?;
                    Some(Climate::new(
                        c.boolean("has_precipitation")?,
                        c.float("temperature")?,
                        c.string("temperature_modifier"),
                    ))
                });
                sent.or_else(|| self.physics.as_ref()?.biomes.get(&e.id).copied())
                    .unwrap_or(Climate::PLAINS)
            })
            .collect();
        let mut level = Level::new(t.min_y, t.height);
        level.set_fast_lava(t.fast_lava);
        level.set_has_skylight(t.has_skylight);
        if let Some(states) = &self.motion_blocking {
            level.set_motion_blocking(states.clone());
        }
        if let Some(table) = &self.light_table {
            level.set_light_table(table.clone());
        }
        level.track_changes(DEFAULT_DIRTY_CAPACITY);
        level.weather = Weather::new(t.has_skylight, t.has_ceiling, &spawn.dimension);
        level.flat = spawn.is_flat;
        level.biomes = BiomeInfo {
            climates: Arc::new(climates),
            seed: spawn.seed,
            sea_level: spawn.sea_level,
            empty_chunk_biome: 0,
        };
        (level, global_bits(biomes.len()))
    }

    /// What a dimension type's levels look like, from the configuration's registries.
    fn environment(&self, dimension_type: i32) -> Arc<Environment> {
        let none = LocalRegistries::new();
        let local = self.physics.as_ref().map_or(&none, |p| &*p.registries);
        Arc::new(Environment::resolve(
            &self.configured.registries,
            &self.configured.tags,
            local,
            dimension_type,
        ))
    }

    /// Which block states count in the motion-blocking heightmap
    /// (`Heightmap.Types.MOTION_BLOCKING`): those in the `blocks_motion_in_heightmap`
    /// tag and those with a fluid. Made again when the server sends tags.
    fn refresh_motion_blocking(&mut self) {
        let Some(physics) = &self.physics else {
            return;
        };
        let count = ruthenium_data::v26_3::blocks::STATE_COUNT;
        let mut bits = vec![0u64; count.div_ceil(64)];
        for state in 0..count as u32 {
            if self.tags.is(state, "minecraft:blocks_motion_in_heightmap")
                || !physics.blocks.fluid(state).is_empty()
            {
                bits[(state / 64) as usize] |= 1 << (state % 64);
            }
        }
        let bits = Arc::new(bits);
        if let Some(w) = &mut self.world {
            w.level.set_motion_blocking(bits.clone());
        }
        self.motion_blocking = Some(bits);
    }

    /// The block transformers the configuration's registry lists: each entry's own
    /// data, else vanilla's of that name.
    fn configured_transformers(&self) -> crate::use_on::Transformers {
        let entries = self
            .configured
            .registries
            .iter()
            .find(|(r, _)| r == "minecraft:block_transformer")
            .map(|(_, e)| e.as_slice())
            .unwrap_or(&[]);
        let vanilla = self
            .physics
            .as_ref()
            .map(|p| p.block_transformers.clone())
            .unwrap_or_default();
        crate::use_on::Transformers::from_registry(entries, &vanilla)
    }

    /// The items read the enchantment registry the server sent (a riptide trident's
    /// spin).
    fn configure_enchantments(&mut self) {
        let Some(physics) = &mut self.physics else {
            return;
        };
        let entries = self
            .configured
            .registries
            .iter()
            .find(|(r, _)| r == "minecraft:enchantment")
            .map(|(_, e)| e.as_slice())
            .unwrap_or(&[]);
        let registry = |name: &str| {
            self.configured
                .registries
                .iter()
                .find(|(r, _)| r == name)
                .map(|(_, e)| e.as_slice())
                .unwrap_or(&[])
        };
        let mut items = (*physics.items).clone();
        items.enchantments =
            crate::enchantment::Enchantments::from_registry(entries, &physics.enchantments);
        // What worn and held things look like: trims, and the patterns of shields.
        items.trims = crate::item::Trims::from_registry(
            registry("minecraft:trim_material"),
            registry("minecraft:trim_pattern"),
        );
        items.banner_patterns = registry("minecraft:banner_pattern")
            .iter()
            .map(|e| {
                e.data
                    .as_ref()
                    .and_then(|d| d.as_compound()?.string("asset_id"))
                    .unwrap_or(&e.id)
                    .into()
            })
            .collect();
        physics.items = std::sync::Arc::new(items);
    }

    /// A level's entities, the player's among them.
    fn new_entities(&self, player_id: i32) -> Entities {
        let improved = self
            .configured
            .features
            .iter()
            .any(|f| f == MINECART_IMPROVEMENTS);
        let mut entities = Entities::new(player_id, improved);
        entities.simulation_distance = self.simulation_distance;
        entities
    }

    /// The block tags movement reads, from the server's tags (block registry ids).
    fn apply_tags(&mut self, tags: &[RegistryTags]) {
        if let Some(items) = tags.iter().find(|t| t.registry == "minecraft:item") {
            for (tag, ids) in &items.tags {
                let names = ids.iter().filter_map(|&id| {
                    ruthenium_data::v26_3::registries::Item::from_id(u32::try_from(id).ok()?)
                        .map(|i| i.name())
                });
                self.item_tags.set(tag, names);
            }
        }
        let Some(blocks) = tags.iter().find(|t| t.registry == "minecraft:block") else {
            return;
        };
        for (tag, ids) in &blocks.tags {
            let names = ids.iter().filter_map(|&id| {
                ruthenium_data::v26_3::Block::from_id(u32::try_from(id).ok()?).map(|b| b.name())
            });
            self.tags.set(tag, names);
        }
        self.refresh_motion_blocking();
    }

    /// One client tick (`Minecraft.tick`), for the parts there are so far.
    fn tick(&mut self) {
        let now = self.millis();
        let Some(w) = &mut self.world else {
            return;
        };
        // `gui.tick`, before anything of the level ticks.
        self.hud.tick(&w.player);
        let mut out = Vec::new();
        // `rightClickDelay`, then `gameMode.tick`: the held slot first, then the
        // connection's tick, where the listener lets the player in once the level is
        // ready.
        w.game_mode.tick_delay();
        w.game_mode.ensure_carried(&w.player.inventory, &mut out);
        for p in out.drain(..) {
            self.connection.send(p);
        }
        if let Some(load) = &mut w.load {
            let (cx, cz) = w.player.chunk();
            let chunk = w.level.has_chunk(cx, cz);
            load.tick(now, chunk, w.player.spectator || !w.player.is_alive());
            if load.is_ready(now) {
                w.load = None;
                if !w.loaded {
                    self.connection.send(play::player_loaded());
                    w.loaded = true;
                    eprintln!(
                        "[session] in the world after {} ms ({} chunks)",
                        now - w.level_started,
                        w.level.chunk_count()
                    );
                    let _ = self.events.send(SessionEvent::Loaded);
                }
            }
        }
        // `Gui.tick`: with no screen open, a dead player gets the death screen, or asks
        // to respawn at once (every tick, until the server answers); then the death
        // screen ticks, its buttons working from its 20th tick.
        // With the bed screen open, it closes once the player is out of bed; with no
        // screen open, a sleeping player gets it.
        let no_screen = !self.screen_open && self.death_screen.is_none() && !self.bed_screen;
        let found_dead = no_screen && w.player.is_dead_or_dying();
        let show_death_screen = w.player.show_death_screen;
        if self.bed_screen && !w.player.is_sleeping() {
            self.bed_screen = false;
            // Grabbing the mouse reads the keys physically held (`KeyMapping.setAll`,
            // except on macOS).
            if !cfg!(target_os = "macos") {
                self.keys = self.held_keys;
            }
            let _ = self.events.send(SessionEvent::WokeUp);
        } else if no_screen && !found_dead && w.player.is_sleeping() {
            self.bed_screen = true;
            self.keys = Keys::default();
            self.use_keys.use_down = false;
            self.use_keys.attack_down = false;
            let _ = self.events.send(SessionEvent::InBed);
        }
        if found_dead {
            if show_death_screen {
                self.open_death_screen(None);
            } else {
                self.connection
                    .send(play::client_command(play::CLIENT_COMMAND_PERFORM_RESPAWN));
            }
        }
        if let Some(t) = &mut self.death_screen {
            *t += 1;
            if *t == 20 {
                let _ = self.events.send(SessionEvent::RespawnReady);
            }
        }
        let Some(w) = &mut self.world else {
            return;
        };
        let Some(physics) = &self.physics else {
            self.use_keys = UseKeys {
                use_down: self.use_keys.use_down,
                attack_down: self.use_keys.attack_down,
                ..UseKeys::default()
            };
            w.tick_time();
            self.connection.send(play::client_tick_end());
            return;
        };
        // `pick`, then `handleKeybinds` unless a screen is open (the level-loading screen
        // or another): one press per hotbar key per tick (spectators' hotbar keys open
        // the spectator menu instead, which isn't there yet), then the hands. A screen
        // makes attacks wait for the attack key to be let go.
        if !w.loaded || self.screen_open || self.death_screen.is_some() || self.bed_screen {
            w.game_mode.screen_open();
        } else {
            let env = Env {
                level: &w.level,
                blocks: &physics.blocks,
                tags: &self.tags,
                colliders: &[],
                others: None,
            };
            let hit = pick_target(&env, &w.entities, &self.players, &w.player);
            for (slot, pending) in self.hotbar_clicks.iter_mut().enumerate() {
                if *pending > 0 {
                    *pending -= 1;
                    if !w.player.spectator {
                        w.player.inventory.selected = slot as i32;
                    }
                }
            }
            let mut world = interact::World::new(
                &mut w.level,
                &mut w.predictions,
                &physics.blocks,
                &self.tags,
                &w.entities,
                &w.fluid_shapes,
            )
            .with_players(&self.players)
            .with_items(&physics.items)
            .with_item_tags(&self.item_tags)
            .with_transformers(&self.transformers)
            .with_recipe_sets(&self.recipe_sets);
            w.game_mode.handle_keybinds(
                &mut w.player,
                &mut world,
                &hit,
                &mut self.use_keys,
                &mut out,
            );
            for p in out.drain(..) {
                self.connection.send(p);
            }
        }
        // The cracks of the block the player is breaking (`destroyBlockProgress` with
        // `getDestroyStage`: none once it is given up or broken).
        let stage = if w.game_mode.is_destroying {
            w.game_mode.destroy_stage()
        } else {
            -1
        };
        w.destruction.set(
            w.player.entity_id,
            w.game_mode.destroy_pos,
            stage,
            w.game_time,
        );
        let env = Env {
            level: &w.level,
            blocks: &physics.blocks,
            tags: &self.tags,
            colliders: &[],
            others: None,
        };
        // `level.tickEntities`: the entities in order, the player among them, which
        // ticks and sends its changes (`sendPosition`) only once it has loaded.
        let cx = TickContext {
            env: &env,
            teams: &self.teams,
            players: &self.players,
            local: LocalInfo {
                name: &self.name,
                spectator: w.player.spectator,
                alive: w.player.is_alive(),
            },
            tick_rate: w.tick_rate,
            runs_normally: !w.frozen,
            game_time: w.game_time,
        };
        let (loaded, keys) = (w.loaded, self.keys);
        w.entities
            .tick(&cx, &mut w.player, |player, entities, colliders| {
                // A player taken out of the level (20 ticks after dying) no longer ticks.
                if loaded && player.is_alive() {
                    let env = Env {
                        colliders,
                        others: Some(Others {
                            entities,
                            players: &self.players,
                        }),
                        ..env
                    };
                    player.tick(&env, keys);
                }
            });
        // `tickBlockEntities`: moving pistons and shulker boxes' lids, which push the
        // player; then `sendChanges`, for a player still in the level.
        if !w.frozen {
            let mut vehicle = w.entities.take_controlled_vehicle(&w.player);
            interact::World::new(
                &mut w.level,
                &mut w.predictions,
                &physics.blocks,
                &self.tags,
                &w.entities,
                &w.fluid_shapes,
            )
            .tick_block_entities_riding(
                &mut w.player,
                vehicle.as_mut().map(|(e, c)| (e, c.as_slice())),
                w.game_time,
            );
            if let Some((e, _)) = vehicle {
                w.entities.put_back(e);
            }
        }
        if loaded && w.player.is_alive() {
            w.entities.sync_riding(&mut w.player);
            let vehicle = w.entities.vehicle_move();
            for p in w.player.send_changes_riding(vehicle) {
                self.connection.send(p);
            }
        }
        // `level.tick`: the world border, then the game time.
        w.tick_time();
        self.connection.send(play::client_tick_end());
    }

    /// The packets every phase handles alike. `cookie` and `pack` build the phase's
    /// answers.
    fn common(
        &mut self,
        p: Common,
        cookie: fn(&str, Option<&[u8]>) -> Vec<u8>,
        pack: fn(u128, PackAction) -> Vec<u8>,
        pong: fn(i32) -> Vec<u8>,
    ) {
        match p {
            Common::Ping { id } => self.connection.send(pong(id)),
            Common::CookieRequest { key } => {
                let c = self.cookies.get(&key).map(Vec::as_slice);
                self.connection.send(cookie(&key, c));
            }
            Common::StoreCookie { key, payload } => {
                self.cookies.insert(key, payload);
            }
            Common::CustomPayload { channel, data } => {
                if channel == "minecraft:brand" {
                    match read_brand(&data) {
                        Ok(b) => self.server_brand = Some(b),
                        Err(e) => eprintln!("[session] bad server brand: {e}"),
                    }
                }
            }
            Common::ResourcePackPush(push) => {
                // Vanilla answers a URL that isn't http(s) at once.
                let web = push.url.starts_with("http://") || push.url.starts_with("https://");
                let action = if web {
                    eprintln!(
                        "[session] server resource pack {} declined: not supported yet{}",
                        push.url,
                        if push.required {
                            " (the server requires it)"
                        } else {
                            ""
                        }
                    );
                    PackAction::Declined
                } else {
                    PackAction::InvalidUrl
                };
                self.connection.send(pack(push.id, action));
            }
            Common::ResourcePackPop { .. } => {}
            Common::Transfer { host, port } => {
                eprintln!("[session] the server transfers to {host}:{port}: not supported yet");
                self.connection.close();
            }
            Common::CustomReportDetails { details } => self.report_details = details,
            // The connection thread handles these itself.
            Common::KeepAlive { .. } | Common::Disconnect { .. } => {}
        }
    }

    fn configuration(&mut self, p: cfg::Clientbound) -> Result<(), String> {
        match p {
            cfg::Clientbound::Common(c) => {
                self.common(c, cfg::cookie_response, cfg::resource_pack, cfg::pong)
            }
            cfg::Clientbound::SelectKnownPacks(asked) => {
                let ours: Vec<KnownPack> = asked
                    .into_iter()
                    .filter(|p| self.known_packs.contains(p))
                    .collect();
                self.connection.send(cfg::select_known_packs(&ours));
            }
            cfg::Clientbound::RegistryData { registry, entries } => {
                self.collecting.add_registry(registry, entries);
            }
            cfg::Clientbound::UpdateTags(tags) => {
                self.apply_tags(&tags);
                self.collecting.tags = tags;
            }
            cfg::Clientbound::UpdateEnabledFeatures { features } => {
                self.collecting.features = features;
            }
            cfg::Clientbound::CodeOfConduct(text) => {
                if self.seen_code_of_conduct {
                    return Err("the server sent a second code of conduct".into());
                }
                self.seen_code_of_conduct = true;
                self.waiting_for_code_of_conduct = true;
                self.report(SessionEvent::CodeOfConduct(text));
            }
            cfg::Clientbound::Finish => {
                let mut c = std::mem::take(&mut self.collecting);
                // A reconfiguration starts from what the last one had.
                for (r, e) in std::mem::take(&mut self.configured.registries) {
                    if !c.registries.iter().any(|(n, _)| *n == r) {
                        c.registries.push((r, e));
                    }
                }
                if c.tags.is_empty() {
                    c.tags = std::mem::take(&mut self.configured.tags);
                }
                self.configured = c;
                self.transformers = self.configured_transformers();
                self.configure_enchantments();
                log_configured(&self.configured, self.server_brand.as_deref());
                self.connection
                    .switch(Protocol::Play, cfg::finish_configuration());
                self.phase = Phase::Play;
                self.play_packets = 0;
                self.report(SessionEvent::Playing);
            }
            cfg::Clientbound::ResetChat | cfg::Clientbound::PostEffects => {}
            cfg::Clientbound::ServerLinks => {}
            cfg::Clientbound::ShowDialog | cfg::Clientbound::ClearDialog => {
                eprintln!("[session] the server sent a dialog: not shown yet");
            }
        }
        Ok(())
    }

    fn play(&mut self, p: play::Clientbound) -> Result<(), String> {
        let now = self.millis();
        match p {
            play::Clientbound::Common(c) => {
                self.common(c, play::cookie_response, play::resource_pack, play::pong)
            }
            play::Clientbound::StartConfiguration => {
                // Vanilla also sends a pending chat acknowledgement first; there is no
                // chat yet. The level goes (`clearLevel`), and with it the ticks' packets.
                self.connection
                    .switch(Protocol::Configuration, play::configuration_acknowledged());
                self.phase = Phase::Configuration;
                self.collecting = Configured::default();
                self.world = None;
                self.publisher.end_level();
                self.death_screen = None;
                self.players = PlayerInfos::default();
                self.teams = Teams::default();
                self.scoreboard = Scoreboard::default();
                self.report(SessionEvent::Reconfiguring);
            }
            play::Clientbound::Login(login) => {
                eprintln!(
                    "[session] joined {} as entity {} (view {}, simulation {}, game type {})",
                    login.spawn.dimension,
                    login.player_id,
                    login.chunk_radius,
                    login.simulation_distance,
                    login.spawn.game_type
                );
                let (level, biome_bits) = self.new_level(&login.spawn);
                self.simulation_distance = login.simulation_distance;
                let entities = self.new_entities(login.player_id);
                let environment = self.environment(login.spawn.dimension_type);
                self.publisher.begin_level(Dims::of(&level));
                let mut world =
                    World::new(&login.spawn, now, level, biome_bits, entities, environment);
                world.player.entity_id = login.player_id;
                world.player.show_death_screen = login.show_death_screen;
                self.hardcore = login.hardcore;
                self.world = Some(world);
            }
            play::Clientbound::Respawn {
                spawn,
                data_to_keep,
            } => {
                let (level, biome_bits) = self.new_level(&spawn);
                let id = self
                    .world
                    .as_ref()
                    .ok_or("a respawn before the login")?
                    .player
                    .entity_id;
                let entities = self.new_entities(id);
                let environment = self.environment(spawn.dimension_type);
                let w = self.world.as_mut().ok_or("a respawn before the login")?;
                if spawn.dimension != w.dimension {
                    // Another level: what was published of the old one is void.
                    self.publisher.begin_level(Dims::of(&level));
                    w.level = level;
                    w.predictions = Predictions::new();
                    w.entities = entities;
                    w.biome_bits = biome_bits;
                    w.dimension = spawn.dimension.clone();
                    w.dimension_name = Arc::from(spawn.dimension.as_str());
                    w.environment = environment;
                    w.ticks = 0;
                } else {
                    w.entities.local_respawned();
                }
                w.placements += 1;
                // `handleRespawn`: a new player, with what the server says to keep, in
                // the new game mode; not loaded until the level is ready.
                w.player = w.player.respawned(data_to_keep);
                w.player.set_game_type(spawn.game_type);
                w.loaded = false;
                w.load = Some(LevelLoad::start(now));
                w.level_started = now;
                // The death screen (or the bed screen) closes for the level loading screen.
                self.death_screen = None;
                self.bed_screen = false;
                self.release_keys();
                self.report(SessionEvent::Respawned);
            }
            play::Clientbound::PlayerPosition(t) => {
                let w = self.world.as_mut().ok_or("a teleport before the login")?;
                w.player.teleport(&t);
                w.placements += 1;
                let p = &w.player;
                eprintln!(
                    "[session] teleport {} to {:.3} {:.3} {:.3} (relative bits {:#x})",
                    t.id, p.position[0], p.position[1], p.position[2], t.relatives
                );
                self.connection.send(play::accept_teleportation(
                    t.id, p.position, p.y_rot, p.x_rot,
                ));
                // Acknowledgements of earlier actions no longer move the player, and
                // breaking stops.
                w.predictions.on_teleport();
                let mut out = Vec::new();
                w.game_mode.stop_destroy_block(&mut w.player, &mut out);
                for p in out {
                    self.connection.send(p);
                }
            }
            play::Clientbound::GameEvent { event, param } => {
                if let Some(w) = &mut self.world {
                    match event {
                        GAME_EVENT_CHANGE_GAME_MODE => {
                            w.player.set_game_type((param + 0.5).floor() as i32)
                        }
                        GAME_EVENT_IMMEDIATE_RESPAWN => w.player.show_death_screen = param == 0.0,
                        GAME_EVENT_LEVEL_CHUNKS_LOAD_START => {
                            if let Some(l) = &mut w.load {
                                l.loading_packets_received();
                            }
                        }
                        _ => {
                            w.level.weather.game_event(event, param);
                        }
                    }
                }
            }
            // `handleLevelChunkWithLight`. Vanilla queues the chunk's light and applies
            // it with a later frame; here it comes with the chunk, so the chunk is never
            // seen without its light.
            play::Clientbound::LevelChunk {
                x,
                z,
                heightmaps,
                data,
                block_entities,
                light,
            } => {
                if let Some(w) = &mut self.world {
                    let packet = ChunkPacket {
                        heightmaps: &heightmaps,
                        data: &data,
                        block_entities: &block_entities,
                        light: &light,
                    };
                    if let Err(e) = w.level.load_chunk_packet(x, z, packet, w.biome_bits) {
                        return Err(format!("chunk {x} {z}: {e}"));
                    }
                    w.entities.chunk_loaded(x, z);
                }
            }
            // `handleLightUpdatePacket`: the server's light after a block changed.
            play::Clientbound::LightUpdate { x, z, light } => {
                if let Some(w) = &mut self.world {
                    w.level.update_light(x, z, &light);
                }
            }
            // `handleBlockEntityData`: kept for drawing.
            play::Clientbound::BlockEntityData { pos, kind, tag } => {
                if let Some(w) = &mut self.world {
                    w.level.set_block_entity_data(pos, kind, tag);
                }
            }
            play::Clientbound::ChunksBiomes { chunks } => {
                if let Some(w) = &mut self.world {
                    for (x, z, data) in chunks {
                        if let Err(e) = w.level.load_biomes(x, z, &data, w.biome_bits) {
                            return Err(format!("biomes of chunk {x} {z}: {e}"));
                        }
                    }
                }
            }
            play::Clientbound::ForgetLevelChunk { x, z } => {
                if let Some(w) = &mut self.world {
                    w.level.forget_chunk(x, z);
                    w.entities.chunk_unloaded(x, z);
                }
            }
            // `setServerVerifiedBlockState`: a predicted block keeps the server's state
            // until the prediction is acknowledged.
            play::Clientbound::BlockUpdate { pos, state } => {
                if let Some(w) = &mut self.world {
                    w.predictions.server_block(&mut w.level, pos, state);
                }
            }
            play::Clientbound::SectionBlocksUpdate { section, changes } => {
                if let Some(w) = &mut self.world {
                    for (rel, state) in changes {
                        let rel = i32::from(rel);
                        let pos = [
                            section[0] * 16 + (rel >> 8 & 15),
                            section[1] * 16 + (rel & 15),
                            section[2] * 16 + (rel >> 4 & 15),
                        ];
                        w.predictions.server_block(&mut w.level, pos, state);
                    }
                }
            }
            // `handleBlockEvent`: pistons move, shulker boxes open.
            play::Clientbound::BlockEvent { pos, b0, b1, .. } => {
                if let Some(w) = &mut self.world
                    && let Some(physics) = &self.physics
                {
                    interact::World::new(
                        &mut w.level,
                        &mut w.predictions,
                        &physics.blocks,
                        &self.tags,
                        &w.entities,
                        &w.fluid_shapes,
                    )
                    .block_event(pos, b0, b1);
                }
            }
            play::Clientbound::BlockChangedAck { sequence } => {
                if let Some(w) = &mut self.world
                    && let Some(physics) = &self.physics
                {
                    w.predictions.acknowledge(
                        sequence,
                        &mut w.level,
                        &physics.blocks,
                        &mut w.player,
                    );
                }
            }
            play::Clientbound::UpdateTags(tags) => self.apply_tags(&tags),
            play::Clientbound::UpdateAttributes {
                entity_id,
                attributes,
            } => {
                if let Some(w) = &mut self.world {
                    if entity_id == w.player.entity_id {
                        w.player.apply_attributes(&attributes);
                    } else {
                        w.entities.update_attributes(entity_id, &attributes);
                    }
                }
            }
            play::Clientbound::SetEntityMotion {
                entity_id,
                movement,
            } => {
                // `lerpMotion`: the velocity is replaced (knockback).
                if let Some(w) = &mut self.world {
                    if entity_id == w.player.entity_id {
                        w.player.delta = movement;
                    } else {
                        w.entities.set_motion(entity_id, movement);
                    }
                }
            }
            play::Clientbound::Explode {
                player_knockback: Some(k),
            } => {
                // `pushFromExplosion`: added to the velocity, if finite.
                if let Some(w) = &mut self.world
                    && k.iter().all(|v| v.is_finite())
                {
                    let d = &mut w.player.delta;
                    *d = [d[0] + k[0], d[1] + k[1], d[2] + k[2]];
                }
            }
            play::Clientbound::Explode {
                player_knockback: None,
            } => {}
            play::Clientbound::UpdateRecipes { item_sets } => {
                self.recipe_sets = crate::item::RecipeSets::new();
                for (key, items) in item_sets {
                    self.recipe_sets.set(&key, items);
                }
            }
            play::Clientbound::SetHeldSlot { slot } => {
                // `Inventory.isHotbarSlot`.
                if let Some(w) = &mut self.world
                    && (0..9).contains(&slot)
                {
                    w.player.inventory.selected = slot;
                }
            }
            play::Clientbound::TickingState { tick_rate, frozen } => {
                if let Some(w) = &mut self.world {
                    w.tick_rate = tick_rate;
                    w.frozen = frozen;
                }
            }
            play::Clientbound::AddEntity(add) => {
                if let Some(w) = &mut self.world
                    && let Some(physics) = &self.physics
                {
                    w.entities.add(&add, &physics.entity_types, &self.players);
                }
            }
            play::Clientbound::RemoveEntities { ids } => {
                if let Some(w) = &mut self.world {
                    for id in ids {
                        w.entities.remove(id);
                    }
                }
            }
            play::Clientbound::MoveEntity {
                entity_id,
                delta,
                rotation,
                on_ground,
            } => {
                if let Some(w) = &mut self.world {
                    w.entities
                        .move_entity(entity_id, delta.as_ref(), rotation, on_ground);
                }
            }
            play::Clientbound::TeleportEntity {
                entity_id,
                change,
                on_ground,
            } => {
                if let Some(w) = &mut self.world
                    && let Some(p) =
                        w.entities
                            .teleport(entity_id, &change, on_ground, &mut w.player)
                {
                    self.connection.send(p);
                }
            }
            play::Clientbound::EntityPositionSync {
                entity_id,
                path,
                y_rot,
                x_rot,
                on_ground,
            } => {
                if let Some(w) = &mut self.world {
                    w.entities.position_sync(
                        entity_id,
                        &path,
                        y_rot,
                        x_rot,
                        on_ground,
                        &mut w.player,
                    );
                }
            }
            play::Clientbound::SetEntityData {
                entity_id, values, ..
            } => {
                if let Some(w) = &mut self.world
                    && let Some(physics) = &self.physics
                {
                    if entity_id == w.player.entity_id {
                        if w.player.set_data(&values, &physics.entity_types) {
                            w.player.move_to_bed(&Env {
                                level: &w.level,
                                blocks: &physics.blocks,
                                tags: &self.tags,
                                colliders: &[],
                                others: None,
                            });
                        }
                    } else {
                        w.entities
                            .set_data(entity_id, &values, &physics.entity_types);
                    }
                }
            }
            play::Clientbound::SetPassengers {
                vehicle,
                passengers,
            } => {
                if let Some(w) = &mut self.world {
                    if let Some((y, x)) = w.entities.set_passengers(vehicle, &passengers) {
                        w.player.y_rot = y % 360.0;
                        // A boat turns the head too.
                        if x.is_none() {
                            w.player.head_y_rot = y;
                        }
                        if let Some(x) = x {
                            w.player.x_rot = x.clamp(-90.0, 90.0) % 360.0;
                        }
                        w.player.old_rot = (w.player.y_rot, w.player.x_rot);
                    }
                    w.entities.sync_riding(&mut w.player);
                }
            }
            play::Clientbound::SetPlayerTeam {
                name,
                method,
                parameters,
                players,
            } => self
                .teams
                .apply(&name, method, parameters.as_ref(), &players),
            play::Clientbound::SetObjective {
                name,
                method,
                display_name,
                number_format,
            } => self
                .scoreboard
                .set_objective(&name, method, display_name, number_format),
            play::Clientbound::SetDisplayObjective { slot, objective } => {
                self.scoreboard.set_display(slot, &objective);
            }
            play::Clientbound::SetScore {
                owner,
                objective,
                score,
                number_format,
                ..
            } => self
                .scoreboard
                .set_score(&owner, &objective, score, number_format),
            play::Clientbound::ResetScore { owner, objective } => {
                self.scoreboard.reset_score(&owner, objective.as_deref());
            }
            play::Clientbound::PlayerInfoUpdate { actions, entries } => {
                if let Some(w) = &mut self.world
                    && let Some(mode) = self.players.changed_game_mode(self.uuid, actions, &entries)
                {
                    w.player.on_game_mode_changed(mode);
                }
                self.players.update(actions, &entries);
                // `isSpectator` reads the player's entry (kept once found, so a removal
                // leaves it as it was).
                if let Some(w) = &mut self.world
                    && let Some(info) = self.players.get(self.uuid)
                {
                    w.player.spectator = info.game_mode == play::SPECTATOR;
                    w.player.creative = info.game_mode == play::CREATIVE;
                }
            }
            play::Clientbound::PlayerInfoRemove { ids } => self.players.remove(&ids),
            play::Clientbound::PlayerCombatKill { player_id, message } => {
                // `handlePlayerCombatKill`: the local player's death opens the death
                // screen, or asks to respawn at once.
                let Some(w) = &self.world else {
                    return Ok(());
                };
                if player_id == w.player.entity_id {
                    if w.player.show_death_screen {
                        self.open_death_screen(Some(message));
                    } else {
                        self.connection
                            .send(play::client_command(play::CLIENT_COMMAND_PERFORM_RESPAWN));
                    }
                }
            }
            play::Clientbound::SetHealth {
                health,
                food,
                saturation,
            } => {
                // `hurtTo` and the food data.
                if let Some(w) = &mut self.world {
                    w.player.hurt_to(health);
                    w.player.food = food;
                    w.player.saturation = saturation;
                }
            }
            play::Clientbound::SetExperience {
                progress,
                level,
                total,
            } => {
                // `setExperienceValues`.
                if let Some(w) = &mut self.world {
                    w.player.experience_progress = progress;
                    w.player.experience_level = level;
                    w.player.total_experience = total;
                }
            }
            play::Clientbound::ContainerSetContent {
                container_id,
                state_id,
                items,
                carried,
            } => {
                // Container 0 is the inventory; other menus aren't tracked yet.
                if let Some(w) = &mut self.world
                    && let Some(physics) = &self.physics
                    && container_id == 0
                {
                    let it = &physics.items;
                    let stacks = items.iter().map(|s| it.stack(s)).collect();
                    w.player
                        .inventory
                        .initialize(state_id, stacks, it.stack(&carried));
                    self.hud.slots_replaced();
                }
            }
            play::Clientbound::ContainerSetSlot {
                container_id,
                state_id,
                slot,
                item,
            } => {
                if let Some(w) = &mut self.world
                    && let Some(physics) = &self.physics
                    && container_id == 0
                {
                    let s = physics.items.stack(&item);
                    // `InventoryMenu.isHotbarSlot`: the hotbar, then the off hand.
                    if let Some(shown @ 0..crate::hud::SLOTS) =
                        usize::try_from(slot).ok().and_then(|i| i.checked_sub(36))
                    {
                        let at = if shown == crate::hud::OFF_HAND {
                            crate::inventory::SLOT_OFFHAND
                        } else {
                            shown
                        };
                        self.hud.slot_set(shown, w.player.inventory.get(at), &s);
                    }
                    w.player
                        .inventory
                        .set_menu_slot(i32::from(slot), state_id, s);
                }
            }
            play::Clientbound::SetPlayerInventory { slot, item } => {
                if let Some(w) = &mut self.world
                    && let Some(physics) = &self.physics
                    && let Ok(slot) = usize::try_from(slot)
                {
                    w.player.inventory.set(slot, physics.items.stack(&item));
                    match slot {
                        0..9 => self.hud.slot_replaced(slot),
                        crate::inventory::SLOT_OFFHAND => {
                            self.hud.slot_replaced(crate::hud::OFF_HAND);
                        }
                        _ => {}
                    }
                }
            }
            play::Clientbound::SetCursorItem { item } => {
                if let Some(w) = &mut self.world
                    && let Some(physics) = &self.physics
                {
                    w.player.inventory.carried = physics.items.stack(&item);
                }
            }
            play::Clientbound::Cooldown { group, duration } => {
                if let Some(w) = &mut self.world {
                    if duration == 0 {
                        w.player.cooldowns.remove(&group);
                    } else {
                        w.player.cooldowns.add(&group, duration);
                    }
                }
            }
            play::Clientbound::EntityEvent { entity_id, event } => {
                // `Player.handleEntityEvent`: the server finished the item's use; the
                // player's operator level (events 24 to 28: levels 0 to 4).
                if let Some(w) = &mut self.world
                    && entity_id == w.player.entity_id
                {
                    match event {
                        2 => {
                            let last = &mut w.player.first_person.last_kinetic_hit;
                            if last.is_none_or(|time| w.game_time.saturating_sub(time) > 10) {
                                *last = Some(w.game_time);
                            }
                        }
                        ENTITY_EVENT_USE_ITEM_COMPLETE => {
                            if let Some(physics) = &self.physics {
                                let colliders = w.entities.colliders(&w.player);
                                let env = Env {
                                    level: &w.level,
                                    blocks: &physics.blocks,
                                    tags: &self.tags,
                                    colliders: &colliders,
                                    others: None,
                                };
                                w.player.complete_using_item(&env);
                            }
                        }
                        24..=28 => w.player.permission_level = i32::from(event) - 24,
                        _ => {}
                    }
                } else if let Some(w) = &mut self.world {
                    w.entities.entity_event(entity_id, event);
                }
            }
            play::Clientbound::PlayerAbilities {
                flags,
                flying_speed,
                walking_speed,
            } => {
                if let Some(w) = &mut self.world {
                    w.player.abilities.apply(flags, flying_speed, walking_speed);
                }
            }
            play::Clientbound::UpdateMobEffect {
                entity_id,
                effect,
                amplifier,
                duration,
                flags,
            } => {
                // Other entities' effects don't move them on the client.
                if let Some(w) = &mut self.world
                    && entity_id == w.player.entity_id
                {
                    w.player.update_effect(Effect {
                        id: effect,
                        amplifier,
                        duration,
                        flags,
                    });
                }
            }
            play::Clientbound::RemoveMobEffect { entity_id, effect } => {
                if let Some(w) = &mut self.world
                    && entity_id == w.player.entity_id
                {
                    w.player.remove_effect(effect);
                }
            }
            // The connection thread answers these.
            play::Clientbound::ChunkBatchStart | play::Clientbound::ChunkBatchFinished { .. } => {}
            play::Clientbound::MoveVehicle {
                position,
                y_rot,
                x_rot,
            } => {
                if let Some(w) = &mut self.world
                    && let Some(p) = w.entities.move_vehicle(position, y_rot, x_rot)
                {
                    self.connection.send(p);
                }
            }
            play::Clientbound::SetEquipment { entity_id, slots } => {
                if let Some(w) = &mut self.world {
                    w.entities.set_equipment(entity_id, &slots);
                    if let Some(physics) = &self.physics {
                        w.entities.set_worn(entity_id, &slots, &physics.items);
                    }
                }
            }
            play::Clientbound::SetSimulationDistance { distance } => {
                self.simulation_distance = distance;
                if let Some(w) = &mut self.world {
                    w.entities.simulation_distance = distance;
                }
            }
            play::Clientbound::RotateHead {
                entity_id,
                y_head_rot,
            } => {
                if let Some(w) = &mut self.world {
                    w.entities.rotate_head(entity_id, y_head_rot);
                }
            }
            play::Clientbound::DamageEvent { entity_id } => {
                if let Some(w) = &mut self.world {
                    if entity_id == w.player.entity_id {
                        w.player.first_person.hurt(None);
                    } else {
                        w.entities.damaged(entity_id);
                    }
                }
            }
            play::Clientbound::HurtAnimation { entity_id, yaw } => {
                if let Some(w) = &mut self.world {
                    if entity_id == w.player.entity_id {
                        w.player.first_person.hurt(Some(yaw));
                    } else {
                        w.entities.hurt(entity_id);
                    }
                }
            }
            // `handleBlockDestruction`.
            play::Clientbound::BlockDestruction { id, pos, progress } => {
                if let Some(w) = &mut self.world {
                    w.destruction.set(id, pos, i32::from(progress), w.game_time);
                }
            }
            play::Clientbound::SwingAnimation {
                entity_id,
                off_hand,
                animation,
                duration,
            } => {
                if let Some(w) = &mut self.world {
                    if entity_id == w.player.entity_id {
                        let hand = if off_hand { Hand::Off } else { Hand::Main };
                        w.player.swing(hand, (animation, duration));
                    } else {
                        let swing = Swing {
                            off_hand,
                            kind: animation,
                            duration,
                        };
                        w.entities.swing(entity_id, swing);
                    }
                }
            }
            // `handleSetTime`: the game time, then the clocks tick up to it and take
            // the server's values.
            play::Clientbound::SetTime { game_time, clocks } => {
                if let Some(w) = &mut self.world {
                    w.game_time = game_time;
                    w.clocks.handle_updates(game_time, &clocks);
                }
            }
            // The world border's packets.
            p @ (play::Clientbound::InitializeBorder { .. }
            | play::Clientbound::SetBorderCenter { .. }
            | play::Clientbound::SetBorderLerpSize { .. }
            | play::Clientbound::SetBorderSize { .. }
            | play::Clientbound::SetBorderWarningDelay { .. }
            | play::Clientbound::SetBorderWarningDistance { .. }) => {
                if let Some(w) = &mut self.world {
                    w.level.border.handle(&p);
                }
            }
            play::Clientbound::Other(kind) => {
                self.play_packets += 1;
                if self.play_packets <= 3 {
                    eprintln!("[session] play: {kind} (not handled yet)");
                }
            }
        }
        Ok(())
    }
}

/// What the crosshair is on `partial` of the way into the tick
/// (`raycastHitResult(partialTicks)`): the eyes and the view where they are drawn.
fn frame_hit(
    env: &Env<'_>,
    entities: &Entities,
    players: &PlayerInfos,
    player: &LocalPlayer,
    partial: f32,
) -> pick::HitResult {
    let eye_height = (player.eye_position()[1] - player.position[1]) as f32;
    let (old_y_rot, old_x_rot) = player.old_rot;
    let camera = pick::Camera {
        id: player.entity_id,
        eye: pick::eye_position(partial, player.old_position, player.position, eye_height),
        view: pick::view_vector(
            pick::lerp_x_rot(partial, old_x_rot, player.x_rot),
            pick::lerp_y_rot(partial, old_y_rot, player.y_rot),
        ),
        bounding_box: player.bounding_box(),
    };
    let attacker = player
        .active_item()
        .c
        .attack_range
        .map(|range| pick::Attacker {
            range,
            creative: player.creative,
            eye: player.eye_position(),
            head_look: pick::view_vector(player.x_rot, player.head_y_rot),
            movement: player.delta,
        });
    pick::raycast(
        env,
        entities,
        players,
        &camera,
        attacker.as_ref(),
        player.hand(Hand::Main).c.name,
        player.attributes.block_interaction_range.value(),
        player.attributes.entity_interaction_range.value(),
    )
}

/// `LocalPlayer.raycastHitResult`: what the crosshair is on, from the eyes as the
/// tick begins.
pub fn pick_target(
    env: &Env<'_>,
    entities: &Entities,
    players: &PlayerInfos,
    player: &LocalPlayer,
) -> pick::HitResult {
    let camera = pick::Camera {
        id: player.entity_id,
        eye: player.pick_eye_position(),
        view: pick::view_vector(player.x_rot, player.y_rot),
        bounding_box: player.bounding_box(),
    };
    let attacker = player
        .active_item()
        .c
        .attack_range
        .map(|range| pick::Attacker {
            range,
            creative: player.creative,
            eye: player.eye_position(),
            head_look: pick::view_vector(player.x_rot, player.head_y_rot),
            movement: player.delta,
        });
    pick::raycast(
        env,
        entities,
        players,
        &camera,
        attacker.as_ref(),
        player.hand(Hand::Main).c.name,
        player.attributes.block_interaction_range.value(),
        player.attributes.entity_interaction_range.value(),
    )
}

fn log_configured(c: &Configured, brand: Option<&str>) {
    let entries: usize = c.registries.iter().map(|(_, e)| e.len()).sum();
    let with_data: usize = c
        .registries
        .iter()
        .flat_map(|(_, e)| e)
        .filter(|e| e.data.is_some())
        .count();
    eprintln!(
        "[session] configured by {}: {} registries ({entries} entries, {with_data} with data from the server), tags for {} registries, features {:?}",
        brand.unwrap_or("an unnamed server"),
        c.registries.len(),
        c.tags.len(),
        c.features
    );
}

fn log_closed(c: &Closed) {
    let why = match &c.reason {
        Reason::Json(j) => j.clone(),
        Reason::Text(t) => format!("{t:?}"),
        Reason::Local(l) => l.to_string(),
    };
    eprintln!("[session] closed: {why}");
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::{SectionKey, WHOLE_COLUMN, dirty};
    use crate::world::testing::{chunk_data, light};
    use ruthenium_net::connection::{Remote, Sent};
    use ruthenium_protocol::nbt::{Compound, Tag};
    use ruthenium_protocol::packets::play::{ChunkBlockEntity, ClockState, LightData};

    fn dimension_type(min_y: i32, height: i32, skylight: bool, nether: bool) -> Tag {
        let mut attributes = Compound::new();
        attributes.put(
            "minecraft:visual/sky_color",
            Tag::String(if nether { "#330808" } else { "#78a7ff" }.into()),
        );
        let mut d = Compound::new();
        d.put("min_y", Tag::Int(min_y));
        d.put("height", Tag::Int(height));
        d.put("has_skylight", Tag::Byte(skylight.into()));
        d.put("has_ceiling", Tag::Byte(nether.into()));
        d.put("ambient_light", Tag::Float(if nether { 0.1 } else { 0.0 }));
        if nether {
            d.put("cardinal_light", Tag::String("nether".into()));
        } else {
            d.put("default_clock", Tag::String("minecraft:overworld".into()));
        }
        d.put("attributes", Tag::Compound(attributes));
        Tag::Compound(d)
    }

    fn entry(id: &str, data: Option<Tag>) -> RegistryEntry {
        RegistryEntry {
            id: id.into(),
            data,
        }
    }

    fn spawn(dimension_type: i32, dimension: &str) -> play::SpawnInfo {
        play::SpawnInfo {
            dimension_type,
            dimension: dimension.into(),
            seed: 0,
            game_type: play::CREATIVE,
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
            spawn: spawn(0, "minecraft:overworld"),
            online_mode: false,
            enforces_secure_chat: false,
        }))
    }

    /// A chunk of the 4-section test level: stone in its bottom section, sky light 15
    /// above it, a sign's data at (x 2, y 5, z 3) of the chunk.
    fn chunk(x: i32, z: i32) -> play::Clientbound {
        let mut sign = Compound::new();
        sign.put("text", Tag::String("hello".into()));
        play::Clientbound::LevelChunk {
            x,
            z,
            heightmaps: Vec::new(),
            data: chunk_data(4, |i| if i == 0 { 1 } else { 0 }),
            block_entities: vec![ChunkBlockEntity {
                xz: 0x23,
                y: 5,
                kind: 7,
                tag: Some(Tag::Compound(sign)),
            }],
            light: light(
                &[(1, None), (2, Some(15)), (3, Some(15)), (4, Some(15))],
                &[(2, Some(3))],
            ),
        }
    }

    /// A game thread's state driven by hand: packets in, frames one at a time on a
    /// clock the test sets.
    struct Rig {
        game: Game,
        remote: Remote,
        scene: Scene,
        _events: Receiver<SessionEvent>,
        ms: i64,
    }

    fn rig(physics: Option<Physics>) -> Rig {
        let (connection, remote) = Connection::detached();
        let (publisher, scene) = Scene::new(DEFAULT_DIRTY_CAPACITY);
        let (tx, events) = channel();
        let mut game = Game::new(
            connection,
            Vec::new(),
            physics,
            tx,
            publisher,
            ClientInformation::default(),
        );
        game.manual_ms = Some(0);
        game.configured.registries = vec![
            (
                crate::environment::DIMENSION_TYPE.into(),
                vec![
                    entry(
                        "minecraft:overworld",
                        Some(dimension_type(-16, 64, true, false)),
                    ),
                    entry(
                        "minecraft:the_nether",
                        Some(dimension_type(0, 128, false, true)),
                    ),
                ],
            ),
            (
                crate::environment::WORLD_CLOCK.into(),
                vec![
                    entry("minecraft:overworld", None),
                    entry("minecraft:the_end", None),
                ],
            ),
            (
                crate::environment::BIOME.into(),
                vec![entry("minecraft:plains", None)],
            ),
        ];
        Rig {
            game,
            remote,
            scene,
            _events: events,
            ms: 0,
        }
    }

    impl Rig {
        fn packet(&self, p: play::Clientbound) {
            self.remote.event(Event::Play(p));
        }

        /// One frame of the game thread, `ms` after the last, with `input`.
        fn frame_with(&mut self, ms: i64, input: FrameInput) -> Arc<Frame> {
            self.ms += ms;
            self.game.manual_ms = Some(self.ms);
            self.game.input_sequence += 1;
            assert!(self.game.frame(input));
            self.scene.frame().expect("a frame is published in a level")
        }

        fn frame(&mut self, ms: i64) -> Arc<Frame> {
            self.frame_with(ms, FrameInput::default())
        }

        fn dirty(&self) -> HashMap<SectionKey, u8> {
            let mut out = Vec::new();
            self.scene.drain_dirty(&mut out);
            out.into_iter().collect()
        }
    }

    #[test]
    fn world_updates_reach_the_scene_in_the_frame_that_handled_them() {
        let mut r = rig(None);
        // No level: nothing is published.
        r.game.manual_ms = Some(0);
        assert!(r.game.frame(FrameInput::default()));
        assert!(r.scene.frame().is_none());

        r.packet(login());
        let f = r.frame(10);
        assert_eq!((f.epoch, f.sequence, f.input_sequence), (1, 1, 1));
        assert_eq!(&*f.dimension, "minecraft:overworld");
        assert_eq!(f.environment.dimension_type, "minecraft:overworld");
        assert_eq!(
            f.environment.attributes["minecraft:visual/sky_color"],
            "#78a7ff"
        );
        assert_eq!(f.environment.default_clock, Some(0));
        assert_eq!(f.player.entity_id, 7);
        assert!(
            r.scene
                .with_terrain(|t| t.is_empty() && t.dims().sections == 4)
        );
        r.dirty();

        // Chunks arrive: blocks, light, block entity data, all in the same frame.
        for (x, z) in [(0, 0), (1, 0)] {
            r.packet(chunk(x, z));
        }
        let f = r.frame(10);
        assert_eq!(f.terrain_stamp, r.scene.with_terrain(|t| t.stamp()));
        let n = r.scene.pin([0, 0, 0]).unwrap();
        assert_eq!((n.block(3, -1, 3), n.block(3, 0, 3)), (1, 0));
        assert_eq!((n.sky_light(3, 0, 3), n.sky_light(3, -1, 3)), (15, 0));
        assert_eq!((n.block_light(3, 0, 3), n.block_light(19, 0, 3)), (3, 3));
        let records = n.column(0, 0).unwrap().block_entities();
        assert_eq!(
            (records.len(), records[0].pos, records[0].kind),
            (1, [2, 5, 3], 7)
        );
        let keys = r.dirty();
        assert_eq!(
            keys[&[0, WHOLE_COLUMN, 0]],
            dirty::LOADED | dirty::NEIGHBOUR
        );
        assert_eq!(keys[&[-1, WHOLE_COLUMN, 0]], dirty::NEIGHBOUR);

        // The server's light after a change, a block update, and a section's updates.
        r.packet(play::Clientbound::LightUpdate {
            x: 0,
            z: 0,
            light: light(&[(2, Some(9))], &[]),
        });
        r.packet(play::Clientbound::BlockUpdate {
            pos: [8, 8, 8],
            state: 1,
        });
        r.packet(play::Clientbound::SectionBlocksUpdate {
            section: [1, 1, 0],
            changes: vec![(8 << 8 | 8 << 4 | 2, 1), (9 << 8 | 8 << 4 | 2, 1)],
        });
        r.frame(10);
        let after = r.scene.pin([0, 0, 0]).unwrap();
        assert_eq!((after.sky_light(3, 0, 3), after.block(8, 8, 8)), (9, 1));
        // The pin from the frame before still has what it had.
        assert_eq!((n.sky_light(3, 0, 3), n.block(8, 8, 8)), (15, 0));
        assert!(!r.scene.is_current(n.stamp()));
        // x 8, y 2 (18 in the level), z 8 of section (1, 1, 0).
        assert_eq!(r.scene.pin([1, 1, 0]).unwrap().block(8, 2, 8), 1);
        assert_eq!(r.scene.pin([1, 1, 0]).unwrap().block(9, 2, 8), 1);
        let keys = r.dirty();
        // The server's changes are not the player's own.
        assert_eq!(keys[&[0, 0, 0]], dirty::BLOCKS | dirty::LIGHT);
        // A section's first blocks: it and the sections around it hold light now.
        assert_eq!(
            keys[&[1, 1, 0]],
            dirty::BLOCKS | dirty::LIGHT | dirty::NEIGHBOUR
        );
        assert_eq!(keys[&[0, 1, 0]], dirty::LIGHT | dirty::NEIGHBOUR);

        // A block entity's new data.
        let mut sign = Compound::new();
        sign.put("text", Tag::String("bye".into()));
        r.packet(play::Clientbound::BlockEntityData {
            pos: [2, 5, 3],
            kind: 7,
            tag: Some(Tag::Compound(sign.clone())),
        });
        r.frame(10);
        let c = r.scene.with_terrain(|t| t.column(0, 0).cloned()).unwrap();
        assert_eq!(
            c.block_entities()[0].tag.as_deref(),
            Some(&Tag::Compound(sign))
        );
        assert_eq!(r.dirty()[&[0, 0, 0]], dirty::BLOCK_ENTITIES);

        // The time, with the day's clock; the ticks then count both on.
        r.packet(play::Clientbound::SetTime {
            game_time: 5000,
            clocks: vec![(
                0,
                ClockState {
                    total_ticks: 6000,
                    partial_tick: 0.0,
                    rate: 1.0,
                },
            )],
        });
        // No time passes in this frame: no tick runs in it.
        let f = r.frame(0);
        assert_eq!((f.game_time, f.clocks[0].total_ticks), (5000, 6000));
        let ticks = f.ticks;
        // 150 ms later three ticks have run.
        let f = r.frame(150);
        assert_eq!(f.ticks, ticks + 3);
        assert_eq!((f.game_time, f.clocks[0].total_ticks), (5003, 6003));
        // Time stopped (the clock's rate 0): the game time goes on, the clock stays.
        r.packet(play::Clientbound::SetTime {
            game_time: 5003,
            clocks: vec![(
                0,
                ClockState {
                    total_ticks: 6003,
                    partial_tick: 0.0,
                    rate: 0.0,
                },
            )],
        });
        let f = r.frame(100);
        assert_eq!((f.game_time, f.clocks[0].total_ticks), (5005, 6003));

        // A teleport is a placement: nothing interpolates across it.
        let placements = f.player.placements;
        r.packet(play::Clientbound::PlayerPosition(play::PlayerPosition {
            id: 1,
            position: [8.5, 0.0, 8.5],
            delta: [0.0; 3],
            y_rot: 90.0,
            x_rot: 10.0,
            relatives: 0,
        }));
        let f = r.frame(10);
        assert_eq!(f.player.placements, placements + 1);
        assert_eq!((f.player.position, f.player.y_rot), ([8.5, 0.0, 8.5], 90.0));

        // A chunk leaves: it is gone from the terrain in that frame.
        r.packet(play::Clientbound::ForgetLevelChunk { x: 1, z: 0 });
        r.frame(10);
        assert!(
            r.scene
                .with_terrain(|t| t.column(1, 0).is_none() && t.len() == 1)
        );
        assert!(r.scene.pin([1, 0, 0]).is_none());
        let keys = r.dirty();
        assert_eq!(keys[&[1, WHOLE_COLUMN, 0]], dirty::UNLOADED);
        assert_eq!(keys[&[0, WHOLE_COLUMN, 0]], dirty::NEIGHBOUR);
        // Light for the chunk that left goes nowhere.
        r.packet(play::Clientbound::LightUpdate {
            x: 1,
            z: 0,
            light: LightData::default(),
        });
        r.frame(10);
        assert!(r.dirty().is_empty());

        // Another dimension: a new epoch, its own environment, nothing of the old
        // level left; the clocks stay, as vanilla's listener keeps them.
        let old = r.scene.pin([0, 0, 0]).unwrap();
        r.packet(play::Clientbound::Respawn {
            spawn: Box::new(spawn(1, "minecraft:the_nether")),
            data_to_keep: 0,
        });
        let f = r.frame(10);
        assert_eq!(f.epoch, 2);
        assert_eq!(&*f.dimension, "minecraft:the_nether");
        assert!(!f.environment.has_skylight);
        assert_eq!(
            f.environment.cardinal_light,
            crate::environment::CardinalLight::Nether
        );
        assert_eq!(f.clocks[0].total_ticks, 6003);
        assert_eq!(f.ticks, 0);
        assert!(
            r.scene
                .with_terrain(|t| t.is_empty() && t.dims().sections == 8)
        );
        assert!(!r.scene.is_current(old.stamp()));
        let mut out = Vec::new();
        let drained = r.scene.drain_dirty(&mut out);
        assert_eq!(drained.epoch, 2);
        assert!(drained.rescan.is_some() && out.is_empty());
        // A respawn in the same dimension keeps the level and its epoch.
        r.packet(play::Clientbound::Respawn {
            spawn: Box::new(spawn(1, "minecraft:the_nether")),
            data_to_keep: 0,
        });
        assert_eq!(r.frame(10).epoch, 2);

        // Back to configuration: the level is gone.
        r.packet(play::Clientbound::StartConfiguration);
        r.game.manual_ms = Some(r.ms + 10);
        assert!(r.game.frame(FrameInput::default()));
        assert!(r.scene.frame().is_none());
        assert!(r.scene.with_terrain(|t| t.is_empty() && t.epoch() == 3));
    }

    /// Frame `i` of a scripted run: turns every frame, with scrolls, hotbar keys, use
    /// and attack clicks and releases, and movement keys spread through it.
    fn scripted(i: u32) -> FrameInput {
        let mut input = FrameInput {
            keys: Keys {
                forward: i % 40 < 25,
                jump: i % 17 == 3,
                left: i.is_multiple_of(9),
                ..Keys::default()
            },
            use_down: (20..26).contains(&(i % 50)),
            attack_down: (30..45).contains(&(i % 50)),
            ..FrameInput::default()
        };
        input.use_clicks = u32::from(i % 50 == 20);
        input.attack_clicks = u32::from(i % 50 == 30) + u32::from(i % 50 == 31);
        input.swap_clicks = u32::from(i % 37 == 5);
        if i % 13 == 4 {
            input.hotbar_clicks[(i % 9) as usize] = 1;
        }
        if i % 11 == 6 {
            input.scrolls = vec![1, -1, -1];
        }
        input.turns = (0..i % 3)
            .map(|k| {
                (
                    f64::from(i % 7) - 3.0 + f64::from(k),
                    2.0 - f64::from(i % 5),
                )
            })
            .collect();
        input
    }

    /// Where the game thread's clock stands when it runs frame `n` (from 1): uneven,
    /// with a stall long enough for ten ticks to run at once.
    fn clock_at(n: u64) -> i64 {
        let stall = if n >= 60 { 700 } else { 0 };
        (n * 16 + (n % 5) * 3) as i64 + stall
    }

    const SCRIPT_FRAMES: u32 = 150;
    /// The frame after which the death screen's Respawn button is pressed.
    const RESPAWN_AFTER: u32 = 90;

    fn start(physics: Option<Physics>) -> Rig {
        let mut r = rig(physics);
        r.packet(login());
        for x in -1..=1 {
            for z in -1..=1 {
                r.packet(chunk(x, z));
            }
        }
        r.packet(play::Clientbound::PlayerPosition(play::PlayerPosition {
            id: 1,
            position: [8.5, 0.0, 8.5],
            delta: [0.0; 3],
            y_rot: 0.0,
            x_rot: 0.0,
            relatives: 0,
        }));
        // The server says the chunks are on their way: the player is let in at the
        // first tick that finds its chunk there.
        r.packet(play::Clientbound::GameEvent {
            event: GAME_EVENT_LEVEL_CHUNKS_LOAD_START,
            param: 0.0,
        });
        r.game.manual_ms = Some(0);
        assert!(r.game.frame(FrameInput::default()));
        r.remote.sent();
        r
    }

    /// What a run left behind: every packet sent, and the player.
    fn outcome(r: &mut Rig) -> (Vec<Sent>, [f64; 3], (f32, f32), i32, u64) {
        let f = r.scene.frame().unwrap();
        (
            r.remote.sent(),
            f.player.position,
            (f.player.y_rot, f.player.x_rot),
            f.player.selected_slot,
            f.ticks,
        )
    }

    /// The prepared game's data, when the tests are told where it is.
    fn physics() -> Option<Physics> {
        let game = std::path::PathBuf::from(std::env::var_os("RUTHENIUM_GAME_DIR")?);
        // Told where the game is, its exports have to be there: a silent skip would
        // pass for a run.
        let exports = game.join("versions/26.3/exports");
        let blocks = Blocks::load(&exports).expect("the prepared game's block exports");
        let entity_types = EntityTypes::load(&exports).expect("its entity types");
        let items = Items::load(&exports, &game.join("versions/26.3/generated/reports"))
            .expect("its items");
        Some(Physics {
            blocks: Arc::new(blocks),
            entity_types: Arc::new(entity_types),
            dimension_types: HashMap::new(),
            items: Arc::new(items),
            block_transformers: Arc::default(),
            enchantments: Arc::default(),
            biomes: HashMap::new(),
            registries: Arc::default(),
        })
    }

    /// The same scripted frames reach the game thread directly, one by one, and
    /// through a handoff of three frames with the main thread held back by it: the
    /// game thread must see the same input at the same frame boundaries either way.
    fn backlog_changes_nothing(physics: Option<Physics>) {
        // Directly: no queue at all.
        let mut direct = start(physics.clone());
        for i in 0..SCRIPT_FRAMES {
            direct.game.manual_ms = Some(clock_at(u64::from(i) + 1));
            direct.game.input_sequence += 1;
            assert!(direct.game.frame(scripted(i)));
            if i == RESPAWN_AFTER {
                direct.game.control(Command::Respawn);
            }
        }
        let expected = outcome(&mut direct);
        assert_eq!(expected.4, direct.scene.frame().unwrap().ticks);
        // Ticks ran, some of them ten at once after the stall.
        assert!(expected.4 > 50, "the script ran {} ticks", expected.4);

        // Through the handoff, the main thread faster than the game thread.
        let mut queued = start(physics);
        let handoff: Arc<Handoff<FrameInput, Command>> = Arc::new(Handoff::new(Limits {
            frames: 3,
            events: 8,
        }));
        let main = {
            let handoff = handoff.clone();
            std::thread::spawn(move || {
                for i in 0..SCRIPT_FRAMES {
                    let input = scripted(i);
                    let events = input.scrolls.len() + input.turns.len();
                    handoff.push_frame(input, events);
                    if i == RESPAWN_AFTER {
                        handoff.push_control(Command::Respawn);
                    }
                }
            })
        };
        let mut frames = 0_u64;
        let mut last_sequence = 0;
        while frames < u64::from(SCRIPT_FRAMES) {
            let take = handoff.take();
            if let Take::Item(t) = &take {
                assert!(t.sequence > last_sequence, "out of order");
                last_sequence = t.sequence;
                if matches!(t.item, Item::Frame(_)) {
                    frames += 1;
                    queued.game.manual_ms = Some(clock_at(frames));
                }
            }
            assert!(queued.game.step(take));
            if frames.is_multiple_of(20) {
                // The game thread falls behind: frames pile up to the limit.
                std::thread::sleep(Duration::from_millis(3));
            }
        }
        main.join().unwrap();
        let stats = handoff.stats();
        assert!(stats.stalls > 0 && stats.high_water == 3, "{stats:?}");
        // The published frame knows the last input it has: the frames and the one
        // control took a number each.
        let f = queued.scene.frame().unwrap();
        assert_eq!(f.input_sequence, u64::from(SCRIPT_FRAMES) + 1);
        let got = outcome(&mut queued);
        assert_eq!(got.0.len(), expected.0.len());
        for (i, (a, b)) in got.0.iter().zip(&expected.0).enumerate() {
            assert_eq!(a, b, "packet {i} differs");
        }
        assert_eq!(got, expected);
    }

    #[test]
    fn a_backlog_changes_no_input_boundary() {
        backlog_changes_nothing(None);
    }

    /// With the prepared game (`RUTHENIUM_GAME_DIR`), the same with a player that
    /// moves, breaks and uses blocks; skipped without it.
    #[test]
    fn a_backlog_changes_no_input_boundary_with_physics() {
        let Some(physics) = physics() else {
            eprintln!("RUTHENIUM_GAME_DIR not set; skipped");
            return;
        };
        backlog_changes_nothing(Some(physics));
        // The player did move: the run is not trivially equal.
        let mut r = start(self::physics());
        for i in 0..SCRIPT_FRAMES {
            r.game.manual_ms = Some(clock_at(u64::from(i) + 1));
            assert!(r.game.frame(scripted(i)));
        }
        let (sent, position, ..) = outcome(&mut r);
        assert_ne!(position, [8.5, 0.0, 8.5]);
        assert!(sent.len() > 100, "{} packets", sent.len());
    }

    #[test]
    fn what_the_hud_shows_reaches_the_frame() {
        let mut r = rig(None);
        r.packet(login());
        let f = r.frame(10);
        assert_eq!((f.hud.experience_level, f.hud.damage_cooldown), (0, 0));
        assert_eq!(
            f.hud.survival,
            matches!(f.player.game_type, play::SURVIVAL | play::ADVENTURE)
        );
        let ticks = f.hud.ticks;

        r.packet(play::Clientbound::SetExperience {
            progress: 0.25,
            level: 7,
            total: 100,
        });
        r.packet(play::Clientbound::SetHealth {
            health: 20.0,
            food: 18,
            saturation: 1.5,
        });
        let f = r.frame(10);
        assert_eq!(
            (f.hud.experience_progress, f.hud.experience_level),
            (0.25, 7)
        );
        assert_eq!(f.hud.experience_needed(), 21);
        assert_eq!(
            (f.hud.health, f.hud.food, f.hud.saturation),
            (20.0, 18, 1.5)
        );
        // The first health a server sends is no change to blink at.
        assert_eq!(f.hud.damage_cooldown, 0);

        // Less health holds off the next hurt for 20 ticks, more for 10; the same, not
        // at all.
        let health = |health| play::Clientbound::SetHealth {
            health,
            food: 18,
            saturation: 1.5,
        };
        r.packet(health(14.0));
        let f = r.frame(1);
        assert_eq!((f.hud.health, f.hud.damage_cooldown), (14.0, 20));
        r.packet(health(16.0));
        let f = r.frame(1);
        assert_eq!((f.hud.health, f.hud.damage_cooldown), (16.0, 10));
        r.packet(health(16.0));
        assert_eq!(r.frame(1).hud.damage_cooldown, 10);

        // The HUD's own ticks go on with the game's.
        let f = r.frame(100);
        assert_eq!(f.hud.ticks, ticks + i32::try_from(f.ticks).unwrap());
        assert!(f.hud.ticks > ticks);
    }

    #[test]
    fn the_camera_runs_ahead_of_the_frames_by_the_turns_not_yet_taken() {
        let (connection, remote) = Connection::detached();
        let session =
            Session::with_connection(connection, Vec::new(), None, ClientInformation::default())
                .unwrap();
        assert!(session.camera().is_none());
        remote.event(Event::Play(login()));
        let wait = |until: &dyn Fn() -> bool| {
            for _ in 0..2000 {
                if until() {
                    return;
                }
                std::thread::sleep(Duration::from_millis(1));
            }
            panic!("the game thread did not get there");
        };
        let first = session.frame();
        wait(&|| {
            session
                .scene()
                .frame()
                .is_some_and(|f| f.input_sequence >= first)
        });
        let before = session.camera().unwrap();
        assert_eq!(before.turns_ahead, 0);
        let (y0, x0) = (before.frame.player.y_rot, before.frame.player.x_rot);
        assert_eq!((before.y_rot, before.x_rot), (y0, x0));

        // The mouse moves: the camera has the turn at once, before any frame has it.
        session.turn(100.0, -50.0);
        session.turn(20.0, 0.0);
        let ahead = session.camera().unwrap();
        assert_eq!(ahead.turns_ahead, 2);
        let (y, x) = crate::player::turned(y0, x0, 100.0, -50.0);
        assert_eq!(
            (ahead.y_rot, ahead.x_rot),
            crate::player::turned(y, x, 20.0, 0.0)
        );
        assert_ne!((ahead.y_rot, ahead.x_rot), (y0, x0));
        assert_eq!(ahead.frame.player.y_rot, y0);
        // Handed over, and until the game thread publishes a frame with it, the camera
        // is where it was; afterwards the frame itself has the same rotation.
        let sequence = session.frame();
        assert!(sequence > first);
        let between = session.camera().unwrap();
        assert_eq!((between.y_rot, between.x_rot), (ahead.y_rot, ahead.x_rot));
        wait(&|| {
            session
                .scene()
                .frame()
                .is_some_and(|f| f.input_sequence >= sequence)
        });
        let after = session.camera().unwrap();
        assert_eq!(after.turns_ahead, 0);
        // The turns a published frame has are let go at the next frame's handover.
        assert_eq!(lock(&session.turns).len(), 2);
        let third = session.frame();
        assert!(lock(&session.turns).is_empty());
        wait(&|| {
            session
                .scene()
                .frame()
                .is_some_and(|f| f.input_sequence >= third)
        });
        assert_eq!((after.y_rot, after.x_rot), (ahead.y_rot, ahead.x_rot));
        assert_eq!(after.frame.player.y_rot, ahead.y_rot);
        assert!(after.frame.input_at.is_some());
        assert!(after.frame.published >= after.frame.input_at.unwrap());
        // Looking straight up stops there.
        session.turn(0.0, -550.0);
        session.turn(0.0, -100.0);
        assert_eq!(session.camera().unwrap().x_rot, -90.0);

        // Closing reaches the game thread; dropping the session ends it.
        let mut remote = remote;
        session.disconnect();
        wait(&|| true);
        let stats = session.handoff_stats();
        assert_eq!((stats.frames, stats.stalls), (3, 0));
        drop(session);
        let mut closed = false;
        for _ in 0..2000 {
            closed |= remote.sent().contains(&Sent::Close);
            if closed {
                break;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(closed);
    }
}

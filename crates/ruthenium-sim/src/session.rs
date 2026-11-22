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

use std::collections::HashMap;
use std::sync::mpsc::{Receiver, Sender, TryRecvError, channel};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::JoinHandle;
use std::time::Instant;

use ruthenium_net::connection::{
    Closed, ConnectOptions, Connection, Event, Local, Protocol, Reason, Stage,
};
use ruthenium_protocol::packets::configuration::{self as cfg, RegistryEntry, RegistryTags};
use ruthenium_protocol::packets::{Common, KnownPack, PackAction, play, read_brand};

use crate::blocks::{BlockTags, Blocks};
use crate::clock::{self, TickClock};
use crate::effect::Effect;
use crate::entity::{Entities, EntityTypes, Local as LocalInfo, PlayerInfos, Teams, TickContext};
use crate::input::next_scroll_selection;
use crate::interact::{self, GameMode, UseKeys};
use crate::inventory::Hand;
use crate::item::Items;
use crate::level_load::LevelLoad;
use crate::pick;
use crate::player::{Env, Keys, LocalPlayer, Pose};
use crate::world::{Level, global_bits};

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
}

/// What movement needs from a dimension type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DimensionType {
    pub min_y: i32,
    pub height: i32,
    /// The `minecraft:gameplay/fast_lava` environment attribute.
    pub fast_lava: bool,
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
    /// The last event.
    Closed(Closed),
}

enum Command {
    CodeOfConduct(bool),
    Disconnect,
    /// One frame of the main thread, with its input.
    Frame(FrameInput),
}

/// What the player did during one frame on the main thread.
#[derive(Debug, Default)]
struct FrameInput {
    /// The movement keys held as the frame's events left them (none while a screen is
    /// open).
    keys: Keys,
    /// Hotbar key presses, by slot (`KeyMapping.click`).
    hotbar_clicks: [u32; 9],
    /// The use key held, and presses of the use, swap-hands and attack keys.
    use_down: bool,
    use_clicks: u32,
    swap_clicks: u32,
    attack_clicks: u32,
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
    pub chunks: usize,
    /// Other entities the client tracks.
    pub entities: usize,
    /// Whether the player has loaded into the level.
    pub loaded: bool,
}

/// A session on the game thread. Dropping it disconnects.
pub struct Session {
    commands: Sender<Command>,
    events: Receiver<SessionEvent>,
    /// The input of the frame under way, sent with [`Session::frame`].
    input: Mutex<FrameInput>,
    view: Arc<Mutex<Option<PlayerView>>>,
    thread: Option<JoinHandle<()>>,
}

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
        let connection = Connection::open(options.connect)?;
        let (commands, command_rx) = channel();
        let (tx, events) = channel();
        let known_packs = options.known_packs;
        let physics = options.physics;
        if physics.is_none() {
            eprintln!("[session] no prepared game: the player won't move or send movement");
        }
        let view = Arc::new(Mutex::new(None));
        let game_view = view.clone();
        let thread = std::thread::Builder::new()
            .name("game".into())
            .spawn(move || {
                let mut game = Game::new(connection, known_packs, physics, tx);
                game.view = game_view;
                game.run(&command_rx);
            })?;
        Ok(Session {
            commands,
            events,
            input: Mutex::default(),
            view,
            thread: Some(thread),
        })
    }

    /// The events since the last call, oldest first.
    pub fn poll(&self) -> impl Iterator<Item = SessionEvent> + '_ {
        self.events.try_iter()
    }

    pub fn answer_code_of_conduct(&self, accept: bool) {
        let _ = self.commands.send(Command::CodeOfConduct(accept));
    }

    /// Disconnects; a [`SessionEvent::Closed`] follows.
    pub fn disconnect(&self) {
        let _ = self.commands.send(Command::Disconnect);
    }

    /// Ends the main thread's frame: the game thread runs it (`Minecraft.runTick`) with
    /// the input given since the last one. Call it once every frame the client runs,
    /// also while the window is hidden or no screen of the session shows; packets wait
    /// for it.
    pub fn frame(&self) {
        let mut input = lock(&self.input);
        let (keys, use_down) = (input.keys, input.use_down);
        let frame = std::mem::take(&mut *input);
        input.keys = keys;
        input.use_down = use_down;
        drop(input);
        let _ = self.commands.send(Command::Frame(frame));
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

    /// A press of the attack key (attacks aren't ported; the press is read and dropped,
    /// as the use key's handling expects).
    pub fn attack_key(&self) {
        lock(&self.input).attack_clicks += 1;
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
        self.disconnect();
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
    clock: TickClock,
    started: Instant,
    view: Arc<Mutex<Option<PlayerView>>>,
    /// The keys the next ticks read.
    keys: Keys,
    /// Hotbar key presses not yet read (`KeyMapping.clickCount`).
    hotbar_clicks: [u32; 9],
    /// The use-related keys not yet read.
    use_keys: UseKeys,
    /// The profile name the server knows the player by (its scoreboard name), and its
    /// UUID (its player list entry).
    name: String,
    uuid: u128,
    /// The player list and the scoreboard's teams (the play listener's, gone with it
    /// on a reconfiguration).
    players: PlayerInfos,
    teams: Teams,
}

/// The feature flag that switches minecarts to their new movement, which doesn't
/// interpolate.
const MINECART_IMPROVEMENTS: &str = "minecraft:minecart_improvements";

/// `ClientboundGameEventPacket` events the session acts on.
const GAME_EVENT_CHANGE_GAME_MODE: u8 = 3;
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
    /// delay, the prediction sequence. The selected slot is the player's inventory's.
    game_mode: GameMode,
    /// The fluid shapes item use's raycasts have met (`FlowingFluid`'s cache).
    fluid_shapes: pick::FluidShapes,
    tick_rate: f32,
    frozen: bool,
}

impl World {
    fn new(
        spawn: &play::SpawnInfo,
        now: i64,
        level: Level,
        biome_bits: u32,
        entities: Entities,
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
            fluid_shapes: pick::FluidShapes::new(),
            tick_rate: 20.0,
            frozen: false,
        }
    }
}

impl Game {
    fn new(
        connection: Connection,
        known_packs: Vec<KnownPack>,
        physics: Option<Physics>,
        events: Sender<SessionEvent>,
    ) -> Game {
        Game {
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
            clock: TickClock::new(0),
            started: Instant::now(),
            view: Arc::default(),
            keys: Keys::default(),
            hotbar_clicks: [0; 9],
            use_keys: UseKeys::default(),
            name: String::new(),
            uuid: 0,
            players: PlayerInfos::default(),
            teams: Teams::default(),
        }
    }

    fn report(&self, e: SessionEvent) {
        let _ = self.events.send(e);
    }

    fn run(&mut self, commands: &Receiver<Command>) {
        loop {
            let Ok(c) = commands.recv() else {
                // The session was dropped (after asking to disconnect): no more frames.
                self.connection.close();
                return;
            };
            match c {
                Command::Frame(input) => {
                    if !self.frame(input) {
                        return;
                    }
                }
                Command::Disconnect => self.connection.close(),
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
        // `handleAccumulatedMovement`, without a player nothing turns.
        if let Some(w) = &mut self.world {
            for &(xo, yo) in &input.turns {
                w.player.turn(xo, yo);
            }
        }
        self.update_view();
        true
    }

    /// The frame's events (`runAllTasks`, after the packets): keys and scrolls apply
    /// now, hotbar presses wait for the ticks.
    fn take_input(&mut self, input: &FrameInput) {
        self.keys = input.keys;
        self.use_keys.use_down = input.use_down;
        self.use_keys.use_clicks += input.use_clicks;
        self.use_keys.swap_clicks += input.swap_clicks;
        self.use_keys.attack_clicks += input.attack_clicks;
        // Without a player (`minecraft.player == null`) the input goes nowhere.
        let Some(w) = &mut self.world else {
            return;
        };
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
            chunks: w.level.chunk_count(),
            entities: w.entities.len(),
            loaded: w.loaded,
        });
        *lock(&self.view) = view;
    }

    /// Vanilla's millisecond clock (`Util.getMillis`).
    fn millis(&self) -> i64 {
        self.started.elapsed().as_millis() as i64
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
            }
        });
        let biomes = registry("minecraft:worldgen/biome").len();
        let mut level = Level::new(t.min_y, t.height);
        level.set_fast_lava(t.fast_lava);
        (level, global_bits(biomes))
    }

    /// A level's entities, the player's among them.
    fn new_entities(&self, player_id: i32) -> Entities {
        let improved = self
            .configured
            .features
            .iter()
            .any(|f| f == MINECART_IMPROVEMENTS);
        Entities::new(player_id, improved)
    }

    /// The block tags movement reads, from the server's tags (block registry ids).
    fn apply_tags(&mut self, tags: &[RegistryTags]) {
        let Some(blocks) = tags.iter().find(|t| t.registry == "minecraft:block") else {
            return;
        };
        for (tag, ids) in &blocks.tags {
            let names = ids.iter().filter_map(|&id| {
                ruthenium_data::v26_3::Block::from_id(u32::try_from(id).ok()?).map(|b| b.name())
            });
            self.tags.set(tag, names);
        }
    }

    /// One client tick (`Minecraft.tick`), for the parts there are so far.
    fn tick(&mut self) {
        let now = self.millis();
        let Some(w) = &mut self.world else {
            return;
        };
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
            load.tick(now, chunk, w.player.spectator);
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
        let Some(physics) = &self.physics else {
            self.use_keys = UseKeys {
                use_down: self.use_keys.use_down,
                ..UseKeys::default()
            };
            self.connection.send(play::client_tick_end());
            return;
        };
        let env = Env {
            level: &w.level,
            blocks: &physics.blocks,
            tags: &self.tags,
            colliders: &[],
        };
        // `pick`, then `handleKeybinds` (not while the level-loading screen shows): one
        // press per hotbar key per tick (spectators' hotbar keys open the spectator menu
        // instead, which isn't there yet), then the hands.
        if w.loaded {
            let hit = pick_target(&env, &w.entities, &self.players, &w.player);
            for (slot, pending) in self.hotbar_clicks.iter_mut().enumerate() {
                if *pending > 0 {
                    *pending -= 1;
                    if !w.player.spectator {
                        w.player.inventory.selected = slot as i32;
                    }
                }
            }
            let world = interact::World {
                env: &env,
                entities: &w.entities,
                fluid_shapes: &w.fluid_shapes,
            };
            w.game_mode
                .handle_keybinds(&mut w.player, &world, &hit, &mut self.use_keys, &mut out);
            for p in out.drain(..) {
                self.connection.send(p);
            }
        }
        // `level.tickEntities`: the entities in order, the player among them, which
        // ticks and sends its changes (`sendPosition`) only once it has loaded.
        let cx = TickContext {
            env: &env,
            teams: &self.teams,
            players: &self.players,
            local: LocalInfo {
                name: &self.name,
                spectator: w.player.spectator,
                health: w.player.health,
            },
            tick_rate: w.tick_rate,
            runs_normally: !w.frozen,
        };
        let (loaded, keys, connection) = (w.loaded, self.keys, &self.connection);
        w.entities.tick(&cx, &mut w.player, |player, colliders| {
            if loaded {
                let env = Env { colliders, ..env };
                player.tick(&env, keys);
                for p in player.send_changes() {
                    connection.send(p);
                }
            }
        });
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
                self.players = PlayerInfos::default();
                self.teams = Teams::default();
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
                let entities = self.new_entities(login.player_id);
                let mut world = World::new(&login.spawn, now, level, biome_bits, entities);
                world.player.entity_id = login.player_id;
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
                let w = self.world.as_mut().ok_or("a respawn before the login")?;
                if spawn.dimension != w.dimension {
                    w.level = level;
                    w.entities = entities;
                    w.biome_bits = biome_bits;
                    w.dimension = spawn.dimension.clone();
                } else {
                    w.entities.local_respawned();
                }
                // `handleRespawn`: a new player, with what the server says to keep, in
                // the new game mode; not loaded until the level is ready.
                w.player = w.player.respawned(data_to_keep);
                w.player.set_game_type(spawn.game_type);
                w.loaded = false;
                w.load = Some(LevelLoad::start(now));
                w.level_started = now;
            }
            play::Clientbound::PlayerPosition(t) => {
                let w = self.world.as_mut().ok_or("a teleport before the login")?;
                w.player.teleport(&t);
                let p = &w.player;
                eprintln!(
                    "[session] teleport {} to {:.3} {:.3} {:.3} (relative bits {:#x})",
                    t.id, p.position[0], p.position[1], p.position[2], t.relatives
                );
                self.connection.send(play::accept_teleportation(
                    t.id, p.position, p.y_rot, p.x_rot,
                ));
            }
            play::Clientbound::GameEvent { event, param } => {
                if let Some(w) = &mut self.world {
                    match event {
                        GAME_EVENT_CHANGE_GAME_MODE => {
                            w.player.set_game_type((param + 0.5).floor() as i32)
                        }
                        GAME_EVENT_LEVEL_CHUNKS_LOAD_START => {
                            if let Some(l) = &mut w.load {
                                l.loading_packets_received();
                            }
                        }
                        _ => {}
                    }
                }
            }
            play::Clientbound::LevelChunk { x, z, data } => {
                if let Some(w) = &mut self.world {
                    if let Err(e) = w.level.load_chunk(x, z, &data, w.biome_bits) {
                        return Err(format!("chunk {x} {z}: {e}"));
                    }
                    w.entities.chunk_loaded(x, z);
                }
            }
            play::Clientbound::ForgetLevelChunk { x, z } => {
                if let Some(w) = &mut self.world {
                    w.level.forget_chunk(x, z);
                    w.entities.chunk_unloaded(x, z);
                }
            }
            play::Clientbound::BlockUpdate { pos, state } => {
                if let Some(w) = &mut self.world {
                    w.level.set_block(pos[0], pos[1], pos[2], state);
                }
            }
            play::Clientbound::SectionBlocksUpdate { section, changes } => {
                if let Some(w) = &mut self.world {
                    for (rel, state) in changes {
                        let rel = i32::from(rel);
                        w.level.set_block(
                            section[0] * 16 + (rel >> 8 & 15),
                            section[1] * 16 + (rel & 15),
                            section[2] * 16 + (rel >> 4 & 15),
                            state,
                        );
                    }
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
                if let Some(w) = &mut self.world {
                    w.entities.teleport(entity_id, &change, on_ground);
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
                    w.entities
                        .position_sync(entity_id, &path, y_rot, x_rot, on_ground);
                }
            }
            play::Clientbound::SetEntityData {
                entity_id, values, ..
            } => {
                if let Some(w) = &mut self.world
                    && let Some(physics) = &self.physics
                {
                    if entity_id == w.player.entity_id {
                        w.player.set_data(&values, &physics.entity_types);
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
                    w.entities.set_passengers(vehicle, &passengers);
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
                }
            }
            play::Clientbound::PlayerInfoRemove { ids } => self.players.remove(&ids),
            play::Clientbound::SetHealth {
                health,
                food,
                saturation,
            } => {
                // `hurtTo` and the food data.
                if let Some(w) = &mut self.world {
                    w.player.health = health;
                    w.player.food = food;
                    w.player.saturation = saturation;
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
                // `Player.handleEntityEvent`: the server finished the item's use.
                if let Some(w) = &mut self.world
                    && entity_id == w.player.entity_id
                    && event == ENTITY_EVENT_USE_ITEM_COMPLETE
                {
                    w.player.complete_using_item();
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
    pick::pick(
        env,
        entities,
        players,
        &camera,
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

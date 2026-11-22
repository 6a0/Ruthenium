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
use crate::input::next_scroll_selection;
use crate::level_load::LevelLoad;
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
    /// Vanilla's dimension types by name: (min_y, height), for servers that send them
    /// as known-pack entries without data.
    pub dimension_types: HashMap<String, (i32, i32)>,
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
    pub selected_slot: i32,
    pub chunks: usize,
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
        let keys = input.keys;
        let frame = std::mem::take(&mut *input);
        input.keys = keys;
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
}

/// `ClientboundGameEventPacket` events the session acts on.
const GAME_EVENT_CHANGE_GAME_MODE: u8 = 3;
const GAME_EVENT_LEVEL_CHUNKS_LOAD_START: u8 = 13;

/// What the game thread knows of the level it is in.
#[derive(Debug)]
struct World {
    dimension: String,
    game_type: i32,
    player: LocalPlayer,
    level: Level,
    /// Bits of the biome palette (from the biome registry's size).
    biome_bits: u32,
    /// Until the player counts as loaded.
    load: Option<LevelLoad>,
    loaded: bool,
    level_started: i64,
    /// The hotbar slot selected, and the last one sent (`MultiPlayerGameMode.carriedIndex`,
    /// 0 for a new game mode, so a server's slot other than 0 is sent back once).
    selected_slot: i32,
    carried_index: i32,
    tick_rate: f32,
    frozen: bool,
}

impl World {
    fn new(spawn: &play::SpawnInfo, now: i64, level: Level, biome_bits: u32) -> World {
        World {
            dimension: spawn.dimension.clone(),
            game_type: spawn.game_type,
            player: LocalPlayer::new(),
            level,
            biome_bits,
            load: Some(LevelLoad::start(now)),
            loaded: false,
            level_started: now,
            selected_slot: 0,
            carried_index: 0,
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
        // Without a player (`minecraft.player == null`) the input goes nowhere.
        let Some(w) = &mut self.world else {
            return;
        };
        if w.game_type != play::SPECTATOR {
            for &wheel in &input.scrolls {
                w.selected_slot = next_scroll_selection(f64::from(wheel), w.selected_slot, 9);
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
            selected_slot: w.selected_slot,
            chunks: w.level.chunk_count(),
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
            Some((c.int("min_y")?, c.int("height")?))
        });
        let vanilla =
            entry.and_then(|e| self.physics.as_ref()?.dimension_types.get(&e.id).copied());
        let (min_y, height) = from_data.or(vanilla).unwrap_or_else(|| {
            eprintln!(
                "[session] dimension type {} unknown; assuming the overworld's height",
                spawn.dimension_type
            );
            (-64, 384)
        });
        let biomes = registry("minecraft:worldgen/biome").len();
        (Level::new(min_y, height), global_bits(biomes))
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
        // `gameMode.tick`: the held slot first, then the connection's tick, where the
        // listener lets the player in once the level is ready.
        if w.selected_slot != w.carried_index {
            w.carried_index = w.selected_slot;
            self.connection
                .send(play::set_carried_item(w.carried_index));
        }
        if let Some(load) = &mut w.load {
            let (cx, cz) = w.player.chunk();
            let chunk = w.level.has_chunk(cx, cz);
            load.tick(now, chunk, w.game_type == play::SPECTATOR);
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
        // `handleKeybinds`: one press per hotbar key per tick (spectators' hotbar keys
        // open the spectator menu instead, which isn't there yet).
        for (slot, pending) in self.hotbar_clicks.iter_mut().enumerate() {
            if *pending > 0 {
                *pending -= 1;
                if w.game_type != play::SPECTATOR {
                    w.selected_slot = slot as i32;
                }
            }
        }
        // `level.tickEntities` ticks the player, then `player.sendChanges`; both only
        // once it has loaded.
        if w.loaded
            && let Some(physics) = &self.physics
        {
            let env = Env {
                level: &w.level,
                blocks: &physics.blocks,
                tags: &self.tags,
            };
            w.player.tick(&env, self.keys);
            for p in w.player.send_changes() {
                self.connection.send(p);
            }
        }
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
                let mut world = World::new(&login.spawn, now, level, biome_bits);
                world.player.entity_id = login.player_id;
                self.world = Some(world);
            }
            play::Clientbound::Respawn { spawn, .. } => {
                let (level, biome_bits) = self.new_level(&spawn);
                let w = self.world.as_mut().ok_or("a respawn before the login")?;
                if spawn.dimension != w.dimension {
                    w.level = level;
                    w.biome_bits = biome_bits;
                    w.dimension = spawn.dimension.clone();
                }
                w.game_type = spawn.game_type;
                // `handleRespawn`: a new player, not loaded until the level is ready.
                let old = std::mem::take(&mut w.player);
                w.player.entity_id = old.entity_id;
                (w.player.y_rot, w.player.x_rot) = (old.y_rot, old.x_rot);
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
                        GAME_EVENT_CHANGE_GAME_MODE => w.game_type = param as i32,
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
                if let Some(w) = &mut self.world
                    && let Err(e) = w.level.load_chunk(x, z, &data, w.biome_bits)
                {
                    return Err(format!("chunk {x} {z}: {e}"));
                }
            }
            play::Clientbound::ForgetLevelChunk { x, z } => {
                if let Some(w) = &mut self.world {
                    w.level.forget_chunk(x, z);
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
                if let Some(w) = &mut self.world
                    && entity_id == w.player.entity_id
                {
                    apply_attributes(&mut w.player, &attributes);
                }
            }
            play::Clientbound::SetEntityMotion {
                entity_id,
                movement,
            } => {
                // `lerpMotion`: the velocity is replaced (knockback).
                if let Some(w) = &mut self.world
                    && entity_id == w.player.entity_id
                {
                    w.player.delta = movement;
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
                    w.selected_slot = slot;
                }
            }
            play::Clientbound::TickingState { tick_rate, frozen } => {
                if let Some(w) = &mut self.world {
                    w.tick_rate = tick_rate;
                    w.frozen = frozen;
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

/// `handleUpdateAttributes` for the local player: each attribute's base, and its
/// modifiers replaced by the server's.
fn apply_attributes(player: &mut LocalPlayer, attributes: &[play::AttributeSnapshot]) {
    for a in attributes {
        let Some(name) = u32::try_from(a.attribute)
            .ok()
            .and_then(ruthenium_data::v26_3::registries::Attribute::from_id)
        else {
            continue;
        };
        if let Some(instance) = player.attributes.get_mut(name.name()) {
            instance.base = a.base;
            instance.modifiers = a
                .modifiers
                .iter()
                .map(|(id, amount, operation)| crate::player::Modifier {
                    id: id.clone(),
                    amount: *amount,
                    operation: *operation,
                })
                .collect();
        }
    }
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

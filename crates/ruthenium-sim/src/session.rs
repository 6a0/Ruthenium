//! One session with a server, on the game thread (`ClientConfigurationPacketListenerImpl`,
//! `ClientCommonPacketListenerImpl` and, for now, the switch parts of
//! `ClientPacketListener`).
//!
//! The connection's thread does the login and answers keep-alives; this thread takes
//! every other packet in order and answers the ones vanilla answers on its main thread:
//! pings (the "transactions" anticheats time), known packs, cookies, resource packs. The
//! menus hear about it through [`SessionEvent`]s and answer the code of conduct.
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

use std::collections::{HashMap, HashSet};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use ruthenium_net::connection::{
    Closed, ConnectOptions, Connection, Event, Local, Protocol, Reason, Stage,
};
use ruthenium_protocol::packets::configuration::{self as cfg, RegistryEntry, RegistryTags};
use ruthenium_protocol::packets::{Common, KnownPack, PackAction, play, read_brand};

use crate::clock::{self, TickClock};
use crate::level_load::LevelLoad;
use crate::player::LocalPlayer;

/// How often the thread looks for commands while no packet arrives.
const COMMAND_POLL: Duration = Duration::from_millis(20);

#[derive(Debug)]
pub struct SessionOptions {
    pub connect: ConnectOptions,
    /// The data packs the client has (the imported game's), offered back when the server
    /// lists them.
    pub known_packs: Vec<KnownPack>,
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
}

/// A session on the game thread. Dropping it disconnects.
pub struct Session {
    commands: Sender<Command>,
    events: Receiver<SessionEvent>,
    thread: Option<JoinHandle<()>>,
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
        let thread = std::thread::Builder::new()
            .name("game".into())
            .spawn(move || {
                let mut game = Game::new(connection, known_packs, tx);
                game.run(&command_rx);
            })?;
        Ok(Session {
            commands,
            events,
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
    clock: TickClock,
    started: Instant,
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
    /// The chunks the server has sent and not taken back.
    chunks: HashSet<(i32, i32)>,
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
    fn new(spawn: &play::SpawnInfo, now: i64) -> World {
        World {
            dimension: spawn.dimension.clone(),
            game_type: spawn.game_type,
            player: LocalPlayer::new(),
            chunks: HashSet::new(),
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
            clock: TickClock::new(0),
            started: Instant::now(),
        }
    }

    fn report(&self, e: SessionEvent) {
        let _ = self.events.send(e);
    }

    fn run(&mut self, commands: &Receiver<Command>) {
        loop {
            while let Ok(c) = commands.try_recv() {
                match c {
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
            // Wait for a packet, or until the next tick falls due.
            let mspt = self.ms_per_tick();
            let now = self.millis();
            let wait = Duration::from_millis(
                (self.clock.next_tick_ms(mspt) - now).clamp(0, COMMAND_POLL.as_millis() as i64)
                    as u64,
            );
            let first = match self.connection.events().recv_timeout(wait) {
                Ok(e) => Some(e),
                Err(RecvTimeoutError::Timeout) => None,
                Err(RecvTimeoutError::Disconnected) => {
                    self.report(SessionEvent::Closed(Closed {
                        during_login: self.phase == Phase::Login,
                        reason: Reason::Local(Local::Io("the connection thread stopped".into())),
                    }));
                    return;
                }
            };
            // Packets first, then the ticks due (\`Minecraft.runTick\`).
            let mut next = first;
            while let Some(e) = next {
                if !self.handle(e) {
                    return;
                }
                next = self.connection.events().try_recv().ok();
            }
            let ticks = self.clock.advance(self.millis(), self.ms_per_tick());
            for _ in 0..ticks {
                self.tick();
            }
        }
    }

    /// Vanilla's millisecond clock (\`Util.getMillis\`).
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

    /// One client tick (\`Minecraft.tick\`), for the parts there are so far.
    fn tick(&mut self) {
        let now = self.millis();
        let Some(w) = &mut self.world else {
            return;
        };
        // \`gameMode.tick\`: the held slot first, then the connection's tick, where the
        // listener lets the player in once the level is ready.
        if w.selected_slot != w.carried_index {
            w.carried_index = w.selected_slot;
            self.connection
                .send(play::set_carried_item(w.carried_index));
        }
        if let Some(load) = &mut w.load {
            let chunk = w.chunks.contains(&w.player.chunk());
            load.tick(now, chunk, w.game_type == play::SPECTATOR);
            if load.is_ready(now) {
                w.load = None;
                if !w.loaded {
                    self.connection.send(play::player_loaded());
                    w.loaded = true;
                    eprintln!(
                        "[session] in the world after {} ms ({} chunks)",
                        now - w.level_started,
                        w.chunks.len()
                    );
                    let _ = self.events.send(SessionEvent::Loaded);
                }
            }
        }
        // \`player.sendChanges\` once loaded: needs physics, which comes next.
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
            cfg::Clientbound::UpdateTags(tags) => self.collecting.tags = tags,
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
                // chat yet. The level goes (\`clearLevel\`), and with it the ticks' packets.
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
                self.world = Some(World::new(&login.spawn, now));
            }
            play::Clientbound::Respawn { spawn, .. } => {
                let w = self.world.as_mut().ok_or("a respawn before the login")?;
                if spawn.dimension != w.dimension {
                    w.chunks.clear();
                    w.dimension = spawn.dimension.clone();
                }
                w.game_type = spawn.game_type;
                // \`handleRespawn\`: a new player, not loaded until the level is ready.
                let rotation = (w.player.y_rot, w.player.x_rot);
                w.player = LocalPlayer::new();
                (w.player.y_rot, w.player.x_rot) = rotation;
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
            play::Clientbound::LevelChunk { x, z } => {
                if let Some(w) = &mut self.world {
                    w.chunks.insert((x, z));
                }
            }
            play::Clientbound::ForgetLevelChunk { x, z } => {
                if let Some(w) = &mut self.world {
                    w.chunks.remove(&(x, z));
                }
            }
            play::Clientbound::SetHeldSlot { slot } => {
                // \`Inventory.isHotbarSlot\`.
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

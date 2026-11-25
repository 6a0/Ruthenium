//! Routes: input keyed by client tick, written by `cargo xtask bench dedicated` and
//! played by `ruthenium connect --route` (no window) and by the windowed client
//! (`--join <address> --route <file>`), which is how the world renderer is measured
//! on the same scenes as the vanilla bench client.
//!
//! A [`RoutePlayer`] is stepped once before every frame handed to the session. It waits
//! until a benchmark controller has put the player at the route's start (and, where a
//! renderer says so, until every section there has its mesh), holds each tick's keys and
//! makes its turn, records where the player is after each tick and which chunks are
//! loaded at the route's marks, and fails if any of it differs from what the route
//! says. The record ([`RoutePlayer::record`]) carries the two checksums a vanilla client
//! driven by the same file must reproduce.
//!
//! The player never waits for the game thread: it acts on the view as it is. A tick's
//! input is handed over in the frame after the tick before it was seen, some fifty
//! milliseconds early, and a run in which a stall ate that margin fails instead of
//! recording something else.

use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use ruthenium_sim::player::Keys;
use ruthenium_sim::session::{PlayerView, Session};
use serde_json::{Value, json};

/// How many ticks the start conditions must hold before a route begins, and how many
/// the position and the loaded chunks must stay unchanged before its end is recorded.
const START_TICKS: u64 = 20;
const SETTLE_TICKS: u64 = 40;
/// How long joining, reaching the start and coming to rest at the end may each take.
const JOIN_LIMIT: Duration = Duration::from_secs(60);
const START_LIMIT: Duration = Duration::from_secs(120);
const SETTLE_LIMIT: Duration = Duration::from_secs(30);
/// `GameType.CREATIVE`'s id, and `SURVIVAL`'s.
const CREATIVE: i32 = 1;
const SURVIVAL: i32 = 0;

/// What is held during a tick and the turn of the camera (`Entity.turn`'s amounts)
/// applied before it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Input {
    pub keys: Keys,
    /// The attack key (the left mouse button).
    pub attack: bool,
    pub turn: (f64, f64),
}

impl Input {
    const NONE: Input = Input {
        keys: Keys {
            forward: false,
            backward: false,
            left: false,
            right: false,
            jump: false,
            shift: false,
            sprint: false,
        },
        attack: false,
        turn: (0.0, 0.0),
    };
}

/// A stretch of a route: the same input on each of its ticks.
#[derive(Debug, Clone, PartialEq)]
struct Segment {
    ticks: u64,
    input: Input,
}

/// A route: where the player starts and what it does on every client tick from then
/// on, read from the file that this client and the vanilla bench client are both given.
///
/// Tick 0 is the first tick after the start conditions have held for a second. Before
/// tick `n` runs, its segment's turn is applied and its keys are held; after it, the
/// position and the rotation are recorded. The loaded chunks are recorded (a mark) at
/// tick 0, every `mark_every` ticks while the route is measured, and once more when
/// the player and the chunks have come to rest after the last tick.
#[derive(Debug, Clone, PartialEq)]
pub struct Route {
    pub id: String,
    pub name: String,
    position: [f64; 3],
    yaw: f32,
    pitch: f32,
    flying: bool,
    /// Begun in survival mode instead of creative.
    survival: bool,
    /// The chunks loaded at the start, sorted.
    start_chunks: Vec<(i32, i32)>,
    segments: Vec<Segment>,
    warmup_ticks: u64,
    capture_ticks: u64,
    mark_every: u64,
    /// The player must not move at all (the idle and spin routes).
    stationary: bool,
}

impl Route {
    pub fn parse(text: &str) -> Result<Route, String> {
        let v: Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
        if v["format"] != "ruthenium-route/1" {
            return Err(format!("unknown format {}", v["format"]));
        }
        let number = |v: &Value, what: &str| v.as_f64().ok_or_else(|| format!("no {what}"));
        let count = |v: &Value, what: &str| v.as_u64().ok_or_else(|| format!("no {what}"));
        let pair =
            |v: &Value, what: &str| Ok::<_, String>((number(&v[0], what)?, number(&v[1], what)?));
        let start = &v["start"];
        let p = &start["position"];
        let mut start_chunks = Vec::new();
        for c in start["chunks"].as_array().ok_or("no start chunks")? {
            let (x, z) = pair(c, "start chunk")?;
            start_chunks.push((x as i32, z as i32));
        }
        start_chunks.sort_unstable();
        let mut segments = Vec::new();
        for s in v["segments"].as_array().ok_or("no segments")? {
            let mut input = Input::NONE;
            for k in s["keys"].as_array().ok_or("a segment without keys")? {
                match k.as_str() {
                    Some("forward") => input.keys.forward = true,
                    Some("backward") => input.keys.backward = true,
                    Some("left") => input.keys.left = true,
                    Some("right") => input.keys.right = true,
                    Some("jump") => input.keys.jump = true,
                    Some("sneak") => input.keys.shift = true,
                    Some("sprint") => input.keys.sprint = true,
                    Some("attack") => input.attack = true,
                    _ => return Err(format!("unknown key {k}")),
                }
            }
            input.turn = pair(&s["turn"], "segment turn")?;
            segments.push(Segment {
                ticks: count(&s["ticks"], "segment ticks")?,
                input,
            });
        }
        let route = Route {
            id: v["id"].as_str().ok_or("no id")?.to_owned(),
            name: v["name"].as_str().ok_or("no name")?.to_owned(),
            position: [
                number(&p[0], "start position")?,
                number(&p[1], "start position")?,
                number(&p[2], "start position")?,
            ],
            yaw: number(&start["yaw"], "start yaw")? as f32,
            pitch: number(&start["pitch"], "start pitch")? as f32,
            flying: start["flying"].as_bool().ok_or("no start flying")?,
            survival: start["survival"].as_bool().unwrap_or(false),
            start_chunks,
            segments,
            warmup_ticks: count(&v["warmupTicks"], "warmupTicks")?,
            capture_ticks: count(&v["captureTicks"], "captureTicks")?,
            mark_every: count(&v["markEvery"], "markEvery")?.max(1),
            stationary: v["stationary"].as_bool().ok_or("no stationary")?,
        };
        if route.total() < route.warmup_ticks + route.capture_ticks {
            return Err("the segments end before the capture does".into());
        }
        Ok(route)
    }

    pub fn total(&self) -> u64 {
        self.segments.iter().map(|s| s.ticks).sum()
    }

    /// What is held during `tick` and the turn applied before it.
    pub fn input(&self, tick: u64) -> Input {
        let mut first = 0;
        for s in &self.segments {
            if tick < first + s.ticks {
                return s.input;
            }
            first += s.ticks;
        }
        Input::NONE
    }
}

/// The rotation after `Entity.turn(xo, yo)`, in the game's float arithmetic.
fn turned((yaw, pitch): (f32, f32), (xo, yo): (f64, f64)) -> (f32, f32) {
    (
        yaw + xo as f32 * 0.15,
        ((pitch + yo as f32 * 0.15) % 360.0).clamp(-90.0, 90.0),
    )
}

/// FNV-1a, 64 bits, as 16 hex digits: the checksum both clients compute.
fn fnv1a(text: &str) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for b in text.bytes() {
        hash = (hash ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{hash:016x}")
}

/// One tick's record: the bits of x, y, z (doubles), then of the yaw and the pitch
/// (floats), as 64 hex digits. The position checksum is [`fnv1a`] of all of a route's
/// records one after the other.
fn tick_record(position: [f64; 3], yaw: f32, pitch: f32) -> String {
    format!(
        "{:016x}{:016x}{:016x}{:08x}{:08x}",
        position[0].to_bits(),
        position[1].to_bits(),
        position[2].to_bits(),
        yaw.to_bits(),
        pitch.to_bits()
    )
}

/// Sorted chunk positions as `x,z;` each. The footprint checksum is [`fnv1a`] of the
/// start mark's text, a `|`, and the end mark's.
fn chunk_text(chunks: &[(i32, i32)]) -> String {
    chunks.iter().map(|(x, z)| format!("{x},{z};")).collect()
}

/// The loaded chunks as the game thread last published them (before it showed the view
/// of the same frame), sorted.
fn sorted_chunks(session: &Session) -> Vec<(i32, i32)> {
    let mut chunks: Vec<(i32, i32)> = session
        .scene()
        .with_terrain(|t| t.columns().map(|(at, _)| at).collect());
    chunks.sort_unstable();
    chunks
}

pub fn epoch_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}

fn same_position(a: [f64; 3], b: [f64; 3]) -> bool {
    a.map(f64::to_bits) == b.map(f64::to_bits)
}

/// What a route run recorded, written out whether it passed or not.
#[derive(Debug, Default)]
struct RouteLog {
    /// Where the server put the player when it joined.
    join_position: Option<[f64; 3]>,
    /// One [`tick_record`] per route tick.
    ticks: Vec<String>,
    marks: Vec<Value>,
    /// The chunks at the first and the last mark.
    start_chunks: Vec<(i32, i32)>,
    end_chunks: Option<Vec<(i32, i32)>>,
    end_position: Option<[f64; 3]>,
    /// Epoch milliseconds of the readiness markers, by name.
    markers: serde_json::Map<String, Value>,
}

impl RouteLog {
    fn marker(&mut self, name: &str) {
        self.markers.insert(name.to_owned(), json!(epoch_ms()));
    }

    fn mark(&mut self, tick: u64, v: &PlayerView, chunks: &[(i32, i32)], terrain: Option<Value>) {
        let center = v.position.map(|c| (c.floor() as i32) >> 4);
        println!(
            "          route tick {tick}: at {:.3} {:.3} {:.3}, {} chunks",
            v.position[0],
            v.position[1],
            v.position[2],
            chunks.len()
        );
        let mut mark = json!({
            "tick": tick,
            "epochMs": epoch_ms(),
            "position": v.position,
            "center": [center[0], center[2]],
            "chunks": chunks.iter().map(|(x, z)| json!([x, z])).collect::<Vec<_>>(),
        });
        if let Some(t) = terrain {
            mark["terrain"] = t;
        }
        self.marks.push(mark);
    }

    fn positions_checksum(&self) -> String {
        fnv1a(&self.ticks.concat())
    }

    fn footprint_checksum(&self) -> Option<String> {
        let end = self.end_chunks.as_ref()?;
        Some(fnv1a(&format!(
            "{}|{}",
            chunk_text(&self.start_chunks),
            chunk_text(end)
        )))
    }

    fn to_json(&self, route: &Route, error: Option<&str>) -> Value {
        json!({
            "format": "ruthenium-route-run/1",
            "client": "ruthenium",
            "route": { "id": route.id, "name": route.name },
            "joinPosition": self.join_position,
            "endPosition": self.end_position,
            "ticks": self.ticks,
            "marks": self.marks,
            "markers": self.markers,
            "checksums": {
                "positions": self.positions_checksum(),
                "footprint": self.footprint_checksum(),
            },
            "error": error,
        })
    }
}

/// The checksums a run is expected to end with (`--expect-positions`, `--expect-footprint`).
#[derive(Debug, Default, Clone)]
pub struct Expected {
    pub positions: Option<String>,
    pub footprint: Option<String>,
}

/// Why the player isn't at the route's start yet.
fn not_at_start(v: &PlayerView, route: &Route, chunks: &[(i32, i32)]) -> Option<String> {
    let p = v.position;
    if !v.loaded {
        Some("the player hasn't loaded into the level".into())
    } else if v.game_type != if route.survival { SURVIVAL } else { CREATIVE } {
        Some(format!("the game mode is {}, not the route's", v.game_type))
    } else if !same_position(p, route.position) {
        Some(format!(
            "the player is at {} {} {}, the route starts at {} {} {}",
            p[0], p[1], p[2], route.position[0], route.position[1], route.position[2]
        ))
    } else if (v.y_rot.to_bits(), v.x_rot.to_bits()) != (route.yaw.to_bits(), route.pitch.to_bits())
    {
        Some(format!(
            "the player faces {} {}, the route starts facing {} {}",
            v.y_rot, v.x_rot, route.yaw, route.pitch
        ))
    } else if v.flying != route.flying {
        Some(format!(
            "flying is {}, the route starts with {}",
            v.flying, route.flying
        ))
    } else if v.on_ground == route.flying {
        Some(format!("on the ground is {}", v.on_ground))
    } else if chunks != route.start_chunks {
        let loaded = chunks
            .iter()
            .filter(|c| route.start_chunks.binary_search(c).is_ok())
            .count();
        Some(format!(
            "{loaded} of the start's {} chunks are loaded, and {} others",
            route.start_chunks.len(),
            chunks.len() - loaded
        ))
    } else {
        None
    }
}

/// What a renderer says about the terrain it draws, for a route played in the window.
#[derive(Debug, Clone)]
pub struct Terrain {
    /// Why the terrain is not all there yet (sections still to mesh, textures on their
    /// way): the route's start and its end wait for a moment at which this is `None`.
    pub pending: Option<String>,
    /// What is recorded with a mark.
    pub mark: Value,
}

#[derive(Debug)]
enum Phase {
    Joining,
    /// Waiting for the start conditions: they have held since this tick, and whether
    /// the terrain has been all there at some moment since.
    Waiting(Option<u64>, bool),
    Running,
    /// The route's ticks are done; the position and the chunks have been as they are
    /// since this tick.
    Settling(u64, [f64; 3], Vec<(i32, i32)>),
    /// Over: played to its end and found right, or failed.
    Over(Result<(), String>),
}

/// See the module docs.
#[derive(Debug)]
pub struct RoutePlayer {
    route: Route,
    expected: Expected,
    log: RouteLog,
    phase: Phase,
    started: Instant,
    deadline: Instant,
    /// Frames handed to the session since it began: steps that went on.
    sent: u64,
    /// The session's tick count at route tick 0; route ticks done; route ticks whose
    /// input has been given.
    first_tick: u64,
    done: u64,
    armed: u64,
    /// The number of the frame that carries the turn of the tick to come, until a view
    /// from that frame or a later one has shown the tick still to come. A frame's turn
    /// is applied after its ticks: a tick that runs in that very frame runs before its
    /// turn.
    turn_frame: Option<u64>,
    rotation: (f32, f32),
    /// What is held now.
    held: Input,
    waiting_for: String,
    /// When it last said what it waits for.
    said: Instant,
}

impl RoutePlayer {
    pub fn new(route: Route, expected: Expected) -> RoutePlayer {
        let now = Instant::now();
        RoutePlayer {
            rotation: (route.yaw, route.pitch),
            route,
            expected,
            log: RouteLog::default(),
            phase: Phase::Joining,
            started: now,
            deadline: now + JOIN_LIMIT,
            sent: 0,
            first_tick: 0,
            done: 0,
            armed: 0,
            turn_frame: None,
            held: Input::NONE,
            waiting_for: String::from("the server"),
            said: now,
        }
    }

    pub fn route(&self) -> &Route {
        &self.route
    }

    /// Frames handed to the session so far, if one followed every step that went on.
    pub fn sent(&self) -> u64 {
        self.sent
    }

    /// Whether the ticks being played are the ones the route is measured over.
    pub fn capturing(&self) -> bool {
        matches!(self.phase, Phase::Running)
            && (self.route.warmup_ticks..self.route.warmup_ticks + self.route.capture_ticks)
                .contains(&self.done)
    }

    /// Whether the route's ticks are being played.
    pub fn running(&self) -> bool {
        matches!(self.phase, Phase::Running)
    }

    /// How the route ended, once it has.
    pub fn outcome(&self) -> Option<&Result<(), String>> {
        match &self.phase {
            Phase::Over(r) => Some(r),
            _ => None,
        }
    }

    /// Ends the route as failed (the session closed, a screen opened over the world).
    pub fn fail(&mut self, why: String) {
        if self.outcome().is_none() {
            self.phase = Phase::Over(Err(why));
        }
    }

    /// The run's record (`ruthenium-route-run/1`), with what was recorded up to an error.
    pub fn record(&self) -> Value {
        let error = match &self.phase {
            Phase::Over(Err(e)) => Some(e.as_str()),
            Phase::Over(Ok(())) => None,
            _ => Some("the route was not played to its end"),
        };
        self.log.to_json(&self.route, error)
    }

    /// One step, before a frame is handed to the session (`Session::frame`): takes in
    /// `view` (the session's, as it is now) and sets what the next ticks hold. `terrain`
    /// is asked when the route has to know what is drawn (at its start, its marks and
    /// its end); `None` where nothing is. Returns whether the route is over: see
    /// [`Self::outcome`].
    pub fn step(
        &mut self,
        session: &Session,
        view: Option<&PlayerView>,
        terrain: &dyn Fn() -> Option<Terrain>,
    ) -> bool {
        if self.outcome().is_some() {
            return true;
        }
        if let Err(e) = self.advance(session, view, terrain) {
            self.phase = Phase::Over(Err(e));
        }
        // What is held is said every frame: nothing else (a key event, a lost focus)
        // changes it for longer than that.
        let held = match self.phase {
            Phase::Running => self.held,
            _ => Input::NONE,
        };
        session.set_keys(held.keys);
        session.set_attack(held.attack);
        if self.outcome().is_some() {
            return true;
        }
        self.sent += 1;
        false
    }

    fn advance(
        &mut self,
        session: &Session,
        view: Option<&PlayerView>,
        terrain: &dyn Fn() -> Option<Terrain>,
    ) -> Result<(), String> {
        let total = self.route.total();
        let mut ended = false;
        if let Some(v) = view {
            match &mut self.phase {
                Phase::Joining => {
                    if v.loaded {
                        self.log.join_position = Some(v.position);
                        self.log.marker("joinedEpochMs");
                        self.phase = Phase::Waiting(None, false);
                        self.deadline = Instant::now() + START_LIMIT;
                    }
                }
                Phase::Waiting(since, built) => {
                    let chunks = sorted_chunks(session);
                    let mut state = None;
                    let away = not_at_start(v, &self.route, &chunks);
                    if away.is_some() {
                        *built = false;
                    }
                    // Everything loaded has to have had its mesh once, as the vanilla
                    // bench client waits for every section to have been built: in a
                    // world where something keeps changing (lava flows under the
                    // spawn for minutes) there is no moment after which nothing is
                    // being built.
                    let why = away.or_else(|| {
                        state = terrain();
                        let pending = state.as_ref().and_then(|t| t.pending.clone());
                        *built |= pending.is_none();
                        pending.filter(|_| !*built)
                    });
                    match why {
                        Some(why) => {
                            // Said when it has lasted, and then now and again.
                            if self.said.elapsed() > Duration::from_secs(2) {
                                self.said = Instant::now();
                                println!(
                                    "{:7.3}s  waiting for the route's start: {why}",
                                    self.started.elapsed().as_secs_f64()
                                );
                            }
                            self.waiting_for = why;
                            *since = None;
                        }
                        None if v.ticks - *since.get_or_insert(v.ticks) >= START_TICKS => {
                            println!(
                                "{:7.3}s  at the start of the {} route ({} ticks)",
                                self.started.elapsed().as_secs_f64(),
                                self.route.name,
                                total
                            );
                            self.log.marker("sceneReadyEpochMs");
                            self.log.marker("warmupStartEpochMs");
                            let state = state.or_else(terrain);
                            self.log.mark(0, v, &chunks, state.map(|t| t.mark));
                            self.log.start_chunks = chunks;
                            self.first_tick = v.ticks;
                            self.phase = Phase::Running;
                            // Twenty ticks a second, and a margin for a slow machine.
                            self.deadline =
                                Instant::now() + Duration::from_millis(total * 75 + 30_000);
                        }
                        None => {}
                    }
                }
                Phase::Running => {
                    let now_done = v.ticks - self.first_tick;
                    let done = self.done;
                    if now_done > done + 1 {
                        return Err(format!(
                            "{} client ticks ran between two frames at route tick {done}: the machine stalled, and input keyed by tick was missed",
                            now_done - done
                        ));
                    }
                    if now_done > done {
                        if self.turn_frame.is_some() {
                            return Err(format!(
                                "route tick {done} ran in the frame that carried its turn: the machine stalled, and the turn came after the tick"
                            ));
                        }
                        if (v.y_rot.to_bits(), v.x_rot.to_bits())
                            != (self.rotation.0.to_bits(), self.rotation.1.to_bits())
                        {
                            return Err(format!(
                                "route tick {done}: the player faces {} {}, the route says {} {}",
                                v.y_rot, v.x_rot, self.rotation.0, self.rotation.1
                            ));
                        }
                        if self.route.stationary && !same_position(v.position, self.route.position)
                        {
                            return Err(format!(
                                "route tick {done}: the player moved to {} {} {} on a route that stands still",
                                v.position[0], v.position[1], v.position[2]
                            ));
                        }
                        self.log
                            .ticks
                            .push(tick_record(v.position, v.y_rot, v.x_rot));
                        self.done = now_done;
                        let done = now_done;
                        let capture_end = self.route.warmup_ticks + self.route.capture_ticks;
                        if done == self.route.warmup_ticks {
                            self.log.marker("captureStartEpochMs");
                        }
                        if done == capture_end {
                            self.log.marker("captureEndEpochMs");
                        }
                        if done.is_multiple_of(self.route.mark_every) && done <= capture_end {
                            self.log.mark(
                                done,
                                v,
                                &sorted_chunks(session),
                                terrain().map(|t| t.mark),
                            );
                        }
                    } else if self.turn_frame.is_some_and(|f| v.frames >= f) {
                        self.turn_frame = None;
                    }
                    if self.done == total {
                        self.held = Input::NONE;
                        self.phase = Phase::Settling(v.ticks, v.position, sorted_chunks(session));
                        self.deadline = Instant::now() + SETTLE_LIMIT;
                    } else if self.armed == self.done {
                        let input = self.route.input(self.done);
                        self.held = input;
                        if input.turn != (0.0, 0.0) {
                            session.turn(input.turn.0, input.turn.1);
                            self.rotation = turned(self.rotation, input.turn);
                            // The frame about to be handed over.
                            self.turn_frame = Some(self.sent + 1);
                        }
                        self.armed = self.done + 1;
                    }
                }
                Phase::Settling(since, position, chunks) => {
                    let now = sorted_chunks(session);
                    let state = terrain();
                    let pending = state.as_ref().and_then(|t| t.pending.clone());
                    if !same_position(v.position, *position) || now != *chunks {
                        (*since, *position, *chunks) = (v.ticks, v.position, now);
                    } else if let Some(why) = pending {
                        self.waiting_for = why;
                    } else if v.ticks - *since >= SETTLE_TICKS {
                        let chunks = std::mem::take(chunks);
                        self.log.marker("routeEndEpochMs");
                        self.log.mark(total, v, &chunks, state.map(|t| t.mark));
                        self.log.end_position = Some(v.position);
                        self.log.end_chunks = Some(chunks);
                        ended = true;
                    }
                }
                Phase::Over(_) => return Ok(()),
            }
        }
        if ended {
            let verdict = self.judge();
            self.phase = Phase::Over(verdict);
            return Ok(());
        }
        if Instant::now() > self.deadline {
            return Err(match self.phase {
                Phase::Joining => "the player never loaded into the level".into(),
                Phase::Waiting(..) => format!("the route never started: {}", self.waiting_for),
                Phase::Running => {
                    format!("the route stopped at tick {} of {total}", self.done)
                }
                Phase::Settling(..) => format!(
                    "the player, the loaded chunks and what is drawn of them never came to rest after the route ({})",
                    self.waiting_for
                ),
                Phase::Over(_) => return Ok(()),
            });
        }
        Ok(())
    }

    /// The finished run against the checksums it was expected to end with.
    fn judge(&self) -> Result<(), String> {
        let positions = self.log.positions_checksum();
        let footprint = self.log.footprint_checksum().unwrap_or_default();
        println!(
            "{:7.3}s  the {} route ({}) ended: positions {positions}, footprint {footprint}",
            self.started.elapsed().as_secs_f64(),
            self.route.name,
            self.route.id
        );
        if let Some(e) = self
            .expected
            .positions
            .as_ref()
            .filter(|e| **e != positions)
        {
            return Err(format!(
                "the position checksum is {positions}, expected {e}"
            ));
        }
        if let Some(e) = self
            .expected
            .footprint
            .as_ref()
            .filter(|e| **e != footprint)
        {
            return Err(format!(
                "the footprint checksum is {footprint}, expected {e}"
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ROUTE: &str = r#"{
        "format": "ruthenium-route/1", "id": "0123456789abcdef", "name": "walk",
        "viewDistance": 2,
        "start": { "position": [0.5, 64.0, -3.5], "yaw": 0.0, "pitch": 0.0, "flying": false,
                   "chunks": [[1, 0], [0, 0], [0, -1]] },
        "segments": [{ "ticks": 3, "keys": ["forward", "jump"], "turn": [4.0, 0.0] },
                     { "ticks": 2, "keys": ["attack"], "turn": [0.0, 0.0] },
                     { "ticks": 2, "keys": [], "turn": [0.0, 0.0] }],
        "warmupTicks": 1, "captureTicks": 2, "markEvery": 100, "stationary": false }"#;

    #[test]
    fn reads_routes() {
        let r = Route::parse(ROUTE).unwrap();
        assert_eq!(
            (r.name.as_str(), r.id.as_str()),
            ("walk", "0123456789abcdef")
        );
        assert_eq!(r.start_chunks, [(0, -1), (0, 0), (1, 0)], "sorted");
        assert_eq!(r.total(), 7);
        let held = r.input(2);
        assert!(held.keys.forward && held.keys.jump && !held.keys.sprint && !held.attack);
        assert_eq!(held.turn, (4.0, 0.0));
        let held = r.input(3);
        assert!(held.attack && !held.keys.forward);
        assert_eq!(r.input(5), Input::NONE);
        assert_eq!(r.input(99), Input::NONE);
        assert!(
            Route::parse(&ROUTE.replace("\"captureTicks\": 2", "\"captureTicks\": 9")).is_err()
        );
        assert!(Route::parse(&ROUTE.replace("jump", "fly")).is_err());
    }

    #[test]
    fn checksums_are_fnv1a_of_the_records() {
        assert_eq!(fnv1a(""), "cbf29ce484222325");
        assert_eq!(fnv1a("a"), "af63dc4c8601ec8c");
        assert_eq!(
            tick_record([4.5, 71.0, -2.5], 1.8, 0.0),
            "40120000000000004051c00000000000c0040000000000003fe6666600000000"
        );
        assert_eq!(chunk_text(&[(0, -1), (12, 3)]), "0,-1;12,3;");
        // Turns add up in floats, as the game adds them: a tick's 12 counts are not
        // exactly 1.8 degrees.
        let mut rotation = (0.0f32, 0.0f32);
        for _ in 0..200 {
            rotation = turned(rotation, (12.0, 0.0));
        }
        assert_eq!(rotation, (359.9996, 0.0));
        assert_eq!(turned((0.0, 80.0), (0.0, 100.0)), (0.0, 90.0));
    }

    #[test]
    fn a_route_is_measured_over_its_capture_only() {
        let mut p = RoutePlayer::new(Route::parse(ROUTE).unwrap(), Expected::default());
        assert!(!p.capturing(), "not while joining");
        p.phase = Phase::Running;
        let capturing: Vec<bool> = (0..5)
            .map(|done| {
                p.done = done;
                p.capturing()
            })
            .collect();
        assert_eq!(capturing, [false, true, true, false, false]);
        p.fail("the session closed".into());
        assert!(!p.capturing());
        assert_eq!(p.outcome(), Some(&Err("the session closed".into())));
        assert_eq!(p.record()["error"], "the session closed");
        // The first failure stays.
        p.fail("another".into());
        assert_eq!(p.record()["error"], "the session closed");
    }
}

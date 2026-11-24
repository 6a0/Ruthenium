//! `ruthenium connect`: joins a server without a window and prints what happens, to
//! try the connection path from a shell (development).
//!
//! `--script` plays input once the player is in the world, through the same calls the
//! window makes: steps separated by spaces, each `KEYS@MS` or `KEYS@MS:DX,DY`. KEYS are
//! `+`-joined `w a s d jump sneak sprint use hit` (`use` holds the right button, `hit`
//! the left), hotbar digits `1`-`9`, `click` (one press of the right button), `attack`
//! (one press of the left), `swap` (the swap-hands key) and `wake` (the bed screen's
//! Leave Bed), the last five pressed once
//! at the step's start, or `-` for none; held for MS milliseconds while the mouse moves
//! DX, DY counts every frame (60 a second, the rate frames run at here), turned as the window turns them with the
//! saved mouse sensitivity. For example
//! `w@2000 w+sprint+jump@3000 w@2000:12,0 sneak+w@1500 2@100 -@1000`.
//!
//! A death is printed, and Respawn pressed as soon as the death screen's buttons work;
//! the script goes on (it doesn't start over) once the player is back.
//!
//! `--route` plays a route file written by `cargo xtask bench dedicated` instead: input
//! keyed by client tick (see [`Route`]). The client waits until a benchmark controller
//! has put the player at the route's start, plays every tick's input, records where the
//! player is after each tick and which chunks are loaded at the route's marks, and
//! fails if any of it differs from what the route says. `--route-out` gets the record
//! as JSON, with the two checksums a vanilla client driven by the same file must
//! reproduce.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use ruthenium_prepare::discover::Env;
use ruthenium_sim::input::{self, MouseOptions};
use ruthenium_sim::player::{Keys, Pose};
use ruthenium_sim::session::{PlayerView, Session, SessionEvent};
use serde_json::{Value, json};

use crate::account::{AccountStatus, Accounts};
use crate::join::{Joiner, closed_text, plain_text, stage_text};
use crate::settings::SettingsFile;

/// The frame rate without a window: vanilla's default frame limit's neighbourhood.
const FRAME: Duration = Duration::from_micros(16_667);

/// One step of a `--script`.
#[derive(Debug, Clone, PartialEq)]
struct Step {
    keys: Keys,
    hotbar: Vec<u8>,
    use_held: bool,
    attack_held: bool,
    clicks: u32,
    attacks: u32,
    swaps: u32,
    /// Leave Bed pressed at the step's start (`wake`).
    wake: bool,
    ms: u64,
    turn: (f32, f32),
}

fn parse_script(s: &str) -> Result<Vec<Step>, String> {
    s.split_whitespace()
        .map(|step| {
            let bad = || format!("--script: can't read the step {step:?}");
            let (keys, rest) = step.split_once('@').ok_or_else(bad)?;
            let (ms, turn) = match rest.split_once(':') {
                Some((ms, t)) => {
                    let (dx, dy) = t.split_once(',').ok_or_else(bad)?;
                    (
                        ms,
                        (
                            dx.parse().map_err(|_| bad())?,
                            dy.parse().map_err(|_| bad())?,
                        ),
                    )
                }
                None => (rest, (0.0, 0.0)),
            };
            let mut st = Step {
                keys: Keys::default(),
                hotbar: Vec::new(),
                use_held: false,
                attack_held: false,
                clicks: 0,
                attacks: 0,
                swaps: 0,
                wake: false,
                ms: ms.parse().map_err(|_| bad())?,
                turn,
            };
            for k in keys.split('+').filter(|k| *k != "-") {
                match k {
                    "w" => st.keys.forward = true,
                    "s" => st.keys.backward = true,
                    "a" => st.keys.left = true,
                    "d" => st.keys.right = true,
                    "jump" => st.keys.jump = true,
                    "sneak" => st.keys.shift = true,
                    "sprint" => st.keys.sprint = true,
                    "use" => st.use_held = true,
                    "hit" => st.attack_held = true,
                    "click" => st.clicks += 1,
                    "attack" => st.attacks += 1,
                    "swap" => st.swaps += 1,
                    "wake" => st.wake = true,
                    d if d.len() == 1 && ("1"..="9").contains(&d) => {
                        st.hotbar.push(d.parse::<u8>().map_err(|_| bad())? - 1);
                    }
                    _ => return Err(format!("--script: unknown key {k:?} in {step:?}")),
                }
            }
            Ok(st)
        })
        .collect()
}

/// Plays a script, one frame at a time.
struct Player {
    steps: Vec<Step>,
    at: usize,
    step_started: Instant,
    last_report: Instant,
    mouse: MouseOptions,
    motion: input::MouseMotion,
}

impl Player {
    fn start(&mut self, session: &Session) {
        self.at = 0;
        self.step_started = Instant::now();
        self.begin(session);
    }

    fn begin(&mut self, session: &Session) {
        let Some(st) = self.steps.get(self.at) else {
            session.set_keys(Keys::default());
            session.set_use(false);
            session.set_attack(false);
            return;
        };
        println!(
            "          script: {:?}, {} ms, turning {:?}",
            st.keys, st.ms, st.turn
        );
        session.set_keys(st.keys);
        for &slot in &st.hotbar {
            session.hotbar_key(slot);
        }
        if st.wake {
            session.leave_bed();
        }
        for _ in 0..st.swaps {
            session.swap_hands_key();
        }
        // A click: down and up again before the tick reads it.
        for _ in 0..st.clicks {
            session.set_use(false);
            session.set_use(true);
        }
        session.set_use(st.use_held);
        for _ in 0..st.attacks {
            session.set_attack(false);
            session.set_attack(true);
        }
        session.set_attack(st.attack_held);
    }

    /// False once the script has ended.
    fn frame(&mut self, session: &Session) -> bool {
        let Some(st) = self.steps.get(self.at) else {
            return false;
        };
        let now = Instant::now();
        self.motion.moved(st.turn.0, st.turn.1, true);
        let (dx, dy) = self.motion.take();
        if (dx, dy) != (0.0, 0.0) {
            let (xo, yo) = input::turn_amount(dx, dy, &self.mouse);
            session.turn(xo, yo);
        }
        if now - self.last_report >= Duration::from_secs(1) {
            self.last_report = now;
            if let Some(v) = session.view() {
                let p = v.position;
                let using = match &v.using {
                    Some((hand, item, left)) => format!(", using {item} ({hand}, {left} left)"),
                    None => String::new(),
                };
                let pose = match v.pose {
                    Pose::FallFlying => ", gliding",
                    Pose::SpinAttack => ", spinning",
                    _ => "",
                };
                println!(
                    "          at {:.3} {:.3} {:.3}, facing {:.2} {:.2}, slot {}{}{}{pose}, hands [{}] [{}], food {}{using}",
                    p[0],
                    p[1],
                    p[2],
                    v.y_rot,
                    v.x_rot,
                    v.selected_slot + 1,
                    if v.on_ground { ", on the ground" } else { "" },
                    if v.sprinting { ", sprinting" } else { "" },
                    v.main_hand,
                    v.off_hand,
                    v.food,
                );
            }
        }
        if now - self.step_started >= Duration::from_millis(st.ms) {
            self.at += 1;
            self.step_started = now;
            self.begin(session);
        }
        true
    }
}

/// The frame period while a route plays: far shorter than a tick, so each tick runs
/// within milliseconds of falling due and the next tick's input is in place long
/// before it.
const ROUTE_FRAME: Duration = Duration::from_millis(2);
/// How many ticks the start conditions must hold before a route begins, and how many
/// the position and the loaded chunks must stay unchanged before its end is recorded.
const START_TICKS: u64 = 20;
const SETTLE_TICKS: u64 = 40;
/// How long joining, reaching the start and coming to rest at the end may each take.
const JOIN_LIMIT: Duration = Duration::from_secs(60);
const START_LIMIT: Duration = Duration::from_secs(120);
const SETTLE_LIMIT: Duration = Duration::from_secs(30);
/// `GameType.CREATIVE`'s id.
const CREATIVE: i32 = 1;

/// A stretch of a route: the keys held during each of its ticks, and the turn of the
/// camera (`Entity.turn`'s amounts) applied before each of them.
#[derive(Debug, Clone, PartialEq)]
struct Segment {
    ticks: u64,
    keys: Keys,
    turn: (f64, f64),
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
struct Route {
    id: String,
    name: String,
    position: [f64; 3],
    yaw: f32,
    pitch: f32,
    flying: bool,
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
    fn parse(text: &str) -> Result<Route, String> {
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
            let mut keys = Keys::default();
            for k in s["keys"].as_array().ok_or("a segment without keys")? {
                match k.as_str() {
                    Some("forward") => keys.forward = true,
                    Some("backward") => keys.backward = true,
                    Some("left") => keys.left = true,
                    Some("right") => keys.right = true,
                    Some("jump") => keys.jump = true,
                    Some("sneak") => keys.shift = true,
                    Some("sprint") => keys.sprint = true,
                    _ => return Err(format!("unknown key {k}")),
                }
            }
            segments.push(Segment {
                ticks: count(&s["ticks"], "segment ticks")?,
                keys,
                turn: pair(&s["turn"], "segment turn")?,
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

    fn total(&self) -> u64 {
        self.segments.iter().map(|s| s.ticks).sum()
    }

    /// The keys held during `tick` and the turn applied before it.
    fn input(&self, tick: u64) -> (Keys, (f64, f64)) {
        let mut first = 0;
        for s in &self.segments {
            if tick < first + s.ticks {
                return (s.keys, s.turn);
            }
            first += s.ticks;
        }
        (Keys::default(), (0.0, 0.0))
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

fn epoch_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}

fn same_position(a: [f64; 3], b: [f64; 3]) -> bool {
    a.map(f64::to_bits) == b.map(f64::to_bits)
}

/// What a route run recorded, written to `--route-out` whether it passed or not.
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

    fn mark(&mut self, tick: u64, v: &PlayerView, chunks: &[(i32, i32)]) {
        let center = v.position.map(|c| (c.floor() as i32) >> 4);
        println!(
            "          route tick {tick}: at {:.3} {:.3} {:.3}, {} chunks",
            v.position[0],
            v.position[1],
            v.position[2],
            chunks.len()
        );
        self.marks.push(json!({
            "tick": tick,
            "epochMs": epoch_ms(),
            "position": v.position,
            "center": [center[0], center[2]],
            "chunks": chunks.iter().map(|(x, z)| json!([x, z])).collect::<Vec<_>>(),
        }));
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
#[derive(Debug, Default)]
struct Expected {
    positions: Option<String>,
    footprint: Option<String>,
}

/// Why the player isn't at the route's start yet.
fn not_at_start(v: &PlayerView, route: &Route, chunks: &[(i32, i32)]) -> Option<String> {
    let p = v.position;
    if !v.loaded {
        Some("the player hasn't loaded into the level".into())
    } else if v.game_type != CREATIVE {
        Some(format!("the game mode is {}, not creative", v.game_type))
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

/// The view once the game thread has run every frame sent so far (`None` outside a
/// level), so that what it shows is the state the next frame's input meets.
fn synced_view(session: &Session, sent: u64) -> Result<Option<PlayerView>, String> {
    let limit = Instant::now() + Duration::from_secs(5);
    loop {
        match session.view() {
            Some(v) if v.frames < sent => {}
            view => return Ok(view),
        }
        if Instant::now() > limit {
            return Err("the game thread stopped running frames".into());
        }
        std::thread::sleep(Duration::from_micros(100));
    }
}

/// Plays `route` to its end and checks it; `log` holds what was recorded up to an error.
fn play_route(
    session: &Session,
    route: &Route,
    expected: &Expected,
    lang: &HashMap<String, String>,
    log: &mut RouteLog,
) -> Result<(), String> {
    enum Phase {
        Joining,
        /// Waiting for the start conditions; they have held since this tick.
        Waiting(Option<u64>),
        Running,
        /// The route's ticks are done; the position and the chunks have been as they
        /// are since this tick.
        Settling(u64, [f64; 3], Vec<(i32, i32)>),
    }
    let started = Instant::now();
    let total = route.total();
    let mut phase = Phase::Joining;
    let mut deadline = Instant::now() + JOIN_LIMIT;
    // Frames sent; the session's tick count at route tick 0; route ticks done; route
    // ticks whose input has been given; the tick whose input the last frame carried.
    let mut sent = 0u64;
    let mut first_tick = 0u64;
    let mut done = 0u64;
    let mut armed = 0u64;
    let mut armed_last = None;
    let mut rotation = (route.yaw, route.pitch);
    let mut waiting_for = String::from("the server");
    loop {
        for e in session.poll() {
            let t = started.elapsed().as_secs_f64();
            match &e {
                SessionEvent::Closed(c) => {
                    let (title, reason) = closed_text(c, lang);
                    return Err(format!("{title}: {reason}"));
                }
                SessionEvent::Died { .. } => return Err("the player died".into()),
                SessionEvent::CodeOfConduct(_) => {
                    return Err("the server asks for a code of conduct".into());
                }
                SessionEvent::Stage(s) => println!("{t:7.3}s  {}", stage_text(*s)),
                SessionEvent::LoggedIn { name } => println!("{t:7.3}s  logged in as {name}"),
                SessionEvent::Loaded => println!("{t:7.3}s  in the world"),
                _ => {}
            }
        }
        if let Some(v) = synced_view(session, sent)? {
            match &mut phase {
                Phase::Joining => {
                    if v.loaded {
                        log.join_position = Some(v.position);
                        log.marker("joinedEpochMs");
                        phase = Phase::Waiting(None);
                        deadline = Instant::now() + START_LIMIT;
                    }
                }
                Phase::Waiting(since) => {
                    let chunks = sorted_chunks(session);
                    match not_at_start(&v, route, &chunks) {
                        Some(why) => {
                            waiting_for = why;
                            *since = None;
                        }
                        None if v.ticks - *since.get_or_insert(v.ticks) >= START_TICKS => {
                            println!(
                                "{:7.3}s  at the start of the {} route ({} ticks)",
                                started.elapsed().as_secs_f64(),
                                route.name,
                                total
                            );
                            log.marker("sceneReadyEpochMs");
                            log.marker("warmupStartEpochMs");
                            log.mark(0, &v, &chunks);
                            log.start_chunks = chunks;
                            first_tick = v.ticks;
                            phase = Phase::Running;
                            // Twenty ticks a second, and a margin for a slow machine.
                            deadline = Instant::now() + Duration::from_millis(total * 75 + 30_000);
                        }
                        None => {}
                    }
                }
                Phase::Running => {
                    let now_done = v.ticks - first_tick;
                    if now_done > done + 1 {
                        return Err(format!(
                            "{} client ticks ran in one frame at route tick {done}: the machine stalled, and input keyed by tick was missed",
                            now_done - done
                        ));
                    }
                    if now_done > done {
                        if armed_last == Some(done) && route.input(done).1 != (0.0, 0.0) {
                            return Err(format!(
                                "route tick {done} ran in the frame that carried its turn: the machine stalled, and the turn came after the tick"
                            ));
                        }
                        if (v.y_rot.to_bits(), v.x_rot.to_bits())
                            != (rotation.0.to_bits(), rotation.1.to_bits())
                        {
                            return Err(format!(
                                "route tick {done}: the player faces {} {}, the route says {} {}",
                                v.y_rot, v.x_rot, rotation.0, rotation.1
                            ));
                        }
                        if route.stationary && !same_position(v.position, route.position) {
                            return Err(format!(
                                "route tick {done}: the player moved to {} {} {} on a route that stands still",
                                v.position[0], v.position[1], v.position[2]
                            ));
                        }
                        log.ticks.push(tick_record(v.position, v.y_rot, v.x_rot));
                        done = now_done;
                        if done == route.warmup_ticks {
                            log.marker("captureStartEpochMs");
                        }
                        if done == route.warmup_ticks + route.capture_ticks {
                            log.marker("captureEndEpochMs");
                        }
                        if done.is_multiple_of(route.mark_every)
                            && done <= route.warmup_ticks + route.capture_ticks
                        {
                            log.mark(done, &v, &sorted_chunks(session));
                        }
                    }
                    armed_last = None;
                    if done == total {
                        session.set_keys(Keys::default());
                        phase = Phase::Settling(v.ticks, v.position, sorted_chunks(session));
                        deadline = Instant::now() + SETTLE_LIMIT;
                    } else if armed == done {
                        let (keys, turn) = route.input(done);
                        session.set_keys(keys);
                        if turn != (0.0, 0.0) {
                            session.turn(turn.0, turn.1);
                            rotation = turned(rotation, turn);
                        }
                        armed = done + 1;
                        armed_last = Some(done);
                    }
                }
                Phase::Settling(since, position, chunks) => {
                    let now = sorted_chunks(session);
                    if !same_position(v.position, *position) || now != *chunks {
                        (*since, *position, *chunks) = (v.ticks, v.position, now);
                    } else if v.ticks - *since >= SETTLE_TICKS {
                        log.marker("routeEndEpochMs");
                        log.mark(total, &v, chunks);
                        log.end_position = Some(v.position);
                        log.end_chunks = Some(std::mem::take(chunks));
                        break;
                    }
                }
            }
        }
        if Instant::now() > deadline {
            return Err(match phase {
                Phase::Joining => "the player never loaded into the level".into(),
                Phase::Waiting(_) => format!("the route never started: {waiting_for}"),
                Phase::Running => format!("the route stopped at tick {done} of {total}"),
                Phase::Settling(..) => {
                    "the player and the loaded chunks never came to rest after the route".into()
                }
            });
        }
        session.frame();
        sent += 1;
        std::thread::sleep(ROUTE_FRAME);
    }
    let positions = log.positions_checksum();
    let footprint = log.footprint_checksum().unwrap_or_default();
    println!(
        "{:7.3}s  the {} route ({}) ended: positions {positions}, footprint {footprint}",
        started.elapsed().as_secs_f64(),
        route.name,
        route.id
    );
    if let Some(e) = expected.positions.as_ref().filter(|e| **e != positions) {
        return Err(format!(
            "the position checksum is {positions}, expected {e}"
        ));
    }
    if let Some(e) = expected.footprint.as_ref().filter(|e| **e != footprint) {
        return Err(format!(
            "the footprint checksum is {footprint}, expected {e}"
        ));
    }
    Ok(())
}

/// Joins, plays the route, writes the record and leaves. Fails if the run differed
/// from the route in any way.
fn run_route(
    session: &Session,
    route: &Route,
    expected: &Expected,
    out: Option<&Path>,
    lang: &HashMap<String, String>,
) -> Result<ExitCode, String> {
    let mut log = RouteLog::default();
    let result = play_route(session, route, expected, lang, &mut log);
    if let Some(out) = out {
        let record = log.to_json(route, result.as_ref().err().map(String::as_str));
        std::fs::write(out, record.to_string()).map_err(|e| format!("{}: {e}", out.display()))?;
    }
    session.disconnect();
    result.map_err(|e| format!("the {} route failed: {e}", route.name))?;
    Ok(ExitCode::SUCCESS)
}

pub fn run(args: &[String]) -> ExitCode {
    match run_inner(args) {
        Ok(code) => code,
        Err(e) => {
            eprintln!("ruthenium connect: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run_inner(args: &[String]) -> Result<ExitCode, String> {
    let mut address = None;
    let mut seconds = 10.0;
    let mut data_dir = None;
    let mut script = None;
    let mut route = None;
    let mut route_out = None;
    let mut expected = Expected::default();
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--seconds" => {
                let v = it.next().ok_or("--seconds needs a number")?;
                seconds = v
                    .parse::<f64>()
                    .ok()
                    .filter(|s| (0.0..=86_400.0).contains(s))
                    .ok_or(format!("--seconds: not a number of seconds: {v}"))?;
            }
            "--script" => {
                script = Some(parse_script(it.next().ok_or("--script needs steps")?)?);
            }
            "--data-dir" => {
                data_dir = Some(PathBuf::from(it.next().ok_or("--data-dir needs a folder")?));
            }
            "--route" => {
                let path = it.next().ok_or("--route needs a file")?;
                let text = std::fs::read_to_string(path).map_err(|e| format!("{path}: {e}"))?;
                route = Some(Route::parse(&text).map_err(|e| format!("{path}: {e}"))?);
            }
            "--route-out" => {
                route_out = Some(PathBuf::from(it.next().ok_or("--route-out needs a file")?));
            }
            "--expect-positions" => {
                expected.positions = Some(
                    it.next()
                        .ok_or("--expect-positions needs a checksum")?
                        .clone(),
                );
            }
            "--expect-footprint" => {
                expected.footprint = Some(
                    it.next()
                        .ok_or("--expect-footprint needs a checksum")?
                        .clone(),
                );
            }
            other if !other.starts_with("--") && address.is_none() => {
                address = Some(other.to_owned());
            }
            other => return Err(format!("unknown option {other}")),
        }
    }
    let address = address.ok_or("give the server's address")?;
    let env = Env::from_process();
    let data_dir = data_dir
        .or_else(|| ruthenium_prepare::default_data_dir(&env))
        .ok_or("no data folder: set --data-dir")?;
    let cache_dir = ruthenium_prepare::default_cache_dir(&env);
    let mut accounts = Accounts::start(Some(data_dir.clone()), cache_dir.as_deref());
    // The account in use signs in again first (for online servers).
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        accounts.poll();
        if !matches!(accounts.status(), AccountStatus::Checking(_)) || Instant::now() > deadline {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let mut settings = SettingsFile::load(SettingsFile::path_in(&data_dir));
    settings.import_mouse(crate::controls::vanilla_mouse);
    let now = Instant::now();
    let mut player = script.map(|steps| Player {
        steps,
        at: 0,
        step_started: now,
        last_report: now,
        mouse: settings.settings.mouse,
        // In the world, the mouse has been grabbed: the first motion is dropped.
        motion: input::MouseMotion::new(),
    });
    let mut scripting = false;
    let mut script_started = false;
    let mut joiner = Joiner::new(Some(data_dir.join("game")), cache_dir);
    let data = joiner.data();
    let session = joiner.join(&address, &accounts)?;
    if let Some(route) = &route {
        return run_route(&session, route, &expected, route_out.as_deref(), &data.lang);
    }
    let started = Instant::now();
    let stay = Duration::from_secs_f64(seconds);
    let mut playing = false;
    let mut leaving = false;
    let mut next_frame = Instant::now();
    loop {
        let mut closed = None;
        for e in session.poll() {
            let t = started.elapsed().as_secs_f64();
            match &e {
                SessionEvent::Stage(s) => println!("{t:7.3}s  {}", stage_text(*s)),
                SessionEvent::LoggedIn { name } => println!("{t:7.3}s  logged in as {name}"),
                SessionEvent::CodeOfConduct(text) => {
                    println!(
                        "{t:7.3}s  code of conduct (declined: answer it in the window):\n{text}"
                    );
                    session.answer_code_of_conduct(false);
                }
                SessionEvent::Playing => {
                    playing = true;
                    println!("{t:7.3}s  playing");
                }
                SessionEvent::Reconfiguring => println!("{t:7.3}s  reconfiguring"),
                SessionEvent::Loaded => {
                    println!("{t:7.3}s  in the world");
                    if let Some(p) = &mut player
                        && !script_started
                    {
                        p.start(&session);
                        scripting = true;
                        script_started = true;
                    }
                }
                SessionEvent::Died {
                    cause,
                    hardcore,
                    score,
                } => {
                    let cause = cause
                        .as_ref()
                        .map_or_else(String::new, |c| format!(": {}", plain_text(c, &data.lang)));
                    let kind = if *hardcore { " (hardcore)" } else { "" };
                    println!("{t:7.3}s  died{kind}{cause} (score {score})");
                }
                SessionEvent::RespawnReady => {
                    println!("{t:7.3}s  respawning");
                    session.respawn();
                }
                SessionEvent::Respawned => println!("{t:7.3}s  respawned"),
                SessionEvent::InBed => println!("{t:7.3}s  in bed"),
                SessionEvent::WokeUp => println!("{t:7.3}s  woke up"),
                SessionEvent::Closed(c) => {
                    let (title, reason) = closed_text(c, &data.lang);
                    println!("{t:7.3}s  closed: {title}: {reason}");
                    closed = Some(c.clone());
                }
            }
        }
        if let Some(c) = closed {
            let ok = playing && (leaving || crate::join::left_on_purpose(&c));
            return Ok(if ok {
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            });
        }
        // A frame: the script's input, then the session's frame (packets, ticks, turn).
        if scripting && let Some(p) = &mut player {
            scripting = p.frame(&session);
        }
        session.frame();
        if !leaving && started.elapsed() >= stay {
            leaving = true;
            session.disconnect();
        }
        next_frame += FRAME;
        let now = Instant::now();
        if next_frame > now {
            std::thread::sleep(next_frame - now);
        } else {
            next_frame = now;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_scripts() {
        let s = parse_script("w+sprint@2000 -@500:12,-3.5 2@100").unwrap();
        assert_eq!(s.len(), 3);
        assert!(s[0].keys.forward && s[0].keys.sprint && !s[0].keys.jump);
        assert_eq!((s[0].ms, s[0].turn), (2000, (0.0, 0.0)));
        assert_eq!((s[1].keys, s[1].turn), (Keys::default(), (12.0, -3.5)));
        assert_eq!(s[2].hotbar, vec![1]);
        let s = parse_script("w+use@500 click+click+swap@50").unwrap();
        assert!(s[0].use_held && s[0].keys.forward);
        assert_eq!((s[1].clicks, s[1].swaps, s[1].use_held), (2, 1, false));
        assert!(parse_script("w@").is_err());
        assert!(parse_script("fly@100").is_err());
    }

    const ROUTE: &str = r#"{
        "format": "ruthenium-route/1", "id": "0123456789abcdef", "name": "walk",
        "start": { "position": [4.5, 71.0, 2.5], "yaw": 0.0, "pitch": 0.0, "flying": false,
                   "chunks": [[1, 0], [0, 0], [0, -1]] },
        "segments": [ { "ticks": 3, "keys": ["forward", "jump"], "turn": [4.0, 0.0] },
                      { "ticks": 2, "keys": [], "turn": [0.0, 0.0] } ],
        "warmupTicks": 1, "captureTicks": 2, "markEvery": 100, "stationary": false
    }"#;

    #[test]
    fn reads_routes() {
        let r = Route::parse(ROUTE).unwrap();
        assert_eq!((r.total(), r.warmup_ticks, r.capture_ticks), (5, 1, 2));
        assert_eq!(r.start_chunks, [(0, -1), (0, 0), (1, 0)]);
        let (keys, turn) = r.input(2);
        assert!(keys.forward && keys.jump && !keys.sprint);
        assert_eq!(turn, (4.0, 0.0));
        assert_eq!(r.input(3), (Keys::default(), (0.0, 0.0)));
        assert_eq!(r.input(99), (Keys::default(), (0.0, 0.0)));
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
}

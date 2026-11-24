//! `cargo xtask bench dedicated`: the world scenes on a vanilla dedicated server, for
//! the vanilla client (frame times, as `bench run` measures them) and for Ruthenium:
//! in its window (`--client ruthenium`: the same frame times, and what its terrain,
//! uploads and main thread did), and without one (`--client headless`, which checks
//! that it joins, follows the route and holds the same chunks).
//!
//! The windowed client runs once per variant: `--variant <label>=<options>` names a
//! set of client options (`--variant mdi=--terrain-draws mdi`), and the variants of a
//! repetition run one after the other, so that they meet the same machine. `--baseline
//! <folder>` names recorded vanilla runs of the same routes: each variant's rows are
//! printed next to them with the ratio, and a route that is not the baseline's fails
//! the task.
//!
//! Every trial gets a folder of its own under `cache/bench/dedicated`: a fresh copy of
//! the world template, a server started on it (see [`crate::server`]) and the route file
//! (see [`crate::routes`]). A benchmark-only controller then fixes the scene over RCON
//! as the integrated-server scenes fix it: time at noon, clear weather, no mob spawning,
//! view and simulation distance 12, and, once the client has joined, the player at the
//! route's start in creative mode (flying, on the fly route). The client plays the route
//! by client tick; the task meanwhile samples the server's process and asks it for its
//! tick times every five seconds. The server is stopped when the client is done, and
//! killed if anything fails first.
//!
//! Results go to `cache/bench/results/<run>` as `bench run` writes them
//! (`<backend>-<scene>-<rep>.json` with the frame intervals and samples), each with the
//! route, the client's record of it, the options, and the server's side: world copy,
//! controller commands, tick times, process samples. The windowed client's are
//! `<variant>-<scene>-<rep>.json`, with its own summary under `ruthenium`; the
//! headless records are `headless-<scene>-<rep>.json`. At the end every record of a scene must be the same
//! run: same positions tick for tick, same chunks at the start and the end.

use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitCode, Stdio};
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::bench::{self, Row};
use crate::game;
use crate::gpu::Gpu;
use crate::routes::{self, Checked, Route};
use crate::server::{self, Server};

const USAGE: &str = "usage: cargo xtask bench dedicated --accept-eula [--client vanilla,headless,ruthenium] [--variant <label>[=<client options>]]... [--baseline <folder>] [--backend vulkan,opengl] [--scene idle,spin,walk,fly,dig] [--reps N] [--headless-reps N] [--warmup S] [--duration S] [--gpu <name>] [--java <java>] [--server-jar <server.jar>] [--bin <ruthenium>] [--data-dir <folder>] [--expect <scene>=<positions>[,<footprint>]]... [--results <folder>] [--compare <folder>] [--timeout S] [--width W] [--height H] [--any-drawable] [--screenshot] [--keep-worlds]";
/// The view and simulation distance of the server and the render distance of the
/// clients, as in the integrated-server scenes.
const VIEW_DISTANCE: i32 = 12;
/// Both clients join under this name, so the server holds the same player for both.
const PLAYER: &str = game::USERNAME;
/// The scoreboard team whose collision rule keeps mobs from pushing the player off
/// its route.
const TEAM: &str = "bench";
const BACKENDS: &[&str] = &["opengl", "vulkan"];
/// The drawable size the performance gates and the integrated-server runs use, in
/// pixels.
const DRAWABLE: (u32, u32) = (1920, 1080);

/// The windowed client's options when no variant is given, under this label.
const DEFAULT_VARIANT: &str = "ruthenium";

#[derive(Debug)]
struct Options {
    vanilla: bool,
    headless: bool,
    /// Run Ruthenium in its window, once for each of these: a label and client options.
    windowed: bool,
    variants: Vec<(String, Vec<String>)>,
    /// Recorded vanilla runs of the same routes to print the windowed runs next to.
    baseline: Option<PathBuf>,
    backends: Vec<String>,
    scenes: Vec<String>,
    reps: u32,
    headless_reps: u32,
    warmup: f64,
    duration: f64,
    gpu: Gpu,
    java: Option<PathBuf>,
    server_jar: Option<PathBuf>,
    binary: Option<PathBuf>,
    data_dir: Option<PathBuf>,
    /// Checksums a scene's records must have: scene, positions, footprint.
    expect: Vec<(String, String, Option<String>)>,
    results: Option<PathBuf>,
    compare: Option<PathBuf>,
    /// Seconds a headless client may take beyond its route's length.
    timeout: f64,
    /// The window's size as the launcher is given it, and whether a drawable of
    /// another size than the gates' is let through.
    width: u32,
    height: u32,
    any_drawable: bool,
    screenshot: bool,
    keep_worlds: bool,
    accept_eula: bool,
}

fn parse(args: &[String]) -> Result<Options, String> {
    let mut o = Options {
        vanilla: true,
        headless: true,
        windowed: false,
        variants: Vec::new(),
        baseline: None,
        backends: vec!["vulkan".into()],
        scenes: routes::SCENES.iter().map(|s| s.to_string()).collect(),
        reps: 3,
        headless_reps: 1,
        warmup: 10.0,
        duration: 60.0,
        gpu: Gpu::Auto,
        java: None,
        server_jar: None,
        binary: None,
        data_dir: None,
        expect: Vec::new(),
        results: None,
        compare: None,
        timeout: 300.0,
        width: DRAWABLE.0,
        height: DRAWABLE.1,
        any_drawable: false,
        screenshot: false,
        keep_worlds: false,
        accept_eula: false,
    };
    let list = |v: &str| v.split(',').map(str::to_owned).collect::<Vec<_>>();
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--accept-eula" => o.accept_eula = true,
            "--screenshot" => o.screenshot = true,
            "--keep-worlds" => o.keep_worlds = true,
            "--any-drawable" => o.any_drawable = true,
            _ => {
                let value = it
                    .next()
                    .ok_or_else(|| format!("{arg} needs a value\n{USAGE}"))?;
                let num = |v: &str| v.parse::<f64>().map_err(|e| format!("{arg}: {e}"));
                match arg.as_str() {
                    "--client" => {
                        let clients = list(value);
                        if let Some(c) = clients
                            .iter()
                            .find(|c| !matches!(c.as_str(), "vanilla" | "headless" | "ruthenium"))
                        {
                            return Err(format!(
                                "unknown client {c:?} (vanilla, headless, ruthenium)"
                            ));
                        }
                        o.vanilla = clients.iter().any(|c| c == "vanilla");
                        o.headless = clients.iter().any(|c| c == "headless");
                        o.windowed = clients.iter().any(|c| c == "ruthenium");
                    }
                    "--variant" => {
                        let (label, options) = value.split_once('=').unwrap_or((value, ""));
                        // A label is a file name's first part and a column's name.
                        let plain = !label.is_empty()
                            && label
                                .chars()
                                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');
                        if !plain
                            || label == "headless"
                            || BACKENDS.contains(&label)
                            || o.variants.iter().any(|v| v.0 == label)
                        {
                            return Err(format!(
                                "--variant: {label:?} is not a new label of lowercase letters, digits and underscores"
                            ));
                        }
                        o.variants.push((
                            label.to_owned(),
                            options.split_whitespace().map(str::to_owned).collect(),
                        ));
                    }
                    "--baseline" => o.baseline = Some(PathBuf::from(value)),
                    "--backend" => o.backends = list(value),
                    "--scene" => o.scenes = list(value),
                    "--reps" => o.reps = (num(value)? as u32).max(1),
                    "--headless-reps" => o.headless_reps = (num(value)? as u32).max(1),
                    "--warmup" => o.warmup = num(value)?,
                    "--duration" => o.duration = num(value)?,
                    "--gpu" => o.gpu = Gpu::parse(value)?,
                    "--java" => o.java = Some(PathBuf::from(value)),
                    "--server-jar" => o.server_jar = Some(PathBuf::from(value)),
                    "--bin" => o.binary = Some(PathBuf::from(value)),
                    "--data-dir" => o.data_dir = Some(PathBuf::from(value)),
                    "--results" => o.results = Some(PathBuf::from(value)),
                    "--compare" => o.compare = Some(PathBuf::from(value)),
                    "--timeout" => o.timeout = num(value)?.max(0.0),
                    "--width" => o.width = num(value)? as u32,
                    "--height" => o.height = num(value)? as u32,
                    "--expect" => {
                        let (scene, sums) = value
                            .split_once('=')
                            .ok_or("--expect takes <scene>=<positions>[,<footprint>]")?;
                        let (positions, footprint) = match sums.split_once(',') {
                            Some((p, f)) => (p, Some(f.to_owned())),
                            None => (sums, None),
                        };
                        o.expect
                            .push((scene.to_owned(), positions.to_owned(), footprint));
                    }
                    other => return Err(format!("unknown argument {other:?}\n{USAGE}")),
                }
            }
        }
    }
    if let Some(b) = o.backends.iter().find(|b| !BACKENDS.contains(&b.as_str())) {
        return Err(format!("unknown backend {b:?} (opengl, vulkan)"));
    }
    if o.variants.is_empty() {
        o.variants.push((DEFAULT_VARIANT.to_owned(), Vec::new()));
    } else if !o.windowed {
        return Err("--variant is for the windowed client: add ruthenium to --client".into());
    }
    let known = |s: &str| routes::SCENES.contains(&s) || s == routes::DIG;
    if let Some(s) = o
        .scenes
        .iter()
        .chain(o.expect.iter().map(|e| &e.0))
        .find(|s| !known(s))
    {
        return Err(format!(
            "unknown scene {s:?} ({}, {})",
            routes::SCENES.join(", "),
            routes::DIG
        ));
    }
    if o.vanilla && o.scenes.iter().any(|s| s == routes::DIG) {
        return Err(format!(
            "the vanilla bench client cannot play the {} scene: leave it or vanilla out",
            routes::DIG
        ));
    }
    Ok(o)
}

/// What every trial of a run shares.
struct Harness<'a> {
    root: &'a Path,
    o: &'a Options,
    game: &'a game::Game,
    server_jar: PathBuf,
    template: PathBuf,
    /// The spawn block of the template and its seed, from `bench setup`.
    spawn: [f64; 3],
    seed: i64,
    results: PathBuf,
    trials: PathBuf,
}

/// A trial under way: its folder, route and server.
struct Trial {
    dir: PathBuf,
    route: Route,
    route_file: PathBuf,
    world: Value,
    server: Server,
    /// Whether the controller has put the player at the route's start.
    placed: bool,
    last_health: Instant,
    health: Vec<Value>,
    samples: Vec<Value>,
}

/// The files under `dir`, relative to it.
fn files(dir: &Path, under: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let path = entry.path();
        if path.is_dir() {
            files(&path, under, out);
        } else if let Ok(rel) = path.strip_prefix(under) {
            out.push(rel.to_owned());
        }
    }
}

/// A world folder's identifier: SHA-256 over every file's path, size and bytes, in
/// path order. Two trials with the same identifier started from the same world.
fn world_id(dir: &Path) -> Result<String, String> {
    let mut names = Vec::new();
    files(dir, dir, &mut names);
    names.sort();
    let mut hash = Sha256::new();
    for name in names {
        let bytes =
            std::fs::read(dir.join(&name)).map_err(|e| format!("{}: {e}", name.display()))?;
        hash.update(name.to_string_lossy().replace('\\', "/").as_bytes());
        hash.update([0]);
        hash.update((bytes.len() as u64).to_le_bytes());
        hash.update(&bytes);
    }
    Ok(hash.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

/// The controller's world setup, before anyone joins: what the integrated-server
/// scenes fix, a spawn at the scene's, the team that switches pushing off, and no
/// random ticks. With them, plants grow while a route runs: in the first recorded
/// baseline a vine had grown into the walk route in one vanilla run of three, and that
/// run climbed it from tick 625 on while the others, and Ruthenium, walked through.
fn fix_world(server: &mut Server, spawn: [f64; 3]) -> Result<(), String> {
    let spawn = format!("setworldspawn {} {} {}", spawn[0], spawn[1], spawn[2]);
    for (command, expect) in [
        ("gamerule advance_time false", "advance_time"),
        ("gamerule advance_weather false", "advance_weather"),
        ("gamerule spawn_mobs false", "spawn_mobs"),
        ("gamerule respawn_radius 0", "respawn_radius"),
        ("gamerule random_tick_speed 0", "random_tick_speed"),
        ("time set 6000", "6000"),
        ("weather clear", "clear"),
        (spawn.as_str(), "spawn point"),
        (&format!("team add {TEAM}"), TEAM),
        (&format!("team modify {TEAM} collisionRule never"), "Never"),
    ] {
        server.command(command, expect)?;
    }
    Ok(())
}

/// The controller's setup of the player once it has joined: on the team, at the route's
/// start, and in creative mode last, which is what the clients wait for. A player
/// switched from spectator to creative in the air stays flying.
fn place_player(server: &mut Server, route: &Route) -> Result<(), String> {
    let [x, y, z] = route.position;
    let teleport = format!("tp {PLAYER} {x} {y} {z} {} {}", route.yaw, route.pitch);
    server.command(&format!("team join {TEAM} {PLAYER}"), "Added")?;
    if route.flying {
        server.command(&format!("gamemode spectator {PLAYER}"), "Spectator")?;
    }
    server.command(&teleport, "Teleported")?;
    server.command(&format!("gamemode creative {PLAYER}"), "Creative")?;
    Ok(())
}

impl Harness<'_> {
    fn route(&self, scene: &str) -> Result<Route, String> {
        Route::new(
            scene,
            self.spawn,
            self.o.warmup,
            self.o.duration,
            VIEW_DISTANCE,
        )
    }

    /// A fresh world copy and a server on it, set up for `scene`.
    fn start_trial(&self, label: &str, scene: &str) -> Result<Trial, String> {
        let route = self.route(scene)?;
        let dir = self.trials.join(label);
        let _ = std::fs::remove_dir_all(&dir);
        let server_dir = dir.join("server");
        let world = server_dir.join("world");
        bench::copy_dir(&self.template, &world)?;
        let id = world_id(&world)?;
        let route_file = dir.join("route.json");
        std::fs::write(&route_file, route.json().to_string())
            .map_err(|e| format!("{}: {e}", route_file.display()))?;
        let runtime = std::path::absolute(bench::bench_dir(self.root).join("server-runtime"))
            .map_err(|e| e.to_string())?;
        let mut server = Server::start(
            &server_dir,
            &server::Settings {
                java: &self.game.java,
                jar: &self.server_jar,
                runtime: &runtime,
                view_distance: VIEW_DISTANCE,
                accept_eula: self.o.accept_eula,
            },
        )?;
        fix_world(&mut server, self.spawn)?;
        Ok(Trial {
            dir,
            route,
            route_file,
            world: json!({
                "id": id,
                "template": self.template.display().to_string(),
                "seed": self.seed,
            }),
            server,
            placed: false,
            last_health: Instant::now(),
            health: Vec::new(),
            samples: Vec::new(),
        })
    }
}

impl Trial {
    /// Twice a second while the client runs: places the player as soon as it is on
    /// the server, samples the server's process, and asks for its tick times every
    /// five seconds (the hundred ticks the server averages over).
    fn watch(&mut self) -> Result<(), String> {
        if !self.server.running() {
            return Err(format!(
                "the server exited during the run:\n{}",
                server::tail(&self.server.log, 15)
            ));
        }
        if !self.placed && self.server.players()?.iter().any(|p| p == PLAYER) {
            place_player(&mut self.server, &self.route)?;
            self.placed = true;
        }
        self.samples.push(self.server.sample());
        if self.last_health.elapsed() >= Duration::from_secs(5) {
            self.last_health = Instant::now();
            self.health.push(self.server.tick_health()?);
        }
        Ok(())
    }

    /// Stops the server and drops the world copy. Returns the server's side of the
    /// trial for the archive.
    fn finish(mut self, harness: &Harness<'_>) -> Result<Value, String> {
        self.health.push(self.server.tick_health()?);
        let record = json!({
            "kind": "dedicated",
            "address": self.server.address(),
            "placement": "same machine as the client, over loopback TCP",
            "jarSha1": server::JAR_SHA1,
            "java": harness.game.java_description,
            "jvmArgs": server::JVM_ARGS,
            "viewDistance": VIEW_DISTANCE,
            "simulationDistance": VIEW_DISTANCE,
            "player": PLAYER,
            "worldCopy": self.world,
            "joinedAt": logged_in_at(&std::fs::read_to_string(&self.server.log).unwrap_or_default()),
            "controller": self.server.commands,
            "tickHealth": self.health,
            "sampleColumns": ["epochMs", "rssKiB", "cpuSeconds"],
            "samples": self.samples,
            "trial": self.dir.display().to_string(),
        });
        self.server.stop()?;
        if !harness.o.keep_worlds {
            let _ = std::fs::remove_dir_all(self.dir.join("server/world"));
        }
        Ok(record)
    }
}

/// Where the server says the player was when it logged in, from the server's log
/// (`<name>[<address>] logged in with entity id <n> at (<x>, <y>, <z>)`).
fn logged_in_at(log: &str) -> Option<[f64; 3]> {
    let line = log
        .lines()
        .find(|l| l.contains(&format!("{PLAYER}[")) && l.contains("logged in with entity id"))?;
    let (_, at) = line.rsplit_once(" at (")?;
    let mut parts = at.trim_end().trim_end_matches(')').split(", ");
    let mut next = || parts.next()?.parse::<f64>().ok();
    Some([next()?, next()?, next()?])
}

/// A client process that is killed if the task lets go of it early.
struct Client(Child);

impl Drop for Client {
    fn drop(&mut self) {
        if matches!(self.0.try_wait(), Ok(None)) {
            let _ = self.0.kill();
        }
        let _ = self.0.wait();
    }
}

impl Harness<'_> {
    /// Fails unless the server put the joining player where the world template has
    /// it: at the centre of the spawn block, which is where the idle, spin and walk
    /// routes start.
    fn check_spawn(&self, label: &str, server: &Value) -> Result<[f64; 3], String> {
        let expected = [self.spawn[0] + 0.5, self.spawn[1], self.spawn[2] + 0.5];
        let at = &server["joinedAt"];
        match (at[0].as_f64(), at[1].as_f64(), at[2].as_f64()) {
            (Some(x), Some(y), Some(z)) if [x, y, z] == expected => Ok(expected),
            (Some(x), Some(y), Some(z)) => Err(format!(
                "{label}: the player joined at {x} {y} {z}, not at the spawn {expected:?}"
            )),
            _ => Err(format!(
                "{label}: the server's log has no login of {PLAYER}"
            )),
        }
    }

    /// One measured run of the vanilla client on a server of its own.
    fn vanilla_run(
        &self,
        client: &bench::Options,
        jar: &Path,
        backend: &str,
        scene: &str,
        rep: u32,
    ) -> Result<(Checked, (u64, u64)), String> {
        let label = format!("{backend}-{scene}-{rep}");
        let mut trial = self.start_trial(&label, scene)?;
        let out = self.results.join(format!("{label}.json"));
        let dedicated = Some((trial.server.address(), trial.route_file.clone()));
        // Quick play adds every server it joins to the client's list; each trial's
        // server is gone afterwards.
        for list in ["servers.dat", "servers.dat_old"] {
            let _ = std::fs::remove_file(bench::bench_dir(self.root).join("game").join(list));
        }
        let mut watch = || trial.watch();
        let mut v = bench::launch(
            self.root,
            self.game,
            jar,
            client,
            &mut bench::Run {
                scene,
                backend,
                out: out.clone(),
                quick_play: false,
                dedicated,
                watch: Some(&mut watch),
            },
        )?;
        let route = trial.route.clone();
        v["server"] = trial.finish(self)?;
        v["route"] = route.json();
        v["settings"] = bench::options(backend, client.render_distance)
            .into_iter()
            .map(|(k, v)| (k.to_owned(), Value::String(v)))
            .collect::<serde_json::Map<_, _>>()
            .into();
        std::fs::write(&out, v.to_string()).map_err(|e| format!("{}: {e}", out.display()))?;
        self.check_spawn(&label, &v["server"])?;
        let checked = routes::check(&route, &v["routeRun"]).map_err(|e| format!("{label}: {e}"))?;
        let s = bench::summarize(&v);
        println!(
            "bench: {backend:<6} {scene:<4} #{rep}: {:.0} fps avg, 1% low {:.0}, 0.1% low {:.0}; server tick {} ms avg; at most {} chunks behind on the move; positions {}, footprint {} ({})",
            s["avgFps"].as_f64().unwrap_or(0.0),
            s["low1Fps"].as_f64().unwrap_or(0.0),
            s["low01Fps"].as_f64().unwrap_or(0.0),
            tick_health(&v).0,
            checked.most_missing,
            checked.positions,
            checked.footprint,
            v["device"]["backend"].as_str().unwrap_or("?"),
        );
        let drawable = (
            v["framebufferWidth"].as_u64().unwrap_or(0),
            v["framebufferHeight"].as_u64().unwrap_or(0),
        );
        Ok((checked, drawable))
    }

    /// One measured run of Ruthenium in its window on a server of its own: the client
    /// plays the route, measures the frames of its capture and fails by itself if the
    /// run isn't the route's. Returns the checked record and the drawable's size.
    fn windowed_run(
        &self,
        binary: &Path,
        data_dir: &Path,
        variant: &(String, Vec<String>),
        scene: &str,
        rep: u32,
    ) -> Result<(Checked, (u64, u64)), String> {
        let (name, options) = variant;
        let label = format!("{name}-{scene}-{rep}");
        let mut trial = self.start_trial(&label, scene)?;
        let record = trial.dir.join("route-run.json");
        let summary = trial.dir.join("bench.json");
        let log_path = trial.dir.join("client.log");
        let log =
            std::fs::File::create(&log_path).map_err(|e| format!("{}: {e}", log_path.display()))?;
        let mut command = Command::new(binary);
        command
            .arg("--data-dir")
            .arg(data_dir)
            .arg("--join")
            .arg(trial.server.address())
            .arg("--window-size")
            .arg(format!("{}x{}", self.o.width, self.o.height))
            .args(["--no-vsync", "--no-validation"])
            .arg("--route")
            .arg(&trial.route_file)
            .arg("--route-out")
            .arg(&record)
            .arg("--bench-out")
            .arg(&summary)
            .args(options);
        if self.o.screenshot {
            // The frame after the route's end, next to the run's file.
            command
                .arg("--screenshot")
                .arg(self.results.join(format!("{label}.png")));
        }
        if let Some((_, positions, footprint)) = self.o.expect.iter().find(|e| e.0 == scene) {
            command.args(["--expect-positions", positions]);
            if let Some(f) = footprint {
                command.args(["--expect-footprint", f]);
            }
        }
        for (key, value) in self.o.gpu.env()? {
            match value {
                Some(v) => command.env(key, v),
                None => command.env_remove(key),
            };
        }
        let child = command
            .env("RUTHENIUM_OFFLINE_NAME", PLAYER)
            .current_dir(self.root)
            .stdin(Stdio::null())
            .stdout(log.try_clone().map_err(|e| e.to_string())?)
            .stderr(log)
            .spawn()
            .map_err(|e| format!("running {}: {e}", binary.display()))?;
        let mut client = Client(child);
        let pid = client.0.id();
        let limit = Duration::from_secs_f64(trial.route.total() as f64 / 20.0 + self.o.timeout);
        let started = Instant::now();
        let mut samples = Vec::new();
        let encoding_before = video_encoding();
        let status = loop {
            if let Some(status) = client.0.try_wait().map_err(|e| e.to_string())? {
                break status;
            }
            if started.elapsed() > limit {
                return Err(format!("{label} timed out; see {}", log_path.display()));
            }
            trial.watch()?;
            // The columns of `bench run`'s samples.
            let g = self.o.gpu.sample(pid);
            samples.push(json!([
                bench::epoch_ms(),
                bench::rss_kib(pid),
                g.mem_mib,
                bench::cpu_temp_c(),
                bench::cpu_mhz(),
                g.temp_c,
                g.reasons,
                g.clock_mhz
            ]));
            std::thread::sleep(Duration::from_millis(500));
        };
        let read = |path: &Path| -> Value {
            std::fs::read_to_string(path)
                .ok()
                .and_then(|t| serde_json::from_str(&t).ok())
                .unwrap_or(Value::Null)
        };
        let run = read(&record);
        let mut own = read(&summary);
        let route = trial.route.clone();
        let server = trial.finish(self)?;
        let intervals = own["frame_intervals_ns"].take();
        // The client's own copy of the record is the one archived as `routeRun`.
        own["route_run"] = Value::Null;
        let markers = &run["markers"];
        let v = json!({
            "format": "ruthenium-bench-windowed/1",
            "client": "ruthenium",
            "scene": scene,
            "variant": { "label": name, "options": options },
            "binary": binary.display().to_string(),
            "exit": status.to_string(),
            // Someone watching the desktop through a streaming host takes GPU time.
            "videoEncoding": { "before": encoding_before, "after": video_encoding() },
            "backendRequested": name,
            "gpuRequested": self.o.gpu.name(),
            "device": {
                "backend": "Ruthenium, Vulkan",
                "name": own["gpu"],
                "driver": own["device"]["driver"],
                "vendor": vendor_name(own["device"]["vendor_id"].as_u64()),
            },
            "framebufferWidth": own["swapchain"]["width"],
            "framebufferHeight": own["swapchain"]["height"],
            "renderDistance": VIEW_DISTANCE,
            "frameIntervalsNs": intervals,
            "jvmStartEpochMs": own["started_epoch_ms"],
            "measureStartEpochMs": markers["captureStartEpochMs"],
            "measureEndEpochMs": markers["captureEndEpochMs"],
            "worldReadyEpochMs": markers["sceneReadyEpochMs"],
            "samples": samples,
            "routeRun": run,
            "route": route.json(),
            "server": server,
            "ruthenium": own,
        });
        let out = self.results.join(format!("{label}.json"));
        std::fs::write(&out, v.to_string()).map_err(|e| format!("{}: {e}", out.display()))?;
        let output = std::fs::read_to_string(&log_path).unwrap_or_default();
        if output.contains("no block physics") {
            return Err(format!(
                "{label}: the data folder {} has no block physics, so the player can't move: run `cargo xtask exporters` and `ruthenium prepare --data-dir` on it",
                data_dir.display()
            ));
        }
        if !status.success() {
            let last: Vec<&str> = output.lines().rev().take(6).collect();
            return Err(format!(
                "{label}: ruthenium failed ({status}); see {}:\n{}",
                log_path.display(),
                last.into_iter().rev().collect::<Vec<_>>().join("\n")
            ));
        }
        let (vendor, device) = (
            v["device"]["vendor"].as_str().unwrap_or(""),
            v["device"]["name"].as_str().unwrap_or(""),
        );
        if !self.o.gpu.matches(vendor, device) {
            return Err(format!(
                "{label}: asked for the {} GPU, but the client used {device:?} ({vendor})",
                self.o.gpu.name()
            ));
        }
        self.check_spawn(&label, &v["server"])?;
        let checked = routes::check(&route, &v["routeRun"]).map_err(|e| format!("{label}: {e}"))?;
        left_out(&v).map_err(|e| format!("{label}: {e}"))?;
        let s = bench::summarize(&v);
        let phase = |name: &str, k: &str| {
            v["ruthenium"]["phases"][name][k]
                .as_f64()
                .unwrap_or(f64::NAN)
        };
        let terrain = &v["ruthenium"]["world"]["at_capture_end"]["terrain"];
        println!(
            "bench: {name:<10} {scene:<4} #{rep}: {:.0} fps avg, 1% low {:.0}, 0.1% low {:.0}; main thread {:.3} ms a frame (P99 {:.3}), GPU {:.3} ms; {} quads in {} draws; pages {:.0} MiB; server tick {} ms avg; positions {}, footprint {}",
            s["avgFps"].as_f64().unwrap_or(0.0),
            s["low1Fps"].as_f64().unwrap_or(0.0),
            s["low01Fps"].as_f64().unwrap_or(0.0),
            phase("work", "p50"),
            phase("work", "p99"),
            v["ruthenium"]["gpu_time"]["p50"]
                .as_f64()
                .unwrap_or(f64::NAN),
            terrain["quads"].as_u64().unwrap_or(0),
            terrain["draws"].as_u64().unwrap_or(0),
            terrain["pool"]["reserved_bytes"]
                .as_f64()
                .unwrap_or(f64::NAN)
                / f64::from(1 << 20),
            tick_health(&v).0,
            checked.positions,
            checked.footprint,
        );
        let drawable = (
            v["framebufferWidth"].as_u64().unwrap_or(0),
            v["framebufferHeight"].as_u64().unwrap_or(0),
        );
        Ok((checked, drawable))
    }

    /// One headless run of Ruthenium on a server of its own: `ruthenium connect` with
    /// the route, which fails by itself if the run isn't the route's.
    fn headless_run(
        &self,
        binary: &Path,
        data_dir: &Path,
        scene: &str,
        rep: u32,
    ) -> Result<Checked, String> {
        let label = format!("headless-{scene}-{rep}");
        let mut trial = self.start_trial(&label, scene)?;
        let record = trial.dir.join("route-run.json");
        let log_path = trial.dir.join("client.log");
        let log =
            std::fs::File::create(&log_path).map_err(|e| format!("{}: {e}", log_path.display()))?;
        let mut command = Command::new(binary);
        command
            .arg("connect")
            .arg(trial.server.address())
            .arg("--data-dir")
            .arg(data_dir)
            .arg("--route")
            .arg(&trial.route_file)
            .arg("--route-out")
            .arg(&record);
        if let Some((_, positions, footprint)) = self.o.expect.iter().find(|e| e.0 == scene) {
            command.args(["--expect-positions", positions]);
            if let Some(f) = footprint {
                command.args(["--expect-footprint", f]);
            }
        }
        let child = command
            .env("RUTHENIUM_OFFLINE_NAME", PLAYER)
            .current_dir(self.root)
            .stdin(Stdio::null())
            .stdout(log.try_clone().map_err(|e| e.to_string())?)
            .stderr(log)
            .spawn()
            .map_err(|e| format!("running {}: {e}", binary.display()))?;
        let mut client = Client(child);
        let limit = Duration::from_secs_f64(trial.route.total() as f64 / 20.0 + self.o.timeout);
        let started = Instant::now();
        let status = loop {
            if let Some(status) = client.0.try_wait().map_err(|e| e.to_string())? {
                break status;
            }
            if started.elapsed() > limit {
                return Err(format!("{label} timed out; see {}", log_path.display()));
            }
            trial.watch()?;
            std::thread::sleep(Duration::from_millis(500));
        };
        let run: Value = std::fs::read_to_string(&record)
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or(Value::Null);
        let route = trial.route.clone();
        let server = trial.finish(self)?;
        let out = self.results.join(format!("{label}.json"));
        let archived = json!({
            "format": "ruthenium-bench-headless/1",
            "client": "ruthenium",
            "scene": scene,
            "binary": binary.display().to_string(),
            "exit": status.to_string(),
            "routeRun": run,
            "route": route.json(),
            "server": server,
        });
        std::fs::write(&out, archived.to_string())
            .map_err(|e| format!("{}: {e}", out.display()))?;
        let output = std::fs::read_to_string(&log_path).unwrap_or_default();
        if output.contains("no block physics") {
            return Err(format!(
                "{label}: the data folder {} has no block physics, so the player can't move: run `cargo xtask exporters` and `ruthenium prepare --data-dir` on it",
                data_dir.display()
            ));
        }
        if !status.success() {
            let last: Vec<&str> = output.lines().rev().take(4).collect();
            return Err(format!(
                "{label}: ruthenium connect failed ({status}); see {}:\n{}",
                log_path.display(),
                last.into_iter().rev().collect::<Vec<_>>().join("\n")
            ));
        }
        let joined = self.check_spawn(&label, &archived["server"])?;
        let checked =
            routes::check(&route, &archived["routeRun"]).map_err(|e| format!("{label}: {e}"))?;
        println!(
            "bench: headless {scene:<4} #{rep}: joined at {:?}, {} ticks, {} chunks at the start and {} at the end, at most {} behind on the move; positions {}, footprint {}",
            joined,
            route.total(),
            checked.start_chunks,
            checked.end_chunks,
            checked.most_missing,
            checked.positions,
            checked.footprint,
        );
        Ok(checked)
    }
}

/// The NVIDIA GPU's video encoding sessions now and their average frame rate (none
/// without `nvidia-smi`): a desktop streamed to someone shares the GPU with the run.
fn video_encoding() -> Value {
    let out = Command::new("nvidia-smi")
        .args([
            "--query-gpu=encoder.stats.sessionCount,encoder.stats.averageFps",
            "--format=csv,noheader,nounits",
        ])
        .stderr(Stdio::null())
        .output();
    let Ok(out) = out else {
        return Value::Null;
    };
    let text = String::from_utf8_lossy(&out.stdout);
    let mut numbers = text
        .lines()
        .next()
        .unwrap_or("")
        .split(',')
        .map(|n| n.trim().parse::<f64>().ok());
    match (numbers.next().flatten(), numbers.next().flatten()) {
        (Some(sessions), Some(fps)) => json!({ "sessions": sessions, "averageFps": fps }),
        _ => Value::Null,
    }
}

/// A PCI vendor id as the bench mod names GPU vendors, for [`Gpu::matches`].
fn vendor_name(id: Option<u64>) -> &'static str {
    match id {
        Some(0x10de) => "NVIDIA",
        Some(0x8086) => "Intel",
        Some(0x1002) => "AMD",
        Some(0x106b) => "Apple",
        _ => "",
    }
}

/// Fails if a windowed run left out part of the scene: a mesh that found no room in the
/// terrain's memory, sections a frame's data had no room for, or frames held back
/// because the window was hidden. Such a run drew less than it was measured for.
fn left_out(v: &Value) -> Result<(), String> {
    let world = &v["ruthenium"]["world"];
    let (start, end) = (
        &world["at_capture_start"]["terrain"],
        &world["at_capture_end"]["terrain"],
    );
    if !end.is_object() {
        return Err("the client recorded no terrain at the capture's end".into());
    }
    let during = |key: &str| {
        end[key]
            .as_u64()
            .unwrap_or(0)
            .saturating_sub(start[key].as_u64().unwrap_or(0))
    };
    let (dropped, overflowed) = (
        end["meshes_dropped"].as_u64().unwrap_or(0),
        during("sections_overflowed"),
    );
    let throttled = v["ruthenium"]["throttled_frames"].as_u64().unwrap_or(0);
    if dropped > 0 || overflowed > 0 || throttled > 0 {
        return Err(format!(
            "the run drew less than its scene: {dropped} meshes found no room in the terrain's memory, {overflowed} sections were left out of frames for want of frame data, {throttled} frames were held back behind a hidden window"
        ));
    }
    Ok(())
}

/// The Ruthenium binary (built with the development join path unless given: optimised
/// for the windowed runs, which are measured) and its data folder, prepared from the
/// player's install.
fn ruthenium(root: &Path, o: &Options) -> Result<(PathBuf, PathBuf), String> {
    let run = |command: &mut Command, what: &str| {
        let status = command.status().map_err(|e| format!("{what}: {e}"))?;
        if status.success() {
            Ok(())
        } else {
            Err(format!("{what} exited with {status}"))
        }
    };
    let binary = match &o.binary {
        Some(b) => std::path::absolute(b).map_err(|e| e.to_string())?,
        None => {
            let mut build = Command::new(env!("CARGO"));
            build
                .current_dir(root)
                .args(["build", "-p", "ruthenium", "--features", "dev-offline"]);
            if o.windowed {
                build.arg("--release");
            }
            run(&mut build, "cargo build")?;
            root.join(if o.windowed {
                "target/release"
            } else {
                "target/debug"
            })
            .join(format!("ruthenium{}", std::env::consts::EXE_SUFFIX))
        }
    };
    let data_dir = match &o.data_dir {
        Some(d) => std::path::absolute(d).map_err(|e| e.to_string())?,
        None => bench::bench_dir(root).join("ruthenium"),
    };
    // The player moves by the exported block physics: bring the folder up to date.
    if crate::exporters::run(root, &[]) != ExitCode::SUCCESS {
        return Err("building the exporters failed".into());
    }
    let mut prepare = Command::new(&binary);
    prepare
        .current_dir(root)
        .arg("prepare")
        .arg("--data-dir")
        .arg(&data_dir)
        .arg("--exporters")
        .arg(root.join("target/java/ruthenium-exporters.jar"));
    if let Some(java) = &o.java {
        prepare.arg("--java").arg(java);
    }
    run(&mut prepare, "ruthenium prepare")?;
    Ok((binary, data_dir))
}

pub fn run(root: &Path, args: &[String]) -> Result<(), String> {
    let o = parse(args)?;
    if !o.accept_eula {
        return Err(format!(
            "this task starts Mojang's dedicated server, which needs their EULA accepted (https://aka.ms/MinecraftEULA). Read it, then pass --accept-eula; nothing was started.\n{USAGE}"
        ));
    }
    let template = bench::bench_dir(root).join("world");
    let setup = std::fs::read_to_string(bench::bench_dir(root).join("setup.json"))
        .ok()
        .and_then(|t| serde_json::from_str::<Value>(&t).ok());
    let (Some(setup), true) = (setup, template.is_dir()) else {
        return Err("no benchmark world: run `cargo xtask bench setup` first".into());
    };
    let spawn = &setup["spawn"];
    let (Some(x), Some(y), Some(z)) = (spawn[0].as_f64(), spawn[1].as_f64(), spawn[2].as_f64())
    else {
        return Err(
            "cache/bench/setup.json has no spawn: run `cargo xtask bench setup` again".into(),
        );
    };
    let client = bench::Options {
        backends: o.backends.clone(),
        scenes: o.scenes.clone(),
        reps: o.reps,
        warmup: o.warmup,
        duration: o.duration,
        width: o.width,
        height: o.height,
        seed: setup["seed"].as_i64().unwrap_or(0),
        java: o.java.clone(),
        results: o.results.clone(),
        screenshot: o.screenshot,
        render_distance: VIEW_DISTANCE as u32,
        gpu: o.gpu,
    };
    let server_jar = server::find_jar(root, o.server_jar.as_deref())?;
    let (game, jar) = if o.vanilla {
        let (game, jar) = bench::prepare(root, &client)?;
        (game, Some(jar))
    } else {
        (game::locate(o.java.as_deref())?, None)
    };
    let ours = if o.headless || o.windowed {
        Some(ruthenium(root, &o)?)
    } else {
        None
    };

    let results = o.results.clone().unwrap_or_else(|| {
        bench::bench_dir(root)
            .join("results")
            .join(format!("{}", bench::epoch_ms() / 1000))
    });
    std::fs::create_dir_all(&results).map_err(|e| format!("{}: {e}", results.display()))?;
    let name = results
        .file_name()
        .map_or_else(|| "run".into(), |n| n.to_owned());
    let harness = Harness {
        root,
        o: &o,
        game: &game,
        server_jar,
        template,
        spawn: [x, y, z],
        seed: client.seed,
        results: results.clone(),
        trials: bench::bench_dir(root).join("dedicated").join(name),
    };
    let mut machine = bench::machine(&game, &client);
    machine["dedicated"] = json!({
        "server": "vanilla 26.3 dedicated server, same machine, loopback",
        "serverJarSha1": server::JAR_SHA1,
        "serverJvmArgs": server::JVM_ARGS,
        "viewDistance": VIEW_DISTANCE,
        "warmupSeconds": o.warmup,
        "captureSeconds": o.duration,
        "trials": harness.trials.display().to_string(),
    });
    std::fs::write(results.join("machine.json"), machine.to_string()).map_err(|e| e.to_string())?;

    // Every record of a scene, to compare at the end.
    let mut records: Vec<(String, String, Checked)> = Vec::new();
    let mut wrong = Vec::new();
    if let Some(jar) = &jar {
        for rep in 1..=o.reps {
            for backend in &o.backends {
                for scene in &o.scenes {
                    let (checked, drawable) =
                        harness.vanilla_run(&client, jar, backend, scene, rep)?;
                    records.push((scene.clone(), format!("{backend} #{rep}"), checked));
                    let expected = (u64::from(DRAWABLE.0), u64::from(DRAWABLE.1));
                    if drawable != expected && !o.any_drawable {
                        wrong.push(format!(
                            "{backend} {scene} #{rep} drew {} x {} pixels, not the {} x {} of the gates and the integrated-server runs: the desktop scales the window. Set the display scale to 1, or pass --width and --height that come to that many pixels (or --any-drawable to let it through)",
                            drawable.0, drawable.1, DRAWABLE.0, DRAWABLE.1
                        ));
                    }
                }
            }
        }
    }
    let expected = (u64::from(DRAWABLE.0), u64::from(DRAWABLE.1));
    if let Some((binary, data_dir)) = ours.as_ref().filter(|_| o.windowed) {
        // A repetition's variants one after the other, scene by scene: what the
        // machine does over the minutes is spread over all of them.
        for rep in 1..=o.reps {
            for scene in &o.scenes {
                for variant in &o.variants {
                    let (checked, drawable) =
                        harness.windowed_run(binary, data_dir, variant, scene, rep)?;
                    records.push((scene.clone(), format!("{} #{rep}", variant.0), checked));
                    if drawable != expected && !o.any_drawable {
                        wrong.push(format!(
                            "{} {scene} #{rep} drew {} x {} pixels, not the {} x {} of the gates and the baseline: the desktop scales the window. Set the display scale to 1, or pass --width and --height that come to that many pixels (or --any-drawable to let it through)",
                            variant.0, drawable.0, drawable.1, DRAWABLE.0, DRAWABLE.1
                        ));
                    }
                }
            }
        }
    }
    if let Some((binary, data_dir)) = ours.as_ref().filter(|_| o.headless) {
        for rep in 1..=o.headless_reps {
            for scene in &o.scenes {
                let checked = harness.headless_run(binary, data_dir, scene, rep)?;
                records.push((scene.clone(), format!("headless #{rep}"), checked));
            }
        }
    }
    // The baseline's runs of the same scenes must be the same routes, tick for tick.
    if let Some(dir) = &o.baseline {
        for scene in &o.scenes {
            let route = harness.route(scene)?;
            let Some(first) = records.iter().find(|r| &r.0 == scene) else {
                continue;
            };
            let rows = bench::load_rows(dir)?;
            let Some(row) = rows.iter().find(|r| &r.1 == scene) else {
                wrong.push(format!(
                    "{scene}: the baseline {} has no run of it",
                    dir.display()
                ));
                continue;
            };
            let same = routes::check(&route, &row.4["routeRun"])
                .and_then(|theirs| routes::compare(&first.2, &theirs));
            if let Err(e) = same {
                wrong.push(format!(
                    "{scene}: {} is not the run the baseline {} measured ({} {scene} #{}): {e}",
                    first.1,
                    dir.display(),
                    row.0,
                    row.2
                ));
            }
        }
    }

    for scene in &o.scenes {
        let of_scene: Vec<&(String, String, Checked)> =
            records.iter().filter(|r| &r.0 == scene).collect();
        let Some(first) = of_scene.first() else {
            continue;
        };
        for other in &of_scene[1..] {
            if let Err(e) = routes::compare(&first.2, &other.2) {
                wrong.push(format!(
                    "{scene}: {} and {} are not the same run: {e}",
                    first.1, other.1
                ));
            }
        }
        for (_, positions, footprint) in o.expect.iter().filter(|e| &e.0 == scene) {
            for (_, label, c) in &of_scene {
                if &c.positions != positions
                    || footprint.as_ref().is_some_and(|f| f != &c.footprint)
                {
                    wrong.push(format!(
                        "{scene}: {label} has positions {} and footprint {}, expected {positions}{}",
                        c.positions,
                        c.footprint,
                        footprint.as_ref().map_or_else(String::new, |f| format!(" and {f}"))
                    ));
                }
            }
        }
        if wrong.is_empty() {
            println!(
                "bench: {scene}: route {}, positions {}, footprint {} in all of {}",
                harness.route(scene)?.id,
                first.2.positions,
                first.2.footprint,
                of_scene
                    .iter()
                    .map(|r| r.1.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            );
        }
    }
    println!("bench: results in {}", results.display());
    bench::report(root, Some(results), o.compare.clone(), o.baseline.clone())?;
    if wrong.is_empty() {
        Ok(())
    } else {
        Err(wrong.join("\n"))
    }
}

/// Whether a results folder holds dedicated-server runs.
pub(crate) fn is_dedicated(dir: &Path) -> bool {
    std::fs::read_to_string(dir.join("machine.json"))
        .ok()
        .and_then(|t| serde_json::from_str::<Value>(&t).ok())
        .is_some_and(|m| m["dedicated"].is_object())
}

/// The server's tick times over a run's capture: the mean of its averages and the
/// highest 99th percentile, in milliseconds, as text; and the server's CPU use as a
/// percentage of one core. Each answer covers the five seconds before it.
fn tick_health(v: &Value) -> (String, String, String) {
    let markers = &v["routeRun"]["markers"];
    let (start, end) = (
        markers["captureStartEpochMs"].as_f64().unwrap_or(0.0),
        markers["captureEndEpochMs"].as_f64().unwrap_or(0.0),
    );
    let within = |list: &Value, from: f64, to: f64| -> Vec<Value> {
        list.as_array()
            .map(|a| {
                a.iter()
                    .filter(|s| {
                        let t = s["epochMs"]
                            .as_f64()
                            .or_else(|| s[0].as_f64())
                            .unwrap_or(-1.0);
                        t >= from && t <= to
                    })
                    .cloned()
                    .collect()
            })
            .unwrap_or_default()
    };
    let health = within(&v["server"]["tickHealth"], start, end + 5000.0);
    let averages: Vec<f64> = health
        .iter()
        .filter_map(|h| h["averageMs"].as_f64())
        .collect();
    let worst = health
        .iter()
        .filter_map(|h| h["p99Ms"].as_f64())
        .fold(f64::NAN, f64::max);
    let average = averages.iter().sum::<f64>() / averages.len() as f64;
    let samples = within(&v["server"]["samples"], start, end);
    let cpu = match (samples.first(), samples.last()) {
        (Some(a), Some(b)) if b[0].as_f64() > a[0].as_f64() => {
            let seconds = (b[0].as_f64().unwrap_or(0.0) - a[0].as_f64().unwrap_or(0.0)) / 1000.0;
            match (a[2].as_f64(), b[2].as_f64()) {
                (Some(a), Some(b)) => format!("{:.0}", (b - a) / seconds * 100.0),
                _ => "–".into(),
            }
        }
        _ => "–".into(),
    };
    let ms = |x: f64| {
        if x.is_nan() {
            "–".into()
        } else {
            format!("{x:.1}")
        }
    };
    (ms(average), ms(worst), cpu)
}

/// Prints a folder of dedicated-server runs as Markdown: each backend and scene's
/// median next to the integrated-server runs of `compare` (by default the newest other
/// results folder that has any), then what identifies each scene's runs and how the
/// server fared, then every run.
pub(crate) fn report(
    root: &Path,
    dir: &Path,
    rows: &[Row],
    compare: Option<PathBuf>,
    baseline: Option<PathBuf>,
) -> Result<(), String> {
    let scene_order = |s: &str| routes::SCENES.iter().position(|x| *x == s).unwrap_or(9);
    let mut rows: Vec<&Row> = rows.iter().collect();
    rows.sort_by_key(|r| (r.0.clone(), scene_order(&r.1), r.2));
    let (compared, integrated) = match compare {
        Some(c) => {
            let rows = bench::load_rows(&c)?;
            (Some(c), rows)
        }
        None => bench::result_dirs(root)?
            .into_iter()
            .rev()
            .filter(|d| d.as_path() != dir && !is_dedicated(d))
            .find_map(|d| {
                let rows = bench::load_rows(&d).ok().filter(|r| !r.is_empty())?;
                Some((Some(d), rows))
            })
            .unwrap_or((None, Vec::new())),
    };
    let f = |v: &Value, k: &str| v[k].as_f64().unwrap_or(f64::NAN);

    let mut devices: Vec<String> = rows
        .iter()
        .map(|r| {
            let d = &r.4["device"];
            format!(
                "{} → {} ({})",
                r.0,
                d["name"].as_str().unwrap_or("?"),
                d["driver"].as_str().unwrap_or("?")
            )
        })
        .collect();
    devices.sort();
    devices.dedup();
    if !devices.is_empty() {
        println!("\nDevices: {}", devices.join("; "));
    }
    println!(
        "\nDedicated-server runs: a vanilla 26.3 server on the same machine over loopback, scenes played as routes keyed by client tick."
    );
    match &compared {
        Some(c) => println!(
            "Integrated-server rows are the runs in {}: there the server runs inside the client and the scenes are driven by time. The two protocols differ, so a ratio between a dedicated and an integrated row has to say so.",
            c.display()
        ),
        None => println!("No integrated-server runs were found to print next to them."),
    }
    if !rows.is_empty() {
        println!(
            "\n| Server | Backend | Scene | Avg FPS | Median ms | 1% low FPS | 0.1% low FPS | Max ms | RSS MiB (mean / max) | VRAM MiB (mean / max) | CPU / GPU °C max | Runs |"
        );
        println!("|---|---|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|");
    }
    let line = |server: &str, backend: &str, scene: &str, group: &[&Row]| {
        let m = |k: &str| bench::median(group.iter().map(|r| f(&r.3, k)).collect());
        let actual = group
            .first()
            .and_then(|r| r.4["device"]["backend"].as_str())
            .unwrap_or("?");
        println!(
            "| {server} | {backend} ({actual}) | {scene} | {:.0} | {:.2} | {:.0} | {:.0} | {:.1} | {:.0} / {:.0} | {:.0} / {:.0} | {:.0} / {} | {} |",
            m("avgFps"),
            m("medianMs"),
            m("low1Fps"),
            m("low01Fps"),
            m("maxMs"),
            m("rssMeanMiB"),
            m("rssMaxMiB"),
            m("vramMeanMiB"),
            m("vramMaxMiB"),
            m("cpuTempMaxC"),
            bench::dash(m("gpuTempMaxC")),
            group.len()
        );
    };
    let mut groups: Vec<(&str, &str)> = rows.iter().map(|r| (r.0.as_str(), r.1.as_str())).collect();
    groups.dedup();
    for (backend, scene) in &groups {
        let group: Vec<&Row> = rows
            .iter()
            .copied()
            .filter(|r| r.0 == *backend && r.1 == *scene)
            .collect();
        line("dedicated", backend, scene, &group);
        let other: Vec<&Row> = integrated
            .iter()
            .filter(|r| r.0 == *backend && r.1 == *scene)
            .collect();
        if !other.is_empty() {
            line("integrated", backend, scene, &other);
        }
    }

    // Ruthenium's headless records of the same routes.
    let mut headless: Vec<(String, u32, Value)> = Vec::new();
    for entry in std::fs::read_dir(dir)
        .map_err(|e| format!("{}: {e}", dir.display()))?
        .flatten()
    {
        let name = entry.file_name().to_string_lossy().into_owned();
        let Some(stem) = name
            .strip_suffix(".json")
            .and_then(|s| s.strip_prefix("headless-"))
        else {
            continue;
        };
        let Some((scene, rep)) = stem.rsplit_once('-') else {
            continue;
        };
        let text = std::fs::read_to_string(entry.path()).map_err(|e| e.to_string())?;
        let v: Value = serde_json::from_str(&text).map_err(|e| format!("{name}: {e}"))?;
        headless.push((scene.to_owned(), rep.parse().unwrap_or(0), v));
    }
    headless.sort_by_key(|h| (scene_order(&h.0), h.1));

    println!(
        "\n| Scene | Route | World copy | Positions | Footprint | Chunks (start / end) | Runs with these |"
    );
    println!("|---|---|---|---|---|---:|---|");
    let mut scenes: Vec<&str> = rows
        .iter()
        .map(|r| r.1.as_str())
        .chain(headless.iter().map(|h| h.0.as_str()))
        .collect();
    scenes.sort_by_key(|s| scene_order(s));
    scenes.dedup();
    for scene in scenes {
        // What identifies a run, and the runs that share it.
        let mut kinds: Vec<([String; 5], Vec<String>)> = Vec::new();
        let runs = rows
            .iter()
            .filter(|r| r.1 == scene)
            .map(|r| (format!("{} #{}", r.0, r.2), &r.4))
            .chain(
                headless
                    .iter()
                    .filter(|h| h.0 == scene)
                    .map(|h| (format!("headless #{}", h.1), &h.2)),
            );
        for (label, v) in runs {
            let run = &v["routeRun"];
            let text = |v: &Value| v.as_str().unwrap_or("–").to_owned();
            let chunks = |mark: Option<&Value>| {
                mark.and_then(|m| m["chunks"].as_array())
                    .map_or(0, Vec::len)
            };
            let marks = run["marks"].as_array();
            let short = |s: String| s.chars().take(16).collect::<String>();
            let key = [
                text(&v["route"]["id"]),
                short(text(&v["server"]["worldCopy"]["id"])),
                text(&run["checksums"]["positions"]),
                text(&run["checksums"]["footprint"]),
                format!(
                    "{} / {}",
                    chunks(marks.and_then(|m| m.first())),
                    chunks(marks.and_then(|m| m.last()))
                ),
            ];
            match kinds.iter_mut().find(|k| k.0 == key) {
                Some(k) => k.1.push(label),
                None => kinds.push((key, vec![label])),
            }
        }
        let differ = if kinds.len() > 1 {
            " (the runs differ)"
        } else {
            ""
        };
        for (key, labels) in &kinds {
            println!(
                "| {scene}{differ} | {} | {} | {} | {} | {} | {} |",
                key[0],
                key[1],
                key[2],
                key[3],
                key[4],
                labels.join(", ")
            );
        }
    }

    println!(
        "\n| Run | Avg FPS | 1% low | 0.1% low | Max ms | RSS mean MiB | VRAM mean MiB | Scene ready s | Server tick ms (avg / worst P99) | Server CPU % of a core | Video encoding sessions (before / after) |"
    );
    println!("|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|");
    for (backend, scene, rep, s, v) in rows.iter().copied() {
        let (average, worst, cpu) = tick_health(v);
        let ready = v["routeRun"]["markers"]["sceneReadyEpochMs"]
            .as_f64()
            .unwrap_or(f64::NAN)
            - v["jvmStartEpochMs"].as_f64().unwrap_or(f64::NAN);
        let encoding = |when: &str| {
            v["videoEncoding"][when]["sessions"]
                .as_f64()
                .map_or_else(|| "–".to_owned(), |n| format!("{n:.0}"))
        };
        println!(
            "| {backend} {scene} #{rep} | {:.0} | {:.0} | {:.0} | {:.1} | {:.0} | {:.0} | {:.1} | {average} / {worst} | {cpu} | {} / {} |",
            f(s, "avgFps"),
            f(s, "low1Fps"),
            f(s, "low01Fps"),
            f(s, "maxMs"),
            f(s, "rssMeanMiB"),
            f(s, "vramMeanMiB"),
            ready / 1000.0,
            encoding("before"),
            encoding("after"),
        );
    }
    for (scene, rep, v) in &headless {
        let (average, worst, cpu) = tick_health(v);
        println!(
            "| headless {scene} #{rep} | – | – | – | – | – | – | – | {average} / {worst} | {cpu} | – |"
        );
    }
    windowed_report(&rows, baseline.as_deref())?;
    println!("\n(results: {})", dir.display());
    Ok(())
}

/// A number deep in a run's file, or NaN.
fn at(v: &Value, path: &[&str]) -> f64 {
    path.iter()
        .fold(v, |v, key| &v[*key])
        .as_f64()
        .unwrap_or(f64::NAN)
}

/// Ruthenium's windowed runs: each variant and scene's median next to the baseline's
/// vanilla runs of the same route, where the main thread's and the GPU's time went,
/// what was drawn and what it took, and how long changes took to show.
fn windowed_report(rows: &[&Row], baseline: Option<&Path>) -> Result<(), String> {
    let ours: Vec<&Row> = rows
        .iter()
        .copied()
        .filter(|r| r.4["client"] == "ruthenium")
        .collect();
    if ours.is_empty() {
        return Ok(());
    }
    let theirs = match baseline {
        Some(dir) => bench::load_rows(dir)?
            .into_iter()
            .filter(|r| r.4["client"] != "ruthenium")
            .collect(),
        None => Vec::new(),
    };
    let mut groups: Vec<(&str, &str)> = ours.iter().map(|r| (r.0.as_str(), r.1.as_str())).collect();
    groups.dedup();
    let median = |group: &[&Row], value: &dyn Fn(&Row) -> f64| {
        bench::median(group.iter().map(|r| value(r)).collect())
    };
    let summary = |key: &'static str| move |r: &Row| r.3[key].as_f64().unwrap_or(f64::NAN);
    let own = |path: &'static [&'static str]| move |r: &Row| at(&r.4["ruthenium"], path);
    let mib = |bytes: f64| bytes / f64::from(1 << 20);

    match baseline {
        Some(dir) => println!(
            "\nRuthenium in its window, next to the vanilla runs of the same routes in {} (each ratio is Ruthenium's median over vanilla's). Ruthenium draws less than vanilla does here: see what its scene leaves out before reading a ratio as a result.",
            dir.display()
        ),
        None => println!("\nRuthenium in its window (no baseline was given to print next to it)."),
    }
    println!(
        "\n| Variant | Scene | Avg FPS | × vanilla | 1% low | × vanilla | 0.1% low | × vanilla | Median ms | Max ms | Runs |"
    );
    println!("|---|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|");
    for (variant, scene) in &groups {
        let group: Vec<&Row> = ours
            .iter()
            .copied()
            .filter(|r| r.0 == *variant && r.1 == *scene)
            .collect();
        let vanilla: Vec<&Row> = theirs.iter().filter(|r| r.1 == *scene).collect();
        let ratio = |key: &'static str| {
            if vanilla.is_empty() {
                "–".to_owned()
            } else {
                format!(
                    "{:.2}",
                    median(&group, &summary(key)) / median(&vanilla, &summary(key))
                )
            }
        };
        println!(
            "| {variant} | {scene} | {:.0} | {} | {:.0} | {} | {:.0} | {} | {:.3} | {:.1} | {} |",
            median(&group, &summary("avgFps")),
            ratio("avgFps"),
            median(&group, &summary("low1Fps")),
            ratio("low1Fps"),
            median(&group, &summary("low01Fps")),
            ratio("low01Fps"),
            median(&group, &summary("medianMs")),
            median(&group, &summary("maxMs")),
            group.len()
        );
    }

    println!(
        "\nWhere a frame's time goes, medians over a variant's runs of each run's median (or the percentile named; the stage's maximum is the worst frame of any run), in milliseconds. Work is the main thread's own (events, the game's frame, the GUI, recording and submitting); the terrain's two parts are inside recording."
    );
    println!(
        "\n| Variant | Scene | Work | Work P99 | Events | Game | GUI | Record | Terrain stage | Stage P99 | Stage P99.9 | Stage max | Terrain draw | Submit | Governor | Acquire | Present | GPU | GPU P99 |"
    );
    println!(
        "|---|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|"
    );
    for (variant, scene) in &groups {
        let group: Vec<&Row> = ours
            .iter()
            .copied()
            .filter(|r| r.0 == *variant && r.1 == *scene)
            .collect();
        let m = |path: &'static [&'static str]| median(&group, &own(path));
        println!(
            "| {variant} | {scene} | {:.3} | {:.3} | {:.3} | {:.3} | {:.3} | {:.3} | {:.3} | {:.3} | {:.3} | {:.2} | {:.3} | {:.3} | {:.3} | {:.3} | {:.3} | {:.3} | {:.3} |",
            m(&["phases", "work", "p50"]),
            m(&["phases", "work", "p99"]),
            m(&["phases", "events", "p50"]),
            m(&["phases", "game", "p50"]),
            m(&["phases", "gui", "p50"]),
            m(&["phases", "record", "p50"]),
            m(&["phases", "world_stage", "p50"]),
            m(&["phases", "world_stage", "p99"]),
            m(&["phases", "world_stage", "p999"]),
            // The worst frame of any run.
            group
                .iter()
                .map(|r| at(&r.4["ruthenium"], &["phases", "world_stage", "max"]))
                .fold(f64::NAN, f64::max),
            m(&["phases", "world_draw", "p50"]),
            m(&["phases", "submit", "p50"]),
            m(&["phases", "governor", "p50"]),
            m(&["phases", "acquire", "p50"]),
            m(&["phases", "present", "p50"]),
            m(&["gpu_time", "p50"]),
            m(&["gpu_time", "p99"]),
        );
    }

    println!(
        "\nWhat was drawn at the capture's end and what the terrain holds (medians over runs). Translucent quads are those Ruthenium leaves out for now; the pages are its vertex buffers."
    );
    println!(
        "\n| Variant | Scene | Drawn as | Sections drawn / with a mesh / loaded | Quads | Draws / calls / binds | Translucent quads left out | Pages MiB (payload, free, largest free) | Allocations failed (with room) | RSS MiB | VRAM MiB |"
    );
    println!("|---|---|---|---:|---:|---:|---:|---:|---:|---:|---:|");
    for (variant, scene) in &groups {
        let group: Vec<&Row> = ours
            .iter()
            .copied()
            .filter(|r| r.0 == *variant && r.1 == *scene)
            .collect();
        let end = |key: &'static str| {
            median(&group, &|r: &Row| {
                at(
                    &r.4["ruthenium"]["world"]["at_capture_end"]["terrain"],
                    &[key],
                )
            })
        };
        let pool = |key: &'static str| {
            median(&group, &|r: &Row| {
                at(
                    &r.4["ruthenium"]["world"]["at_capture_end"]["terrain"]["pool"],
                    &[key],
                )
            })
        };
        println!(
            "| {variant} | {scene} | {} | {:.0} / {:.0} / {:.0} | {:.0} | {:.0} / {:.0} / {:.0} | {:.0} | {:.0} ({:.0}, {:.1}, {:.2}) | {:.0} ({:.0}) | {:.0} | {:.0} |",
            group[0].4["ruthenium"]["world"]["draws"]
                .as_str()
                .unwrap_or("?"),
            end("visible"),
            end("meshed"),
            end("sections"),
            end("quads"),
            end("draws"),
            end("calls"),
            end("binds"),
            end("translucent_quads_left_out"),
            mib(pool("reserved_bytes")),
            mib(pool("payload_bytes")),
            mib(pool("free_bytes")),
            mib(pool("largest_free")),
            pool("failures"),
            pool("failures_with_space"),
            median(&group, &summary("rssMeanMiB")),
            median(&group, &summary("vramMeanMiB")),
        );
    }

    println!(
        "\nMeshing and uploads over the capture, and from a change to the frame that shows it (the worst run of a variant: P99 in milliseconds and in frames). A first mesh counts from when the chunks around its section were there."
    );
    println!(
        "\n| Variant | Scene | Meshes installed | MiB uploaded | Stale results | Slices refused | Budget refusals | First mesh ms / frames | Server's change ms / frames | Own change ms / frames (n) | Input queue most | Oldest input ms | Main thread waits |"
    );
    println!("|---|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|");
    for (variant, scene) in &groups {
        let group: Vec<&Row> = ours
            .iter()
            .copied()
            .filter(|r| r.0 == *variant && r.1 == *scene)
            .collect();
        let worst = |value: &dyn Fn(&Row) -> f64| {
            group
                .iter()
                .map(|r| value(r))
                .filter(|x| !x.is_nan())
                .fold(f64::NAN, f64::max)
        };
        let during = |part: &'static str, key: &'static str| {
            median(&group, &|r: &Row| {
                let w = &r.4["ruthenium"]["world"];
                at(&w["at_capture_end"][part], &[key]) - at(&w["at_capture_start"][part], &[key])
            })
        };
        let latency = |kind: &'static str| {
            let of = |key: &'static str| {
                worst(&|r: &Row| at(&r.4["ruthenium"]["world"]["latency"][kind], &[key]))
            };
            if of("samples").is_nan() || of("samples") == 0.0 {
                "–".to_owned()
            } else {
                format!("{:.1} / {:.0}", of("p99_ms"), of("p99_frames"))
            }
        };
        let session =
            |key: &'static str| worst(&|r: &Row| at(&r.4["ruthenium"]["world"]["session"], &[key]));
        println!(
            "| {variant} | {scene} | {:.0} | {:.1} | {:.0} | {:.0} | {:.0} | {} | {} | {} ({:.0}) | {:.0} | {:.1} | {:.0} |",
            during("terrain", "installed"),
            mib(during("uploads", "gpu_copy_bytes")),
            during("terrain", "stale_results"),
            during("uploads", "reservations_refused"),
            during("uploads", "budget_refusals"),
            latency("first_mesh"),
            latency("server_change"),
            latency("own_change"),
            worst(&|r: &Row| at(
                &r.4["ruthenium"]["world"]["latency"]["own_change"],
                &["samples"]
            )),
            session("input_queue_high_water"),
            session("oldest_input_ms"),
            session("main_thread_waits"),
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn options_default_to_the_baseline() {
        let o = parse(&["--accept-eula".into()]).unwrap();
        assert!(o.accept_eula && o.vanilla && o.headless && !o.windowed);
        assert_eq!(
            (o.reps, o.headless_reps, o.warmup, o.duration),
            (3, 1, 10.0, 60.0)
        );
        assert_eq!(o.backends, ["vulkan"]);
        assert_eq!(o.scenes, ["idle", "spin", "walk", "fly"]);
        // Without the flag the EULA is not accepted, whatever else is given.
        assert!(!parse(&["--reps".into(), "1".into()]).unwrap().accept_eula);
    }

    #[test]
    fn reads_expectations_and_clients() {
        let args: Vec<String> = [
            "--client",
            "ruthenium",
            "--expect",
            "fly=00ff,11aa",
            "--expect",
            "idle=22bb",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        let o = parse(&args).unwrap();
        assert!(!o.vanilla && !o.headless && o.windowed);
        assert_eq!(o.variants, [("ruthenium".to_owned(), Vec::<String>::new())]);
        assert_eq!(
            o.expect[0],
            ("fly".into(), "00ff".into(), Some("11aa".into()))
        );
        assert_eq!(o.expect[1], ("idle".into(), "22bb".into(), None));
        assert!(parse(&["--scene".into(), "menu".into()]).is_err());
        assert!(parse(&["--client".into(), "bedrock".into()]).is_err());

        let words = |list: &[&str]| list.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let o = parse(&words(&[
            "--client",
            "ruthenium,headless",
            "--variant",
            "direct",
            "--variant",
            "mdi=--terrain-draws mdi --frames-ahead 1",
            "--scene",
            "fly,dig",
        ]))
        .unwrap();
        assert!(o.windowed && o.headless && !o.vanilla);
        assert_eq!(o.variants[0], ("direct".to_owned(), Vec::<String>::new()));
        assert_eq!(
            o.variants[1],
            (
                "mdi".to_owned(),
                words(&["--terrain-draws", "mdi", "--frames-ahead", "1"])
            )
        );
        // A label is one plain word that names nothing else; variants need the window;
        // vanilla cannot dig.
        for bad in ["a-b", "vulkan", "headless", "", "Mdi"] {
            let args = words(&["--client", "ruthenium", "--variant", bad]);
            assert!(parse(&args).is_err(), "{bad:?}");
        }
        assert!(
            parse(&words(&[
                "--client",
                "ruthenium",
                "--variant",
                "a",
                "--variant",
                "a"
            ]))
            .is_err()
        );
        assert!(parse(&words(&["--client", "headless", "--variant", "a"])).is_err());
        assert!(parse(&words(&["--scene", "dig"])).is_err());
        assert!(parse(&words(&["--client", "headless", "--scene", "dig"])).is_ok());
    }

    #[test]
    fn reads_the_login_position_from_the_server_log() {
        let log = "[23:02:00] [Server thread/INFO]: Starting\n\
            [23:02:00] [Server thread/INFO]: Ruthenium[/127.0.0.1:49414] logged in with entity id 27 at (4.5, 71.0, -2.5)\n\
            [23:02:00] [Server thread/INFO]: System chat: Ruthenium joined the game\n";
        assert_eq!(logged_in_at(log), Some([4.5, 71.0, -2.5]));
        assert_eq!(logged_in_at("[23:02:00] [Server thread/INFO]: Done"), None);
    }

    #[test]
    fn tick_times_are_read_over_the_capture() {
        let v = json!({
            "routeRun": { "markers": { "captureStartEpochMs": 10_000, "captureEndEpochMs": 20_000 } },
            "server": {
                "tickHealth": [
                    { "epochMs": 9_000, "averageMs": 50.0, "p99Ms": 90.0 },
                    { "epochMs": 15_000, "averageMs": 2.0, "p99Ms": 4.0 },
                    { "epochMs": 24_000, "averageMs": 4.0, "p99Ms": 7.5 },
                    { "epochMs": 30_000, "averageMs": 50.0, "p99Ms": 90.0 },
                ],
                "samples": [[9_000, 1, 1.0], [10_000, 1, 2.0], [20_000, 1, 4.5], [21_000, 1, 9.0]],
            },
        });
        assert_eq!(tick_health(&v), ("3.0".into(), "7.5".into(), "25".into()));
        assert_eq!(
            tick_health(&json!({})),
            ("–".into(), "–".into(), "–".into())
        );
    }
}

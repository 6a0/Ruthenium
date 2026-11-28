//! `ruthenium connect`: joins a server without a window and prints what happens, to
//! try the connection path from a shell (development).
//!
//! `--script` plays input once the player is in the world, through the same calls the
//! window makes: steps separated by spaces, each `KEYS@MS` or `KEYS@MS:DX,DY`. KEYS are
//! `+`-joined `w a s d jump sneak sprint use hit` (`use` holds the right button, `hit`
//! the left), hotbar digits `1`-`9`, `click` (one press of the right button), `attack`
//! (one press of the left), `swap` (the swap-hands key) and `wake` (the bed screen's
//! Leave Bed), the last five pressed once, `inv` (the inventory key), or `chat=TEXT` alone, sent to the chat as if
//! typed and entered at the step's start (`_` for a space, `%XX` for any byte; a
//! leading `/` makes it a command)
//! at the step's start, or `-` for none; held for MS milliseconds while the mouse moves
//! DX, DY counts every frame (60 a second, the rate frames run at here), turned as the window turns them with the
//! saved mouse sensitivity. For example
//! `w@2000 w+sprint+jump@3000 w@2000:12,0 sneak+w@1500 2@100 -@1000`.
//!
//! An open container screen takes these, each its events at the step's start, at a
//! slot's middle (slots are the menu's numbers): `lclick.N`, `rclick.N`, `mclick.N`
//! (a press and a release of the left, right, middle button), `sclick.N` and
//! `srclick.N` (with shift), `dbl.N` and `sdbl.N` (a double click, with shift: the
//! second press 0 ms after the first), `drag.N.M...` and `rdrag.N.M...` (pressed
//! over the first, dragged over each, let go over the last), `out.l` and `out.r` (a
//! click outside the window), `hot.K.N` (the hotbar key K, 1 to 9, over slot N),
//! `off.N` (the swap-hands key), `drop.N` and `cdrop.N` (the drop key, with ctrl),
//! `esc` (Escape), and `raw.SLOT.BUTTON.INPUT` (a click as the screen's
//! `slotClicked` makes it: INPUT is `pickup`, `quick_move`, `swap`, `clone`,
//! `throw`, `quick_craft` or `pickup_all`; `raw.-999.0.pickup` outside).
//!
//! What the chat shows is printed as it comes. A death is printed, and Respawn pressed as soon as the death screen's buttons work;
//! the script goes on (it doesn't start over) once the player is back.
//!
//! `--light-out` writes the light of every loaded chunk when the stay ends, and
//! `--light-against` compares it with a file an earlier run wrote. That is the check of
//! the light the client computes: change blocks on the server while one run is joined,
//! join again, and the light the server then sends must be what the first run had
//! (keep the chunks loaded on the server and the world still in between).
//!
//! `--route` plays a route file written by `cargo xtask bench dedicated` instead: input
//! keyed by client tick (see [`crate::route`]). The client waits until a benchmark controller
//! has put the player at the route's start, plays every tick's input, records where the
//! player is after each tick and which chunks are loaded at the route's marks, and
//! fails if any of it differs from what the route says. `--route-out` gets the record
//! as JSON, with the two checksums a vanilla client driven by the same file must
//! reproduce.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;
use std::time::{Duration, Instant};

use ruthenium_prepare::discover::Env;
use ruthenium_sim::hud_text::ChatMessage;
use ruthenium_sim::input::{self, MouseOptions};
use ruthenium_sim::menu::Input;
use ruthenium_sim::menu::screen::Binds;
use ruthenium_sim::player::{Keys, Pose};
use ruthenium_sim::session::{At, PlayerView, ScreenInput, Session, SessionEvent};

use ruthenium_net::server_list::PackStatus;

use crate::account::{AccountStatus, Accounts};
use crate::join::{Joiner, closed_text, plain_text, stage_text};
use crate::route::{Expected, Route, RoutePlayer};
use crate::settings::SettingsFile;

/// The largest texture a reload's atlases may be without a window (a common limit).
const HEADLESS_MAX_TEXTURE: u32 = 16384;

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
    /// Sent to the chat at the step's start.
    chat: Option<String>,
    /// Inventory key presses, and container screen events, at the step's start.
    inventory: u32,
    screen: Vec<ScreenEvent>,
    ms: u64,
    turn: (f32, f32),
}

fn parse_script(s: &str) -> Result<Vec<Step>, String> {
    s.split_whitespace()
        .map(|step| {
            let bad = || format!("--script: can't read the step {step:?}");
            let (keys, rest) = step.rsplit_once('@').ok_or_else(bad)?;
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
                chat: None,
                inventory: 0,
                screen: Vec::new(),
                ms: ms.parse().map_err(|_| bad())?,
                turn,
            };
            if let Some(text) = keys.strip_prefix("chat=") {
                st.chat = Some(script_text(text).ok_or_else(bad)?);
                return Ok(st);
            }
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
                    "inv" => st.inventory += 1,
                    k if screen_events(k).is_some() => {
                        st.screen.extend(screen_events(k).unwrap_or_default());
                    }
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

/// A script's container screen event, made when its step begins.
#[derive(Debug, Clone, PartialEq)]
enum ScreenEvent {
    Press(u8, At, bool),
    Release(u8, At, bool),
    Drag(At),
    Key(Binds, bool, At),
    Click(i32, i32, Input),
}

/// The screen events of a script token (see the module docs), if it is one.
fn screen_events(k: &str) -> Option<Vec<ScreenEvent>> {
    use ScreenEvent::{Click, Drag, Key, Press, Release};
    let parts: Vec<&str> = k.split('.').collect();
    let slot = |s: &str| s.parse::<usize>().ok().map(At::Slot);
    let click = |button: u8, at: At, shift: bool| {
        vec![Press(button, at, shift), Release(button, at, shift)]
    };
    let key = |binds: Binds, ctrl: bool, at: &str| Some(vec![Key(binds, ctrl, slot(at)?)]);
    Some(match parts.as_slice() {
        ["lclick", n] => click(1, slot(n)?, false),
        ["rclick", n] => click(3, slot(n)?, false),
        ["mclick", n] => click(2, slot(n)?, false),
        ["sclick", n] => click(1, slot(n)?, true),
        ["srclick", n] => click(3, slot(n)?, true),
        ["dbl", n] | ["sdbl", n] => {
            let shift = parts[0] == "sdbl";
            let at = slot(n)?;
            [click(1, at, shift), click(1, at, shift)].concat()
        }
        ["drag" | "rdrag", rest @ ..] if !rest.is_empty() => {
            let button = if parts[0] == "drag" { 1 } else { 3 };
            let at: Vec<At> = rest.iter().map(|s| slot(s)).collect::<Option<_>>()?;
            let mut out = vec![Press(button, at[0], false)];
            out.extend(at.iter().map(|a| Drag(*a)));
            out.push(Release(button, *at.last()?, false));
            out
        }
        ["out", "l"] => click(1, At::Outside, false),
        ["out", "r"] => click(3, At::Outside, false),
        ["hot", k, n] => {
            let k: usize = k.parse().ok().filter(|k| (1..=9).contains(k))?;
            let mut b = Binds::default();
            b.hotbar[k - 1] = true;
            key(b, false, n)?
        }
        ["off", n] => key(
            Binds {
                swap_offhand: true,
                ..Binds::default()
            },
            false,
            n,
        )?,
        ["drop", n] | ["cdrop", n] => key(
            Binds {
                drop: true,
                ..Binds::default()
            },
            parts[0] == "cdrop",
            n,
        )?,
        ["esc"] => vec![Key(
            Binds {
                escape: true,
                ..Binds::default()
            },
            false,
            At::Outside,
        )],
        ["raw", slot, button, input] => {
            // (A negative slot reads as "raw.-999...": its minus was kept.)
            let input = Input::from_name(input).or_else(|| Input::from_id(input.parse().ok()?))?;
            vec![Click(slot.parse().ok()?, button.parse().ok()?, input)]
        }
        _ => return None,
    })
}

/// A `chat=` step's text: `_` a space, `%XX` a byte, the rest as it is.
fn script_text(s: &str) -> Option<String> {
    let mut bytes = Vec::with_capacity(s.len());
    let mut it = s.bytes();
    while let Some(b) = it.next() {
        match b {
            b'_' => bytes.push(b' '),
            b'%' => {
                let hex = [it.next()?, it.next()?];
                bytes.push(u8::from_str_radix(std::str::from_utf8(&hex).ok()?, 16).ok()?);
            }
            b => bytes.push(b),
        }
    }
    String::from_utf8(bytes).ok()
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
        if let Some(text) = &st.chat {
            println!("          chat: {text}");
            session.send_chat(text);
        }
        for _ in 0..st.inventory {
            session.inventory_key();
        }
        let now = Instant::now();
        let mouse = |button: u8| Binds {
            // The default bindings: the middle button picks.
            pick: button == 2,
            ..Binds::default()
        };
        for e in &st.screen {
            println!("          screen: {e:?}");
            session.screen_input(match *e {
                ScreenEvent::Press(button, at, shift) => ScreenInput::Press {
                    button,
                    at,
                    shift,
                    binds: mouse(button),
                    when: now,
                },
                ScreenEvent::Release(button, at, shift) => ScreenInput::Release {
                    button,
                    at,
                    shift,
                    binds: mouse(button),
                },
                ScreenEvent::Drag(at) => ScreenInput::Drag { at },
                ScreenEvent::Key(binds, ctrl, at) => ScreenInput::Key { binds, ctrl, at },
                ScreenEvent::Click(slot, button, input) => ScreenInput::Click {
                    slot,
                    button,
                    input,
                },
            });
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

/// Plays the route to its end, a frame every two milliseconds, each after the game
/// thread has run the one before.
fn play_route(
    session: &Session,
    player: &mut RoutePlayer,
    lang: &HashMap<String, String>,
) -> Result<(), String> {
    let started = Instant::now();
    loop {
        for e in session.poll() {
            let t = started.elapsed().as_secs_f64();
            match &e {
                SessionEvent::Closed(c) => {
                    let (title, reason) = closed_text(c, lang, false);
                    return Err(format!("{title}: {reason}"));
                }
                // A route is played on one server.
                SessionEvent::Transfer(to) => {
                    return Err(format!("the server transferred the player to {}", to.to));
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
        let view = synced_view(session, player.sent())?;
        if player.step(session, view.as_ref(), &|| None) {
            return player.outcome().cloned().unwrap_or(Ok(()));
        }
        session.frame();
        std::thread::sleep(ROUTE_FRAME);
    }
}

/// Joins, plays the route, writes the record and leaves. Fails if the run differed
/// from the route in any way.
fn run_route(
    session: &Session,
    route: Route,
    expected: Expected,
    out: Option<&Path>,
    lang: &HashMap<String, String>,
) -> Result<ExitCode, String> {
    let name = route.name.clone();
    let mut player = RoutePlayer::new(route, expected);
    let result = play_route(session, &mut player, lang);
    if let Err(e) = &result {
        player.fail(e.clone());
    }
    if let Some(out) = out {
        std::fs::write(out, player.record().to_string())
            .map_err(|e| format!("{}: {e}", out.display()))?;
    }
    session.disconnect();
    result.map_err(|e| format!("the {name} route failed: {e}"))?;
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
    let mut light_out = None;
    let mut light_against = None;
    let mut route_out = None;
    let mut expected = Expected::default();
    let mut sound = crate::sound::SoundMode::Off;
    let mut server_packs = PackStatus::Prompt;
    let mut pack_answer = false;
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
            "--light-out" => {
                light_out = Some(PathBuf::from(it.next().ok_or("--light-out needs a file")?));
            }
            "--light-against" => {
                let path = it.next().ok_or("--light-against needs a file")?;
                let text = std::fs::read_to_string(path).map_err(|e| format!("{path}: {e}"))?;
                light_against = Some((path.clone(), text));
            }
            "--route-out" => {
                route_out = Some(PathBuf::from(it.next().ok_or("--route-out needs a file")?));
            }
            "--sound" => {
                sound = match it.next().map(String::as_str) {
                    Some("on") => crate::sound::SoundMode::Device,
                    Some("off") => crate::sound::SoundMode::Off,
                    _ => return Err("--sound needs on or off".into()),
                };
            }
            "--server-packs" => {
                server_packs = match it.next().map(String::as_str) {
                    Some("prompt") => PackStatus::Prompt,
                    Some("enabled") => PackStatus::Enabled,
                    Some("disabled") => PackStatus::Disabled,
                    _ => return Err("--server-packs needs prompt, enabled or disabled".into()),
                };
            }
            "--pack-answer" => {
                pack_answer = match it.next().map(String::as_str) {
                    Some("yes") => true,
                    Some("no") => false,
                    _ => return Err("--pack-answer needs yes or no".into()),
                };
            }
            // A process-wide switch (`ruthenium_sim::packet_check`), set as it is read.
            "--check-packets" => match it.next().map(String::as_str) {
                Some("on") => ruthenium_sim::packet_check::set_enabled(true),
                Some("off") => ruthenium_sim::packet_check::set_enabled(false),
                _ => return Err("--check-packets needs on or off".into()),
            },
            "--sound-wav" => {
                sound = crate::sound::SoundMode::Wav(PathBuf::from(
                    it.next().ok_or("--sound-wav needs a file")?,
                ));
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
    settings.import_sound(crate::sound::vanilla_volumes);
    let audio = crate::sound::start(
        &sound,
        Some(data_dir.join("game")),
        settings.settings.sound.volumes(),
    );
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
    joiner.set_sound(audio.handle());
    let data = joiner.data();
    let mut session = joiner.join(&address, &accounts, server_packs)?;
    let mut reloader = crate::resource_reload::Reloader::new(Some(data_dir.join("game")), true);
    if let Some(route) = route {
        return run_route(&session, route, expected, route_out.as_deref(), &data.lang);
    }
    let started = Instant::now();
    let stay = Duration::from_secs_f64(seconds);
    let mut playing = false;
    let mut leaving = false;
    let mut light_same = true;
    let mut next_frame = Instant::now();
    // The newest chat message printed.
    let mut chat_seen: Option<Arc<ChatMessage>> = None;
    // Transfers are followed as in the window: the session numbers tell the reloads of
    // one from the next's.
    let mut transfer = None;
    let mut transferred = false;
    let mut session_number = 1;
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
                SessionEvent::PackPrompt(p) => {
                    let text = p
                        .prompt
                        .as_ref()
                        .map(|t| plain_text(t, &data.lang))
                        .unwrap_or_default();
                    println!(
                        "{t:7.3}s  resource pack prompt ({} pack(s){}): {:?}; answering {}",
                        p.packs,
                        if p.required { ", required" } else { "" },
                        crate::menu::prompt_lines(&text).join(" / "),
                        if pack_answer { "yes" } else { "no" }
                    );
                    session.answer_pack_prompt(pack_answer);
                }
                SessionEvent::PackStatus(s) => {
                    println!("{t:7.3}s  the server's pack setting is now {s:?}");
                }
                SessionEvent::PackToast(toast) => {
                    if let Some((title, line)) = crate::menu::toast_lines(toast) {
                        println!(
                            "{t:7.3}s  {title}{}",
                            line.map_or_else(String::new, |l| format!(": {l}"))
                        );
                    }
                }
                SessionEvent::ReloadResources(r) => {
                    println!(
                        "{t:7.3}s  reloading resources with {} server pack(s){}",
                        r.packs.len(),
                        if r.recovery { " (recovery)" } else { "" }
                    );
                    reloader.request(
                        crate::resource_reload::Request {
                            packs: r.packs.clone(),
                            session: session_number,
                        },
                        HEADLESS_MAX_TEXTURE,
                    );
                }
                SessionEvent::ContainerOpened => println!("{t:7.3}s  container screen open"),
                SessionEvent::ContainerClosed => println!("{t:7.3}s  container screen closed"),
                SessionEvent::Transfer(to) => {
                    println!("{t:7.3}s  transferred to {}", to.to);
                    transfer = Some(to.clone());
                }
                SessionEvent::Closed(c) => {
                    let (title, reason) = closed_text(c, &data.lang, transferred);
                    println!("{t:7.3}s  closed: {title}: {reason}");
                    closed = Some(c.clone());
                }
            }
        }
        // The session ended for a transfer: the next one, at once (`handleTransfer`).
        if closed.is_some()
            && let Some(to) = transfer.take()
        {
            // A transfer in play keeps the server's packs: the reloads they asked for
            // answer the next session.
            if to.packs.kept() {
                reloader.move_session(session_number, session_number + 1);
            }
            session = match joiner.join_transfer(to, &accounts, server_packs) {
                Ok(s) => s,
                Err(e) => {
                    println!("closed: Connection failed while transferring to the server: {e}");
                    return Ok(ExitCode::FAILURE);
                }
            };
            session_number += 1;
            transferred = true;
            playing = false;
            continue;
        }
        if let Some(c) = closed {
            let ok = playing && light_same && (leaving || crate::join::left_on_purpose(&c));
            return Ok(if ok {
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            });
        }
        // A reload that ended: how it went goes to the session (the world's assets are
        // built and checked, not drawn).
        if let Some((request, result)) = reloader.poll(HEADLESS_MAX_TEXTURE)
            && request.session == session_number
        {
            let t = started.elapsed().as_secs_f64();
            match result {
                Ok(r) => {
                    println!(
                        "{t:7.3}s  resources reloaded with {} server pack(s)",
                        request.packs.len()
                    );
                    joiner.set_server_lang((r.packs > 0).then(|| r.lang.clone()));
                    joiner.sound().replace_files(r.sounds);
                    if let Some(world) = &r.world {
                        println!(
                            "{t:7.3}s  block atlas {}x{}",
                            world.atlas_width, world.atlas_height
                        );
                    }
                    session.resources_reloaded(true);
                }
                Err(e) => {
                    println!("{t:7.3}s  the resource reload failed: {e}");
                    session.resources_reloaded(false);
                }
            }
        }
        // A frame: the script's input, then the session's frame (packets, ticks, turn).
        if scripting && let Some(p) = &mut player {
            scripting = p.frame(&session);
        }
        session.frame();
        if let Some(f) = session.scene().frame() {
            let chat = &f.hud.texts.chat;
            let new = chat
                .iter()
                .position(|m| chat_seen.as_ref().is_some_and(|s| Arc::ptr_eq(s, m)))
                .unwrap_or(chat.len());
            let t = started.elapsed().as_secs_f64();
            for m in chat[..new].iter().rev() {
                println!(
                    "{t:7.3}s  chat: {}",
                    plain_text(&m.content, &joiner.server_lang())
                );
            }
            chat_seen = chat.first().cloned();
        }
        if let Some(v) = crate::ingame::world_view(&session, Instant::now(), 70.0) {
            audio.set_listener(&crate::sound::listener(&v.camera));
        }
        if !leaving && started.elapsed() >= stay {
            leaving = true;
            if light_out.is_some() || light_against.is_some() {
                let dump = light_dump(&session);
                if let Some(path) = &light_out {
                    std::fs::write(path, &dump).map_err(|e| format!("{}: {e}", path.display()))?;
                    println!("light written to {}", path.display());
                }
                if let Some((path, other)) = &light_against {
                    let min_y = session.scene().with_terrain(|t| t.dims().min_section * 16);
                    let differences = light_differences(&dump, other, min_y)?;
                    for d in differences.iter().take(40) {
                        println!("light: {d}");
                    }
                    println!("light: {} layers differ from {path}", differences.len());
                    light_same = differences.is_empty();
                }
            }
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

/// The light of every published chunk as text (`--light-out`): a line per light
/// section, `x z index sky block`, each layer as `-` (the section holds none), `u`
/// and a level (all of it at that level) or its 2048 bytes in hex.
fn light_dump(session: &Session) -> String {
    use std::fmt::Write as _;
    fn layer(p: &ruthenium_sim::world::LightPlane) -> String {
        match p.bytes() {
            None => "-".into(),
            Some(b) if b.iter().all(|v| *v == b[0]) && b[0] >> 4 == b[0] & 15 => {
                format!("u{}", b[0] & 15)
            }
            Some(b) => b.iter().map(|v| format!("{v:02x}")).collect(),
        }
    }
    let mut out = String::new();
    if let Some(c) = session.camera() {
        let p = c.frame.player.position;
        let _ = writeln!(out, "# player {:.3} {:.3} {:.3}", p[0], p[1], p[2]);
    }
    session.scene().with_terrain(|t| {
        let mut columns: Vec<_> = t.columns().collect();
        columns.sort_by_key(|(key, _)| *key);
        for ((x, z), c) in columns {
            for (i, l) in c.light().iter().enumerate() {
                let _ = writeln!(out, "{x} {z} {i} {} {}", layer(&l.sky), layer(&l.block));
            }
        }
    });
    out
}

/// A layer of a light dump as 2048 bytes, `None` for a section that holds none.
fn dumped_layer(text: &str) -> Result<Option<Vec<u8>>, String> {
    if text == "-" {
        return Ok(None);
    }
    if let Some(level) = text.strip_prefix('u') {
        let v: u8 = level.parse().map_err(|_| format!("bad level {level}"))?;
        return Ok(Some(vec![v & 15 | v << 4; 2048]));
    }
    if text.len() != 4096 || !text.is_ascii() {
        return Err(format!("a layer of {} characters", text.len()));
    }
    (0..2048)
        .map(|i| u8::from_str_radix(&text[i * 2..i * 2 + 2], 16).map_err(|e| e.to_string()))
        .collect::<Result<Vec<u8>, String>>()
        .map(Some)
}

/// Where two light dumps differ (`--light-against`), a line per layer, compared level
/// by level: light computed here after block changes against the light the server
/// sends for the same chunks after a rejoin must be the same. `min_y` is the y of
/// the level's lowest block.
fn light_differences(ours: &str, theirs: &str, min_y: i32) -> Result<Vec<String>, String> {
    type Layers<'a> = HashMap<[i32; 3], (&'a str, &'a str)>;
    fn read(text: &str) -> Result<Layers<'_>, String> {
        let mut out = HashMap::new();
        for line in text.lines().filter(|l| !l.starts_with('#')) {
            let f: Vec<&str> = line.split_whitespace().collect();
            let [x, z, i, sky, block] = f[..] else {
                return Err(format!("bad line: {line}"));
            };
            let n = |s: &str| s.parse::<i32>().map_err(|_| format!("bad line: {line}"));
            out.insert([n(x)?, n(z)?, n(i)?], (sky, block));
        }
        Ok(out)
    }
    let (a, b) = (read(ours)?, read(theirs)?);
    let mut keys: Vec<[i32; 3]> = a.keys().chain(b.keys()).copied().collect();
    keys.sort_unstable();
    keys.dedup();
    let mut out = Vec::new();
    for key in keys {
        let [x, z, i] = key;
        let (Some(ours), Some(theirs)) = (a.get(&key), b.get(&key)) else {
            // A chunk only one run had loaded says nothing.
            continue;
        };
        for (name, ours, theirs) in [("sky", ours.0, theirs.0), ("block", ours.1, theirs.1)] {
            if ours == theirs {
                continue;
            }
            let at = format!("chunk {x} {z}, light section {i}, {name} light");
            match (dumped_layer(ours)?, dumped_layer(theirs)?) {
                (None, None) => {}
                (Some(_), None) => out.push(format!("{at}: held here only")),
                (None, Some(_)) => out.push(format!("{at}: held there only")),
                (Some(p), Some(q)) => {
                    let level = |l: &[u8], j: usize| l[j >> 1] >> ((j & 1) * 4) & 15;
                    let differing: Vec<usize> = (0..4096)
                        .filter(|j| level(&p, *j) != level(&q, *j))
                        .collect();
                    if let Some(&j) = differing.first() {
                        let pos = [
                            x * 16 + (j & 15) as i32,
                            min_y + (i - 1) * 16 + (j >> 8) as i32,
                            z * 16 + (j >> 4 & 15) as i32,
                        ];
                        out.push(format!(
                            "{at}: {} levels differ, the first at {} {} {}: {} here, {} there",
                            differing.len(),
                            pos[0],
                            pos[1],
                            pos[2],
                            level(&p, j),
                            level(&q, j)
                        ));
                    }
                }
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn light_dumps_are_compared_level_by_level() {
        // An array of one level is the same light as the level alone.
        let fives = "55".repeat(2048);
        let ours = format!("# player 0 0 0\n0 0 1 u15 {fives}\n0 0 2 - u0\n1 0 1 u15 u0\n");
        let same = format!("0 0 1 {} u5\n0 0 2 - u0\n", "ff".repeat(2048));
        assert_eq!(
            light_differences(&ours, &same, -64).unwrap(),
            Vec::<String>::new()
        );
        // One level of 4096: the block at x 1, y 2, z 3 of the section (index 561, the
        // high half of byte 280).
        let mut other = vec![0x55u8; 2048];
        other[280] = 0x75;
        let hex: String = other.iter().map(|v| format!("{v:02x}")).collect();
        let theirs = format!("0 0 1 u15 {hex}\n0 0 2 u15 u0\n");
        assert_eq!(
            light_differences(&ours, &theirs, -64).unwrap(),
            [
                "chunk 0 0, light section 1, block light: 1 levels differ, the first at 1 -62 3: 5 here, 7 there",
                "chunk 0 0, light section 2, sky light: held there only",
            ]
        );
        assert!(light_differences("0 0 1 u15", &theirs, -64).is_err());
    }

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
        let s = parse_script("chat=/msg_@a_hi%5Fthere+1@200 chat=héllo@0").unwrap();
        assert_eq!(s[0].chat.as_deref(), Some("/msg @a hi_there+1"));
        assert_eq!((s[0].ms, s[0].keys), (200, Keys::default()));
        assert_eq!(s[1].chat.as_deref(), Some("héllo"));
        assert!(parse_script("chat=%G1@5").is_err());
        assert!(parse_script("w@").is_err());
        assert!(parse_script("fly@100").is_err());
    }
}

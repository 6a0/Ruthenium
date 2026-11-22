//! `ruthenium connect`: joins a server without a window and prints what happens, to
//! try the connection path from a shell (development).
//!
//! `--script` plays input once the player is in the world, through the same calls the
//! window makes: steps separated by spaces, each `KEYS@MS` or `KEYS@MS:DX,DY`. KEYS are
//! `+`-joined `w a s d jump sneak sprint use hit` (`use` holds the right button, `hit`
//! the left), hotbar digits `1`-`9`, `click` (one press of the right button), `attack`
//! (one press of the left) and `swap` (the swap-hands key), the last four pressed once
//! at the step's start, or `-` for none; held for MS milliseconds while the mouse moves
//! DX, DY counts every frame (60 a second, the rate frames run at here), turned as the window turns them with the
//! saved mouse sensitivity. For example
//! `w@2000 w+sprint+jump@3000 w@2000:12,0 sneak+w@1500 2@100 -@1000`.

use std::path::PathBuf;
use std::process::ExitCode;
use std::time::{Duration, Instant};

use ruthenium_prepare::discover::Env;
use ruthenium_sim::input::{self, MouseOptions};
use ruthenium_sim::player::Keys;
use ruthenium_sim::session::{Session, SessionEvent};

use crate::account::{AccountStatus, Accounts};
use crate::join::{Joiner, closed_text, stage_text};
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
                println!(
                    "          at {:.3} {:.3} {:.3}, facing {:.2} {:.2}, slot {}{}{}, hands [{}] [{}], food {}{using}",
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
    let mut joiner = Joiner::new(Some(data_dir.join("game")), cache_dir);
    let data = joiner.data();
    let session = joiner.join(&address, &accounts)?;
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
                    if let Some(p) = &mut player {
                        p.start(&session);
                        scripting = true;
                    }
                }
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
}

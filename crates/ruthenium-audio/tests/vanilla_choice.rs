//! Which sound a play picks, against vanilla's own `SoundManager.Preparations` and
//! `WeighedSoundEvents` run headless on the JVM (a local tool against the reference
//! jar writes `golden/choice.rs`). Both sides draw the seeds from the same generator.
//!
//! The made-up packs are checked in full. The real `sounds.json` (every event, four
//! seeds each) is checked by hashes of batches, and only where `RUTHENIUM_GAME_DIR`
//! names an imported game folder, as the game's files are never in the repository.

mod common;
#[path = "golden/choice.rs"]
mod golden;

use common::vanilla_files;
use ruthenium_audio::events::{Choice, SoundEvents, SoundKind, SoundsJson, id_key};
use ruthenium_javamath::random::JavaRandom;

/// The tool's generator: a 64-bit LCG whose whole state is the next seed.
struct Seeds(u64);

impl Iterator for Seeds {
    type Item = i64;
    fn next(&mut self) -> Option<i64> {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        Some(self.0 as i64)
    }
}

fn seeds() -> Seeds {
    Seeds(0x536f756e64476f6c)
}

const PACK1: &str = r#"
    {"a": {"sounds": [{"name": "f/a1", "weight": 3, "volume": 0.8, "pitch": 1.1}, "f/a2",
                      {"name": "b", "type": "event", "weight": 50, "volume": 0.5, "pitch": 0.9}]},
     "b": {"sounds": [{"name": "f/b1", "weight": 2, "attenuation_distance": 8, "stream": true},
                      {"name": "c", "type": "event", "volume": 0.7},
                      {"name": "f/b2", "weight": 5, "pitch": 1.4}]},
     "c": {"sounds": [{"name": "f/c1", "weight": 7, "volume": 0.3}, {"name": "f/c2", "weight": 1}]},
     "d": {"sounds": [{"name": "missing", "type": "event"}, {"name": "f/d1", "weight": 2}]},
     "e": {"sounds": []},
     "g": {"sounds": [{"name": "f/gone"}, "f/g1"]},
     "h": {"sounds": [{"name": "a", "type": "event", "stream": true, "pitch": 0.75},
                      {"name": "c", "type": "event", "weight": 1000}]}}
"#;
const PACK2: &str = r#"
    {"a": {"sounds": [{"name": "f/a3", "weight": 4}]},
     "c": {"replace": true, "sounds": [{"name": "f/c3", "weight": 2}, {"name": "d", "type": "event", "weight": 9}]}}
"#;

/// One play as the tool writes it.
fn line(events: &SoundEvents, event: &str, seed: i64) -> String {
    match events.choose(event, &mut JavaRandom::new(seed)) {
        Choice::Empty => "EMPTY".into(),
        Choice::Unknown => "UNKNOWN".into(),
        Choice::Sound(s) => format!(
            "{}|{:x}|{:x}|{}|{}",
            id_key(&s.name),
            s.volume.to_bits(),
            s.pitch.to_bits(),
            s.stream,
            s.attenuation_distance
        ),
    }
}

#[test]
fn made_up_packs_choose_what_vanilla_chooses() {
    let (events, warnings) = SoundEvents::load(
        [PACK1, PACK2].map(|json| SoundsJson {
            namespace: "minecraft",
            pack: "test",
            bytes: json.as_bytes(),
            limits: ruthenium_assets::Limits::trusted(),
        }),
        |f| id_key(f) != "minecraft:sounds/f/gone.ogg",
    );
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    let mut seeds = seeds();
    let mut n = 0;
    for event in ["a", "b", "c", "d", "e", "g", "h"] {
        for _ in 0..64 {
            let seed = seeds.next().unwrap();
            let (e, s, want) = golden::SYNTHETIC[n];
            assert_eq!((e, s), (format!("minecraft:{event}").as_str(), seed));
            assert_eq!(line(&events, e, seed), want, "{e} seed {seed}");
            n += 1;
        }
    }
    assert_eq!(n, golden::SYNTHETIC.len());
}

fn fnv(mut h: u64, line: &str) -> u64 {
    for b in line.bytes().chain(*b"\n") {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

const FNV_START: u64 = 0xcbf29ce484222325;

#[test]
fn the_real_sounds_json_chooses_what_vanilla_chooses() {
    let Some(files) = vanilla_files() else {
        eprintln!("RUTHENIUM_GAME_DIR not set; skipped");
        return;
    };
    let (events, warnings) = files.events();
    assert!(warnings.is_empty(), "{warnings:?}");
    let mut names: Vec<&str> = events.names().collect();
    names.sort_unstable();
    assert_eq!(names.len(), golden::REAL_EVENTS);
    let mut seeds = seeds().skip(golden::SYNTHETIC.len());
    let mut hashes = Vec::new();
    let mut h = FNV_START;
    let mut n = 0;
    for e in &names {
        for _ in 0..golden::REAL_SEEDS_PER_EVENT {
            let seed = seeds.next().unwrap();
            h = fnv(h, &format!("{e}|{seed}|{}", line(&events, e, seed)));
            n += 1;
            if n % golden::REAL_BATCH == 0 {
                hashes.push(h);
                h = FNV_START;
            }
        }
    }
    if n % golden::REAL_BATCH != 0 {
        hashes.push(h);
    }
    for (i, (got, want)) in hashes.iter().zip(golden::REAL_HASHES).enumerate() {
        assert_eq!(
            got, want,
            "batch {i} (the tool prints it with SOUND_GOLDEN_PRINT={i})"
        );
    }
    assert_eq!(hashes.len(), golden::REAL_HASHES.len());
    // Every sound of every event is a file that exists, or an event that does; a few
    // events have no sounds at all (vanilla plays nothing for them either).
    let mut empty = 0;
    for e in &names {
        for s in &events.get(e).unwrap().sounds {
            match s.kind {
                SoundKind::File => {
                    assert!(files.has(&s.file_id()), "{e}: {}", id_key(&s.file_id()))
                }
                SoundKind::Event => assert!(events.get(&id_key(&s.name)).is_some(), "{e}"),
            }
        }
        match events.choose(e, &mut JavaRandom::new(1)) {
            Choice::Sound(s) => assert!(files.has(&s.file_id())),
            Choice::Empty => empty += 1,
            Choice::Unknown => panic!("{e}"),
        }
    }
    eprintln!("{} events, {empty} of them with no sound", names.len());
}

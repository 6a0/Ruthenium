//! A server's \`sounds.json\` and sound files are hostile: absurd counts, references that
//! fan out, broken Ogg files. Each must fail cleanly or be skipped as vanilla skips them,
//! in bounded time.

mod common;

use std::time::{Duration, Instant};

use ruthenium_assets::{Id, Limits};
use ruthenium_audio::decode::{DecodeError, Decoder, decode, decode_limited};
use ruthenium_audio::events::{Choice, SoundEvents, SoundsJson};
use ruthenium_javamath::random::JavaRandom;

fn load(json: &str, limits: Limits) -> (SoundEvents, Vec<String>) {
    SoundEvents::load(
        [SoundsJson {
            namespace: "minecraft",
            pack: "server",
            bytes: json.as_bytes(),
            limits,
        }],
        |_| true,
    )
}

#[test]
fn references_that_fan_out_are_weighed_once() {
    // Each event names the next a thousand times, 32 deep: summed afresh for each play,
    // 1000^32 lookups.
    let mut events = Vec::new();
    for i in 0..32 {
        let refs = vec![format!(r#"{{"name": "e{}", "type": "event"}}"#, i + 1); 1000].join(",");
        events.push(format!(r#""e{i}": {{"sounds": [{refs}, "f/plain"]}}"#));
    }
    events.push(r#""e32": {"sounds": ["f/deep"]}"#.into());
    let json = format!("{{{}}}", events.join(","));
    let started = Instant::now();
    let (events, warnings) = load(&json, Limits::untrusted());
    assert!(warnings.is_empty(), "{warnings:?}");
    let mut random = JavaRandom::new(7);
    for _ in 0..1000 {
        assert!(matches!(
            events.choose("minecraft:e0", &mut random),
            Choice::Sound(_) | Choice::Empty
        ));
    }
    assert!(
        started.elapsed() < Duration::from_secs(30),
        "{:?}",
        started.elapsed()
    );
}

#[test]
fn files_of_too_many_events_or_sounds_add_nothing() {
    let limits = Limits {
        sound_events: 3,
        sounds_per_event: 2,
        ..Limits::untrusted()
    };
    let (e, w) = load(
        r#"{"a": {"sounds": ["x"]}, "b": {}, "c": {}, "d": {}}"#,
        limits,
    );
    assert!(e.is_empty() && w.len() == 1, "{w:?}");
    let (e, w) = load(r#"{"a": {"sounds": ["x", "y", "z"]}}"#, limits);
    assert!(e.is_empty() && w.len() == 1, "{w:?}");
    let (e, w) = load(r#"{"a": {"sounds": ["x", "y"]}}"#, limits);
    assert!(e.len() == 1 && w.is_empty(), "{w:?}");
    // Larger than a JSON file may be.
    let big = format!(
        r#"{{"a": {{"sounds": ["x"], "subtitle": "{}"}}}}"#,
        "s".repeat(64)
    );
    let (e, _) = load(
        &big,
        Limits {
            json_bytes: 32,
            ..Limits::untrusted()
        },
    );
    assert!(e.is_empty());
}

#[test]
fn weights_and_numbers_vanilla_refuses_are_refused() {
    // Weights summing past an int wrap as Java's do, and a wrapped total plays nothing.
    let (e, w) = load(
        r#"{"a": {"sounds": [{"name": "x", "weight": 2147483647}, {"name": "y", "weight": 2147483647}]},
            "b": {"sounds": [{"name": "x", "weight": 0}]},
            "c": {"sounds": [{"name": "x", "volume": 1e39, "pitch": 1e39}]}}"#,
        Limits::untrusted(),
    );
    assert_eq!(w.len(), 1, "a zero weight fails the file: {w:?}");
    assert!(e.is_empty());
    let (e, w) = load(
        r#"{"a": {"sounds": [{"name": "x", "weight": 2147483647}, {"name": "y", "weight": 2147483647}]}}"#,
        Limits::untrusted(),
    );
    assert!(w.is_empty());
    assert_eq!(
        e.choose("minecraft:a", &mut JavaRandom::new(1)),
        Choice::Empty
    );
}

#[test]
fn broken_ogg_files_are_errors() {
    let mut page = b"OggS\0\x02".to_vec();
    page.extend_from_slice(&[0xFF; 200]);
    for bytes in [Vec::new(), b"not an ogg".to_vec(), page, vec![0; 100_000]] {
        assert!(decode(bytes.clone()).is_err());
        assert!(Decoder::new(bytes).is_err());
    }
}

/// A sound of the player's game (never committed): the first by name.
fn game_ogg() -> Option<Vec<u8>> {
    let files = common::vanilla_files()?;
    let mut names: Vec<&str> = files.names().collect();
    names.sort_unstable();
    files.read(&Id::parse(names.first()?)?).ok()
}

/// The Ogg pages of \`bytes\`: (start, length, granule position).
fn pages(bytes: &[u8]) -> Vec<(usize, usize, u64)> {
    let mut out = Vec::new();
    let mut at = 0;
    while at + 27 <= bytes.len() && &bytes[at..at + 4] == b"OggS" {
        let segments = usize::from(bytes[at + 26]);
        let body: usize = bytes[at + 27..at + 27 + segments]
            .iter()
            .map(|&s| usize::from(s))
            .sum();
        let granule = u64::from_le_bytes(bytes[at + 6..at + 14].try_into().unwrap());
        let len = 27 + segments + body;
        out.push((at, len, granule));
        at += len;
    }
    out
}

/// Ogg's CRC (polynomial 0x04C11DB7, not reflected) over a page with its CRC zeroed.
fn fix_crc(page: &mut [u8]) {
    page[22..26].fill(0);
    let mut crc = 0u32;
    for &b in page.iter() {
        crc ^= u32::from(b) << 24;
        for _ in 0..8 {
            crc = if crc & 0x8000_0000 != 0 {
                crc << 1 ^ 0x04C1_1DB7
            } else {
                crc << 1
            };
        }
    }
    page[22..26].copy_from_slice(&crc.to_le_bytes());
}

#[test]
fn a_sample_rate_past_the_limit_is_refused_before_decoding() {
    let Some(mut ogg) = game_ogg() else {
        eprintln!("RUTHENIUM_GAME_DIR not set; skipped");
        return;
    };
    let (start, len, _) = pages(&ogg)[0];
    let segments = usize::from(ogg[start + 26]);
    let packet = start + 27 + segments;
    assert_eq!(&ogg[packet..packet + 7], b"\x01vorbis");
    ogg[packet + 12..packet + 16].copy_from_slice(&u32::MAX.to_le_bytes());
    fix_crc(&mut ogg[start..start + len]);
    let e = Decoder::with_limits(ogg.clone(), &Limits::untrusted()).unwrap_err();
    assert!(e.contains("Hz"), "{e}");
}

#[test]
fn a_sound_decoded_whole_is_no_longer_than_the_limit() {
    let Some(ogg) = game_ogg() else {
        eprintln!("RUTHENIUM_GAME_DIR not set; skipped");
        return;
    };
    let whole = decode(ogg.clone()).unwrap();
    assert!(whole.samples.len() > 1000);
    let limits = Limits {
        sound_samples: 1000,
        ..Limits::untrusted()
    };
    // Longer than a sound may be: the file's own, however much room there is.
    assert!(matches!(
        decode_limited(ogg.clone(), &limits, usize::MAX),
        Err(DecodeError::Broken(_))
    ));
    assert!(matches!(
        decode_limited(ogg.clone(), &limits, 10),
        Err(DecodeError::Broken(_))
    ));
    // It would fit, but not in the room left: for now.
    assert_eq!(
        decode_limited(ogg.clone(), &Limits::untrusted(), 1000),
        Err(DecodeError::HeldBack)
    );
    // Broken in its data with little room: still broken.
    let mut broken = ogg.clone();
    let (start, len, _) = pages(&broken)[2];
    broken[start + len - 40..start + len].fill(0xFF);
    broken.truncate(start + len);
    let little = decode_limited(broken, &Limits::untrusted(), 1000);
    assert!(little != Err(DecodeError::HeldBack), "{little:?}");
    assert_eq!(
        decode_limited(ogg, &Limits::untrusted(), usize::MAX).unwrap(),
        whole
    );
}

#[test]
fn a_real_sound_changed_at_random_decodes_or_fails() {
    let Some(ogg) = game_ogg() else {
        eprintln!("RUTHENIUM_GAME_DIR not set; skipped");
        return;
    };
    let mut state = 0x000A_0D10_u64;
    let mut next = |n: usize| {
        state = state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        ((state >> 33) as usize) % n.max(1)
    };
    let mut panics = 0;
    for _ in 0..300 {
        let mut b = ogg.clone();
        for _ in 0..1 + next(8) {
            let at = next(b.len());
            b[at] = next(256) as u8;
        }
        if next(4) == 0 {
            let at = next(b.len());
            b.truncate(at);
        }
        // The loader runs each decode so that a panic costs one sound; count them here.
        let r = std::panic::catch_unwind(|| decode_limited(b, &Limits::untrusted(), usize::MAX));
        panics += usize::from(r.is_err());
    }
    eprintln!("{panics} of 300 broken files panicked in the decoder");
}

/// A `sounds.json` with a little of everything, its bytes changed at random (fixed seed):
/// every load and every choice ends without a panic.
#[test]
fn sounds_json_changed_at_random_loads_without_a_panic() {
    let base = br#"{"a": {"sounds": ["f/a", {"name": "f/b", "weight": 3, "volume": 0.5, "pitch": 1.5, "stream": true, "attenuation_distance": 8}, {"name": "b", "type": "event", "weight": 2}], "subtitle": "s.a"},
        "b": {"replace": true, "sounds": [{"name": "c", "type": "event"}, "f/c"]},
        "c": {"sounds": [{"name": "a", "type": "event", "volume": 2}]}}"#;
    let mut state = 0x50_0D5_u64;
    let mut next = |n: usize| {
        state = state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        ((state >> 33) as usize) % n.max(1)
    };
    let mut loaded = 0;
    for _ in 0..3000 {
        let mut b = base.to_vec();
        for _ in 0..1 + next(4) {
            let at = next(b.len());
            match next(3) {
                0 => b[at] = b"0123456789-.e\"{}[],:"[next(20)],
                1 => {
                    b.remove(at);
                }
                _ => b.insert(at, b"9e9"[next(3)]),
            }
        }
        let Ok(json) = std::str::from_utf8(&b) else {
            continue;
        };
        let (events, _) = load(json, Limits::untrusted());
        loaded += usize::from(!events.is_empty());
        let mut random = JavaRandom::new(next(1000) as i64);
        for name in ["minecraft:a", "minecraft:b", "minecraft:c"] {
            let _ = events.choose(name, &mut random);
        }
    }
    assert!(loaded > 300, "{loaded}");
}

/// A stored zip of these files.
fn zip(files: &[(&str, &[u8])]) -> Vec<u8> {
    let (mut out, mut central) = (Vec::new(), Vec::new());
    for (name, data) in files {
        let offset = out.len() as u32;
        let head = |v: &mut Vec<u8>| {
            v.extend_from_slice(&[0; 2]);
            v.extend_from_slice(&[0; 8]);
            v.extend_from_slice(&(data.len() as u32).to_le_bytes());
            v.extend_from_slice(&(data.len() as u32).to_le_bytes());
            v.extend_from_slice(&(name.len() as u16).to_le_bytes());
        };
        out.extend_from_slice(&0x0403_4b50u32.to_le_bytes());
        out.extend_from_slice(&[20, 0, 0, 0]);
        head(&mut out);
        out.extend_from_slice(&[0; 2]);
        out.extend_from_slice(name.as_bytes());
        out.extend_from_slice(data);
        central.extend_from_slice(&0x0201_4b50u32.to_le_bytes());
        central.extend_from_slice(&[20, 0, 20, 0, 0, 0]);
        head(&mut central);
        central.extend_from_slice(&[0; 12]);
        central.extend_from_slice(&offset.to_le_bytes());
        central.extend_from_slice(name.as_bytes());
    }
    let cd = out.len() as u32;
    out.extend_from_slice(&central);
    out.extend_from_slice(&0x0605_4b50u32.to_le_bytes());
    out.extend_from_slice(&[0; 4]);
    out.extend_from_slice(&(files.len() as u16).to_le_bytes());
    out.extend_from_slice(&(files.len() as u16).to_le_bytes());
    out.extend_from_slice(&(central.len() as u32).to_le_bytes());
    out.extend_from_slice(&cd.to_le_bytes());
    out.extend_from_slice(&[0; 2]);
    out
}

/// Sounds are read while the game plays, long after the load: what they cost is not taken
/// from the load's budget, or a server's music would fall silent after an hour.
#[test]
fn sounds_read_while_playing_are_not_counted_against_the_load() {
    let ogg = vec![7u8; 100 << 10];
    let z = zip(&[
        ("pack.mcmeta", br#"{"pack": {"description": ""}}"#),
        ("assets/minecraft/sounds/music.ogg", &ogg),
        (
            "assets/minecraft/sounds.json",
            br#"{"m": {"sounds": [{"name": "music", "stream": true}]}}"#,
        ),
    ]);
    let path =
        std::env::temp_dir().join(format!("ruthenium-sound-budget-{}.zip", std::process::id()));
    std::fs::write(&path, z).unwrap();
    let limits = Limits {
        load_bytes: 1 << 20,
        ..Limits::untrusted()
    };
    let pack = ruthenium_assets::Pack::open_untrusted(&path, &limits).unwrap();
    let files = ruthenium_audio::SoundFiles::new(ruthenium_assets::Resources::new(vec![pack]));
    let (events, w) = files.events();
    assert!(w.is_empty() && events.len() == 1, "{w:?}");
    let music = Id::minecraft("sounds/music.ogg").unwrap();
    for _ in 0..30 {
        assert_eq!(files.read(&music).unwrap().len(), 100 << 10);
    }
    let _ = std::fs::remove_file(&path);
}

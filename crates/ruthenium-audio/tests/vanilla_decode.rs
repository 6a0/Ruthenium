//! Every sound file of the player's real 26.3 assets decodes, to what vanilla's own
//! `JOrbisAudioStream` makes of it (a local tool ran it headless and wrote
//! `golden/decode.rs`): the same frames, channels and rate exactly, and the same level.
//! Skipped unless `RUTHENIUM_GAME_DIR` names an imported game folder.

mod common;
#[path = "golden/decode.rs"]
mod golden;

use rayon::prelude::*;
use ruthenium_assets::Id;
use ruthenium_audio::decode::decode;

#[test]
fn every_sound_file_decodes_as_vanillas_decoder_does() {
    let Some(files) = common::vanilla_files() else {
        eprintln!("RUTHENIUM_GAME_DIR not set; skipped");
        return;
    };
    let mut names: Vec<&str> = files.names().collect();
    names.sort_unstable_by_key(|n| n.split_once(':').map_or(*n, |(_, p)| p));
    assert_eq!(names.len(), golden::FILES);
    let started = std::time::Instant::now();
    let decoded: Vec<Result<(String, f64), String>> = names
        .par_iter()
        .map(|name| {
            let id = Id::parse(name).ok_or("bad name")?;
            let pcm = files
                .read(&id)
                .map_err(|e| e.to_string())
                .and_then(decode)
                .map_err(|e| format!("{name}: {e}"))?;
            let sq: f64 = pcm
                .samples
                .iter()
                .map(|&s| f64::from(s) * f64::from(s))
                .sum();
            let rms = (sq / pcm.samples.len().max(1) as f64).sqrt();
            Ok((
                format!("{}|{}|{}", pcm.frames(), pcm.channels, pcm.rate),
                rms,
            ))
        })
        .collect();
    eprintln!(
        "{} files decoded in {:.1} s",
        decoded.len(),
        started.elapsed().as_secs_f64()
    );
    let failed: Vec<&String> = decoded.iter().filter_map(|d| d.as_ref().err()).collect();
    assert!(failed.is_empty(), "{failed:#?}");
    for (b, (batch, want)) in decoded
        .chunks(golden::BATCH)
        .zip(golden::BATCHES)
        .enumerate()
    {
        let mut h: u64 = 0xcbf29ce484222325;
        let mut rms = 0.0;
        for d in batch {
            let (line, level) = d.as_ref().expect("checked");
            for byte in line.bytes().chain([b'\n']) {
                h ^= u64::from(byte);
                h = h.wrapping_mul(0x100000001b3);
            }
            rms += level;
        }
        assert_eq!(h, want.0, "batch {b}: frames, channels or rate");
        assert!(
            (rms - want.1).abs() <= want.1 * 1e-5 + 0.01,
            "batch {b}: level {rms} against {}",
            want.1
        );
    }
}

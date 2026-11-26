//! The thread that reads and decodes sound files: whole sounds for the cache, and the
//! streams (`"stream": true`) a little ahead of where they play. The game and frame
//! threads only send it requests; it hands what it made to the engine.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::sync::{Arc, Mutex, PoisonError, Weak};
use std::time::{Duration, Instant};

use ruthenium_assets::Id;

use ruthenium_assets::Limits;

use crate::decode::{Decoder, Pcm, decode_limited};
use crate::engine::{Engine, Fetch};
use crate::events::id_key;
use crate::files::SoundFiles;
use crate::mixer::StreamFeed;

/// How much of a stream is decoded ahead, in seconds.
const STREAM_AHEAD: u32 = 2;
/// How often streams are topped up.
const STREAM_PERIOD: Duration = Duration::from_millis(20);
/// Bytes of whole decoded sounds alive at once (in the cache and playing). A server can
/// have 247 of a pack's sounds play at once; this keeps them from holding gigabytes. The
/// cache keeps 96 MiB; the game's sounds playing at once are a few MiB.
const LIVE_BYTES: usize = 256 << 20;

/// Runs a decoder's work so that a panic in it (a bug Symphonia has with some broken
/// file) costs that sound, not the thread every sound is decoded on.
fn guarded<T>(what: &Id, f: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(f))
        .unwrap_or_else(|_| Err(format!("{}: the decoder failed", id_key(what))))
}

enum Job {
    Decode(Id),
    Stream { slot: u16, file: Id, looping: bool },
}

/// The engine's way to the thread.
pub(crate) struct Loader {
    jobs: Sender<Job>,
}

impl Fetch for Loader {
    fn decode(&mut self, file: &Id) {
        let _ = self.jobs.send(Job::Decode(file.clone()));
    }

    fn stream(&mut self, slot: u16, file: &Id, looping: bool) {
        let _ = self.jobs.send(Job::Stream {
            slot,
            file: file.clone(),
            looping,
        });
    }
}

/// Whether to log every decode and play with its time (`RUTHENIUM_SOUND_TRACE=1`).
pub fn trace() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("RUTHENIUM_SOUND_TRACE").is_some_and(|v| v == "1"))
}

pub(crate) fn spawn(
    files: Arc<SoundFiles>,
    engine: Weak<Mutex<Engine>>,
) -> std::io::Result<Loader> {
    let (jobs, rx) = channel();
    std::thread::Builder::new()
        .name("sound-loader".into())
        .spawn(move || run(&files, &engine, &rx))?;
    Ok(Loader { jobs })
}

struct OpenStream {
    file: Id,
    bytes: Vec<u8>,
    limits: Limits,
    decoder: Decoder,
    /// Nothing decoded since the decoder (re)started: a looping file that gives no
    /// samples would otherwise start over forever.
    fresh: bool,
    looping: bool,
    out: rtrb::Producer<i16>,
    ended: Arc<AtomicBool>,
    /// Decoded samples that didn't fit yet.
    pending: Vec<i16>,
    at: usize,
}

/// Samples a new whole sound may have, with those decoded before still alive
/// ([`LIVE_BYTES`]); forgets the ones gone.
fn room(live: &mut Vec<Weak<Pcm>>) -> usize {
    live.retain(|p| p.strong_count() > 0);
    let held: usize = live
        .iter()
        .filter_map(Weak::upgrade)
        .map(|p| p.bytes())
        .sum();
    LIVE_BYTES.saturating_sub(held) / 2
}

fn lock(engine: &Weak<Mutex<Engine>>) -> Option<Arc<Mutex<Engine>>> {
    engine.upgrade()
}

fn run(files: &SoundFiles, engine: &Weak<Mutex<Engine>>, rx: &Receiver<Job>) {
    let mut streams: Vec<OpenStream> = Vec::new();
    // The whole sounds decoded here, for as long as anything holds them.
    let mut live: Vec<Weak<Pcm>> = Vec::new();
    loop {
        let job = if streams.is_empty() {
            rx.recv().map_err(|_| RecvTimeoutError::Disconnected)
        } else {
            rx.recv_timeout(STREAM_PERIOD)
        };
        match job {
            Ok(Job::Decode(file)) => {
                let started = Instant::now();
                let room = room(&mut live);
                let pcm = files
                    .open(&file)
                    .map_err(|e| e.to_string())
                    .and_then(|(bytes, limits)| {
                        guarded(&file, || decode_limited(bytes, &limits, room))
                    })
                    .map(Arc::new);
                if let Ok(p) = &pcm {
                    live.push(Arc::downgrade(p));
                }
                if trace() {
                    match &pcm {
                        Ok(p) => eprintln!(
                            "[sound] decoded {} ({} frames, {} ch, {} Hz) in {:.2} ms on {:?}",
                            id_key(&file),
                            p.frames(),
                            p.channels,
                            p.rate,
                            started.elapsed().as_secs_f64() * 1e3,
                            std::thread::current().name().unwrap_or("?")
                        ),
                        Err(e) => eprintln!("[sound] {}: {e}", id_key(&file)),
                    }
                }
                let Some(engine) = lock(engine) else { return };
                engine
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .decoded(&file, pcm);
            }
            Ok(Job::Stream {
                slot,
                file,
                looping,
            }) => {
                let opened = open(files, &file, looping);
                let Some(engine) = lock(engine) else { return };
                let feed = opened.map(|(s, feed)| {
                    streams.push(s);
                    feed
                });
                engine
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .stream_opened(slot, feed);
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return,
        }
        streams.retain_mut(fill);
    }
}

fn open(files: &SoundFiles, file: &Id, looping: bool) -> Result<(OpenStream, StreamFeed), String> {
    let (bytes, limits) = files
        .open(file)
        .map_err(|e| format!("{}: {e}", id_key(file)))?;
    let decoder = guarded(file, || Decoder::with_limits(bytes.clone(), &limits))
        .map_err(|e| format!("{}: {e}", id_key(file)))?;
    // In 64 bits: a rate is a pack's number (`Limits::sample_rate` bounds it).
    let size = (u64::from(decoder.rate) * u64::from(decoder.channels) * u64::from(STREAM_AHEAD))
        .min(1 << 26) as usize;
    let (out, samples) = rtrb::RingBuffer::new(size);
    let ended = Arc::new(AtomicBool::new(false));
    let feed = StreamFeed {
        channels: decoder.channels,
        rate: decoder.rate,
        samples,
        ended: ended.clone(),
    };
    let mut s = OpenStream {
        file: file.clone(),
        bytes,
        limits,
        decoder,
        fresh: true,
        looping,
        out,
        ended,
        pending: Vec::new(),
        at: 0,
    };
    fill(&mut s);
    Ok((s, feed))
}

/// Tops a stream up; false once it is done with (ended, or no longer played).
fn fill(s: &mut OpenStream) -> bool {
    if s.out.is_abandoned() {
        return false;
    }
    loop {
        while s.at < s.pending.len() {
            if s.out.push(s.pending[s.at]).is_err() {
                return true;
            }
            s.at += 1;
        }
        s.pending.clear();
        s.at = 0;
        let decoder = &mut s.decoder;
        let pending = &mut s.pending;
        match guarded(&s.file, || decoder.next_into(pending)) {
            Ok(true) => s.fresh &= s.pending.is_empty(),
            Ok(false) if s.looping && !s.fresh => {
                // `LoopingAudioStream`: from the start again.
                let (bytes, limits) = (&s.bytes, &s.limits);
                match guarded(&s.file, || Decoder::with_limits(bytes.clone(), limits)) {
                    Ok(d) => {
                        s.decoder = d;
                        s.fresh = true;
                    }
                    Err(e) => {
                        eprintln!("[sound] {}: {e}", id_key(&s.file));
                        s.ended.store(true, Ordering::Release);
                        return false;
                    }
                }
            }
            Ok(false) => {
                s.ended.store(true, Ordering::Release);
                return false;
            }
            Err(e) => {
                eprintln!("[sound] {}: {e}", id_key(&s.file));
                s.ended.store(true, Ordering::Release);
                return false;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn whole_sounds_alive_at_once_are_bounded() {
        let pcm = |samples: usize| {
            Arc::new(Pcm {
                channels: 1,
                rate: 48_000,
                samples: vec![0; samples].into_boxed_slice(),
            })
        };
        let mut live = Vec::new();
        assert_eq!(room(&mut live), LIVE_BYTES / 2);
        let a = pcm(LIVE_BYTES / 4);
        live.push(Arc::downgrade(&a));
        let left = LIVE_BYTES - a.bytes();
        assert_eq!(room(&mut live), left / 2);
        let b = pcm(left / 2);
        live.push(Arc::downgrade(&b));
        assert_eq!(room(&mut live), 0);
        // Once nothing holds a sound, its room is free again.
        drop((a, b));
        assert_eq!(room(&mut live), LIVE_BYTES / 2);
        assert!(live.is_empty());
    }

    #[test]
    fn a_panic_in_the_decoder_costs_one_sound() {
        let file = Id::minecraft("sounds/broken.ogg").unwrap();
        let r: Result<(), String> = guarded(&file, || panic!("a decoder's bug"));
        assert!(r.unwrap_err().contains("the decoder failed"));
        assert_eq!(guarded(&file, || Ok(3)), Ok(3));
    }

    /// A file of the game's headers and no audio, looped: it would start over forever.
    #[test]
    fn a_looping_stream_of_no_samples_ends() {
        let Some(dir) = std::env::var_os("RUTHENIUM_GAME_DIR").map(std::path::PathBuf::from) else {
            eprintln!("RUTHENIUM_GAME_DIR not set; skipped");
            return;
        };
        let pack = ruthenium_assets::Pack::vanilla(
            &dir.join("versions/26.3/client.jar"),
            &dir.join("assets/indexes/34.json"),
            &dir.join("assets/objects"),
        )
        .unwrap();
        let files = SoundFiles::new(ruthenium_assets::Resources::new(vec![pack]));
        let mut names: Vec<&str> = files.names().collect();
        names.sort_unstable();
        let file = Id::parse(names[0]).unwrap();
        let mut bytes = files.read(&file).unwrap();
        // The headers and the first page of audio, whose granule position says that no
        // sample of it is to be played: the file gives no samples.
        let (mut at, mut packets) = (0, 0);
        loop {
            let segments = usize::from(bytes[at + 26]);
            let lacing = &bytes[at + 27..at + 27 + segments];
            let body: usize = lacing.iter().map(|&s| usize::from(s)).sum();
            // The three header packets end before the first page of audio.
            let headers = packets < 3;
            packets += lacing.iter().filter(|&&s| s < 255).count();
            if !headers {
                let page = &mut bytes[at..at + 27 + segments + body];
                page[5] |= 4;
                page[6..14].fill(0);
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
                bytes.truncate(at + 27 + segments + body);
                break;
            }
            at += 27 + segments + body;
        }
        let decoder = Decoder::new(bytes.clone()).unwrap();
        let (out, samples) = rtrb::RingBuffer::new(1024);
        let ended = Arc::new(AtomicBool::new(false));
        let mut s = OpenStream {
            file,
            bytes,
            limits: Limits::trusted(),
            decoder,
            fresh: true,
            looping: true,
            out,
            ended: ended.clone(),
            pending: Vec::new(),
            at: 0,
        };
        let (tx, rx) = channel();
        std::thread::spawn(move || {
            let _ = tx.send(fill(&mut s));
        });
        let kept = rx.recv_timeout(Duration::from_secs(20));
        assert_eq!(kept, Ok(false), "the stream ended");
        assert!(ended.load(Ordering::Acquire));
        drop(samples);
    }
}

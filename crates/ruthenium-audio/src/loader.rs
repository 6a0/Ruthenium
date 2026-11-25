//! The thread that reads and decodes sound files: whole sounds for the cache, and the
//! streams (`"stream": true`) a little ahead of where they play. The game and frame
//! threads only send it requests; it hands what it made to the engine.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::sync::{Arc, Mutex, PoisonError, Weak};
use std::time::{Duration, Instant};

use ruthenium_assets::Id;

use crate::decode::{Decoder, decode};
use crate::engine::{Engine, Fetch};
use crate::events::id_key;
use crate::files::SoundFiles;
use crate::mixer::StreamFeed;

/// How much of a stream is decoded ahead, in seconds.
const STREAM_AHEAD: u32 = 2;
/// How often streams are topped up.
const STREAM_PERIOD: Duration = Duration::from_millis(20);

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
    decoder: Decoder,
    looping: bool,
    out: rtrb::Producer<i16>,
    ended: Arc<AtomicBool>,
    /// Decoded samples that didn't fit yet.
    pending: Vec<i16>,
    at: usize,
}

fn lock(engine: &Weak<Mutex<Engine>>) -> Option<Arc<Mutex<Engine>>> {
    engine.upgrade()
}

fn run(files: &SoundFiles, engine: &Weak<Mutex<Engine>>, rx: &Receiver<Job>) {
    let mut streams: Vec<OpenStream> = Vec::new();
    loop {
        let job = if streams.is_empty() {
            rx.recv().map_err(|_| RecvTimeoutError::Disconnected)
        } else {
            rx.recv_timeout(STREAM_PERIOD)
        };
        match job {
            Ok(Job::Decode(file)) => {
                let started = Instant::now();
                let pcm = files
                    .read(&file)
                    .map_err(|e| e.to_string())
                    .and_then(decode)
                    .map(Arc::new);
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
    let bytes = files
        .read(file)
        .map_err(|e| format!("{}: {e}", id_key(file)))?;
    let decoder = Decoder::new(bytes.clone()).map_err(|e| format!("{}: {e}", id_key(file)))?;
    let size = (decoder.rate * u32::from(decoder.channels) * STREAM_AHEAD) as usize;
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
        decoder,
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
        match s.decoder.next_into(&mut s.pending) {
            Ok(true) => {}
            Ok(false) if s.looping => {
                // `LoopingAudioStream`: from the start again.
                match Decoder::new(s.bytes.clone()) {
                    Ok(d) => s.decoder = d,
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

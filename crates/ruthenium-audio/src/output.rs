//! Where the mixer's output goes: the sound card (through cpal, with the `device`
//! feature), or a WAV file paced in real time. The device is behind this seam so that
//! everything else builds and tests with no sound card or system library.
//!
//! The device's thread opens the default output, gives the engine a new mixer for it,
//! and watches it: when the stream fails or the system's default output changes, the
//! sounds stop and it opens again (vanilla reloads its sound engine then). When nothing
//! opens, the game runs silent, says so once, and tries again every few seconds.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crate::engine::Engine;
use crate::mixer::Mixer;
use crate::spatial::ListenerCell;
use crate::wav::WavWriter;

/// Where sound goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Output {
    /// The system's default output device.
    Device,
    /// A WAV file, written in real time, in place of the device.
    Wav(PathBuf),
}

/// The WAV file's rate.
pub const WAV_RATE: u32 = 48_000;

pub(crate) fn spawn(
    output: Output,
    engine: Arc<Mutex<Engine>>,
    listener: Arc<ListenerCell>,
    stop: Arc<AtomicBool>,
) -> std::io::Result<JoinHandle<()>> {
    std::thread::Builder::new()
        .name("sound-output".into())
        .spawn(move || match output {
            Output::Wav(path) => wav(&path, &engine, listener, &stop),
            Output::Device => device(&engine, &listener, &stop),
        })
}

fn lock(engine: &Mutex<Engine>) -> std::sync::MutexGuard<'_, Engine> {
    engine.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Renders at `WAV_RATE` as time passes and writes it to `path`.
fn wav(
    path: &std::path::Path,
    engine: &Mutex<Engine>,
    listener: Arc<ListenerCell>,
    stop: &AtomicBool,
) {
    let mut file = match WavWriter::create(path, WAV_RATE) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("[sound] {}: {e}; the game is silent", path.display());
            return;
        }
    };
    let (mut mixer, link) = Mixer::new(WAV_RATE, listener);
    lock(engine).attach(link);
    eprintln!("[sound] output: {} at {WAV_RATE} Hz", path.display());
    let started = Instant::now();
    let mut written: u64 = 0;
    let mut block = vec![0.0f32; 2 * 4096];
    let mut last_flush = Instant::now();
    while !stop.load(Ordering::Acquire) {
        let due = (started.elapsed().as_secs_f64() * f64::from(WAV_RATE)) as u64;
        while written < due {
            let n = ((due - written) as usize).min(block.len() / 2);
            mixer.render(&mut block[..2 * n]);
            if let Err(e) = file.write(&block[..2 * n]) {
                eprintln!("[sound] {}: {e}", path.display());
                lock(engine).detach();
                return;
            }
            written += n as u64;
        }
        if last_flush.elapsed() > Duration::from_secs(1) {
            let _ = file.flush(WAV_RATE);
            last_flush = Instant::now();
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    let _ = file.flush(WAV_RATE);
    lock(engine).detach();
}

#[cfg(not(feature = "device"))]
fn device(_engine: &Mutex<Engine>, _listener: &Arc<ListenerCell>, _stop: &AtomicBool) {
    eprintln!("[sound] built without a sound device: the game is silent");
}

#[cfg(feature = "device")]
fn device(engine: &Mutex<Engine>, listener: &Arc<ListenerCell>, stop: &AtomicBool) {
    let mut said = false;
    while !stop.load(Ordering::Acquire) {
        match cpal_out::open(engine, listener) {
            Ok(open) => {
                said = false;
                eprintln!("[sound] output: {}", open.describe());
                let mut checked = Instant::now();
                while !stop.load(Ordering::Acquire) {
                    std::thread::sleep(Duration::from_millis(100));
                    if open.failed() {
                        eprintln!("[sound] the output device went away");
                        break;
                    }
                    if checked.elapsed() > Duration::from_secs(2) {
                        checked = Instant::now();
                        if open.default_changed() {
                            eprintln!("[sound] the system's default output changed");
                            break;
                        }
                    }
                }
                lock(engine).detach();
                drop(open);
            }
            Err(e) => {
                if !said {
                    eprintln!("[sound] no output device ({e}): the game is silent");
                    said = true;
                }
                // Try again in a while, unless asked to stop.
                for _ in 0..30 {
                    if stop.load(Ordering::Acquire) {
                        return;
                    }
                    std::thread::sleep(Duration::from_millis(100));
                }
            }
        }
    }
}

#[cfg(feature = "device")]
mod cpal_out {
    use std::sync::Arc;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicBool, Ordering};

    use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
    use cpal::{BufferSize, FromSample, SampleFormat, SizedSample, StreamConfig};

    use super::lock;
    use crate::engine::Engine;
    use crate::mixer::Mixer;
    use crate::spatial::ListenerCell;

    /// Frames asked for per callback: 5.3 ms at 48 kHz (the device's own size if refused).
    const BUFFER: u32 = 256;
    /// The most frames rendered at once; larger callbacks are filled in pieces.
    const CHUNK: usize = 4096;

    pub(super) struct Open {
        _stream: cpal::Stream,
        failed: Arc<AtomicBool>,
        name: String,
        rate: u32,
        channels: u16,
        buffer: BufferSize,
        format: SampleFormat,
    }

    impl Open {
        pub(super) fn describe(&self) -> String {
            let buffer = match self.buffer {
                BufferSize::Fixed(n) => format!("{n} frames"),
                BufferSize::Default => "the device's buffer".into(),
            };
            format!(
                "{}, {} Hz, {} channels, {:?}, {buffer}",
                self.name, self.rate, self.channels, self.format
            )
        }

        pub(super) fn failed(&self) -> bool {
            self.failed.load(Ordering::Acquire)
        }

        pub(super) fn default_changed(&self) -> bool {
            name(cpal::default_host().default_output_device().as_ref()) != self.name
        }
    }

    fn name(d: Option<&cpal::Device>) -> String {
        d.and_then(|d| d.description().ok())
            .map_or_else(|| "(none)".into(), |d| d.name().to_owned())
    }

    pub(super) fn open(
        engine: &Mutex<Engine>,
        listener: &Arc<ListenerCell>,
    ) -> Result<Open, String> {
        let host = cpal::default_host();
        let device = host.default_output_device().ok_or("no default output")?;
        let supported = device.default_output_config().map_err(|e| e.to_string())?;
        let format = supported.sample_format();
        let channels = supported.channels();
        let rate = supported.sample_rate();
        let failed = Arc::new(AtomicBool::new(false));
        let mut last = String::new();
        for buffer in [BufferSize::Fixed(BUFFER), BufferSize::Default] {
            let config = StreamConfig {
                channels,
                sample_rate: rate,
                buffer_size: buffer,
            };
            let (mixer, link) = Mixer::new(rate, listener.clone());
            let built = match format {
                SampleFormat::F32 => build::<f32>(&device, &config, mixer, &failed),
                SampleFormat::I16 => build::<i16>(&device, &config, mixer, &failed),
                SampleFormat::U16 => build::<u16>(&device, &config, mixer, &failed),
                SampleFormat::I32 => build::<i32>(&device, &config, mixer, &failed),
                SampleFormat::F64 => build::<f64>(&device, &config, mixer, &failed),
                other => return Err(format!("unsupported sample format {other:?}")),
            };
            match built.and_then(|s| s.play().map(|()| s).map_err(|e| e.to_string())) {
                Ok(stream) => {
                    lock(engine).attach(link);
                    return Ok(Open {
                        _stream: stream,
                        failed,
                        name: name(Some(&device)),
                        rate,
                        channels,
                        buffer,
                        format,
                    });
                }
                Err(e) => last = e,
            }
        }
        Err(last)
    }

    fn build<T: SizedSample + FromSample<f32>>(
        device: &cpal::Device,
        config: &StreamConfig,
        mut mixer: Mixer,
        failed: &Arc<AtomicBool>,
    ) -> Result<cpal::Stream, String> {
        let channels = usize::from(config.channels.max(1));
        let mut scratch = vec![0.0f32; 2 * CHUNK];
        let failed = failed.clone();
        device
            .build_output_stream::<T, _, _>(
                *config,
                move |out: &mut [T], _| {
                    for piece in out.chunks_mut(CHUNK * channels) {
                        let frames = piece.len() / channels;
                        let mixed = &mut scratch[..2 * frames];
                        mixer.render(mixed);
                        for (f, frame) in piece.chunks_mut(channels).enumerate() {
                            let (l, r) = (mixed[2 * f], mixed[2 * f + 1]);
                            if channels == 1 {
                                frame[0] = T::from_sample((l + r) * 0.5);
                                continue;
                            }
                            for (c, s) in frame.iter_mut().enumerate() {
                                *s = T::from_sample(match c {
                                    0 => l,
                                    1 => r,
                                    _ => 0.0,
                                });
                            }
                        }
                    }
                },
                move |e| {
                    if e.kind() != cpal::ErrorKind::DeviceChanged {
                        failed.store(true, Ordering::Release);
                    }
                },
                None,
            )
            .map_err(|e| e.to_string())
    }
}

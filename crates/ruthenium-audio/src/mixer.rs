//! The mixer: runs in the output's callback, so it never allocates, frees, locks or
//! waits. The engine drives it through a single-producer ring of [`Command`]s; what a
//! voice played is handed back through another ring ([`Retired`]), so the last
//! reference to a sound's samples is never dropped here.
//!
//! Each voice is a slot the engine owns (vanilla's OpenAL sources): its samples (a whole
//! decoded sound or a stream being decoded elsewhere), gain, pitch and placement. A
//! block's gains are worked out once from the listener of the moment, and ramp from the
//! last block's over its first frames. Pitch is the playback rate (with the file's rate
//! against the device's), read with a four-point cubic spline as OpenAL Soft's default
//! resampler does.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use rtrb::{Consumer, Producer};

use crate::decode::Pcm;
use crate::spatial::{Listener, ListenerCell, Placement, mono_gains};

/// Vanilla's channel pools on OpenAL Soft's 255 sources (`Library.init`): the streaming
/// pool is `clamp(sqrt(255), 2, 8)`, the static pool the rest.
pub const STREAM_VOICES: usize = 8;
pub const STATIC_VOICES: usize = 247;
pub const VOICES: usize = STATIC_VOICES + STREAM_VOICES;

/// Commands the ring holds at once.
pub const COMMAND_CAPACITY: usize = 4096;
/// Every feed in a voice or in the command ring can be retired at once, so retiring
/// never finds the ring full.
pub const RETIRED_CAPACITY: usize = VOICES + COMMAND_CAPACITY;

/// Frames over which a voice's gains move to new ones, and a stopped voice fades.
const RAMP: usize = 64;

/// What a voice plays.
#[derive(Debug)]
pub enum Feed {
    Static(Arc<Pcm>),
    Stream(Box<StreamFeed>),
}

/// Samples decoded on another thread as a stream plays.
#[derive(Debug)]
pub struct StreamFeed {
    pub channels: u8,
    pub rate: u32,
    pub samples: Consumer<i16>,
    /// Set once the decoder has put the last samples in.
    pub ended: Arc<AtomicBool>,
}

/// How loud, how fast and where a voice plays.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VoiceParams {
    /// Vanilla's final volume: instance, category and master, each clamped to 0..1.
    pub gain: f32,
    /// Vanilla's pitch, clamped to 0.5..2: the playback rate.
    pub pitch: f32,
    pub place: Placement,
}

#[derive(Debug)]
pub enum Command {
    Play {
        slot: u16,
        feed: Feed,
        params: VoiceParams,
        looping: bool,
    },
    Set {
        slot: u16,
        params: VoiceParams,
    },
    Stop {
        slot: u16,
    },
}

/// A voice that has stopped, with what it played.
#[derive(Debug)]
pub struct Retired {
    pub slot: u16,
    pub feed: Feed,
}

#[derive(Debug, Default)]
struct Voice {
    feed: Option<Feed>,
    params: Option<VoiceParams>,
    looping: bool,
    /// The read position, in the source's frames.
    position: f64,
    /// Per input channel, the gain to each output channel: as last mixed.
    gains: [[f32; 2]; 2],
    fresh: bool,
    /// Frames left of the fade after a stop.
    stopping: Option<usize>,
    /// A stream's last four frames (`position` is between the second and third).
    window: [[f32; 2]; 4],
    /// Zero frames a stream has been given after its end.
    tail: u8,
}

/// See the module docs.
pub struct Mixer {
    rate: u32,
    voices: Box<[Voice]>,
    commands: Consumer<Command>,
    retired: Producer<Retired>,
    listener: Arc<ListenerCell>,
    last_listener: Listener,
}

impl std::fmt::Debug for Mixer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Mixer")
            .field("rate", &self.rate)
            .field("playing", &self.playing())
            .finish_non_exhaustive()
    }
}

/// The engine's ends of a mixer's rings.
#[derive(Debug)]
pub struct MixerLink {
    pub commands: Producer<Command>,
    pub retired: Consumer<Retired>,
    pub rate: u32,
}

impl Mixer {
    /// A mixer for output at `rate` frames a second, and the engine's ends of its rings.
    pub fn new(rate: u32, listener: Arc<ListenerCell>) -> (Mixer, MixerLink) {
        let (commands_in, commands) = rtrb::RingBuffer::new(COMMAND_CAPACITY);
        let (retired, retired_out) = rtrb::RingBuffer::new(RETIRED_CAPACITY);
        let mixer = Mixer {
            rate,
            voices: (0..VOICES).map(|_| Voice::default()).collect(),
            commands,
            retired,
            listener,
            last_listener: Listener::INITIAL,
        };
        (
            mixer,
            MixerLink {
                commands: commands_in,
                retired: retired_out,
                rate,
            },
        )
    }

    pub fn rate(&self) -> u32 {
        self.rate
    }

    /// Voices playing.
    pub fn playing(&self) -> usize {
        self.voices.iter().filter(|v| v.feed.is_some()).count()
    }

    /// Mixes the next `out.len() / 2` frames, left and right interleaved.
    pub fn render(&mut self, out: &mut [f32]) {
        out.fill(0.0);
        self.take_commands();
        if let Some(l) = self.listener.load() {
            self.last_listener = l;
        }
        let listener = self.last_listener;
        let frames = out.len() / 2;
        let ratio = 1.0 / f64::from(self.rate.max(1));
        for slot in 0..self.voices.len() {
            let v = &mut self.voices[slot];
            if v.feed.is_none() {
                continue;
            }
            let Some(params) = v.params else { continue };
            let (channels, rate) = match &v.feed {
                Some(Feed::Static(p)) => (p.channels, p.rate),
                Some(Feed::Stream(s)) => (s.channels, s.rate),
                None => continue,
            };
            let target = target_gains(&listener, &params, channels);
            if v.fresh {
                v.gains = target;
                v.fresh = false;
            }
            let start = v.gains;
            let step = f64::from(rate) * ratio * f64::from(params.pitch);
            let finished = match v.feed.as_mut() {
                Some(Feed::Static(pcm)) => mix_static(
                    pcm,
                    out,
                    frames,
                    &mut v.position,
                    step,
                    v.looping,
                    start,
                    target,
                    &mut v.stopping,
                ),
                Some(Feed::Stream(s)) => mix_stream(
                    s,
                    out,
                    frames,
                    &mut v.position,
                    step,
                    &mut v.window,
                    &mut v.tail,
                    start,
                    target,
                    &mut v.stopping,
                ),
                None => false,
            };
            v.gains = target;
            if finished {
                self.retire(slot);
            }
        }
        for s in out.iter_mut() {
            *s = s.clamp(-1.0, 1.0);
        }
    }

    fn take_commands(&mut self) {
        while let Ok(c) = self.commands.pop() {
            match c {
                Command::Play {
                    slot,
                    feed,
                    params,
                    looping,
                } => {
                    let slot = usize::from(slot);
                    if slot >= self.voices.len() {
                        self.give_back(slot as u16, feed);
                        continue;
                    }
                    self.retire(slot);
                    self.voices[slot] = Voice {
                        feed: Some(feed),
                        params: Some(params),
                        looping,
                        fresh: true,
                        ..Voice::default()
                    };
                }
                Command::Set { slot, params } => {
                    if let Some(v) = self.voices.get_mut(usize::from(slot)) {
                        v.params = Some(params);
                    }
                }
                Command::Stop { slot } => {
                    if let Some(v) = self.voices.get_mut(usize::from(slot))
                        && v.feed.is_some()
                        && v.stopping.is_none()
                    {
                        v.stopping = Some(RAMP);
                    }
                }
            }
        }
    }

    fn retire(&mut self, slot: usize) {
        if let Some(feed) = self.voices[slot].feed.take() {
            self.give_back(slot as u16, feed);
        }
        self.voices[slot] = Voice::default();
    }

    fn give_back(&mut self, slot: u16, feed: Feed) {
        if let Err(rtrb::PushError::Full(r)) = self.retired.push(Retired { slot, feed }) {
            // Can't happen (see RETIRED_CAPACITY); leaking beats freeing here.
            debug_assert!(false, "the retired ring is full");
            std::mem::forget(r);
        }
    }
}

/// Per input channel, the gains to the left and right outputs.
fn target_gains(listener: &Listener, params: &VoiceParams, channels: u8) -> [[f32; 2]; 2] {
    if channels == 1 {
        [mono_gains(listener, &params.place, params.gain), [0.0; 2]]
    } else {
        let g = params.gain.clamp(0.0, 1.0);
        [[g, 0.0], [0.0, g]]
    }
}

/// The gains for frame `i` of a block ramping from `a` to `b`, faded if stopping.
fn gains_at(i: usize, a: [[f32; 2]; 2], b: [[f32; 2]; 2], fade: f32) -> [[f32; 2]; 2] {
    let t = if i >= RAMP {
        1.0
    } else {
        (i + 1) as f32 / RAMP as f32
    };
    std::array::from_fn(|c| std::array::from_fn(|o| (a[c][o] + (b[c][o] - a[c][o]) * t) * fade))
}

/// Catmull-Rom through four samples, at `t` between the middle two.
fn cubic(s: [f32; 4], t: f32) -> f32 {
    let [a, b, c, d] = s;
    b + 0.5 * t * (c - a + t * (2.0 * a - 5.0 * b + 4.0 * c - d + t * (3.0 * (b - c) + d - a)))
}

const SCALE: f32 = 1.0 / 32768.0;

/// Frame `i`'s sample on `channel`, silence outside the sound (wrapping if looping).
fn sample(pcm: &Pcm, i: i64, channel: usize, looping: bool) -> f32 {
    let frames = pcm.frames() as i64;
    let i = if looping && frames > 0 {
        i.rem_euclid(frames)
    } else if i < 0 || i >= frames {
        return 0.0;
    } else {
        i
    };
    let c = usize::from(pcm.channels);
    f32::from(pcm.samples[i as usize * c + channel.min(c - 1)]) * SCALE
}

/// Mixes a whole sound into `out`; true once it has ended.
#[allow(clippy::too_many_arguments)]
fn mix_static(
    pcm: &Pcm,
    out: &mut [f32],
    frames: usize,
    position: &mut f64,
    step: f64,
    looping: bool,
    from: [[f32; 2]; 2],
    to: [[f32; 2]; 2],
    stopping: &mut Option<usize>,
) -> bool {
    let total = pcm.frames() as f64;
    let channels = usize::from(pcm.channels);
    for i in 0..frames {
        let fade = match stopping {
            Some(0) => return true,
            Some(n) => {
                *n -= 1;
                *n as f32 / RAMP as f32
            }
            None => 1.0,
        };
        if !looping && *position >= total {
            return true;
        }
        let base = position.floor();
        let t = (*position - base) as f32;
        let k = base as i64;
        let g = gains_at(i, from, to, fade);
        for (c, gc) in g.iter().enumerate().take(channels) {
            let s = cubic(
                [
                    sample(pcm, k - 1, c, looping),
                    sample(pcm, k, c, looping),
                    sample(pcm, k + 1, c, looping),
                    sample(pcm, k + 2, c, looping),
                ],
                t,
            );
            out[2 * i] += s * gc[0];
            out[2 * i + 1] += s * gc[1];
        }
        *position += step;
        if looping && *position >= total && total > 0.0 {
            *position -= total;
        }
    }
    matches!(stopping, Some(0))
}

/// Mixes a stream into `out`; true once it has ended.
#[allow(clippy::too_many_arguments)]
fn mix_stream(
    s: &mut StreamFeed,
    out: &mut [f32],
    frames: usize,
    position: &mut f64,
    step: f64,
    window: &mut [[f32; 2]; 4],
    tail: &mut u8,
    from: [[f32; 2]; 2],
    to: [[f32; 2]; 2],
    stopping: &mut Option<usize>,
) -> bool {
    let channels = usize::from(s.channels);
    for i in 0..frames {
        let fade = match stopping {
            Some(0) => return true,
            Some(n) => {
                *n -= 1;
                *n as f32 / RAMP as f32
            }
            None => 1.0,
        };
        // Move the window up to the read position.
        while *position >= 1.0 {
            *position -= 1.0;
            window.copy_within(1.., 0);
            window[3] = next_frame(s, channels, tail);
        }
        if *tail >= 3 {
            return true;
        }
        let g = gains_at(i, from, to, fade);
        let t = *position as f32;
        for c in 0..channels {
            let v = cubic([window[0][c], window[1][c], window[2][c], window[3][c]], t);
            out[2 * i] += v * g[c][0];
            out[2 * i + 1] += v * g[c][1];
        }
        *position += step;
    }
    matches!(stopping, Some(0))
}

/// The stream's next frame: silence while the decoder is behind, and after its end.
fn next_frame(s: &mut StreamFeed, channels: usize, tail: &mut u8) -> [f32; 2] {
    if s.samples.slots() >= channels {
        let mut f = [0.0; 2];
        for v in f.iter_mut().take(channels) {
            *v = s.samples.pop().map_or(0.0, |x| f32::from(x) * SCALE);
        }
        return f;
    }
    if s.ended.load(Ordering::Acquire) && s.samples.slots() < channels {
        *tail = tail.saturating_add(1);
    }
    [0.0; 2]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spatial::Placement;

    pub(crate) fn tone(channels: u8, rate: u32, frames: usize, value: i16) -> Arc<Pcm> {
        Arc::new(Pcm {
            channels,
            rate,
            samples: vec![value; frames * usize::from(channels)].into_boxed_slice(),
        })
    }

    fn params(gain: f32, pitch: f32, position: [f32; 3], max: Option<f32>) -> VoiceParams {
        VoiceParams {
            gain,
            pitch,
            place: Placement {
                position,
                relative: false,
                max_distance: max,
            },
        }
    }

    fn mixer() -> (Mixer, MixerLink) {
        Mixer::new(48_000, Arc::new(ListenerCell::default()))
    }

    fn play(link: &mut MixerLink, slot: u16, feed: Feed, p: VoiceParams, looping: bool) {
        link.commands
            .push(Command::Play {
                slot,
                feed,
                params: p,
                looping,
            })
            .unwrap();
    }

    /// The level of frame `at` of a block of `frames`.
    fn level(m: &mut Mixer, frames: usize, at: usize) -> [f32; 2] {
        let mut out = vec![0.0; frames * 2];
        m.render(&mut out);
        [out[2 * at], out[2 * at + 1]]
    }

    const HALF: i16 = 16384;

    #[test]
    fn a_mono_sound_is_placed_and_attenuated() {
        let (mut m, mut link) = mixer();
        // 4 blocks to the left of a listener at the origin facing -z, range 16.
        play(
            &mut link,
            0,
            Feed::Static(tone(1, 48_000, 48_000, HALF)),
            params(0.8, 1.0, [-4.0, 0.0, 0.0], Some(16.0)),
            false,
        );
        let [l, r] = level(&mut m, 256, 100);
        assert!(
            (l - 0.5 * 0.8 * 0.75).abs() < 1e-4 && r.abs() < 1e-6,
            "{l} {r}"
        );
    }

    #[test]
    fn a_stereo_sound_is_neither_placed_nor_attenuated() {
        let (mut m, mut link) = mixer();
        let pcm = Arc::new(Pcm {
            channels: 2,
            rate: 48_000,
            samples: [HALF, -HALF].repeat(1000).into_boxed_slice(),
        });
        play(
            &mut link,
            3,
            Feed::Static(pcm),
            params(0.6, 1.0, [-100.0, 0.0, 0.0], Some(16.0)),
            false,
        );
        let [l, r] = level(&mut m, 256, 100);
        assert!((l - 0.3).abs() < 1e-4 && (r + 0.3).abs() < 1e-4, "{l} {r}");
    }

    #[test]
    fn pitch_is_the_playback_rate() {
        // A 44.1 kHz sound of 4410 frames lasts 0.1 s: 4800 frames at 48 kHz, 2400 at
        // pitch 2 and 9600 at pitch 0.5.
        for (pitch, want) in [(1.0, 4800), (2.0, 2400), (0.5, 9600)] {
            let (mut m, mut link) = mixer();
            play(
                &mut link,
                0,
                Feed::Static(tone(1, 44_100, 4410, HALF)),
                params(1.0, pitch, [0.0, 0.0, -1.0], None),
                false,
            );
            let mut out = vec![0.0; 20_000 * 2];
            m.render(&mut out);
            let last = out.chunks(2).rposition(|f| f[0] > 0.05).unwrap() + 1;
            assert!(last.abs_diff(want) <= 2, "pitch {pitch}: {last} frames");
            assert_eq!(m.playing(), 0);
            assert_eq!(link.retired.pop().unwrap().slot, 0);
        }
    }

    #[test]
    fn looping_goes_on_and_stop_fades_and_retires() {
        let (mut m, mut link) = mixer();
        play(
            &mut link,
            7,
            Feed::Static(tone(1, 48_000, 100, HALF)),
            params(1.0, 1.0, [0.0, 0.0, -1.0], None),
            true,
        );
        let mut out = vec![0.0; 1000 * 2];
        m.render(&mut out);
        assert!(out[2 * 990] > 0.25);
        assert_eq!(m.playing(), 1);
        link.commands.push(Command::Stop { slot: 7 }).unwrap();
        m.render(&mut out);
        assert!(
            out[2 * 10] > 0.0 && out[2 * 100] == 0.0,
            "faded within 64 frames"
        );
        assert_eq!(m.playing(), 0);
        let r = link.retired.pop().unwrap();
        assert_eq!(r.slot, 7);
        assert!(matches!(r.feed, Feed::Static(_)));
    }

    #[test]
    fn new_params_reach_a_playing_voice() {
        let (mut m, mut link) = mixer();
        let p = params(1.0, 1.0, [0.0, 0.0, -1.0], None);
        play(
            &mut link,
            1,
            Feed::Static(tone(1, 48_000, 48_000, HALF)),
            p,
            false,
        );
        let front = 0.5 + 0.095_662_1;
        assert!((level(&mut m, 256, 200)[0] - 0.5 * front).abs() < 1e-4);
        link.commands
            .push(Command::Set {
                slot: 1,
                params: VoiceParams { gain: 0.25, ..p },
            })
            .unwrap();
        // Ramped over the first 64 frames, then held.
        assert!((level(&mut m, 256, 200)[0] - 0.125 * front).abs() < 1e-4);
    }

    #[test]
    fn the_listener_moves_the_sound() {
        let cell = Arc::new(ListenerCell::default());
        let (mut m, mut link) = Mixer::new(48_000, cell.clone());
        play(
            &mut link,
            0,
            Feed::Static(tone(1, 48_000, 48_000, HALF)),
            params(1.0, 1.0, [10.0, 64.0, 0.0], Some(16.0)),
            false,
        );
        // A listener at x 2 facing south: the sound is 8 blocks to the left.
        cell.store(&Listener::camera([2.0, 64.0, 0.0], 0.0, 0.0));
        let [l, r] = level(&mut m, 256, 200);
        assert!((l - 0.5 * 0.5).abs() < 1e-4 && r.abs() < 1e-5, "{l} {r}");
    }

    #[test]
    fn many_voices_add_up_and_the_output_clips() {
        let (mut m, mut link) = mixer();
        for slot in 0..4 {
            play(
                &mut link,
                slot,
                Feed::Static(tone(2, 48_000, 1000, HALF)),
                params(1.0, 1.0, [0.0; 3], None),
                false,
            );
        }
        assert_eq!(level(&mut m, 64, 10), [1.0, 1.0]);
        assert_eq!(m.playing(), 4);
    }

    #[test]
    fn a_stream_plays_what_arrives_and_ends_with_its_decoder() {
        let (mut m, mut link) = mixer();
        let (mut tx, rx) = rtrb::RingBuffer::new(4096);
        let ended = Arc::new(AtomicBool::new(false));
        for _ in 0..1000 {
            tx.push(HALF).unwrap();
        }
        play(
            &mut link,
            250,
            Feed::Stream(Box::new(StreamFeed {
                channels: 1,
                rate: 48_000,
                samples: rx,
                ended: ended.clone(),
            })),
            params(1.0, 1.0, [0.0; 3], None),
            false,
        );
        let mut out = vec![0.0; 2000 * 2];
        m.render(&mut out);
        let front = 0.5 + 0.095_662_1;
        assert!((out[2 * 500] - 0.5 * front).abs() < 1e-4);
        // The decoder is behind: silence, but the voice stays.
        assert!(out[2 * 1500].abs() < 1e-6);
        assert_eq!(m.playing(), 1);
        ended.store(true, Ordering::Release);
        m.render(&mut out);
        assert_eq!(m.playing(), 0);
        assert!(matches!(link.retired.pop().unwrap().feed, Feed::Stream(_)));
    }
}

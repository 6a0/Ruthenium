//! Ogg Vorbis to 16-bit samples, as vanilla's `JOrbisAudioStream` delivers them to
//! OpenAL: interleaved, each float sample `s` stored as `(int)(s * 32767.5 - 0.5)`
//! clamped to 16 bits (`ChunkedSampleByteBuf`).
//!
//! Decoding is Symphonia's (pure Rust, maintained, MPL-2.0). Lewton was the other pure
//! Rust choice; it has had no release since 2021.

use std::io::Cursor;

use ruthenium_assets::Limits;
use symphonia::core::codecs::CodecParameters;
use symphonia::core::codecs::audio::{AudioDecoder, AudioDecoderOptions};
use symphonia::core::errors::Error;
use symphonia::core::formats::{FormatOptions, FormatReader, TrackType};
use symphonia::core::io::{MediaSourceStream, MediaSourceStreamOptions};
use symphonia::default::codecs::VorbisDecoder;
use symphonia::default::formats::OggReader;

/// A whole decoded sound.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pcm {
    /// 1 (placed in the world) or 2 (played as it is).
    pub channels: u8,
    pub rate: u32,
    /// Interleaved samples.
    pub samples: Box<[i16]>,
}

impl Pcm {
    pub fn frames(&self) -> usize {
        self.samples.len() / usize::from(self.channels.max(1))
    }

    /// What the cache counts it as.
    pub fn bytes(&self) -> usize {
        self.samples.len() * 2 + std::mem::size_of::<Pcm>()
    }
}

/// `ChunkedSampleByteBuf.accept`.
pub fn to_i16(s: f32) -> i16 {
    ((s * 32767.5 - 0.5) as i32).clamp(-32768, 32767) as i16
}

/// A Vorbis stream being decoded a packet at a time.
pub struct Decoder {
    reader: OggReader<'static>,
    decoder: VorbisDecoder,
    track: u32,
    pub channels: u8,
    pub rate: u32,
    /// The frames the file says it has (its last page's granule position), if it says.
    pub frames: Option<u64>,
    floats: Vec<f32>,
}

impl std::fmt::Debug for Decoder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Decoder")
            .field("channels", &self.channels)
            .field("rate", &self.rate)
            .finish_non_exhaustive()
    }
}

impl Decoder {
    pub fn new(bytes: Vec<u8>) -> Result<Decoder, String> {
        Decoder::with_limits(bytes, &Limits::trusted())
    }

    /// A decoder for a file of a pack read under `limits`: a sample rate above
    /// [`Limits::sample_rate`] is refused (the stream buffers are seconds of it).
    /// `bytes` may be shared (`Arc<[u8]>`): a looping stream starts its decoder again
    /// on the same bytes, not a copy of them.
    pub fn with_limits(
        bytes: impl AsRef<[u8]> + Send + Sync + 'static,
        limits: &Limits,
    ) -> Result<Decoder, String> {
        let source = MediaSourceStream::new(
            Box::new(Cursor::new(bytes)),
            MediaSourceStreamOptions::default(),
        );
        let reader = OggReader::try_new(source, FormatOptions::default()).map_err(err)?;
        let track = reader
            .default_track(TrackType::Audio)
            .ok_or("no audio track")?;
        let Some(CodecParameters::Audio(params)) = &track.codec_params else {
            return Err("no audio codec parameters".into());
        };
        let channels = params
            .channels
            .as_ref()
            .map_or(0, |c| c.count())
            .try_into()
            .unwrap_or(0);
        if !(1..=2).contains(&channels) {
            return Err(format!("{channels} channels"));
        }
        let rate = params
            .sample_rate
            .filter(|&r| r > 0)
            .ok_or("no sample rate")?;
        if rate > limits.sample_rate {
            return Err(format!(
                "{rate} Hz is above what a sound may have ({} Hz)",
                limits.sample_rate
            ));
        }
        // Checked before the decoder reads the headers: they are the pack's numbers.
        let decoder =
            VorbisDecoder::try_new(params, &AudioDecoderOptions::default()).map_err(err)?;
        let frames = track.num_frames;
        Ok(Decoder {
            track: track.id,
            reader,
            decoder,
            channels,
            rate,
            frames,
            floats: Vec::new(),
        })
    }

    /// Appends the next packet's samples to `out`; false at the end of the stream.
    pub fn next_into(&mut self, out: &mut Vec<i16>) -> Result<bool, String> {
        loop {
            let packet = match self.reader.next_packet() {
                Ok(Some(p)) => p,
                Ok(None) => return Ok(false),
                Err(Error::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                    return Ok(false);
                }
                Err(e) => return Err(err(e)),
            };
            if packet.track_id != self.track {
                continue;
            }
            let buf = match self.decoder.decode(&packet) {
                Ok(b) => b,
                // A damaged packet is skipped, as JOrbis skips one it can't synthesise.
                Err(Error::DecodeError(_)) => continue,
                Err(e) => return Err(err(e)),
            };
            if usize::from(self.channels) != buf.spec().channels().count() {
                return Err("the channel count changed".into());
            }
            buf.copy_to_vec_interleaved(&mut self.floats);
            out.extend(self.floats.iter().map(|&s| to_i16(s)));
            return Ok(true);
        }
    }
}

fn err(e: Error) -> String {
    e.to_string()
}

/// A whole file (`SoundBufferLibrary.getCompleteBuffer`).
pub fn decode(bytes: Vec<u8>) -> Result<Pcm, String> {
    decode_limited(bytes, &Limits::trusted(), usize::MAX).map_err(|e| e.to_string())
}

/// Why a whole sound was not decoded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DecodeError {
    /// It would fit a sound's limit, but not the room the sounds alive now leave: a later
    /// play may have it.
    HeldBack,
    /// The file itself: broken, or longer than a sound may be. It never will decode.
    Broken(String),
}

impl std::fmt::Display for DecodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DecodeError::HeldBack => f.write_str("too much sound is decoded at once"),
            DecodeError::Broken(e) => f.write_str(e),
        }
    }
}

/// [`decode`] for a file of a pack read under `limits`, of at most
/// [`Limits::sound_samples`] (a longer one is refused: vanilla would decode it all; one
/// not streamed is held whole while it plays) and at most `room`, what the sounds alive
/// now leave. Only a sound refused for the room alone is [`DecodeError::HeldBack`]; a
/// file that says it is longer than a sound may be is refused before it is decoded.
pub fn decode_limited(bytes: Vec<u8>, limits: &Limits, room: usize) -> Result<Pcm, DecodeError> {
    let broken = DecodeError::Broken;
    let too_long = || {
        broken(format!(
            "longer than a sound decoded whole may be ({} samples)",
            limits.sound_samples
        ))
    };
    let mut d = Decoder::with_limits(bytes, limits).map_err(broken)?;
    let declared = d.frames.map(|f| f.saturating_mul(u64::from(d.channels)));
    if declared.is_some_and(|n| n > limits.sound_samples as u64) {
        return Err(too_long());
    }
    if declared.is_some_and(|n| n > room as u64) {
        return Err(DecodeError::HeldBack);
    }
    let mut samples = Vec::new();
    while d.next_into(&mut samples).map_err(broken)? {
        if samples.len() > limits.sound_samples {
            return Err(too_long());
        }
        if samples.len() > room {
            return Err(DecodeError::HeldBack);
        }
    }
    Ok(Pcm {
        channels: d.channels,
        rate: d.rate,
        samples: samples.into_boxed_slice(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn floats_become_vanillas_shorts() {
        // (int)(s * 32767.5f - 0.5f), truncated toward zero, clamped.
        assert_eq!(to_i16(0.0), 0);
        assert_eq!(to_i16(1.0), 32767);
        assert_eq!(to_i16(-1.0), -32768);
        assert_eq!(to_i16(2.0), 32767);
        assert_eq!(to_i16(-2.0), -32768);
        assert_eq!(to_i16(0.5), 16383);
        assert_eq!(to_i16(-0.5), -16384);
        assert_eq!(to_i16(1.0 / 32767.5), 0);
        assert_eq!(to_i16(f32::NAN), 0);
    }

    #[test]
    fn refuses_what_is_not_vorbis() {
        assert!(decode(b"not an ogg file".to_vec()).is_err());
        assert!(decode(Vec::new()).is_err());
    }
}

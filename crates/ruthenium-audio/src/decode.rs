//! Ogg Vorbis to 16-bit samples, as vanilla's `JOrbisAudioStream` delivers them to
//! OpenAL: interleaved, each float sample `s` stored as `(int)(s * 32767.5 - 0.5)`
//! clamped to 16 bits (`ChunkedSampleByteBuf`).
//!
//! Decoding is Symphonia's (pure Rust, maintained, MPL-2.0). Lewton was the other pure
//! Rust choice; it has had no release since 2021.

use std::io::Cursor;

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
        let decoder =
            VorbisDecoder::try_new(params, &AudioDecoderOptions::default()).map_err(err)?;
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
        Ok(Decoder {
            track: track.id,
            reader,
            decoder,
            channels,
            rate,
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
    let mut d = Decoder::new(bytes)?;
    let mut samples = Vec::new();
    while d.next_into(&mut samples)? {}
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

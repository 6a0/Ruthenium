//! A WAV file of what the mixer made (32-bit float, stereo), for checking levels and
//! balance as numbers instead of by ear.

use std::fs::File;
use std::io::{BufWriter, Seek, SeekFrom, Write};
use std::path::Path;

#[derive(Debug)]
pub struct WavWriter {
    out: BufWriter<File>,
    frames: u64,
}

impl WavWriter {
    pub fn create(path: &Path, rate: u32) -> std::io::Result<WavWriter> {
        let mut out = BufWriter::new(File::create(path)?);
        out.write_all(&header(rate, 0))?;
        Ok(WavWriter { out, frames: 0 })
    }

    /// Appends interleaved stereo frames.
    pub fn write(&mut self, samples: &[f32]) -> std::io::Result<()> {
        for s in samples {
            self.out.write_all(&s.to_le_bytes())?;
        }
        self.frames += samples.len() as u64 / 2;
        Ok(())
    }

    /// Writes the lengths into the header, so the file is whole so far.
    pub fn flush(&mut self, rate: u32) -> std::io::Result<()> {
        self.out.flush()?;
        let file = self.out.get_mut();
        let end = file.stream_position()?;
        file.seek(SeekFrom::Start(0))?;
        file.write_all(&header(rate, self.frames))?;
        file.seek(SeekFrom::Start(end))?;
        Ok(())
    }
}

/// RIFF/WAVE with a `fmt ` chunk of IEEE floats (format 3), two channels.
fn header(rate: u32, frames: u64) -> Vec<u8> {
    let data = (frames * 8).min(u64::from(u32::MAX - 64)) as u32;
    let mut h = Vec::with_capacity(44);
    h.extend_from_slice(b"RIFF");
    h.extend_from_slice(&(36 + data).to_le_bytes());
    h.extend_from_slice(b"WAVEfmt ");
    h.extend_from_slice(&16u32.to_le_bytes());
    h.extend_from_slice(&3u16.to_le_bytes());
    h.extend_from_slice(&2u16.to_le_bytes());
    h.extend_from_slice(&rate.to_le_bytes());
    h.extend_from_slice(&(rate * 8).to_le_bytes());
    h.extend_from_slice(&8u16.to_le_bytes());
    h.extend_from_slice(&32u16.to_le_bytes());
    h.extend_from_slice(b"data");
    h.extend_from_slice(&data.to_le_bytes());
    h
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_a_readable_float_wav() {
        let path = std::env::temp_dir().join(format!("ruthenium-wav-{}.wav", std::process::id()));
        let mut w = WavWriter::create(&path, 48_000).unwrap();
        w.write(&[0.5, -0.25, 1.0, 0.0]).unwrap();
        w.flush(48_000).unwrap();
        drop(w);
        let b = std::fs::read(&path).unwrap();
        let _ = std::fs::remove_file(&path);
        assert_eq!(b.len(), 44 + 16);
        assert_eq!(&b[0..4], b"RIFF");
        assert_eq!(u32::from_le_bytes(b[4..8].try_into().unwrap()), 36 + 16);
        assert_eq!(u16::from_le_bytes(b[20..22].try_into().unwrap()), 3);
        assert_eq!(u32::from_le_bytes(b[24..28].try_into().unwrap()), 48_000);
        assert_eq!(u32::from_le_bytes(b[40..44].try_into().unwrap()), 16);
        assert_eq!(f32::from_le_bytes(b[48..52].try_into().unwrap()), -0.25);
    }
}

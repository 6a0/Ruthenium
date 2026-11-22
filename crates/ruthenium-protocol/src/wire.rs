//! The protocol's basic encodings: VarInts, strings and frames.
//!
//! A frame is a VarInt byte length (at most three bytes, so under 2 MiB) and that many
//! bytes: the packet id as a VarInt, then its fields. Strings are a VarInt byte length and
//! UTF-8; their limit is in UTF-16 units, as Java counts them
//! (`network/Utf8String.java`).

use std::fmt;

/// The longest frame a three-byte VarInt length can announce
/// (`network/Varint21FrameDecoder.java`).
pub const MAX_FRAME: usize = (1 << 21) - 1;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WireError {
    /// The data ended inside a value.
    Truncated,
    /// A VarInt longer than five bytes, or a frame length longer than three.
    VarIntTooLong,
    /// A string or frame over its limit.
    TooLong {
        len: usize,
        max: usize,
    },
    NegativeLength(i32),
    BadUtf8,
}

impl fmt::Display for WireError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            WireError::Truncated => f.write_str("data ended early"),
            WireError::VarIntTooLong => f.write_str("VarInt too long"),
            WireError::TooLong { len, max } => write!(f, "{len} is over the limit of {max}"),
            WireError::NegativeLength(n) => write!(f, "negative length {n}"),
            WireError::BadUtf8 => f.write_str("string is not UTF-8"),
        }
    }
}

impl std::error::Error for WireError {}

pub fn write_varint(out: &mut Vec<u8>, value: i32) {
    let mut v = value as u32;
    loop {
        if v & !0x7F == 0 {
            out.push(v as u8);
            return;
        }
        out.push((v as u8 & 0x7F) | 0x80);
        v >>= 7;
    }
}

pub fn varint_len(value: i32) -> usize {
    let v = value as u32;
    match v {
        0..0x80 => 1,
        0x80..0x4000 => 2,
        0x4000..0x20_0000 => 3,
        0x20_0000..0x1000_0000 => 4,
        _ => 5,
    }
}

/// Decodes a VarInt from the start of `buf`: `Ok(None)` if more bytes are needed,
/// otherwise the value and its length.
pub fn peek_varint(buf: &[u8], max_bytes: usize) -> Result<Option<(i32, usize)>, WireError> {
    let mut v: u32 = 0;
    for (i, &b) in buf.iter().enumerate() {
        if i >= max_bytes {
            return Err(WireError::VarIntTooLong);
        }
        v |= u32::from(b & 0x7F) << (7 * i);
        if b & 0x80 == 0 {
            return Ok(Some((v as i32, i + 1)));
        }
    }
    if buf.len() >= max_bytes {
        Err(WireError::VarIntTooLong)
    } else {
        Ok(None)
    }
}

/// A string field: VarInt byte length, then UTF-8.
pub fn write_string(out: &mut Vec<u8>, s: &str) {
    write_varint(out, s.len() as i32);
    out.extend_from_slice(s.as_bytes());
}

/// Frames a packet: `id` and `body` behind their total length.
pub fn frame(id: u32, body: &[u8]) -> Vec<u8> {
    let mut packet = Vec::with_capacity(body.len() + 5);
    write_varint(&mut packet, id as i32);
    packet.extend_from_slice(body);
    let mut out = Vec::with_capacity(packet.len() + 3);
    write_varint(&mut out, packet.len() as i32);
    out.extend_from_slice(&packet);
    out
}

/// Splits the first complete frame off `buf`: `Ok(None)` until all of it is there,
/// then the frame's bytes (packet id and fields) and how much of `buf` it used.
pub fn split_frame(buf: &[u8]) -> Result<Option<(&[u8], usize)>, WireError> {
    let Some((len, n)) = peek_varint(buf, 3)? else {
        return Ok(None);
    };
    if len < 0 {
        return Err(WireError::NegativeLength(len));
    }
    let len = len as usize;
    if len > MAX_FRAME {
        return Err(WireError::TooLong {
            len,
            max: MAX_FRAME,
        });
    }
    Ok(buf.get(n..n + len).map(|f| (f, n + len)))
}

/// Reads fields from a packet's bytes.
#[derive(Debug, Clone)]
pub struct Reader<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    pub fn new(buf: &'a [u8]) -> Reader<'a> {
        Reader { buf, pos: 0 }
    }

    pub fn remaining(&self) -> usize {
        self.buf.len() - self.pos
    }

    pub fn bytes(&mut self, n: usize) -> Result<&'a [u8], WireError> {
        let b = self
            .buf
            .get(self.pos..self.pos + n)
            .ok_or(WireError::Truncated)?;
        self.pos += n;
        Ok(b)
    }

    pub fn varint(&mut self) -> Result<i32, WireError> {
        match peek_varint(&self.buf[self.pos..], 5)? {
            Some((v, n)) => {
                self.pos += n;
                Ok(v)
            }
            None => Err(WireError::Truncated),
        }
    }

    pub fn i64(&mut self) -> Result<i64, WireError> {
        Ok(i64::from_be_bytes(self.bytes(8)?.try_into().unwrap()))
    }

    /// A string of at most `max` UTF-16 units (and so at most `3 * max` bytes).
    pub fn string(&mut self, max: usize) -> Result<&'a str, WireError> {
        let len = self.varint()?;
        if len < 0 {
            return Err(WireError::NegativeLength(len));
        }
        let len = len as usize;
        if len > max * 3 {
            return Err(WireError::TooLong { len, max: max * 3 });
        }
        let s = std::str::from_utf8(self.bytes(len)?).map_err(|_| WireError::BadUtf8)?;
        let units = s.encode_utf16().count();
        if units > max {
            return Err(WireError::TooLong { len: units, max });
        }
        Ok(s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn varints_round_trip() {
        for v in [
            0,
            1,
            127,
            128,
            255,
            25565,
            2_097_151,
            i32::MAX,
            -1,
            i32::MIN,
        ] {
            let mut out = Vec::new();
            write_varint(&mut out, v);
            assert_eq!(out.len(), varint_len(v), "{v}");
            assert_eq!(peek_varint(&out, 5), Ok(Some((v, out.len()))));
            assert_eq!(peek_varint(&out[..out.len() - 1], 5), Ok(None));
        }
        let mut out = Vec::new();
        write_varint(&mut out, 25565);
        assert_eq!(out, [0xDD, 0xC7, 0x01]);
        assert_eq!(peek_varint(&[0x80; 6], 5), Err(WireError::VarIntTooLong));
    }

    #[test]
    fn frames_split_when_whole() {
        let f = frame(1, &[9, 8, 7]);
        assert_eq!(f, [4, 1, 9, 8, 7]);
        assert_eq!(split_frame(&f[..3]), Ok(None));
        let mut two = f.clone();
        two.extend_from_slice(&f);
        assert_eq!(split_frame(&two), Ok(Some((&f[1..], 5))));
        assert!(split_frame(&[0x80, 0x80, 0x80, 0x01]).is_err());
    }

    #[test]
    fn strings_count_utf16_units() {
        let mut out = Vec::new();
        write_string(&mut out, "héllo 𝄞");
        let mut r = Reader::new(&out);
        assert_eq!(r.clone().string(8), Ok("héllo 𝄞"));
        // The clef is two UTF-16 units, so the string is 8 long.
        assert!(matches!(
            r.string(7),
            Err(WireError::TooLong { len: 8, max: 7 })
        ));
    }
}

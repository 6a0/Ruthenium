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
    /// Bytes left after a packet's last field: vanilla refuses the packet
    /// (`PacketDecoder.java`), and so do we.
    TrailingBytes(usize),
    /// A value its field can't hold: an identifier with bad characters, an id out of
    /// range, NBT that doesn't parse.
    Invalid(&'static str),
}

impl fmt::Display for WireError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            WireError::Truncated => f.write_str("data ended early"),
            WireError::VarIntTooLong => f.write_str("VarInt too long"),
            WireError::TooLong { len, max } => write!(f, "{len} is over the limit of {max}"),
            WireError::NegativeLength(n) => write!(f, "negative length {n}"),
            WireError::BadUtf8 => f.write_str("string is not UTF-8"),
            WireError::TrailingBytes(n) => write!(f, "{n} byte(s) left over after the packet"),
            WireError::Invalid(what) => write!(f, "bad {what}"),
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

/// Java's `writeUtf` default limit, and the length of most strings on the wire.
pub const MAX_STRING: usize = 32767;

pub fn write_bool(out: &mut Vec<u8>, v: bool) {
    out.push(u8::from(v));
}

pub fn write_u16(out: &mut Vec<u8>, v: u16) {
    out.extend_from_slice(&v.to_be_bytes());
}

pub fn write_i32(out: &mut Vec<u8>, v: i32) {
    out.extend_from_slice(&v.to_be_bytes());
}

pub fn write_i64(out: &mut Vec<u8>, v: i64) {
    out.extend_from_slice(&v.to_be_bytes());
}

pub fn write_f32(out: &mut Vec<u8>, v: f32) {
    out.extend_from_slice(&v.to_be_bytes());
}

/// `LpVec3.write` (the server's side; for tests).
pub fn write_lp_vec3(out: &mut Vec<u8>, v: [f64; 3]) {
    let sanitize = |x: f64| {
        if x.is_nan() {
            0.0
        } else {
            x.clamp(-1.7179869183E10, 1.7179869183E10)
        }
    };
    let [x, y, z] = v.map(sanitize);
    let length = x.abs().max(y.abs()).max(z.abs());
    if length < 3.051944088384301E-5 {
        out.push(0);
        return;
    }
    let scale = length.ceil() as i64;
    let partial = scale & 3 != scale;
    let markers = if partial { scale & 3 | 4 } else { scale };
    // Java's Math.round on a value that is never negative here.
    let pack = |x: f64| ((x * 0.5 + 0.5) * 32766.0 + 0.5).floor() as i64;
    let s = scale as f64;
    let buffer = markers | pack(x / s) << 3 | pack(y / s) << 18 | pack(z / s) << 33;
    out.push(buffer as u8);
    out.push((buffer >> 8) as u8);
    out.extend_from_slice(&((buffer >> 16) as i32).to_be_bytes());
    if partial {
        write_varint(out, (scale >> 2) as i32);
    }
}

pub fn write_f64(out: &mut Vec<u8>, v: f64) {
    out.extend_from_slice(&v.to_be_bytes());
}

/// A UUID as two big-endian longs, most significant first (`UUIDUtil.STREAM_CODEC`).
pub fn write_uuid(out: &mut Vec<u8>, v: u128) {
    out.extend_from_slice(&v.to_be_bytes());
}

/// A byte array: VarInt length, then the bytes (`FriendlyByteBuf.writeByteArray`).
pub fn write_byte_array(out: &mut Vec<u8>, bytes: &[u8]) {
    write_varint(out, bytes.len() as i32);
    out.extend_from_slice(bytes);
}

/// A packet's bytes before framing: its id as a VarInt, then what `body` writes.
pub fn packet(id: u32, body: impl FnOnce(&mut Vec<u8>)) -> Vec<u8> {
    let mut out = Vec::with_capacity(32);
    write_varint(&mut out, id as i32);
    body(&mut out);
    out
}

/// Whether `s` is an identifier vanilla accepts (`Identifier.parse`): an optional
/// namespace of `[a-z0-9_.-]` and a colon, then a path of `[a-z0-9_.-/]`.
pub fn valid_identifier(s: &str) -> bool {
    let (ns, path) = s.split_once(':').unwrap_or(("minecraft", s));
    let ok = |c: u8, slash: bool| {
        matches!(c, b'a'..=b'z' | b'0'..=b'9' | b'_' | b'.' | b'-') || (slash && c == b'/')
    };
    ns.bytes().all(|c| ok(c, false)) && path.bytes().all(|c| ok(c, true))
}

/// `s` with its namespace spelled out (`stone` is `minecraft:stone`).
pub fn full_identifier(s: &str) -> String {
    match s.split_once(':') {
        Some(("", path)) => format!("minecraft:{path}"),
        Some(_) => s.to_owned(),
        None => format!("minecraft:{s}"),
    }
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

    pub fn u8(&mut self) -> Result<u8, WireError> {
        Ok(self.bytes(1)?[0])
    }

    /// Any byte but zero is true, as Netty's `readBoolean` reads it.
    pub fn bool(&mut self) -> Result<bool, WireError> {
        Ok(self.u8()? != 0)
    }

    pub fn u16(&mut self) -> Result<u16, WireError> {
        Ok(u16::from_be_bytes(self.bytes(2)?.try_into().unwrap()))
    }

    pub fn i32(&mut self) -> Result<i32, WireError> {
        Ok(i32::from_be_bytes(self.bytes(4)?.try_into().unwrap()))
    }

    pub fn f32(&mut self) -> Result<f32, WireError> {
        Ok(f32::from_be_bytes(self.bytes(4)?.try_into().unwrap()))
    }

    /// `LpVec3.read`: a vector in 15 bits per axis times a shared integer scale (velocities
    /// in entity packets); one zero byte is the zero vector.
    pub fn lp_vec3(&mut self) -> Result<[f64; 3], WireError> {
        let lowest = self.u8()?;
        if lowest == 0 {
            return Ok([0.0; 3]);
        }
        let middle = self.u8()?;
        let highest = self.bytes(4)?;
        let highest = u32::from_be_bytes([highest[0], highest[1], highest[2], highest[3]]);
        let buffer = i64::from(highest) << 16 | i64::from(middle) << 8 | i64::from(lowest);
        let mut scale = i64::from(lowest & 3);
        if lowest & 4 == 4 {
            scale |= (i64::from(self.varint()?) & 0xFFFF_FFFF) << 2;
        }
        let unpack = |v: i64| ((v & 32767) as f64).min(32766.0) * 2.0 / 32766.0 - 1.0;
        let s = scale as f64;
        Ok([
            unpack(buffer >> 3) * s,
            unpack(buffer >> 18) * s,
            unpack(buffer >> 33) * s,
        ])
    }

    pub fn f64(&mut self) -> Result<f64, WireError> {
        Ok(f64::from_be_bytes(self.bytes(8)?.try_into().unwrap()))
    }

    pub fn uuid(&mut self) -> Result<u128, WireError> {
        Ok(u128::from_be_bytes(self.bytes(16)?.try_into().unwrap()))
    }

    /// A byte array of at most `max` bytes (vanilla's default limit is what is left).
    pub fn byte_array(&mut self, max: usize) -> Result<&'a [u8], WireError> {
        let len = self.varint()?;
        if len < 0 {
            return Err(WireError::NegativeLength(len));
        }
        let len = len as usize;
        if len > max {
            return Err(WireError::TooLong { len, max });
        }
        self.bytes(len)
    }

    /// A VarLong (`VarLong.read`: at most ten bytes).
    pub fn varlong(&mut self) -> Result<i64, WireError> {
        let mut v: u64 = 0;
        for i in 0..10 {
            let b = self.u8()?;
            v |= u64::from(b & 0x7F) << (7 * i);
            if b & 0x80 == 0 {
                return Ok(v as i64);
            }
        }
        Err(WireError::VarIntTooLong)
    }

    /// A list or map length of at most `max` (`ByteBufCodecs.readCount`).
    pub fn count(&mut self, max: usize) -> Result<usize, WireError> {
        let n = self.varint()?;
        if n < 0 {
            return Err(WireError::NegativeLength(n));
        }
        let n = n as usize;
        if n > max {
            return Err(WireError::TooLong { len: n, max });
        }
        Ok(n)
    }

    /// An identifier, its namespace spelled out (`FriendlyByteBuf.readIdentifier`).
    pub fn identifier(&mut self) -> Result<String, WireError> {
        let s = self.string(MAX_STRING)?;
        if !valid_identifier(s) {
            return Err(WireError::Invalid("identifier"));
        }
        Ok(full_identifier(s))
    }

    /// One NBT tag as the network sends it (`None` for the end tag, vanilla's null).
    pub fn nbt(&mut self) -> Result<Option<crate::nbt::Tag>, WireError> {
        let (tag, used) = crate::nbt::read_network(&self.buf[self.pos..])
            .map_err(|_| WireError::Invalid("NBT"))?;
        self.pos += used;
        Ok(tag)
    }

    /// Everything left.
    pub fn rest(&mut self) -> &'a [u8] {
        let r = &self.buf[self.pos..];
        self.pos = self.buf.len();
        r
    }

    /// Ends a packet: an error if any bytes are left.
    pub fn finish(&self) -> Result<(), WireError> {
        match self.remaining() {
            0 => Ok(()),
            n => Err(WireError::TrailingBytes(n)),
        }
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
    fn low_precision_vectors_read_as_vanilla_writes_them() {
        // Two armor-stand knockbacks from the combat_basics golden trace.
        let read = |b: &[u8]| Reader::new(b).lp_vec3().unwrap();
        let unit = |q: i64| (q as f64).min(32766.0) * 2.0 / 32766.0 - 1.0;
        let a = read(&[0x39, 0xf7, 0x82, 0x82, 0xff, 0xfd]);
        let buffer: i64 = 0x8282fffd << 16 | 0xf7 << 8 | 0x39;
        assert_eq!(
            a,
            [
                unit(buffer >> 3 & 32767),
                unit(buffer >> 18 & 32767),
                unit(buffer >> 33 & 32767)
            ]
        );
        assert!((a[0] + 0.017090886894952084).abs() < 1e-18 && a[1] == 0.0);
        let b = read(&[0x59, 0xff, 0x80, 0x2c, 0xe1, 0x45]);
        assert!((b[1] + 0.12000244155527073).abs() < 1e-18);
        assert_eq!(read(&[0]), [0.0; 3]);
        // Round trips within the format's precision, large scales with the varint too.
        for v in [
            [0.0, 0.4, 0.0],
            [-0.36, 0.3608, 0.21],
            [3.5, -1.25, 0.001],
            [40.0, 0.0, -12.5],
            [1e-6, 0.0, 0.0],
        ] {
            let mut out = Vec::new();
            write_lp_vec3(&mut out, v);
            let mut r = Reader::new(&out);
            let back = r.lp_vec3().unwrap();
            r.finish().unwrap();
            let scale = v.iter().fold(0.0f64, |m, x| m.max(x.abs())).ceil().max(1.0);
            for i in 0..3 {
                assert!(
                    (back[i] - v[i]).abs() <= scale / 32766.0,
                    "{v:?} -> {back:?}"
                );
            }
        }
    }

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
    fn fields_round_trip() {
        let mut out = Vec::new();
        write_bool(&mut out, true);
        write_u16(&mut out, 25565);
        write_i32(&mut out, -2);
        write_i64(&mut out, i64::MIN);
        write_uuid(&mut out, 0x0123_4567_89AB_CDEF_FEDC_BA98_7654_3210);
        write_byte_array(&mut out, &[1, 2, 3]);
        write_string(&mut out, "stone");
        let mut r = Reader::new(&out);
        assert!(r.bool().unwrap());
        assert_eq!(r.u16().unwrap(), 25565);
        assert_eq!(r.i32().unwrap(), -2);
        assert_eq!(r.i64().unwrap(), i64::MIN);
        assert_eq!(
            &out[15..23],
            [0x01, 0x23, 0x45, 0x67, 0x89, 0xAB, 0xCD, 0xEF]
        );
        assert_eq!(r.uuid().unwrap(), 0x0123_4567_89AB_CDEF_FEDC_BA98_7654_3210);
        assert!(r.clone().byte_array(2).is_err());
        assert_eq!(r.byte_array(3).unwrap(), [1, 2, 3]);
        assert_eq!(r.clone().finish(), Err(WireError::TrailingBytes(6)));
        assert_eq!(r.identifier().unwrap(), "minecraft:stone");
        r.finish().unwrap();
        assert!(valid_identifier("ruthenium:policy/v1"));
        assert!(!valid_identifier("Bad:Name"));
        assert!(!valid_identifier("a:b:c"));
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

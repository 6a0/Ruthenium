//! Named binary tags, as Java writes them to files (`servers.dat`): big-endian, with
//! Java's modified UTF-8 for strings (`nbt/NbtIo.java`, `java.io.DataInput`).
//!
//! A file is one compound under a name (usually empty). Reading refuses nesting deeper
//! than 512, as vanilla's `NbtAccounter` does. Lists of mixed types are written as vanilla
//! writes them: as compounds that wrap each element under the empty name, unwrapped
//! again on reading (`nbt/ListTag.java`).

use std::fmt;

const MAX_DEPTH: usize = 512;

const END: u8 = 0;
const BYTE: u8 = 1;
const SHORT: u8 = 2;
const INT: u8 = 3;
const LONG: u8 = 4;
const FLOAT: u8 = 5;
const DOUBLE: u8 = 6;
const BYTE_ARRAY: u8 = 7;
const STRING: u8 = 8;
const LIST: u8 = 9;
const COMPOUND: u8 = 10;
const INT_ARRAY: u8 = 11;
const LONG_ARRAY: u8 = 12;

#[derive(Debug, Clone, PartialEq)]
pub enum Tag {
    Byte(i8),
    Short(i16),
    Int(i32),
    Long(i64),
    Float(f32),
    Double(f64),
    ByteArray(Vec<i8>),
    String(String),
    List(Vec<Tag>),
    Compound(Compound),
    IntArray(Vec<i32>),
    LongArray(Vec<i64>),
}

impl Tag {
    fn id(&self) -> u8 {
        match self {
            Tag::Byte(_) => BYTE,
            Tag::Short(_) => SHORT,
            Tag::Int(_) => INT,
            Tag::Long(_) => LONG,
            Tag::Float(_) => FLOAT,
            Tag::Double(_) => DOUBLE,
            Tag::ByteArray(_) => BYTE_ARRAY,
            Tag::String(_) => STRING,
            Tag::List(_) => LIST,
            Tag::Compound(_) => COMPOUND,
            Tag::IntArray(_) => INT_ARRAY,
            Tag::LongArray(_) => LONG_ARRAY,
        }
    }

    pub fn as_compound(&self) -> Option<&Compound> {
        match self {
            Tag::Compound(c) => Some(c),
            _ => None,
        }
    }
}

/// A compound's entries, in file order. Names are unique; [`Compound::put`] replaces.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Compound {
    entries: Vec<(String, Tag)>,
}

impl Compound {
    pub fn new() -> Compound {
        Compound::default()
    }

    pub fn get(&self, name: &str) -> Option<&Tag> {
        self.entries.iter().find(|(n, _)| n == name).map(|(_, t)| t)
    }

    pub fn put(&mut self, name: &str, tag: Tag) {
        match self.entries.iter_mut().find(|(n, _)| n == name) {
            Some(e) => e.1 = tag,
            None => self.entries.push((name.to_owned(), tag)),
        }
    }

    pub fn iter(&self) -> impl Iterator<Item = (&str, &Tag)> {
        self.entries.iter().map(|(n, t)| (n.as_str(), t))
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// A string entry (vanilla's `getStringOr`: other types count as missing).
    pub fn string(&self, name: &str) -> Option<&str> {
        match self.get(name)? {
            Tag::String(s) => Some(s),
            _ => None,
        }
    }

    /// A numeric entry as an int (vanilla's `getIntOr` accepts any numeric tag).
    pub fn int(&self, name: &str) -> Option<i32> {
        Some(match *self.get(name)? {
            Tag::Byte(v) => i32::from(v),
            Tag::Short(v) => i32::from(v),
            Tag::Int(v) => v,
            Tag::Long(v) => v as i32,
            Tag::Float(v) => v as i32,
            Tag::Double(v) => v as i32,
            _ => return None,
        })
    }

    /// A numeric entry as a float (a codec's `Codec.FLOAT` reads any number).
    pub fn float(&self, name: &str) -> Option<f32> {
        Some(match *self.get(name)? {
            Tag::Byte(v) => f32::from(v),
            Tag::Short(v) => f32::from(v),
            Tag::Int(v) => v as f32,
            Tag::Long(v) => v as f32,
            Tag::Float(v) => v,
            Tag::Double(v) => v as f32,
            _ => return None,
        })
    }

    /// A boolean: any numeric tag, true when not zero (vanilla's `getBooleanOr`).
    pub fn boolean(&self, name: &str) -> Option<bool> {
        Some(self.int(name)? as i8 != 0)
    }

    pub fn list(&self, name: &str) -> &[Tag] {
        match self.get(name) {
            Some(Tag::List(l)) => l,
            _ => &[],
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NbtError {
    Truncated,
    /// The root is not a compound.
    NotCompound(u8),
    UnknownType(u8),
    TooDeep,
    NegativeLength(i32),
    BadString,
    /// A string longer than 65535 bytes in modified UTF-8.
    StringTooLong(usize),
}

impl fmt::Display for NbtError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            NbtError::Truncated => f.write_str("data ended early"),
            NbtError::NotCompound(t) => write!(f, "root tag is type {t}, not a compound"),
            NbtError::UnknownType(t) => write!(f, "unknown tag type {t}"),
            NbtError::TooDeep => write!(f, "nested deeper than {MAX_DEPTH}"),
            NbtError::NegativeLength(n) => write!(f, "negative length {n}"),
            NbtError::BadString => f.write_str("malformed modified UTF-8"),
            NbtError::StringTooLong(n) => write!(f, "string of {n} bytes is too long"),
        }
    }
}

impl std::error::Error for NbtError {}

/// Reads a file's root compound (its name is dropped, as vanilla does).
pub fn read(bytes: &[u8]) -> Result<Compound, NbtError> {
    let mut r = In { buf: bytes, pos: 0 };
    let ty = r.u8()?;
    if ty != COMPOUND {
        return Err(NbtError::NotCompound(ty));
    }
    r.string()?;
    match r.payload(COMPOUND, 0)? {
        Tag::Compound(c) => Ok(c),
        _ => unreachable!(),
    }
}

/// Reads one tag as the network sends it (`FriendlyByteBuf.readNbt` and
/// `NbtIo.readAnyTag` since 1.20.2): a type byte and its payload, with no name. The end
/// tag (a lone zero) is vanilla's "no tag" and reads as `None`. Returns the bytes used.
pub fn read_network(bytes: &[u8]) -> Result<(Option<Tag>, usize), NbtError> {
    let mut r = In { buf: bytes, pos: 0 };
    let ty = r.u8()?;
    if ty == END {
        return Ok((None, 1));
    }
    let tag = r.payload(ty, 0)?;
    Ok((Some(tag), r.pos))
}

/// Writes `tag` as the network sends it: a type byte and its payload, no name.
pub fn write_network(out: &mut Vec<u8>, tag: &Tag) -> Result<(), NbtError> {
    out.push(tag.id());
    put_payload(out, tag)
}

/// Writes `root` as a file's root compound under the empty name.
pub fn write(root: &Compound) -> Result<Vec<u8>, NbtError> {
    let mut out = vec![COMPOUND];
    put_string(&mut out, "")?;
    put_compound(&mut out, root)?;
    Ok(out)
}

struct In<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl In<'_> {
    fn take(&mut self, n: usize) -> Result<&[u8], NbtError> {
        let b = self
            .buf
            .get(self.pos..self.pos + n)
            .ok_or(NbtError::Truncated)?;
        self.pos += n;
        Ok(b)
    }

    fn u8(&mut self) -> Result<u8, NbtError> {
        Ok(self.take(1)?[0])
    }

    fn array<const N: usize>(&mut self) -> Result<[u8; N], NbtError> {
        Ok(self.take(N)?.try_into().unwrap())
    }

    fn len(&mut self) -> Result<usize, NbtError> {
        let n = i32::from_be_bytes(self.array()?);
        if n < 0 {
            return Err(NbtError::NegativeLength(n));
        }
        // Each element takes at least a byte, so a length past the data is truncated.
        if n as usize > self.buf.len() - self.pos {
            return Err(NbtError::Truncated);
        }
        Ok(n as usize)
    }

    fn string(&mut self) -> Result<String, NbtError> {
        let n = u16::from_be_bytes(self.array()?) as usize;
        decode_mutf8(self.take(n)?)
    }

    fn payload(&mut self, ty: u8, depth: usize) -> Result<Tag, NbtError> {
        if depth > MAX_DEPTH {
            return Err(NbtError::TooDeep);
        }
        Ok(match ty {
            BYTE => Tag::Byte(self.u8()? as i8),
            SHORT => Tag::Short(i16::from_be_bytes(self.array()?)),
            INT => Tag::Int(i32::from_be_bytes(self.array()?)),
            LONG => Tag::Long(i64::from_be_bytes(self.array()?)),
            FLOAT => Tag::Float(f32::from_be_bytes(self.array()?)),
            DOUBLE => Tag::Double(f64::from_be_bytes(self.array()?)),
            BYTE_ARRAY => {
                let n = self.len()?;
                Tag::ByteArray(self.take(n)?.iter().map(|&b| b as i8).collect())
            }
            STRING => Tag::String(self.string()?),
            LIST => {
                let elem = self.u8()?;
                let n = self.len()?;
                if elem == END && n > 0 {
                    return Err(NbtError::UnknownType(END));
                }
                let mut items = Vec::with_capacity(n.min(4096));
                for _ in 0..n {
                    let t = self.payload(elem, depth + 1)?;
                    items.push(unwrap_element(t));
                }
                Tag::List(items)
            }
            COMPOUND => {
                let mut c = Compound::new();
                loop {
                    let t = self.u8()?;
                    if t == END {
                        break;
                    }
                    let name = self.string()?;
                    let value = self.payload(t, depth + 1)?;
                    c.put(&name, value);
                }
                Tag::Compound(c)
            }
            INT_ARRAY => {
                let n = self.len()?;
                Tag::IntArray(
                    self.take(n.checked_mul(4).ok_or(NbtError::Truncated)?)?
                        .as_chunks::<4>()
                        .0
                        .iter()
                        .map(|c| i32::from_be_bytes(*c))
                        .collect(),
                )
            }
            LONG_ARRAY => {
                let n = self.len()?;
                Tag::LongArray(
                    self.take(n.checked_mul(8).ok_or(NbtError::Truncated)?)?
                        .as_chunks::<8>()
                        .0
                        .iter()
                        .map(|c| i64::from_be_bytes(*c))
                        .collect(),
                )
            }
            other => return Err(NbtError::UnknownType(other)),
        })
    }
}

/// A list element wrapped in a compound under the empty name (mixed lists) comes back
/// as itself.
fn unwrap_element(t: Tag) -> Tag {
    match t {
        Tag::Compound(mut c) if c.len() == 1 && c.entries[0].0.is_empty() => {
            c.entries.pop().unwrap().1
        }
        other => other,
    }
}

fn put_string(out: &mut Vec<u8>, s: &str) -> Result<(), NbtError> {
    let bytes = encode_mutf8(s);
    let n = u16::try_from(bytes.len()).map_err(|_| NbtError::StringTooLong(bytes.len()))?;
    out.extend_from_slice(&n.to_be_bytes());
    out.extend_from_slice(&bytes);
    Ok(())
}

fn put_compound(out: &mut Vec<u8>, c: &Compound) -> Result<(), NbtError> {
    for (name, tag) in &c.entries {
        out.push(tag.id());
        put_string(out, name)?;
        put_payload(out, tag)?;
    }
    out.push(END);
    Ok(())
}

fn put_len(out: &mut Vec<u8>, n: usize) {
    out.extend_from_slice(&(n as i32).to_be_bytes());
}

fn put_payload(out: &mut Vec<u8>, tag: &Tag) -> Result<(), NbtError> {
    match tag {
        Tag::Byte(v) => out.push(*v as u8),
        Tag::Short(v) => out.extend_from_slice(&v.to_be_bytes()),
        Tag::Int(v) => out.extend_from_slice(&v.to_be_bytes()),
        Tag::Long(v) => out.extend_from_slice(&v.to_be_bytes()),
        Tag::Float(v) => out.extend_from_slice(&v.to_be_bytes()),
        Tag::Double(v) => out.extend_from_slice(&v.to_be_bytes()),
        Tag::ByteArray(v) => {
            put_len(out, v.len());
            out.extend(v.iter().map(|&b| b as u8));
        }
        Tag::String(s) => put_string(out, s)?,
        Tag::List(items) => {
            let first = items.first().map_or(END, Tag::id);
            let mixed = items.iter().any(|t| t.id() != first);
            out.push(if mixed { COMPOUND } else { first });
            put_len(out, items.len());
            for t in items {
                match t {
                    // A compound that is not itself a wrapper stays as it is.
                    Tag::Compound(c) if mixed && !(c.len() == 1 && c.entries[0].0.is_empty()) => {
                        put_compound(out, c)?;
                    }
                    _ if mixed => {
                        let mut wrapper = Compound::new();
                        wrapper.put("", t.clone());
                        put_compound(out, &wrapper)?;
                    }
                    _ => put_payload(out, t)?,
                }
            }
        }
        Tag::Compound(c) => put_compound(out, c)?,
        Tag::IntArray(v) => {
            put_len(out, v.len());
            for x in v {
                out.extend_from_slice(&x.to_be_bytes());
            }
        }
        Tag::LongArray(v) => {
            put_len(out, v.len());
            for x in v {
                out.extend_from_slice(&x.to_be_bytes());
            }
        }
    }
    Ok(())
}

/// Java's modified UTF-8: UTF-16 units, each in one to three bytes, with U+0000 as two
/// bytes and supplementary characters as two three-byte surrogates. Lone surrogates
/// (legal in Java strings) become U+FFFD.
pub fn decode_mutf8(bytes: &[u8]) -> Result<String, NbtError> {
    if bytes.iter().all(|&b| b != 0 && b < 0x80) {
        return Ok(String::from_utf8(bytes.to_vec()).expect("ASCII"));
    }
    let mut units = Vec::with_capacity(bytes.len());
    let mut i = 0;
    let cont = |b: Option<&u8>| match b {
        Some(&b) if b & 0xC0 == 0x80 => Ok(u16::from(b & 0x3F)),
        _ => Err(NbtError::BadString),
    };
    while i < bytes.len() {
        let b = bytes[i];
        match b >> 4 {
            0..=7 => {
                units.push(u16::from(b));
                i += 1;
            }
            12 | 13 => {
                units.push((u16::from(b & 0x1F) << 6) | cont(bytes.get(i + 1))?);
                i += 2;
            }
            14 => {
                units.push(
                    (u16::from(b & 0x0F) << 12)
                        | (cont(bytes.get(i + 1))? << 6)
                        | cont(bytes.get(i + 2))?,
                );
                i += 3;
            }
            _ => return Err(NbtError::BadString),
        }
    }
    Ok(char::decode_utf16(units)
        .map(|c| c.unwrap_or(char::REPLACEMENT_CHARACTER))
        .collect())
}

pub fn encode_mutf8(s: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(s.len());
    for u in s.encode_utf16() {
        match u {
            1..=0x7F => out.push(u as u8),
            0 | 0x80..=0x7FF => {
                out.push(0xC0 | (u >> 6) as u8);
                out.push(0x80 | (u & 0x3F) as u8);
            }
            _ => {
                out.push(0xE0 | (u >> 12) as u8);
                out.push(0x80 | ((u >> 6) & 0x3F) as u8);
                out.push(0x80 | (u & 0x3F) as u8);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modified_utf8_matches_java() {
        // Java's DataOutputStream.writeUTF("a\0é€𝄞").
        let java = [
            0x61, 0xC0, 0x80, 0xC3, 0xA9, 0xE2, 0x82, 0xAC, 0xED, 0xA0, 0xB4, 0xED, 0xB4, 0x9E,
        ];
        assert_eq!(encode_mutf8("a\0é€𝄞"), java);
        assert_eq!(decode_mutf8(&java).unwrap(), "a\0é€𝄞");
        // A lone surrogate is legal in Java.
        assert_eq!(decode_mutf8(&[0xED, 0xA0, 0xB4]).unwrap(), "\u{FFFD}");
        assert!(decode_mutf8(&[0xC3]).is_err());
        assert!(decode_mutf8(&[0xF0, 0x9D, 0x84, 0x9E]).is_err());
    }

    #[test]
    fn network_tags_have_no_name() {
        // A string root: type 8, then the string; then a trailing byte that isn't ours.
        let bytes = [8, 0, 2, b'h', b'i', 99];
        assert_eq!(
            read_network(&bytes).unwrap(),
            (Some(Tag::String("hi".into())), 5)
        );
        assert_eq!(read_network(&[0, 7]).unwrap(), (None, 1));
        let mut c = Compound::new();
        c.put("bold", Tag::Byte(1));
        let mut out = Vec::new();
        write_network(&mut out, &Tag::Compound(c.clone())).unwrap();
        assert_eq!(out, [10, 1, 0, 4, b'b', b'o', b'l', b'd', 1, 0]);
        assert_eq!(read_network(&out).unwrap(), (Some(Tag::Compound(c)), 10));
    }

    #[test]
    fn round_trips_a_server_list() {
        let mut server = Compound::new();
        server.put("name", Tag::String("Hypixel".into()));
        server.put("ip", Tag::String("mc.hypixel.net".into()));
        server.put("hidden", Tag::Byte(0));
        let mut root = Compound::new();
        root.put("servers", Tag::List(vec![Tag::Compound(server)]));
        let bytes = write(&root).unwrap();
        assert_eq!(&bytes[..3], &[10, 0, 0]);
        let back = read(&bytes).unwrap();
        assert_eq!(back, root);
        let s = back.list("servers")[0].as_compound().unwrap();
        assert_eq!(s.string("ip"), Some("mc.hypixel.net"));
        assert_eq!(s.boolean("hidden"), Some(false));
        assert_eq!(s.int("name"), None);
    }

    #[test]
    fn mixed_lists_wrap_and_unwrap() {
        let mut inner = Compound::new();
        inner.put("x", Tag::Int(1));
        let mut root = Compound::new();
        let list = vec![
            Tag::Int(5),
            Tag::String("s".into()),
            Tag::Compound(inner.clone()),
        ];
        root.put("l", Tag::List(list.clone()));
        let bytes = write(&root).unwrap();
        assert_eq!(read(&bytes).unwrap().list("l"), list.as_slice());
    }

    #[test]
    fn refuses_bad_files() {
        assert_eq!(read(&[8, 0, 0]), Err(NbtError::NotCompound(8)));
        assert_eq!(read(&[10, 0, 0, 1, 0, 1]), Err(NbtError::Truncated));
        // A list claiming more elements than there are bytes.
        assert_eq!(
            read(&[10, 0, 0, 9, 0, 1, b'l', 1, 0x7F, 0, 0, 0]),
            Err(NbtError::Truncated)
        );
        // Nesting past the limit.
        let mut deep = vec![10u8, 0, 0];
        for _ in 0..600 {
            deep.extend_from_slice(&[10, 0, 0]);
        }
        deep.extend(std::iter::repeat_n(0, 601));
        assert_eq!(read(&deep), Err(NbtError::TooDeep));
    }
}

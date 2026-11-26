//! Vanilla's `HashOps`: a `DynamicOps` whose values are CRC-32C hashes of a tagged
//! encoding of the codec's output.
//!
//! Every value is hashed on its own: a primitive is its tag byte and its bytes; a map is
//! the start tag, each entry's key hash and value hash (the entries sorted by key hash,
//! then value hash, both read as unsigned), and the end tag; a list is the start tag,
//! each element's hash in order, and the end tag. So a value's hash depends only on the
//! hashes of its parts, and a field's position in a record doesn't matter. Guava writes
//! every number little-endian, a `HashCode` of an `int` as its four little-endian bytes,
//! and a string as its length in UTF-16 units and then each unit.

/// `HashCode` of Guava's `crc32c()`: `asInt` is the checksum itself.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Hash(pub u32);

impl Hash {
    /// `HashCode.asInt`.
    pub fn as_int(self) -> i32 {
        self.0 as i32
    }
}

const TAG_EMPTY: u8 = 1;
const TAG_MAP_START: u8 = 2;
const TAG_MAP_END: u8 = 3;
const TAG_LIST_START: u8 = 4;
const TAG_LIST_END: u8 = 5;
const TAG_BYTE: u8 = 6;
const TAG_SHORT: u8 = 7;
const TAG_INT: u8 = 8;
const TAG_LONG: u8 = 9;
const TAG_FLOAT: u8 = 10;
const TAG_DOUBLE: u8 = 11;
const TAG_STRING: u8 = 12;
const TAG_BOOLEAN: u8 = 13;
const TAG_BYTE_ARRAY_START: u8 = 14;
const TAG_BYTE_ARRAY_END: u8 = 15;
const TAG_INT_ARRAY_START: u8 = 16;
const TAG_INT_ARRAY_END: u8 = 17;
const TAG_LONG_ARRAY_START: u8 = 18;
const TAG_LONG_ARRAY_END: u8 = 19;

/// The CRC-32C (Castagnoli) table, reflected polynomial 0x82F63B78.
const TABLE: [u32; 256] = {
    let mut t = [0u32; 256];
    let mut i = 0;
    while i < 256 {
        let mut c = i as u32;
        let mut k = 0;
        while k < 8 {
            c = if c & 1 != 0 {
                (c >> 1) ^ 0x82F6_3B78
            } else {
                c >> 1
            };
            k += 1;
        }
        t[i] = c;
        i += 1;
    }
    t
};

/// An incremental CRC-32C, as Guava's `Crc32cHasher`.
#[derive(Clone, Debug)]
pub struct Crc32c(u32);

impl Default for Crc32c {
    fn default() -> Self {
        Crc32c(!0)
    }
}

impl Crc32c {
    pub fn update(&mut self, bytes: &[u8]) -> &mut Self {
        let mut c = self.0;
        for &b in bytes {
            c = TABLE[((c ^ b as u32) & 0xFF) as usize] ^ (c >> 8);
        }
        self.0 = c;
        self
    }

    pub fn finish(&self) -> Hash {
        Hash(!self.0)
    }
}

/// CRC-32C of `bytes` (`Hashing.crc32c().hashBytes`).
pub fn crc32c(bytes: &[u8]) -> Hash {
    Crc32c::default().update(bytes).finish()
}

/// `HashOps.empty()`: what a codec that writes nothing gives (`ops.empty()`).
pub fn empty() -> Hash {
    crc32c(&[TAG_EMPTY])
}

pub fn empty_map() -> Hash {
    crc32c(&[TAG_MAP_START, TAG_MAP_END])
}

pub fn empty_list() -> Hash {
    crc32c(&[TAG_LIST_START, TAG_LIST_END])
}

pub fn boolean(v: bool) -> Hash {
    crc32c(&[TAG_BOOLEAN, v as u8])
}

pub fn byte(v: i8) -> Hash {
    crc32c(&[TAG_BYTE, v as u8])
}

pub fn short(v: i16) -> Hash {
    let b = v.to_le_bytes();
    crc32c(&[TAG_SHORT, b[0], b[1]])
}

pub fn int(v: i32) -> Hash {
    let mut h = Crc32c::default();
    h.update(&[TAG_INT]).update(&v.to_le_bytes());
    h.finish()
}

pub fn long(v: i64) -> Hash {
    let mut h = Crc32c::default();
    h.update(&[TAG_LONG]).update(&v.to_le_bytes());
    h.finish()
}

/// `putFloat` is `putInt(floatToRawIntBits)`: a NaN keeps its payload.
pub fn float(v: f32) -> Hash {
    let mut h = Crc32c::default();
    h.update(&[TAG_FLOAT]).update(&v.to_bits().to_le_bytes());
    h.finish()
}

pub fn double(v: f64) -> Hash {
    let mut h = Crc32c::default();
    h.update(&[TAG_DOUBLE]).update(&v.to_bits().to_le_bytes());
    h.finish()
}

/// A string: its length in UTF-16 units, then each unit (`putUnencodedChars`).
pub fn string(s: &str) -> Hash {
    let units: Vec<u16> = s.encode_utf16().collect();
    let mut h = Crc32c::default();
    h.update(&[TAG_STRING])
        .update(&(units.len() as i32).to_le_bytes());
    for u in units {
        h.update(&u.to_le_bytes());
    }
    h.finish()
}

/// A map of (key hash, value hash) entries, in any order.
pub fn map(mut entries: Vec<(Hash, Hash)>) -> Hash {
    entries.sort_unstable();
    let mut h = Crc32c::default();
    h.update(&[TAG_MAP_START]);
    for (k, v) in entries {
        h.update(&k.0.to_le_bytes()).update(&v.0.to_le_bytes());
    }
    h.update(&[TAG_MAP_END]);
    h.finish()
}

pub fn list(items: impl IntoIterator<Item = Hash>) -> Hash {
    let mut h = Crc32c::default();
    h.update(&[TAG_LIST_START]);
    for i in items {
        h.update(&i.0.to_le_bytes());
    }
    h.update(&[TAG_LIST_END]);
    h.finish()
}

pub fn byte_list(bytes: &[u8]) -> Hash {
    let mut h = Crc32c::default();
    h.update(&[TAG_BYTE_ARRAY_START])
        .update(bytes)
        .update(&[TAG_BYTE_ARRAY_END]);
    h.finish()
}

pub fn int_list(v: impl IntoIterator<Item = i32>) -> Hash {
    let mut h = Crc32c::default();
    h.update(&[TAG_INT_ARRAY_START]);
    for i in v {
        h.update(&i.to_le_bytes());
    }
    h.update(&[TAG_INT_ARRAY_END]);
    h.finish()
}

pub fn long_list(v: impl IntoIterator<Item = i64>) -> Hash {
    let mut h = Crc32c::default();
    h.update(&[TAG_LONG_ARRAY_START]);
    for i in v {
        h.update(&i.to_le_bytes());
    }
    h.update(&[TAG_LONG_ARRAY_END]);
    h.finish()
}

/// A record being written (`RecordBuilder`): its fields as (name hash, value hash).
#[derive(Default, Debug)]
pub struct Rec(Vec<(Hash, Hash)>);

impl Rec {
    pub fn new() -> Rec {
        Rec(Vec::new())
    }

    /// `fieldOf(name)`.
    pub fn put(&mut self, name: &str, value: Hash) -> &mut Rec {
        self.0.push((string(name), value));
        self
    }

    /// `optionalFieldOf(name)`: written when present.
    pub fn opt(&mut self, name: &str, value: Option<Hash>) -> &mut Rec {
        if let Some(v) = value {
            self.put(name, v);
        }
        self
    }

    /// An entry whose key is itself a hashed value (`unboundedMap`, `dispatchedMap`).
    pub fn entry(&mut self, key: Hash, value: Hash) -> &mut Rec {
        self.0.push((key, value));
        self
    }

    /// Another record's fields merged in (a `MapCodec` inlined into this one).
    pub fn extend(&mut self, other: Rec) -> &mut Rec {
        self.0.extend(other.0);
        self
    }

    pub fn build(self) -> Hash {
        map(self.0)
    }
}

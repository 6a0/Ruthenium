//! A read-only zip reader over a memory-mapped file.
//!
//! The client jar and zipped resource packs are read in place: the central
//! directory is parsed once, stored entries are borrowed straight from the map
//! and deflated ones are inflated into an exactly sized buffer. Zip64 archives
//! are supported; encrypted entries and methods other than stored and deflate
//! are reported as unreadable.

use std::borrow::Cow;
use std::fs::File;
use std::io;
use std::path::Path;

use miniz_oxide::inflate::TINFLStatus;
use miniz_oxide::inflate::core::{DecompressorOxide, decompress, inflate_flags};

const EOCD_SIG: u32 = 0x0605_4b50;
const ZIP64_LOCATOR_SIG: u32 = 0x0706_4b50;
const ZIP64_EOCD_SIG: u32 = 0x0606_4b50;
const CENTRAL_SIG: u32 = 0x0201_4b50;
const LOCAL_SIG: u32 = 0x0403_4b50;

enum Bytes {
    Mapped(memmap2::Mmap),
    Owned(Vec<u8>),
}

impl std::ops::Deref for Bytes {
    type Target = [u8];
    fn deref(&self) -> &[u8] {
        match self {
            Bytes::Mapped(m) => m,
            Bytes::Owned(v) => v,
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct Entry {
    method: u16,
    encrypted: bool,
    compressed: u64,
    size: u64,
    local_header: u64,
}

/// An opened archive. Entry names are kept sorted, so a directory listing is
/// a range scan.
pub struct Zip {
    bytes: Bytes,
    names: Vec<Box<str>>,
    entries: Vec<Entry>,
}

impl std::fmt::Debug for Zip {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Zip")
            .field("entries", &self.names.len())
            .finish()
    }
}

fn bad(msg: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, msg.into())
}

fn u16_at(b: &[u8], at: usize) -> io::Result<u16> {
    b.get(at..at + 2)
        .map(|s| u16::from_le_bytes([s[0], s[1]]))
        .ok_or_else(|| bad("truncated zip"))
}

fn u32_at(b: &[u8], at: usize) -> io::Result<u32> {
    b.get(at..at + 4)
        .map(|s| u32::from_le_bytes(s.try_into().unwrap()))
        .ok_or_else(|| bad("truncated zip"))
}

fn u64_at(b: &[u8], at: usize) -> io::Result<u64> {
    b.get(at..at + 8)
        .map(|s| u64::from_le_bytes(s.try_into().unwrap()))
        .ok_or_else(|| bad("truncated zip"))
}

impl Zip {
    /// Maps `path` and reads its central directory.
    pub fn open(path: &Path) -> io::Result<Zip> {
        let file = File::open(path)?;
        // SAFETY: the map is read-only. Ruthenium opens jars it imported into
        // its own cache and packs from its own folder; if another process
        // truncates one while it is open, reads past the new end fault. That is
        // the accepted cost of reading in place (no copy of a 40 MiB jar).
        let map = unsafe { memmap2::Mmap::map(&file)? };
        Zip::parse(Bytes::Mapped(map))
    }

    /// An archive held in memory (tests, downloaded packs).
    pub fn from_vec(bytes: Vec<u8>) -> io::Result<Zip> {
        Zip::parse(Bytes::Owned(bytes))
    }

    fn parse(bytes: Bytes) -> io::Result<Zip> {
        let b: &[u8] = &bytes;
        if b.len() < 22 {
            return Err(bad("not a zip file"));
        }
        // The end record is the last 22 bytes plus a comment of up to 64 KiB.
        let lowest = b.len().saturating_sub(22 + 0xFFFF);
        let eocd = (lowest..=b.len() - 22)
            .rev()
            .find(|&i| u32_at(b, i).is_ok_and(|s| s == EOCD_SIG))
            .ok_or_else(|| bad("no end of central directory"))?;
        let mut count = u64::from(u16_at(b, eocd + 10)?);
        let mut cd_size = u64::from(u32_at(b, eocd + 12)?);
        let mut cd_offset = u64::from(u32_at(b, eocd + 16)?);
        if eocd >= 20 && u32_at(b, eocd - 20)? == ZIP64_LOCATOR_SIG {
            let z64 = usize::try_from(u64_at(b, eocd - 12)?).map_err(|_| bad("bad zip64"))?;
            if u32_at(b, z64)? != ZIP64_EOCD_SIG {
                return Err(bad("bad zip64 end record"));
            }
            count = u64_at(b, z64 + 32)?;
            cd_size = u64_at(b, z64 + 40)?;
            cd_offset = u64_at(b, z64 + 48)?;
        }
        let start = usize::try_from(cd_offset).map_err(|_| bad("bad offset"))?;
        let end = start
            .checked_add(usize::try_from(cd_size).map_err(|_| bad("bad size"))?)
            .filter(|&e| e <= b.len())
            .ok_or_else(|| bad("central directory out of range"))?;

        let mut list: Vec<(Box<str>, Entry)> = Vec::with_capacity(count.min(1 << 20) as usize);
        let mut at = start;
        while at < end {
            if u32_at(b, at)? != CENTRAL_SIG {
                return Err(bad("bad central directory entry"));
            }
            let flags = u16_at(b, at + 8)?;
            let method = u16_at(b, at + 10)?;
            let mut compressed = u64::from(u32_at(b, at + 20)?);
            let mut size = u64::from(u32_at(b, at + 24)?);
            let name_len = usize::from(u16_at(b, at + 28)?);
            let extra_len = usize::from(u16_at(b, at + 30)?);
            let comment_len = usize::from(u16_at(b, at + 32)?);
            let mut local_header = u64::from(u32_at(b, at + 42)?);
            let name = b
                .get(at + 46..at + 46 + name_len)
                .ok_or_else(|| bad("truncated zip"))?;
            let extra = b
                .get(at + 46 + name_len..at + 46 + name_len + extra_len)
                .ok_or_else(|| bad("truncated zip"))?;
            // Zip64 extra field: the 0xFFFFFFFF fields, in order.
            let mut x = 0;
            while x + 4 <= extra.len() {
                let id = u16_at(extra, x)?;
                let len = usize::from(u16_at(extra, x + 2)?);
                if id == 1 {
                    let mut p = x + 4;
                    for field in [&mut size, &mut compressed, &mut local_header] {
                        if *field == 0xFFFF_FFFF {
                            *field = u64_at(extra, p)?;
                            p += 8;
                        }
                    }
                }
                x += 4 + len;
            }
            at += 46 + name_len + extra_len + comment_len;
            let Ok(name) = std::str::from_utf8(name) else {
                continue;
            };
            if name.ends_with('/') {
                continue;
            }
            list.push((
                name.into(),
                Entry {
                    method,
                    encrypted: flags & 1 != 0,
                    compressed,
                    size,
                    local_header,
                },
            ));
        }
        list.sort_by(|a, b| a.0.cmp(&b.0));
        // Duplicate names: keep the last one written, as most readers do.
        list.dedup_by(|later, earlier| {
            if later.0 == earlier.0 {
                std::mem::swap(later, earlier);
                true
            } else {
                false
            }
        });
        let (names, entries) = list.into_iter().unzip();
        Ok(Zip {
            bytes,
            names,
            entries,
        })
    }

    pub fn len(&self) -> usize {
        self.names.len()
    }

    pub fn is_empty(&self) -> bool {
        self.names.is_empty()
    }

    /// The index of a file entry.
    pub fn find(&self, name: &str) -> Option<usize> {
        self.names.binary_search_by(|n| (**n).cmp(name)).ok()
    }

    pub fn name(&self, index: usize) -> &str {
        &self.names[index]
    }

    /// Indices of the file entries whose names start with `prefix`, in name order.
    pub fn with_prefix(&self, prefix: &str) -> std::ops::Range<usize> {
        let start = self.names.partition_point(|n| **n < *prefix);
        let len = self.names[start..]
            .iter()
            .take_while(|n| n.starts_with(prefix))
            .count();
        start..start + len
    }

    /// The contents of entry `index`: borrowed if stored, inflated otherwise.
    pub fn read(&self, index: usize) -> io::Result<Cow<'_, [u8]>> {
        let e = self.entries[index];
        let name = &self.names[index];
        if e.encrypted {
            return Err(bad(format!("{name} is encrypted")));
        }
        let b: &[u8] = &self.bytes;
        let lh = usize::try_from(e.local_header).map_err(|_| bad("bad offset"))?;
        if u32_at(b, lh)? != LOCAL_SIG {
            return Err(bad(format!("{name}: bad local header")));
        }
        let data = lh + 30 + usize::from(u16_at(b, lh + 26)?) + usize::from(u16_at(b, lh + 28)?);
        let data = usize::try_from(e.compressed)
            .ok()
            .and_then(|len| b.get(data..data.checked_add(len)?))
            .ok_or_else(|| bad(format!("{name}: data out of range")))?;
        let size = usize::try_from(e.size).map_err(|_| bad("entry too large"))?;
        match e.method {
            0 if data.len() == size => Ok(Cow::Borrowed(data)),
            0 => Err(bad(format!("{name}: stored size mismatch"))),
            8 => inflate(data, size)
                .map(Cow::Owned)
                .ok_or_else(|| bad(format!("{name}: corrupt deflate data"))),
            m => Err(bad(format!("{name}: unsupported compression method {m}"))),
        }
    }
}

/// Inflates raw deflate `data` that must produce exactly `size` bytes.
fn inflate(data: &[u8], size: usize) -> Option<Vec<u8>> {
    let mut out = vec![0u8; size];
    let mut state = Box::<DecompressorOxide>::default();
    let flags = inflate_flags::TINFL_FLAG_USING_NON_WRAPPING_OUTPUT_BUF;
    let (status, read, written) = decompress(&mut state, data, &mut out, 0, flags);
    (status == TINFLStatus::Done && written == size && read <= data.len()).then_some(out)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A minimal zip writer for tests: `(name, contents, deflate?)`.
    pub(crate) fn build(files: &[(&str, &[u8], bool)]) -> Vec<u8> {
        let mut out = Vec::new();
        let mut central = Vec::new();
        for (name, contents, deflate) in files {
            let data = if *deflate {
                miniz_oxide::deflate::compress_to_vec(contents, 6)
            } else {
                contents.to_vec()
            };
            let method: u16 = if *deflate { 8 } else { 0 };
            let offset = out.len() as u32;
            out.extend_from_slice(&LOCAL_SIG.to_le_bytes());
            out.extend_from_slice(&[20, 0, 0, 0]);
            out.extend_from_slice(&method.to_le_bytes());
            out.extend_from_slice(&[0; 8]); // time, date, crc (unchecked)
            out.extend_from_slice(&(data.len() as u32).to_le_bytes());
            out.extend_from_slice(&(contents.len() as u32).to_le_bytes());
            out.extend_from_slice(&(name.len() as u16).to_le_bytes());
            out.extend_from_slice(&0u16.to_le_bytes());
            out.extend_from_slice(name.as_bytes());
            out.extend_from_slice(&data);

            central.extend_from_slice(&CENTRAL_SIG.to_le_bytes());
            central.extend_from_slice(&[20, 0, 20, 0, 0, 0]);
            central.extend_from_slice(&method.to_le_bytes());
            central.extend_from_slice(&[0; 8]);
            central.extend_from_slice(&(data.len() as u32).to_le_bytes());
            central.extend_from_slice(&(contents.len() as u32).to_le_bytes());
            central.extend_from_slice(&(name.len() as u16).to_le_bytes());
            central.extend_from_slice(&[0; 12]); // extra, comment, disk, attrs
            central.extend_from_slice(&offset.to_le_bytes());
            central.extend_from_slice(name.as_bytes());
        }
        let cd_offset = out.len() as u32;
        out.extend_from_slice(&central);
        out.extend_from_slice(&EOCD_SIG.to_le_bytes());
        out.extend_from_slice(&[0; 4]);
        out.extend_from_slice(&(files.len() as u16).to_le_bytes());
        out.extend_from_slice(&(files.len() as u16).to_le_bytes());
        out.extend_from_slice(&(central.len() as u32).to_le_bytes());
        out.extend_from_slice(&cd_offset.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out
    }

    #[test]
    fn reads_stored_and_deflated_entries() {
        let long = b"abcabcabcabcabcabcabcabcabcabc".repeat(50);
        let bytes = build(&[
            ("b/two.txt", b"two", false),
            ("a/one.txt", &long, true),
            ("b/three.txt", b"three", true),
        ]);
        let zip = Zip::from_vec(bytes).unwrap();
        assert_eq!(zip.len(), 3);
        let one = zip.find("a/one.txt").unwrap();
        assert_eq!(&*zip.read(one).unwrap(), &long[..]);
        assert!(matches!(
            zip.read(zip.find("b/two.txt").unwrap()).unwrap(),
            Cow::Borrowed(b"two")
        ));
        let b: Vec<&str> = zip.with_prefix("b/").map(|i| zip.name(i)).collect();
        assert_eq!(b, ["b/three.txt", "b/two.txt"]);
        assert!(zip.find("c").is_none());
        assert_eq!(zip.with_prefix("c/").len(), 0);
    }

    #[test]
    fn rejects_garbage() {
        assert!(Zip::from_vec(vec![0; 100]).is_err());
    }
}

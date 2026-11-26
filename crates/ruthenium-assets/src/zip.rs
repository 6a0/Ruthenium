//! A read-only zip reader over a memory-mapped file.
//!
//! The client jar and zipped resource packs are read in place: the central
//! directory is parsed once, stored entries are borrowed straight from the map
//! and deflated ones are inflated into an exactly sized buffer. Zip64 archives
//! are supported; encrypted entries and methods other than stored and deflate
//! are reported as unreadable.

use std::borrow::Cow;
use std::io;
use std::path::Path;
use std::sync::Arc;

use miniz_oxide::inflate::TINFLStatus;
use miniz_oxide::inflate::core::{DecompressorOxide, decompress, inflate_flags};

use crate::limits::{Budget, Limits};

const EOCD_SIG: u32 = 0x0605_4b50;
const ZIP64_LOCATOR_SIG: u32 = 0x0706_4b50;
const ZIP64_EOCD_SIG: u32 = 0x0606_4b50;
const CENTRAL_SIG: u32 = 0x0201_4b50;
const LOCAL_SIG: u32 = 0x0403_4b50;

enum Bytes {
    Mapped(crate::map::MappedFile),
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
    /// Where an untrusted archive's entry data begins and ends, checked when it opened.
    data: Option<(u64, u64)>,
}

/// An opened archive. Entry names are kept sorted, so a directory listing is
/// a range scan.
pub struct Zip {
    bytes: Bytes,
    names: Vec<Box<str>>,
    entries: Vec<Entry>,
    guard: Option<Guard>,
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
        .map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
        .ok_or_else(|| bad("truncated zip"))
}

fn u64_at(b: &[u8], at: usize) -> io::Result<u64> {
    b.get(at..at + 8)
        .map(|s| u64::from_le_bytes([s[0], s[1], s[2], s[3], s[4], s[5], s[6], s[7]]))
        .ok_or_else(|| bad("truncated zip"))
}

/// What an untrusted archive is read under: the limits, and the budget of its pack's load.
#[derive(Debug, Clone)]
struct Guard {
    limits: Limits,
    budget: Arc<Budget>,
}

/// Deflate's highest ratio, 1032 to 1 (a 258-byte match costs at least two bits). An entry
/// that says it inflates to more than this lies.
const DEFLATE_RATIO: u64 = 1032;

impl Zip {
    /// Maps `path` and reads its central directory.
    pub fn open(path: &Path) -> io::Result<Zip> {
        Zip::parse(Bytes::Mapped(crate::map::MappedFile::open(path)?), None)
    }

    /// An archive held in memory (tests, downloaded packs).
    pub fn from_vec(bytes: Vec<u8>) -> io::Result<Zip> {
        Zip::parse(Bytes::Owned(bytes), None)
    }

    /// An archive whose every byte is hostile, read under `limits`, its reads taken from
    /// `budget`. Beyond the checks every archive gets, its directory must be sane: no
    /// more entries than the limit, every entry's data inside the file, before the
    /// directory, and apart from every other entry's (overlapping entries make one
    /// compressed stream count many times). Names that could mean a place outside the
    /// archive (`..`, a leading `/`, `\`, NUL) or are too long are left out; no lookup
    /// of a resource could find them in vanilla's either.
    pub fn open_untrusted(path: &Path, limits: &Limits, budget: Arc<Budget>) -> io::Result<Zip> {
        let len = std::fs::metadata(path)?.len();
        if len > limits.pack_bytes {
            return Err(bad(format!(
                "{len} bytes is larger than a pack may be ({})",
                limits.pack_bytes
            )));
        }
        let guard = Guard {
            limits: *limits,
            budget,
        };
        Zip::parse(
            Bytes::Mapped(crate::map::MappedFile::open(path)?),
            Some(guard),
        )
    }

    /// [`Self::open_untrusted`] for an archive held in memory (a zip inside a pack).
    pub fn from_vec_untrusted(
        bytes: Vec<u8>,
        limits: &Limits,
        budget: Arc<Budget>,
    ) -> io::Result<Zip> {
        if bytes.len() as u64 > limits.pack_bytes {
            return Err(bad("larger than a pack may be"));
        }
        let guard = Guard {
            limits: *limits,
            budget,
        };
        Zip::parse(Bytes::Owned(bytes), Some(guard))
    }

    fn parse(bytes: Bytes, guard: Option<Guard>) -> io::Result<Zip> {
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
        // Where the directory must end: at the end record, or at the zip64 one.
        let mut cd_limit = eocd;
        if eocd >= 20 && u32_at(b, eocd - 20)? == ZIP64_LOCATOR_SIG {
            let z64 = usize::try_from(u64_at(b, eocd - 12)?).map_err(|_| bad("bad zip64"))?;
            if z64 >= eocd - 20 || u32_at(b, z64)? != ZIP64_EOCD_SIG {
                return Err(bad("bad zip64 end record"));
            }
            count = u64_at(b, z64 + 32)?;
            cd_size = u64_at(b, z64 + 40)?;
            cd_offset = u64_at(b, z64 + 48)?;
            cd_limit = z64;
        }
        let start = usize::try_from(cd_offset).map_err(|_| bad("bad offset"))?;
        let end = start
            .checked_add(usize::try_from(cd_size).map_err(|_| bad("bad size"))?)
            .filter(|&e| e <= b.len())
            .ok_or_else(|| bad("central directory out of range"))?;
        if let Some(g) = &guard {
            if end > cd_limit {
                return Err(bad("central directory overlaps its end record"));
            }
            if count > g.limits.zip_entries as u64 {
                return Err(bad(format!(
                    "{count} entries is more than a pack may have ({})",
                    g.limits.zip_entries
                )));
            }
        }

        // An entry takes at least 46 bytes of the directory, whatever its count says.
        let room = (cd_size / 46).min(count).min(1 << 20);
        let mut list: Vec<(Box<str>, Entry)> = Vec::with_capacity(room as usize);
        let mut seen = 0usize;
        let mut at = start;
        while at < end {
            if u32_at(b, at)? != CENTRAL_SIG {
                return Err(bad("bad central directory entry"));
            }
            seen += 1;
            if let Some(g) = &guard
                && seen > g.limits.zip_entries
            {
                return Err(bad("more entries than a pack may have"));
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
            if at > end {
                return Err(bad("central directory entry out of range"));
            }
            let Ok(name) = std::str::from_utf8(name) else {
                continue;
            };
            let mut entry = Entry {
                method,
                encrypted: flags & 1 != 0,
                compressed,
                size,
                local_header,
                data: None,
            };
            if let Some(g) = &guard {
                // Where its data lies, which must be inside the file and before the
                // directory. Checked for every entry, kept or not, so that none can
                // overlap another unseen.
                let lh = usize::try_from(local_header).map_err(|_| bad("bad offset"))?;
                if u32_at(b, lh)? != LOCAL_SIG {
                    return Err(bad(format!("{name}: bad local header")));
                }
                let data =
                    lh + 30 + usize::from(u16_at(b, lh + 26)?) + usize::from(u16_at(b, lh + 28)?);
                let data_end = usize::try_from(compressed)
                    .ok()
                    .and_then(|c| data.checked_add(c))
                    .filter(|&e| e <= start)
                    .ok_or_else(|| bad(format!("{name}: data out of range")))?;
                entry.data = Some((data as u64, data_end as u64));
                if name.len() > g.limits.zip_name_bytes || !plain_name(name) {
                    // Spans are still checked below.
                    list.push((Box::from(""), entry));
                    continue;
                }
            }
            if name.ends_with('/') {
                continue;
            }
            list.push((name.into(), entry));
        }
        if guard.is_some() {
            let mut spans: Vec<(u64, u64)> = list
                .iter()
                .filter_map(|(_, e)| Some((e.local_header, e.data?.1)))
                .collect();
            spans.sort_unstable();
            if spans.windows(2).any(|w| w[1].0 < w[0].1) {
                return Err(bad("entries overlap"));
            }
            // The names left out above.
            list.retain(|(n, _)| !n.is_empty());
        }
        list.sort_by(|a, b| a.0.cmp(&b.0));
        // Duplicate names: the last one in the directory wins, as in Java's `ZipFile`
        // (`getEntry` finds the last; a listing gives both, and the later replaces the
        // earlier).
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
            guard,
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

    /// The size entry `index` says it inflates to.
    pub fn size(&self, index: usize) -> u64 {
        self.entries[index].size
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

    /// The limits and budget an untrusted archive is read under.
    pub fn guard(&self) -> Option<(&Limits, &Arc<Budget>)> {
        self.guard.as_ref().map(|g| (&g.limits, &g.budget))
    }

    /// The contents of entry `index`: borrowed if stored, inflated otherwise. An untrusted
    /// archive's read is taken from its load's budget.
    pub fn read(&self, index: usize) -> io::Result<Cow<'_, [u8]>> {
        self.read_inner(index, true)
    }

    /// [`Self::read`] without taking from the budget: for files read while playing, after
    /// the load (sounds). Still no larger than a file may be.
    pub fn read_on_demand(&self, index: usize) -> io::Result<Cow<'_, [u8]>> {
        self.read_inner(index, false)
    }

    fn read_inner(&self, index: usize, budgeted: bool) -> io::Result<Cow<'_, [u8]>> {
        let e = self.entries[index];
        let name = &self.names[index];
        if e.encrypted {
            return Err(bad(format!("{name} is encrypted")));
        }
        if let Some(g) = &self.guard {
            if e.size > g.limits.entry_bytes {
                return Err(bad(format!(
                    "{name}: {} bytes is larger than a file may be ({})",
                    e.size, g.limits.entry_bytes
                )));
            }
            if e.method == 8
                && e.size
                    > e.compressed
                        .saturating_mul(DEFLATE_RATIO)
                        .saturating_add(1024)
            {
                return Err(bad(format!(
                    "{name}: {} bytes cannot inflate to {}",
                    e.compressed, e.size
                )));
            }
            if budgeted && !g.budget.take_bytes(e.size, &g.limits) {
                return Err(bad(format!(
                    "{name}: the pack has inflated more than a load may ({} bytes)",
                    g.limits.load_bytes
                )));
            }
        }
        let b: &[u8] = &self.bytes;
        let data = match e.data {
            Some((from, to)) => b.get(from as usize..to as usize),
            None => {
                let lh = usize::try_from(e.local_header).map_err(|_| bad("bad offset"))?;
                if u32_at(b, lh)? != LOCAL_SIG {
                    return Err(bad(format!("{name}: bad local header")));
                }
                let data =
                    lh + 30 + usize::from(u16_at(b, lh + 26)?) + usize::from(u16_at(b, lh + 28)?);
                usize::try_from(e.compressed)
                    .ok()
                    .and_then(|len| b.get(data..data.checked_add(len)?))
            }
        }
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

/// A name that means a place inside the archive only: no `..` segment, no leading `/`,
/// no backslash or NUL.
fn plain_name(name: &str) -> bool {
    !name.starts_with('/') && !name.contains(['\\', '\0']) && name.split('/').all(|s| s != "..")
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

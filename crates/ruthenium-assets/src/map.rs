//! Files read in place through a read-only memory map: the game's jar and zipped packs
//! ([`zip`](crate::zip)), and the system's font files the menus fall back on.
//!
//! A map costs address space, not memory: the kernel reads in only the pages that are
//! touched, and can drop them again under pressure, since they are the file's own.

use std::fs::File;
use std::io;
use std::path::Path;

/// A whole file, mapped read-only.
pub struct MappedFile(memmap2::Mmap);

impl std::fmt::Debug for MappedFile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("MappedFile").field(&self.0.len()).finish()
    }
}

impl MappedFile {
    /// Maps `path`.
    pub fn open(path: &Path) -> io::Result<MappedFile> {
        let file = File::open(path)?;
        // SAFETY: the map is read-only, so nothing in this process writes through it.
        // What another process does to the file shows through,
        // and Rust assumes a shared slice does not change while it is borrowed:
        // - The files mapped do not change while the game runs. Jars and packs are the
        //   ones Ruthenium imported into its own folders; font files belong to the
        //   system, and package managers and font installers write a new file and rename
        //   it over the old one, which leaves this map on the old file, unchanged.
        // - A file truncated in place under the map (`cp` over it, a rewrite) makes the
        //   pages past its new end fault: the game stops with SIGBUS the next time it
        //   reads there. A file rewritten in place to the same length shows its new bytes,
        //   which the readers (the zip reader, ttf-parser) take as any other bytes: they
        //   check every offset, so it is wrong text or a failed read, but formally it is
        //   undefined behaviour.
        // That is the accepted cost of reading in place (no copy of a 40 MiB jar or a
        // 32 MiB font collection), the one every program that maps its fonts (FreeType
        // through fontconfig, for one) accepts.
        let map = unsafe { memmap2::Mmap::map(&file)? };
        Ok(MappedFile(map))
    }

    /// Maps `path` for the rest of the process: unmapped only when it ends.
    pub fn open_for_good(path: &Path) -> io::Result<&'static [u8]> {
        let map: &'static MappedFile = Box::leak(Box::new(MappedFile::open(path)?));
        Ok(map)
    }
}

impl std::ops::Deref for MappedFile {
    type Target = [u8];
    fn deref(&self) -> &[u8] {
        &self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_mapped_file_reads_as_its_bytes() {
        let tmp = tempfile::tempdir().unwrap();
        let path = |name: &str| tmp.path().join(name);
        let (bytes, kept, empty) = (path("bytes"), path("kept"), path("empty"));
        std::fs::write(&bytes, b"mapped bytes").unwrap();
        let map = MappedFile::open(&bytes).unwrap();
        assert_eq!(&*map, b"mapped bytes");
        // Windows neither truncates nor deletes a file while a map of it is open
        // (error 1224, "a user-mapped section open"): unmapped first.
        drop(map);
        std::fs::remove_file(&bytes).unwrap();
        assert!(MappedFile::open(&bytes).is_err());
        // A map for good stays until the process ends, and so on Windows does its file.
        std::fs::write(&kept, b"kept bytes").unwrap();
        assert_eq!(MappedFile::open_for_good(&kept).unwrap(), b"kept bytes");
        let _ = std::fs::remove_file(&kept);
        // An empty file maps to nothing.
        std::fs::write(&empty, b"").unwrap();
        assert!(MappedFile::open(&empty).unwrap().is_empty());
        std::fs::remove_file(&empty).unwrap();
    }
}

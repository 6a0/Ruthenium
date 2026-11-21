//! Streaming file hashes.

use std::fs::File;
use std::io::{self, Read};
use std::path::Path;

use sha1::{Digest, Sha1};
use sha2::Sha256;

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn digest_file<D: Digest>(path: &Path) -> io::Result<String> {
    let mut file = File::open(path)?;
    let mut hasher = D::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hex(&hasher.finalize()))
}

/// Lowercase hex SHA-1 of a file's contents.
pub fn sha1_file(path: &Path) -> io::Result<String> {
    digest_file::<Sha1>(path)
}

/// Lowercase hex SHA-256 of a byte string, used to key preparation steps.
pub fn sha256_hex(bytes: &[u8]) -> String {
    hex(&Sha256::digest(bytes))
}

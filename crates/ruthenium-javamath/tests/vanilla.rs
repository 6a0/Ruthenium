//! Bit-exact comparison with vanilla's `Mth`.
//!
//! Two layers:
//! * Always: the SHA-256 of our sine table must equal the hash of vanilla's table,
//!   recorded from a JVM dump. CI has no game files, so this is what it checks.
//! * When a dump is available (`cargo xtask javamath-dump`, or the
//!   `RUTHENIUM_JAVAMATH_DUMP` environment variable): every table entry and the
//!   sin/cos results for ~460k probe inputs, compared bit for bit.
//!
//! CI's `javamath-jvm` job dumps from a JVM on each architecture (x64 and
//! ARM64) and sets `RUTHENIUM_JAVAMATH_REQUIRE_DUMP=1`, so a missing dump fails
//! instead of skipping, and `RUTHENIUM_JAVAMATH_EXPECT_ARCH` (the JVM's
//! `os.arch`, e.g. `aarch64`), so a dump from the wrong JVM can't pass.

use std::path::PathBuf;

use ruthenium_javamath::mth;
use sha2::{Digest, Sha256};

/// SHA-256 of vanilla 26.3's `Mth` sine table as 65536 little-endian f32 bit
/// patterns, dumped from Mojang's java-runtime-epsilon (OpenJDK 25, amd64).
const VANILLA_26_3_SIN_TABLE_SHA256: &str =
    "cdfaec6870788e193dff3f1ddfa3ca7d3f1a897042d78365a1fe3a433be3627d";

fn table_bytes(table: &[f32]) -> Vec<u8> {
    table
        .iter()
        .flat_map(|f| f.to_bits().to_le_bytes())
        .collect()
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[test]
fn sin_table_hash_matches_vanilla() {
    let ours = hex(&Sha256::digest(table_bytes(mth::sin_table())));
    assert_eq!(ours, VANILLA_26_3_SIN_TABLE_SHA256);
}

struct Dump {
    table: Vec<u32>,
    probes: Vec<(u64, u32, u32)>,
    jvm: String,
}

fn load_dump() -> Option<(PathBuf, Dump)> {
    let path = std::env::var_os("RUTHENIUM_JAVAMATH_DUMP")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../cache/javamath/mth.bin")
        });
    let bytes = std::fs::read(&path).ok()?;
    let mut pos = 0;
    let mut take = |n: usize| {
        let s = &bytes[pos..pos + n];
        pos += n;
        s
    };
    assert_eq!(take(4), b"RJM1", "{}: not a javamath dump", path.display());
    let u32_at = |s: &[u8]| u32::from_le_bytes(s.try_into().unwrap());
    let table_len = u32_at(take(4)) as usize;
    let table = (0..table_len).map(|_| u32_at(take(4))).collect();
    let probe_count = u32_at(take(4)) as usize;
    let probes = (0..probe_count)
        .map(|_| {
            let x = u64::from_le_bytes(take(8).try_into().unwrap());
            (x, u32_at(take(4)), u32_at(take(4)))
        })
        .collect();
    let jvm_len = u32_at(take(4)) as usize;
    let jvm = String::from_utf8_lossy(take(jvm_len)).into_owned();
    Some((path, Dump { table, probes, jvm }))
}

#[test]
fn matches_jvm_dump() {
    let Some((path, dump)) = load_dump() else {
        assert!(
            std::env::var_os("RUTHENIUM_JAVAMATH_REQUIRE_DUMP").is_none(),
            "RUTHENIUM_JAVAMATH_REQUIRE_DUMP is set, but there is no javamath dump"
        );
        eprintln!(
            "no javamath dump found; run `cargo xtask javamath-dump` for the full comparison"
        );
        return;
    };
    eprintln!("comparing against {} ({})", path.display(), dump.jvm);
    if let Ok(arch) = std::env::var("RUTHENIUM_JAVAMATH_EXPECT_ARCH") {
        assert!(
            dump.jvm.ends_with(&format!(" {arch}")),
            "expected a dump from an {arch} JVM, got one from {}",
            dump.jvm
        );
    }

    let ours = mth::sin_table();
    assert_eq!(dump.table.len(), ours.len());
    let table_mismatches: Vec<_> = (0..ours.len())
        .filter(|&i| ours[i].to_bits() != dump.table[i])
        .collect();
    for &i in table_mismatches.iter().take(10) {
        eprintln!(
            "table[{i}]: ours {:e} ({:#010x}), vanilla {:e} ({:#010x})",
            ours[i],
            ours[i].to_bits(),
            f32::from_bits(dump.table[i]),
            dump.table[i]
        );
    }

    let mut probe_mismatches = 0usize;
    for &(x_bits, sin_bits, cos_bits) in &dump.probes {
        let x = f64::from_bits(x_bits);
        let (s, c) = (mth::sin(x).to_bits(), mth::cos(x).to_bits());
        if s != sin_bits || c != cos_bits {
            if probe_mismatches < 10 {
                eprintln!(
                    "x = {x:e} ({x_bits:#018x}): sin ours {s:#010x} vanilla {sin_bits:#010x}, cos ours {c:#010x} vanilla {cos_bits:#010x}"
                );
            }
            probe_mismatches += 1;
        }
    }

    assert!(
        table_mismatches.is_empty() && probe_mismatches == 0,
        "{} table entries and {probe_mismatches} of {} probes differ from vanilla",
        table_mismatches.len(),
        dump.probes.len()
    );
}

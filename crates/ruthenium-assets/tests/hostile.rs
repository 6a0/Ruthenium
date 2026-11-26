//! A server's pack is hostile: each of these files is made to hurt the reader (bombs, lying
//! headers, path tricks, giant pictures, deep JSON, loops, absurd counts), and each must
//! fail cleanly or be skipped as vanilla skips a bad resource, within bounded time and
//! memory. The packs are made here, byte by byte; nothing of the game's is used.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use ruthenium_assets::atlas::{self, AtlasError, AtlasOptions};
use ruthenium_assets::item::ItemModels;
use ruthenium_assets::pack::PackError;
use ruthenium_assets::zip::Zip;
use ruthenium_assets::{BlockModels, Budget, Font, Id, Image, Limits, Pack, Resources};

/// A zip entry as it is written, lies and all.
#[derive(Clone)]
struct Entry {
    name: Vec<u8>,
    data: Vec<u8>,
    deflate: bool,
    /// Overrides of the sizes the headers give.
    size: Option<u32>,
    compressed: Option<u32>,
    /// Overrides the local header offset the directory gives.
    offset: Option<u32>,
    /// The directory entry's extra field.
    extra: Vec<u8>,
}

fn entry(name: &str, data: &[u8]) -> Entry {
    Entry {
        name: name.as_bytes().to_vec(),
        data: data.to_vec(),
        deflate: true,
        size: None,
        compressed: None,
        offset: None,
        extra: Vec::new(),
    }
}

const META: &[u8] = br#"{"pack": {"description": "hostile", "min_format": 97, "max_format": 97}}"#;

/// The bytes of a zip with these entries; \`count\` overrides the end record's.
fn zip_with(entries: &[Entry], count: Option<u16>) -> Vec<u8> {
    let mut out = Vec::new();
    let mut central = Vec::new();
    for e in entries {
        let data = if e.deflate {
            miniz_oxide::deflate::compress_to_vec(&e.data, 6)
        } else {
            e.data.clone()
        };
        let method: u16 = if e.deflate { 8 } else { 0 };
        let offset = out.len() as u32;
        let compressed = e.compressed.unwrap_or(data.len() as u32);
        let size = e.size.unwrap_or(e.data.len() as u32);
        out.extend_from_slice(&0x0403_4b50u32.to_le_bytes());
        out.extend_from_slice(&[20, 0, 0, 0]);
        out.extend_from_slice(&method.to_le_bytes());
        out.extend_from_slice(&[0; 8]);
        out.extend_from_slice(&compressed.to_le_bytes());
        out.extend_from_slice(&size.to_le_bytes());
        out.extend_from_slice(&(e.name.len() as u16).to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&e.name);
        out.extend_from_slice(&data);
        central.extend_from_slice(&0x0201_4b50u32.to_le_bytes());
        central.extend_from_slice(&[20, 0, 20, 0, 0, 0]);
        central.extend_from_slice(&method.to_le_bytes());
        central.extend_from_slice(&[0; 8]);
        central.extend_from_slice(&compressed.to_le_bytes());
        central.extend_from_slice(&size.to_le_bytes());
        central.extend_from_slice(&(e.name.len() as u16).to_le_bytes());
        central.extend_from_slice(&(e.extra.len() as u16).to_le_bytes());
        central.extend_from_slice(&[0; 10]);
        central.extend_from_slice(&e.offset.unwrap_or(offset).to_le_bytes());
        central.extend_from_slice(&e.name);
        central.extend_from_slice(&e.extra);
    }
    let cd_offset = out.len() as u32;
    out.extend_from_slice(&central);
    let n = count.unwrap_or(entries.len() as u16);
    out.extend_from_slice(&0x0605_4b50u32.to_le_bytes());
    out.extend_from_slice(&[0; 4]);
    out.extend_from_slice(&n.to_le_bytes());
    out.extend_from_slice(&n.to_le_bytes());
    out.extend_from_slice(&(central.len() as u32).to_le_bytes());
    out.extend_from_slice(&cd_offset.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out
}

fn zip(files: &[(&str, &[u8])]) -> Vec<u8> {
    let entries: Vec<Entry> = files.iter().map(|(n, d)| entry(n, d)).collect();
    zip_with(&entries, None)
}

/// A file in the temporary folder, deleted when dropped.
#[derive(Debug)]
struct Temp(PathBuf);

impl Temp {
    fn new(name: &str, bytes: &[u8]) -> Temp {
        use std::sync::atomic::{AtomicU32, Ordering};
        static N: AtomicU32 = AtomicU32::new(0);
        let path = std::env::temp_dir().join(format!(
            "ruthenium-hostile-{}-{}-{name}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::write(&path, bytes).unwrap();
        Temp(path)
    }
}

impl Drop for Temp {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

fn open(bytes: &[u8], limits: &Limits) -> Result<(Pack, Temp), PackError> {
    let t = Temp::new("pack.zip", bytes);
    Pack::open_untrusted(&t.0, limits).map(|p| (p, t))
}

/// A pack of these files (and its \`pack.mcmeta\`), as a server's.
fn pack(files: &[(&str, &[u8])], limits: &Limits) -> (Resources, Temp) {
    let mut all = vec![("pack.mcmeta", META)];
    all.extend_from_slice(files);
    let (p, t) = open(&zip(&all), limits).expect("the pack opens");
    (Resources::new(vec![p]), t)
}

fn id(s: &str) -> Id {
    Id::parse(s).unwrap()
}

fn png(w: u32, h: u32, c: u32) -> Vec<u8> {
    let mut i = Image::new(w, h);
    i.pixels.fill(c);
    i.encode_png()
}

fn crc32(bytes: &[u8]) -> u32 {
    let mut c = !0u32;
    for &b in bytes {
        c ^= u32::from(b);
        for _ in 0..8 {
            c = if c & 1 == 1 {
                c >> 1 ^ 0xEDB8_8320
            } else {
                c >> 1
            };
        }
    }
    !c
}

/// A PNG whose header claims \`w\` x \`h\`, with a few bytes of image data.
fn lying_png(w: u32, h: u32) -> Vec<u8> {
    let mut out = vec![0x89, b'P', b'N', b'G', b'\r', b'\n', 0x1A, b'\n'];
    let mut chunk = |kind: &[u8], data: &[u8]| {
        out.extend_from_slice(&(data.len() as u32).to_be_bytes());
        let mut body = kind.to_vec();
        body.extend_from_slice(data);
        out.extend_from_slice(&body);
        out.extend_from_slice(&crc32(&body).to_be_bytes());
    };
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&w.to_be_bytes());
    ihdr.extend_from_slice(&h.to_be_bytes());
    ihdr.extend_from_slice(&[8, 6, 0, 0, 0]);
    chunk(b"IHDR", &ihdr);
    chunk(
        b"IDAT",
        &miniz_oxide::deflate::compress_to_vec_zlib(&[0; 64], 6),
    );
    chunk(b"IEND", &[]);
    out
}

fn quick(started: Instant) {
    // Generous: other builds run beside the tests. An endless loop never gets here.
    assert!(
        started.elapsed() < Duration::from_secs(60),
        "{:?}",
        started.elapsed()
    );
}

// The zip file.

#[test]
fn a_header_that_claims_4_gib_allocates_nothing() {
    let started = Instant::now();
    let mut e = entry("assets/minecraft/a.txt", b"small");
    e.size = Some(0xFFFF_FFF0);
    let (p, _t) = open(
        &zip_with(&[entry("pack.mcmeta", META), e], None),
        &Limits::untrusted(),
    )
    .unwrap();
    let res = Resources::new(vec![p]);
    let r = res.get(&id("a.txt")).unwrap();
    let e = r.read().unwrap_err().to_string();
    assert!(e.contains("larger than a file may be"), "{e}");
    quick(started);
}

#[test]
fn a_zip_bomb_stops_at_the_entry_limit() {
    let limits = Limits {
        entry_bytes: 1 << 20,
        ..Limits::untrusted()
    };
    let bomb = vec![0u8; (1 << 20) + 1];
    let (res, _t) = pack(
        &[
            ("assets/minecraft/bomb.txt", &bomb),
            ("assets/minecraft/ok.txt", &bomb[..1 << 20]),
        ],
        &limits,
    );
    assert!(res.get(&id("bomb.txt")).unwrap().read().is_err());
    assert_eq!(
        res.get(&id("ok.txt")).unwrap().read().unwrap().len(),
        1 << 20
    );
}

#[test]
fn a_load_inflates_no_more_than_its_budget_until_reset() {
    let limits = Limits {
        load_bytes: 3 << 20,
        ..Limits::untrusted()
    };
    let mib = vec![7u8; 1 << 20];
    let names: Vec<String> = (0..5)
        .map(|i| format!("assets/minecraft/f{i}.txt"))
        .collect();
    let files: Vec<(&str, &[u8])> = names.iter().map(|n| (n.as_str(), &mib[..])).collect();
    let (res, _t) = pack(&files, &limits);
    let read = |i: usize| res.get(&id(&format!("f{i}.txt"))).unwrap().read().is_ok();
    // pack.mcmeta took a little of the first MiB.
    assert!(read(0) && read(1));
    assert!(!read(2), "a third MiB is over the budget");
    assert!(!read(3));
    res.reset_budgets();
    assert!(read(3));
    // What is read while playing does not count.
    let r = res.get(&id("f4.txt")).unwrap();
    for _ in 0..10 {
        assert!(r.read_on_demand().is_ok());
    }
}

#[test]
fn an_impossible_ratio_is_a_lie() {
    let mut e = entry("assets/minecraft/a.txt", &[0; 100]);
    e.size = Some(50 << 20);
    let (p, _t) = open(
        &zip_with(&[entry("pack.mcmeta", META), e], None),
        &Limits::untrusted(),
    )
    .unwrap();
    let res = Resources::new(vec![p]);
    let err = res
        .get(&id("a.txt"))
        .unwrap()
        .read()
        .unwrap_err()
        .to_string();
    assert!(err.contains("cannot inflate"), "{err}");
}

#[test]
fn declared_and_actual_sizes_must_agree() {
    // Deflated to more than it says: the exact buffer is full before the stream ends.
    let mut more = entry("assets/minecraft/more.txt", &[1; 5000]);
    more.size = Some(10);
    // Stored with another size than its data.
    let mut stored = entry("assets/minecraft/stored.txt", b"0123456789");
    stored.deflate = false;
    stored.size = Some(5);
    let (p, _t) = open(
        &zip_with(&[entry("pack.mcmeta", META), more, stored], None),
        &Limits::untrusted(),
    )
    .unwrap();
    let res = Resources::new(vec![p]);
    assert!(res.get(&id("more.txt")).unwrap().read().is_err());
    assert!(res.get(&id("stored.txt")).unwrap().read().is_err());
}

#[test]
fn data_outside_the_file_or_over_the_directory_is_refused() {
    let mut far = entry("assets/minecraft/a.txt", b"x");
    far.offset = Some(0x7FFF_FFFF);
    assert!(
        open(
            &zip_with(&[entry("pack.mcmeta", META), far], None),
            &Limits::untrusted()
        )
        .is_err()
    );
    // Compressed data running into the directory (but not past the file's end).
    let mut long = entry("assets/minecraft/a.txt", b"x");
    long.deflate = false;
    long.compressed = Some(30);
    assert!(
        open(
            &zip_with(&[entry("pack.mcmeta", META), long], None),
            &Limits::untrusted()
        )
        .is_err()
    );
}

#[test]
fn a_directory_after_its_end_record_is_refused() {
    // The directory moved into the end record's comment, where Java's reader (which
    // finds the directory just before the end record) would never look.
    let z = zip(&[("pack.mcmeta", META)]);
    let eocd = z.len() - 22;
    let cd_offset = u32::from_le_bytes(z[eocd + 16..eocd + 20].try_into().unwrap()) as usize;
    let central = z[cd_offset..eocd].to_vec();
    let mut moved = z[..cd_offset].to_vec();
    let mut end = z[eocd..].to_vec();
    let new_offset = (moved.len() + 22) as u32;
    end[16..20].copy_from_slice(&new_offset.to_le_bytes());
    end[20..22].copy_from_slice(&(central.len() as u16).to_le_bytes());
    moved.extend_from_slice(&end);
    moved.extend_from_slice(&central);
    assert!(
        Zip::from_vec(moved.clone()).is_ok(),
        "a trusted reader takes it"
    );
    assert!(Zip::from_vec_untrusted(moved, &Limits::untrusted(), Default::default()).is_err());
}

#[test]
fn overlapping_entries_are_refused() {
    // Many names for one compressed stream: the non-recursive zip bomb.
    let mut entries = vec![
        entry("pack.mcmeta", META),
        entry("assets/minecraft/a.txt", &[0; 1000]),
    ];
    for i in 0..10 {
        let mut e = entry(&format!("assets/minecraft/b{i}.txt"), &[0; 1000]);
        e.offset = Some(0);
        entries.push(e);
    }
    let bytes = zip_with(&entries, None);
    let e = open(&bytes, &Limits::untrusted()).unwrap_err().to_string();
    assert!(e.contains("overlap"), "{e}");
    // A trusted reader takes it as it is (local packs are not checked so).
    assert!(Zip::from_vec(bytes).is_ok());
}

#[test]
fn entry_counts_are_bounded() {
    let limits = Limits {
        zip_entries: 10,
        ..Limits::untrusted()
    };
    let many: Vec<Entry> = (0..11).map(|i| entry(&format!("f{i}"), b"x")).collect();
    let untrusted = |z: Vec<u8>| Zip::from_vec_untrusted(z, &limits, Default::default());
    // Eleven entries, and an end record that says so.
    assert!(untrusted(zip_with(&many, None)).is_err());
    // Eleven entries, and an end record that says there are three.
    assert!(untrusted(zip_with(&many, Some(3))).is_err());
    // Three entries, and an end record that says there are 60000.
    assert!(untrusted(zip_with(&many[..3], Some(60000))).is_err());
    assert!(
        Zip::from_vec_untrusted(zip_with(&many[..10], None), &limits, Default::default()).is_ok()
    );
}

#[test]
fn names_that_could_mean_another_place_are_left_out() {
    let long = format!("assets/minecraft/{}", "a".repeat(2000));
    let mut nul = entry("", b"nul");
    nul.name = b"assets/minecraft/n\0ul.txt".to_vec();
    let entries = vec![
        entry("pack.mcmeta", META),
        entry("assets/minecraft/../../escape.txt", b"up"),
        entry("/assets/minecraft/abs.txt", b"abs"),
        entry("assets\\minecraft\\back.txt", b"back"),
        nul,
        entry(&long, b"long"),
        entry("assets/minecraft/fine.txt", b"fine"),
    ];
    let z = Zip::from_vec_untrusted(
        zip_with(&entries, None),
        &Limits::untrusted(),
        Default::default(),
    )
    .unwrap();
    let names: Vec<&str> = (0..z.len()).map(|i| z.name(i)).collect();
    assert_eq!(names, ["assets/minecraft/fine.txt", "pack.mcmeta"]);
}

#[test]
fn of_duplicate_names_the_last_wins_as_in_javas_zip_file() {
    let entries = vec![
        entry("pack.mcmeta", META),
        entry("assets/minecraft/a.txt", b"first"),
        entry("assets/minecraft/a.txt", b"second"),
    ];
    let (p, _t) = open(&zip_with(&entries, None), &Limits::untrusted()).unwrap();
    let res = Resources::new(vec![p]);
    assert_eq!(&*res.get(&id("a.txt")).unwrap().read().unwrap(), b"second");
}

#[test]
fn broken_zip64_records_are_refused() {
    let mut z = zip(&[("pack.mcmeta", META)]);
    let eocd = z.len() - 22;
    // A zip64 locator before the end record, pointing past the file.
    let mut locator = Vec::new();
    locator.extend_from_slice(&0x0706_4b50u32.to_le_bytes());
    locator.extend_from_slice(&[0; 4]);
    locator.extend_from_slice(&u64::MAX.to_le_bytes());
    locator.extend_from_slice(&[1, 0, 0, 0]);
    z.splice(eocd..eocd, locator.iter().copied());
    assert!(Zip::from_vec_untrusted(z.clone(), &Limits::untrusted(), Default::default()).is_err());
    // Or at the locator itself.
    let at = (eocd as u64).to_le_bytes();
    z[eocd + 8..eocd + 16].copy_from_slice(&at);
    assert!(Zip::from_vec_untrusted(z, &Limits::untrusted(), Default::default()).is_err());
}

#[test]
fn garbage_and_truncation_are_errors() {
    let z = zip(&[("pack.mcmeta", META), ("assets/minecraft/a.txt", b"aaaa")]);
    for cut in [0, 10, 21, z.len() / 2, z.len() - 1] {
        let _ =
            Zip::from_vec_untrusted(z[..cut].to_vec(), &Limits::untrusted(), Default::default());
    }
    assert!(
        Zip::from_vec_untrusted(vec![0xFF; 1000], &Limits::untrusted(), Default::default())
            .is_err()
    );
}

#[test]
fn a_pack_too_large_is_refused_before_it_is_read() {
    let limits = Limits {
        pack_bytes: 100,
        ..Limits::untrusted()
    };
    let e = open(&zip(&[("pack.mcmeta", META), ("x", &[0; 200])]), &limits).unwrap_err();
    assert!(e.to_string().contains("larger than a pack may be"), "{e}");
}

#[test]
fn a_pack_needs_metadata_with_a_description() {
    let u = Limits::untrusted();
    assert!(matches!(
        open(&zip(&[("x", b"x")]), &u),
        Err(PackError::NoMetadata(_))
    ));
    assert!(matches!(
        open(&zip(&[("pack.mcmeta", br#"{"pack": {}}"#)]), &u),
        Err(PackError::NoMetadata(_))
    ));
    assert!(matches!(
        open(&zip(&[("pack.mcmeta", b"{not json")]), &u),
        Err(PackError::BadMetadata(..))
    ));
    let huge = format!(
        r#"{{"pack": {{"description": "{}"}}}}"#,
        "a".repeat(5 << 20)
    );
    assert!(open(&zip(&[("pack.mcmeta", huge.as_bytes())]), &u).is_err());
    // A folder is no server pack.
    assert!(Pack::open_untrusted(&std::env::temp_dir(), &u).is_err());
    // Without a format the fallback reads it, as vanilla's does.
    assert!(
        open(
            &zip(&[("pack.mcmeta", br#"{"pack": {"description": ""}}"#)]),
            &u
        )
        .is_ok()
    );
}

#[test]
fn overlays_filters_and_metadata_are_bounded() {
    let limits = Limits {
        pack_overlays: 2,
        pack_filters: 2,
        mcmeta_bytes: 64,
        ..Limits::untrusted()
    };
    let overlays = format!(
        r#"{{"pack": {{"description": ""}}, "overlays": {{"entries": [{}]}}}}"#,
        [r#"{"directory": "o", "min_format": 1, "max_format": 200}"#; 3].join(",")
    );
    assert!(open(&zip(&[("pack.mcmeta", overlays.as_bytes())]), &limits).is_err());
    let filters = format!(
        r#"{{"pack": {{"description": ""}}, "filter": {{"block": [{}]}}}}"#,
        [r#"{"path": "a"}"#; 3].join(",")
    );
    // Too many filter patterns: the filter does not read, and vanilla ignores one that
    // does not.
    let (p, _t) = open(&zip(&[("pack.mcmeta", filters.as_bytes())]), &limits).unwrap();
    assert!(p.filter().is_none());
    let long = format!(r#"{{"animation": {{"frametime": 1{}}}}}"#, " ".repeat(64));
    let (res, _t) = pack(
        &[
            ("assets/minecraft/textures/a.png", &png(1, 1, 0xFFFF_FFFF)),
            ("assets/minecraft/textures/a.png.mcmeta", long.as_bytes()),
            ("assets/minecraft/textures/b.png", &png(1, 1, 0xFFFF_FFFF)),
            (
                "assets/minecraft/textures/b.png.mcmeta",
                br#"{"animation": {}}"#,
            ),
        ],
        &limits,
    );
    assert!(res.get(&id("textures/a.png")).unwrap().metadata().is_err());
    assert!(res.get(&id("textures/b.png")).unwrap().metadata().is_ok());
}

#[test]
fn shaders_and_post_effects_are_hidden() {
    let (res, _t) = pack(
        &[
            (
                "assets/minecraft/shaders/core/terrain.vsh",
                b"void main() {}",
            ),
            ("assets/minecraft/shaders/post/blur.json", b"{}"),
            ("assets/minecraft/post_effect/creeper.json", b"{}"),
            ("assets/other/shaders/x.fsh", b"x"),
            ("assets/minecraft/textures/a.png", &png(1, 1, 0xFFFF_FFFF)),
        ],
        &Limits::untrusted(),
    );
    for f in [
        "shaders/core/terrain.vsh",
        "shaders/post/blur.json",
        "post_effect/creeper.json",
        "other:shaders/x.fsh",
    ] {
        assert!(res.get(&id(f)).is_none(), "{f}");
    }
    assert!(res.list("shaders", |_| true).is_empty());
    assert!(res.list("post_effect", |_| true).is_empty());
    assert_eq!(res.list("textures", |_| true).len(), 1);
}

// Pictures.

#[test]
fn a_png_that_claims_a_million_texels_a_side_allocates_nothing() {
    let started = Instant::now();
    let (res, _t) = pack(
        &[
            (
                "assets/minecraft/textures/huge.png",
                &lying_png(1 << 20, 1 << 20),
            ),
            ("assets/minecraft/textures/wide.png", &lying_png(20000, 1)),
            ("assets/minecraft/textures/many.png", &lying_png(8192, 8192)),
        ],
        &Limits::untrusted(),
    );
    for f in ["huge", "wide", "many"] {
        let e = res
            .get(&id(&format!("textures/{f}.png")))
            .unwrap()
            .image(u32::MAX)
            .unwrap_err();
        assert!(e.contains("larger than a picture may be"), "{f}: {e}");
    }
    // The device's own limit is passed by who uploads it.
    let (res, _t) = pack(
        &[("assets/minecraft/textures/a.png", &png(64, 64, 0xFF00_00FF))],
        &Limits::untrusted(),
    );
    let a = res.get(&id("textures/a.png")).unwrap();
    assert!(a.image(32).is_err() && a.image(64).is_ok());
    // And the untouched decoder still refuses what no GPU takes.
    assert!(Image::decode_png(&lying_png(1 << 20, 1 << 20)).is_err());
    quick(started);
}

#[test]
fn a_loads_pictures_decode_to_no_more_than_its_budget() {
    let limits = Limits {
        load_texels: 3 * 64 * 64,
        ..Limits::untrusted()
    };
    let p = png(64, 64, 0xFF12_3456);
    let (res, _t) = pack(&[("assets/minecraft/a.png", &p)], &limits);
    let a = res.get(&id("a.png")).unwrap();
    assert!(a.image(u32::MAX).is_ok() && a.image(u32::MAX).is_ok() && a.image(u32::MAX).is_ok());
    assert!(a.image(u32::MAX).is_err());
}

// JSON.

#[test]
fn json_nested_a_hundred_thousand_deep_is_an_error_not_a_stack_overflow() {
    let deep = format!("{}{}", "[".repeat(100_000), "]".repeat(100_000));
    let deep_object = format!("{}{}", r#"{"a":"#.repeat(50_000), "}".repeat(50_000));
    let (res, _t) = pack(
        &[
            ("assets/minecraft/deep.json", deep.as_bytes()),
            (
                "assets/minecraft/models/block/deep.json",
                deep_object.as_bytes(),
            ),
            ("assets/minecraft/big.json", &vec![b' '; 5 << 20]),
        ],
        &Limits::untrusted(),
    );
    assert!(res.get(&id("deep.json")).unwrap().json().is_err());
    assert!(
        res.get(&id("models/block/deep.json"))
            .unwrap()
            .json()
            .is_err()
    );
    let e = res.get(&id("big.json")).unwrap().json().unwrap_err();
    assert!(e.contains("larger than a JSON file may be"), "{e}");
}

#[test]
fn language_files_are_bounded_and_flat() {
    let limits = Limits {
        lang_entries: 3,
        ..Limits::untrusted()
    };
    let (res, _t) = pack(
        &[
            ("assets/minecraft/lang/a.json", br#"{"a": "1", "b": "2"}"#),
            (
                "assets/minecraft/lang/b.json",
                br#"{"a": "1", "b": "2", "c": "3", "d": "4"}"#,
            ),
            ("assets/minecraft/lang/c.json", br#"{"a": 1}"#),
            ("assets/minecraft/lang/d.json", b"[]"),
        ],
        &limits,
    );
    let read =
        |f: &str| ruthenium_assets::lang::read(&res.get(&id(&format!("lang/{f}.json"))).unwrap());
    assert_eq!(read("a").unwrap()["b"], "2");
    assert!(read("b").is_err() && read("c").is_err() && read("d").is_err());
}

// Sprites and atlases.

const BLOCKS_ATLAS: &[u8] =
    br#"{"sources": [{"type": "directory", "source": "block", "prefix": "block/"}]}"#;

fn blocks(res: &Resources, max_size: u32) -> Result<atlas::Atlas, AtlasError> {
    let opts = AtlasOptions {
        max_size,
        ..AtlasOptions::default()
    };
    atlas::build(res, atlas::atlas_def("blocks").unwrap(), &opts)
}

#[test]
fn an_animation_of_a_million_frames_drops_its_sprite() {
    let started = Instant::now();
    let frames = format!(
        r#"{{"animation": {{"frames": [{}]}}}}"#,
        vec!["0"; 1_000_000].join(",")
    );
    // Small enough a file, too many frames.
    let some = format!(
        r#"{{"animation": {{"frames": [{}]}}}}"#,
        vec!["1"; 9000].join(",")
    );
    let (res, _t) = pack(
        &[
            ("assets/minecraft/atlases/blocks.json", BLOCKS_ATLAS),
            (
                "assets/minecraft/textures/block/listed.png",
                &png(16, 32, 0xFFFF_0000),
            ),
            (
                "assets/minecraft/textures/block/listed.png.mcmeta",
                frames.as_bytes(),
            ),
            (
                "assets/minecraft/textures/block/some.png",
                &png(16, 32, 0xFFFF_0000),
            ),
            (
                "assets/minecraft/textures/block/some.png.mcmeta",
                some.as_bytes(),
            ),
            // 16384 frames of one texel, by its size alone.
            (
                "assets/minecraft/textures/block/implied.png",
                &png(2, 8192, 0xFF00_FF00),
            ),
            (
                "assets/minecraft/textures/block/implied.png.mcmeta",
                br#"{"animation": {"width": 1, "height": 1}}"#,
            ),
            (
                "assets/minecraft/textures/block/fine.png",
                &png(16, 32, 0xFF00_00FF),
            ),
            (
                "assets/minecraft/textures/block/fine.png.mcmeta",
                br#"{"animation": {"frametime": 2147483647}}"#,
            ),
        ],
        &Limits::untrusted(),
    );
    let a = blocks(&res, 16384).unwrap();
    assert!(a.sprite(&id("block/listed")).is_none());
    assert!(a.sprite(&id("block/some")).is_none());
    assert!(a.sprite(&id("block/implied")).is_none());
    assert!(a.sprite(&id("block/fine")).unwrap().animated.is_some());
    let said = |s: &str| a.warnings.iter().filter(|w| w.contains(s)).count();
    assert_eq!(
        said("more than an animation may have"),
        2,
        "{:?}",
        a.warnings
    );
    quick(started);
}

#[test]
fn gui_scaling_of_nothing_or_too_much_drops_its_sprite() {
    let gui = br#"{"sources": [{"type": "directory", "source": "gui/sprites", "prefix": ""}]}"#;
    let (res, _t) = pack(
        &[
            ("assets/minecraft/atlases/gui.json", gui),
            ("assets/minecraft/textures/gui/sprites/zero.png", &png(8, 8, 0xFFFF_FFFF)),
            ("assets/minecraft/textures/gui/sprites/zero.png.mcmeta", br#"{"gui": {"scaling": {"type": "nine_slice", "width": 0, "height": 0, "border": 0}}}"#),
            ("assets/minecraft/textures/gui/sprites/negative.png", &png(8, 8, 0xFFFF_FFFF)),
            ("assets/minecraft/textures/gui/sprites/negative.png.mcmeta", br#"{"gui": {"scaling": {"type": "tile", "width": -5, "height": 4}}}"#),
            ("assets/minecraft/textures/gui/sprites/border.png", &png(8, 8, 0xFFFF_FFFF)),
            ("assets/minecraft/textures/gui/sprites/border.png.mcmeta", br#"{"gui": {"scaling": {"type": "nine_slice", "width": 8, "height": 8, "border": {"left": 2147483647, "right": 2147483647, "top": 1, "bottom": 1}}}}"#),
            ("assets/minecraft/textures/gui/sprites/huge.png", &png(8, 8, 0xFFFF_FFFF)),
            ("assets/minecraft/textures/gui/sprites/huge.png.mcmeta", br#"{"gui": {"scaling": {"type": "tile", "width": 1e300, "height": 2147483648}}}"#),
            ("assets/minecraft/textures/gui/sprites/one.png", &png(8, 8, 0xFFFF_FFFF)),
            ("assets/minecraft/textures/gui/sprites/one.png.mcmeta", br#"{"gui": {"scaling": {"type": "tile", "width": 1, "height": 1}}}"#),
        ],
        &Limits::untrusted(),
    );
    let a = atlas::build(
        &res,
        atlas::atlas_def("gui").unwrap(),
        &AtlasOptions::default(),
    )
    .unwrap();
    for gone in ["zero", "negative", "border"] {
        assert!(a.sprite(&id(gone)).is_none(), "{gone}");
    }
    // A Java int's largest: kept, as vanilla keeps it (drawing it is the GUI's business).
    assert!(a.sprite(&id("huge")).is_some());
    assert!(a.sprite(&id("one")).is_some());
}

#[test]
fn the_missing_sprite_is_the_atlas_own() {
    let (res, _t) = pack(
        &[
            (
                "assets/minecraft/atlases/blocks.json",
                br#"{"sources": [{"type": "single", "resource": "block/x", "sprite": "minecraft:missingno"}]}"#,
            ),
            ("assets/minecraft/textures/block/x.png", &png(32, 32, 0xFF00_FF00)),
        ],
        &Limits::untrusted(),
    );
    let a = blocks(&res, 16384).unwrap();
    assert_eq!(a.sprites.len(), 1);
    assert_eq!((a.missing().width, a.missing().height), (16, 16));
}

#[test]
fn an_atlas_that_cannot_fit_is_an_error_not_an_allocation() {
    let started = Instant::now();
    let limits = Limits {
        atlas_side: 1024,
        ..Limits::untrusted()
    };
    let big = png(512, 512, 0xFF80_8080);
    let names: Vec<String> = (0..5)
        .map(|i| format!("assets/minecraft/textures/block/b{i}.png"))
        .collect();
    let mut files: Vec<(&str, &[u8])> =
        vec![("assets/minecraft/atlases/blocks.json", BLOCKS_ATLAS)];
    files.extend(names.iter().map(|n| (n.as_str(), &big[..])));
    let (res, _t) = pack(&files, &limits);
    // Five quarters of the atlas: more than all of it.
    assert!(matches!(
        blocks(&res, 16384),
        Err(AtlasError::TooManyTexels { .. })
    ));
    // One sprite larger than the device's largest texture.
    let (res, _t) = pack(
        &[
            ("assets/minecraft/atlases/blocks.json", BLOCKS_ATLAS),
            ("assets/minecraft/textures/block/b.png", &big),
        ],
        &Limits::untrusted(),
    );
    let a = blocks(&res, 256);
    assert!(a.is_err() || a.unwrap().sprite(&id("block/b")).is_none());
    // A sprite whose padded slot is larger than the atlas.
    let (res, _t) = pack(
        &[
            ("assets/minecraft/atlases/blocks.json", BLOCKS_ATLAS),
            (
                "assets/minecraft/textures/block/b.png",
                &png(1024, 1024, 0xFF00_0000),
            ),
        ],
        &limits,
    );
    assert!(matches!(
        blocks(&res, 16384),
        Err(AtlasError::TooLarge { .. })
    ));
    quick(started);
}

#[test]
fn atlas_definitions_are_bounded() {
    let limits = Limits {
        atlas_sources: 3,
        atlas_sprites: 50,
        ..Limits::untrusted()
    };
    let sources = format!(
        r#"{{"sources": [{}]}}"#,
        [r#"{"type": "single", "resource": "block/a"}"#; 4].join(",")
    );
    let (res, _t) = pack(
        &[
            ("assets/minecraft/atlases/blocks.json", sources.as_bytes()),
            (
                "assets/minecraft/textures/block/a.png",
                &png(16, 16, 0xFFFF_FFFF),
            ),
        ],
        &limits,
    );
    let a = blocks(&res, 16384).unwrap();
    assert!(
        a.sprite(&id("block/a")).is_none(),
        "the definition is skipped"
    );
    // Ten textures times ten palettes is a hundred sprites.
    let textures: Vec<String> = (0..10).map(|i| format!(r#""block/t{i}""#)).collect();
    let permutations: Vec<String> = (0..10).map(|i| format!(r#""p{i}": "block/p""#)).collect();
    let paletted = format!(
        r#"{{"sources": [{{"type": "paletted_permutations", "textures": [{}], "palette_key": "block/p", "permutations": {{{}}}}}]}}"#,
        textures.join(","),
        permutations.join(",")
    );
    let tex = png(4, 4, 0xFFFF_FFFF);
    let names: Vec<String> = (0..10)
        .map(|i| format!("assets/minecraft/textures/block/t{i}.png"))
        .collect();
    let mut files: Vec<(&str, &[u8])> = vec![
        ("assets/minecraft/atlases/blocks.json", paletted.as_bytes()),
        ("assets/minecraft/textures/palettes/block/p.png", &tex),
    ];
    files.extend(names.iter().map(|n| (n.as_str(), &tex[..])));
    let (res, _t) = pack(&files, &limits);
    assert!(matches!(
        blocks(&res, 16384),
        Err(AtlasError::TooManySprites { .. })
    ));
}

// Models, block states and item definitions.

fn cube_model() -> &'static [u8] {
    br##"{"textures": {"particle": "#all"}, "elements": [{"from": [0, 0, 0], "to": [16, 16, 16], "faces": {"north": {"texture": "#all"}, "up": {"texture": "#all"}}}]}"##
}

#[test]
fn parent_loops_and_long_chains_end_in_the_missing_model() {
    let started = Instant::now();
    let chain: Vec<(String, String)> = (0..200)
        .map(|i| {
            (
                format!("assets/minecraft/models/block/c{i}.json"),
                format!(r#"{{"parent": "block/c{}"}}"#, i + 1),
            )
        })
        .collect();
    let mut files: Vec<(&str, &[u8])> = vec![
        ("assets/minecraft/atlases/blocks.json", BLOCKS_ATLAS),
        ("assets/minecraft/textures/block/stone.png", &[]),
        (
            "assets/minecraft/blockstates/stone.json",
            br#"{"variants": {"": {"model": "block/a"}}}"#,
        ),
        (
            "assets/minecraft/models/block/a.json",
            br#"{"parent": "block/b"}"#,
        ),
        (
            "assets/minecraft/models/block/b.json",
            br#"{"parent": "block/a"}"#,
        ),
        (
            "assets/minecraft/blockstates/granite.json",
            br#"{"variants": {"": {"model": "block/c0"}}}"#,
        ),
    ];
    files.extend(chain.iter().map(|(n, t)| (n.as_str(), t.as_bytes())));
    let (res, _t) = pack(&files, &Limits::untrusted());
    let a = blocks(&res, 16384).unwrap();
    let models = BlockModels::bake(&res, &a);
    assert!(
        models.warnings.iter().any(|w| w.contains("loop")),
        "{:?}",
        models.warnings
    );
    assert!(
        models.warnings.iter().any(|w| w.contains("more parents")),
        "{:?}",
        models.warnings
    );
    quick(started);
}

#[test]
fn models_of_too_many_elements_or_infinite_numbers_are_broken() {
    let element =
        r##"{"from": [0, 0, 0], "to": [16, 16, 16], "faces": {"north": {"texture": "#all"}}}"##;
    let many = format!(r#"{{"elements": [{}]}}"#, vec![element; 5000].join(","));
    let infinite = r##"{"elements": [{"from": [0, 0, 0], "to": [16, 16, 16], "rotation": {"origin": [8, 8, 8], "axis": "y", "angle": 1e39}, "faces": {"north": {"texture": "#all", "uv": [0, 0, 1e300, 16]}}}]}"##;
    let (res, _t) = pack(
        &[
            ("assets/minecraft/atlases/blocks.json", BLOCKS_ATLAS),
            (
                "assets/minecraft/blockstates/stone.json",
                br#"{"variants": {"": {"model": "block/many"}}}"#,
            ),
            (
                "assets/minecraft/blockstates/granite.json",
                br#"{"variants": {"": {"model": "block/infinite"}}}"#,
            ),
            ("assets/minecraft/models/block/many.json", many.as_bytes()),
            (
                "assets/minecraft/models/block/infinite.json",
                infinite.as_bytes(),
            ),
        ],
        &Limits::untrusted(),
    );
    let a = blocks(&res, 16384).unwrap();
    let models = BlockModels::bake(&res, &a);
    assert!(
        models
            .warnings
            .iter()
            .any(|w| w.contains("more than a model may have")),
        "{:?}",
        models.warnings
    );
    assert!(
        models
            .warnings
            .iter()
            .any(|w| w.contains("is not a finite number")),
        "{:?}",
        models.warnings
    );
}

#[test]
fn baked_quads_stop_at_the_limit() {
    let limits = Limits {
        baked_quads: 100,
        ..Limits::untrusted()
    };
    let element = r##"{"from": [0, 0, 0], "to": [16, 16, 16], "faces": {"north": {"texture": "#all"}, "south": {"texture": "#all"}}}"##;
    let model = format!(r#"{{"elements": [{}]}}"#, vec![element; 40].join(","));
    let states: Vec<String> = ["stone", "granite", "diorite"]
        .iter()
        .map(|b| format!("assets/minecraft/blockstates/{b}.json"))
        .collect();
    let mut files: Vec<(&str, &[u8])> = vec![
        ("assets/minecraft/atlases/blocks.json", BLOCKS_ATLAS),
        ("assets/minecraft/models/block/m.json", model.as_bytes()),
    ];
    let def = br#"{"variants": {"": [{"model": "block/m"}, {"model": "block/m", "y": 90}, {"model": "block/m", "y": 180}]}}"#;
    files.extend(states.iter().map(|s| (s.as_str(), &def[..])));
    let (res, _t) = pack(&files, &limits);
    let a = blocks(&res, 16384).unwrap();
    let models = BlockModels::bake(&res, &a);
    assert!(
        models
            .warnings
            .iter()
            .any(|w| w.contains("more quads than may be baked")),
        "{:?}",
        models.warnings
    );
}

#[test]
fn block_state_files_are_bounded() {
    let limits = Limits {
        blockstate_cases: 8,
        condition_terms: 20,
        ..Limits::untrusted()
    };
    let weights = br#"{"variants": {"": [{"model": "block/m", "weight": 2147483647}, {"model": "block/m", "weight": 2}]}}"#;
    let big = format!(
        r#"{{"variants": {{"": [{}]}}}}"#,
        [r#"{"model": "block/m"}"#; 9].join(",")
    );
    let terms = format!(
        r#"{{"multipart": [{{"when": {{"OR": [{}]}}, "apply": {{"model": "block/m"}}}}]}}"#,
        vec![r#"{"north": "true"}"#; 30].join(",")
    );
    // Nine keys (counted before any is read).
    let keys: Vec<String> = (0..9)
        .map(|i| format!(r#""k{i}": {{"model": "block/m"}}"#))
        .collect();
    let cases = format!(r#"{{"variants": {{{}}}}}"#, keys.join(","));
    let (res, _t) = pack(
        &[
            ("assets/minecraft/atlases/blocks.json", BLOCKS_ATLAS),
            ("assets/minecraft/models/block/m.json", cube_model()),
            ("assets/minecraft/blockstates/stone.json", weights),
            ("assets/minecraft/blockstates/granite.json", big.as_bytes()),
            (
                "assets/minecraft/blockstates/oak_slab.json",
                cases.as_bytes(),
            ),
            (
                "assets/minecraft/blockstates/diorite.json",
                br#"{"variants": {"": [{"model": "block/m", "weight": 3000000000}]}}"#,
            ),
            (
                "assets/minecraft/blockstates/oak_fence.json",
                terms.as_bytes(),
            ),
        ],
        &limits,
    );
    let a = blocks(&res, 16384).unwrap();
    let models = BlockModels::bake(&res, &a);
    let said = |s: &str| models.warnings.iter().any(|w| w.contains(s));
    assert!(said("Sum of weights"), "{:?}", models.warnings);
    assert!(
        said("variants is more than a list may have"),
        "{:?}",
        models.warnings
    );
    assert!(said("condition terms"), "{:?}", models.warnings);
    assert!(
        said("more cases than a block state file"),
        "{:?}",
        models.warnings
    );
    assert!(said("is not a positive int"), "{:?}", models.warnings);
}

#[test]
fn item_models_of_elements_stop_at_the_quad_limit() {
    let element = r##"{"from": [0, 0, 0], "to": [16, 16, 16], "faces": {"north": {"texture": "#all"}, "south": {"texture": "#all"}}}"##;
    let model = format!(r#"{{"elements": [{}]}}"#, vec![element; 40].join(","));
    let defs: Vec<(String, String)> = (0..5)
        .map(|i| {
            (
                format!("assets/minecraft/items/i{i}.json"),
                format!(r#"{{"model": {{"type": "model", "model": "item/m{i}"}}}}"#),
            )
        })
        .collect();
    let models: Vec<String> = (0..5)
        .map(|i| format!("assets/minecraft/models/item/m{i}.json"))
        .collect();
    let mut files: Vec<(&str, &[u8])> = Vec::new();
    files.extend(defs.iter().map(|(n, t)| (n.as_str(), t.as_bytes())));
    files.extend(models.iter().map(|n| (n.as_str(), model.as_bytes())));
    let limits = Limits {
        baked_quads: 200,
        ..Limits::untrusted()
    };
    let (res, _t) = pack(&files, &limits);
    let a = blocks(&res, 16384).unwrap();
    let items = ItemModels::bake(&res, &a, &a, None);
    assert!(
        items
            .warnings
            .iter()
            .any(|w| w.contains("more quads than may be baked")),
        "{:?}",
        items.warnings
    );
    assert!(items.counts().1 <= 200);
}

#[test]
fn item_definitions_nest_no_deeper_than_the_limit() {
    let started = Instant::now();
    let mut node = r#"{"type": "model", "model": "item/x"}"#.to_owned();
    for _ in 0..60 {
        node = format!(
            r#"{{"type": "condition", "property": "broken", "on_true": {node}, "on_false": {node}}}"#
        );
        if node.len() > 1 << 20 {
            break;
        }
    }
    // Deep: past the limit of nesting.
    let mut deep = r#"{"type": "model", "model": "item/x"}"#.to_owned();
    // One JSON level a node, so that JSON's own limit of 128 is not what stops it.
    for _ in 0..100 {
        deep = format!(r#"{{"type": "condition", "property": "broken", "on_true": {deep}}}"#);
    }
    let wide = format!(r#"{{"model": {node}}}"#);
    let deep = format!(r#"{{"model": {deep}}}"#);
    let bake = |name: &str, definition: &str, limits: Limits| {
        let (res, _t) = pack(
            &[
                (name, definition.as_bytes()),
                (
                    "assets/minecraft/models/item/x.json",
                    br#"{"parent": "item/generated", "textures": {"layer0": "item/x"}}"#,
                ),
                (
                    "assets/minecraft/models/item/generated.json",
                    br#"{"parent": "builtin/generated"}"#,
                ),
            ],
            &limits,
        );
        let a = blocks(&res, 16384).unwrap();
        ItemModels::bake(&res, &a, &a, None).warnings
    };
    let w = bake(
        "assets/minecraft/items/deep.json",
        &deep,
        Limits::untrusted(),
    );
    assert!(w.iter().any(|w| w.contains("nests deeper")), "{w:?}");
    let limits = Limits {
        item_model_nodes: 1000,
        ..Limits::untrusted()
    };
    let w = bake("assets/minecraft/items/wide.json", &wide, limits);
    assert!(
        w.iter().any(|w| w.contains("more than 1000 nodes")),
        "{w:?}"
    );
    assert!(!w.iter().any(|w| w.contains("nests deeper")), "{w:?}");
    quick(started);
}

#[test]
fn an_item_models_parents_are_bounded() {
    let chain: Vec<(String, String)> = (0..100)
        .map(|i| {
            (
                format!("assets/minecraft/models/item/c{i}.json"),
                format!(r#"{{"parent": "item/c{}"}}"#, i + 1),
            )
        })
        .collect();
    let mut files: Vec<(&str, &[u8])> = vec![
        (
            "assets/minecraft/items/a.json",
            br#"{"model": {"type": "model", "model": "item/c0"}}"#,
        ),
        (
            "assets/minecraft/items/b.json",
            br#"{"model": {"type": "model", "model": "item/l0"}}"#,
        ),
        (
            "assets/minecraft/models/item/l0.json",
            br#"{"parent": "item/l1"}"#,
        ),
        (
            "assets/minecraft/models/item/l1.json",
            br#"{"parent": "item/l0"}"#,
        ),
    ];
    files.extend(chain.iter().map(|(n, t)| (n.as_str(), t.as_bytes())));
    let (res, _t) = pack(&files, &Limits::untrusted());
    let a = blocks(&res, 16384).unwrap();
    let items = ItemModels::bake(&res, &a, &a, None);
    let said = |s: &str| items.warnings.iter().any(|w| w.contains(s));
    assert!(said("more parents"), "{:?}", items.warnings);
    assert!(said("loop"), "{:?}", items.warnings);
}

#[test]
fn multipart_parts_over_every_state_are_bounded() {
    // Redstone wire has 1296 states: ten parts that all hold are 12960 entries.
    let parts = [r#"{"apply": {"model": "block/m"}}"#; 10].join(",");
    let wire = format!(r#"{{"multipart": [{parts}]}}"#);
    let (res, _t) = pack(
        &[
            ("assets/minecraft/atlases/blocks.json", BLOCKS_ATLAS),
            ("assets/minecraft/models/block/m.json", cube_model()),
            (
                "assets/minecraft/blockstates/redstone_wire.json",
                wire.as_bytes(),
            ),
        ],
        &Limits {
            multipart_parts: 10_000,
            ..Limits::untrusted()
        },
    );
    let a = blocks(&res, 16384).unwrap();
    let models = BlockModels::bake(&res, &a);
    assert!(
        models
            .warnings
            .iter()
            .any(|w| w.contains("multipart parts")),
        "{:?}",
        models.warnings
    );
}

#[test]
fn a_flat_item_of_a_noisy_picture_stops_at_the_quad_limit() {
    let started = Instant::now();
    // Every other texel clear: four sides each, a million quads.
    let mut noisy = Image::new(1024, 1024);
    for (i, p) in noisy.pixels.iter_mut().enumerate() {
        if (i + i / 1024) % 2 == 0 {
            *p = 0xFFFF_FFFF;
        }
    }
    let items_atlas =
        br#"{"sources": [{"type": "directory", "source": "item", "prefix": "item/"}]}"#;
    let defs: Vec<(String, String)> = (0..50)
        .map(|i| {
            (
                format!("assets/minecraft/items/i{i}.json"),
                format!(r#"{{"model": {{"type": "model", "model": "item/m{i}"}}}}"#),
            )
        })
        .collect();
    let models: Vec<(String, &[u8])> = (0..50)
        .map(|i| {
            (
                format!("assets/minecraft/models/item/m{i}.json"),
                &br#"{"parent": "builtin/generated", "textures": {"layer0": "item/noisy"}}"#[..],
            )
        })
        .collect();
    let noisy = noisy.encode_png();
    let mut files: Vec<(&str, &[u8])> = vec![
        ("assets/minecraft/atlases/items.json", items_atlas),
        ("assets/minecraft/textures/item/noisy.png", &noisy),
    ];
    files.extend(defs.iter().map(|(n, t)| (n.as_str(), t.as_bytes())));
    files.extend(models.iter().map(|(n, t)| (n.as_str(), *t)));
    let (res, _t) = pack(&files, &Limits::untrusted());
    let item_atlas = atlas::build(
        &res,
        atlas::atlas_def("items").unwrap(),
        &AtlasOptions::default(),
    )
    .unwrap();
    let block_atlas = blocks(&res, 16384).unwrap();
    let items = ItemModels::bake(&res, &block_atlas, &item_atlas, None);
    assert!(
        items
            .warnings
            .iter()
            .any(|w| w.contains("more edges than may be baked")),
        "{:?}",
        items.warnings
    );
    assert!(items.counts().1 <= Limits::untrusted().baked_quads);
    quick(started);
}

// Fonts.

fn font(res: &Resources) -> Font {
    Font::load(res, &id("default"))
}

#[test]
fn a_font_of_a_giant_picture_loses_that_provider() {
    let started = Instant::now();
    let (res, _t) = pack(
        &[
            ("assets/minecraft/font/default.json", br#"{"providers": [
                {"type": "bitmap", "file": "minecraft:font/huge.png", "ascent": 7, "chars": ["A"]},
                {"type": "bitmap", "file": "minecraft:font/fine.png", "ascent": 7, "height": 2147483647, "chars": ["B"]},
                {"type": "space", "advances": {"C": 1e300, "D": 4}},
                {"type": "ttf", "file": "minecraft:font/evil.ttf", "size": 11}
            ]}"#),
            ("assets/minecraft/textures/font/huge.png", &lying_png(100_000, 100_000)),
            ("assets/minecraft/textures/font/fine.png", &png(8, 8, 0xFFFF_FFFF)),
            ("assets/minecraft/font/evil.ttf", &[0xFF; 64]),
        ],
        &Limits::untrusted(),
    );
    let f = font(&res);
    assert!(!f.has('A') && f.has('B') && f.has('D'));
    assert!(!f.has('C'), "an infinite advance is no glyph");
    assert!(f.glyph('B').advance.is_finite());
    assert!(
        f.warnings.iter().any(|w| w.contains("ttf")),
        "{:?}",
        f.warnings
    );
    quick(started);
}

#[test]
fn font_references_are_read_once_and_bounded() {
    let started = Instant::now();
    // Each font refers to the next one twice, twenty deep: a million providers if each
    // reference read its font again.
    let fonts: Vec<(String, String)> = (0..20)
        .map(|i| {
            let next = format!(r#"{{"type": "reference", "id": "minecraft:f{}"}}"#, i + 1);
            (format!("assets/minecraft/font/f{i}.json"), format!(r#"{{"providers": [{next}, {next}, {{"type": "space", "advances": {{"a": {i}}}}}]}}"#))
        })
        .collect();
    let mut files: Vec<(&str, &[u8])> = vec![
        ("assets/minecraft/font/default.json", br#"{"providers": [{"type": "reference", "id": "minecraft:default"}, {"type": "reference", "id": "minecraft:f0"}]}"#),
        ("assets/minecraft/font/f20.json", br#"{"providers": [{"type": "space", "advances": {"z": 1}}]}"#),
    ];
    files.extend(fonts.iter().map(|(n, t)| (n.as_str(), t.as_bytes())));
    let (res, _t) = pack(&files, &Limits::untrusted());
    let f = font(&res);
    assert!(
        f.warnings.iter().any(|w| w.contains("refers to itself")),
        "{:?}",
        f.warnings
    );
    assert!(
        f.warnings.iter().any(|w| w.contains("nest deeper")),
        "{:?}",
        f.warnings
    );
    assert!(
        f.warnings.iter().any(|w| w.contains("more providers")),
        "{:?}",
        f.warnings
    );
    assert!(f.has('a'));
    quick(started);
}

#[test]
fn a_font_referred_to_many_times_is_read_once() {
    // A thousand references to a font of one large picture: one decoding, not a
    // thousand (which would also overdraw the load's budget for pictures).
    let refs = vec![r#"{"type": "reference", "id": "minecraft:big"}"#; 1000].join(",");
    let default = format!(r#"{{"providers": [{refs}]}}"#);
    let big = png(1024, 1024, 0xFFFF_FFFF);
    let (res, _t) = pack(
        &[
            ("assets/minecraft/font/default.json", default.as_bytes()),
            (
                "assets/minecraft/font/big.json",
                br#"{"providers": [{"type": "bitmap", "file": "minecraft:font/big.png", "ascent": 7, "chars": ["A"]}]}"#,
            ),
            ("assets/minecraft/textures/font/big.png", &big),
        ],
        &Limits {
            load_texels: 4 << 20,
            ..Limits::untrusted()
        },
    );
    let f = font(&res);
    assert!(f.warnings.is_empty(), "{:?}", f.warnings);
    assert!(f.has('A'));
    assert_eq!(res.packs()[0].budget().used().1, 1 << 20);
}

#[test]
fn a_font_file_has_no_more_providers_or_glyphs_than_the_limits() {
    let space = r#"{"type": "space", "advances": {"a": 1}}"#;
    let five = format!(r#"{{"providers": [{}]}}"#, [space; 5].join(","));
    let (res, _t) = pack(
        &[("assets/minecraft/font/default.json", five.as_bytes())],
        &Limits {
            font_providers: 3,
            ..Limits::untrusted()
        },
    );
    let f = font(&res);
    assert!(
        f.warnings.iter().any(|w| w.contains("more providers")),
        "{:?}",
        f.warnings
    );
    let (res, _t) = pack(
        &[(
            "assets/minecraft/font/default.json",
            br#"{"providers": [{"type": "space", "advances": {"a": 1, "b": 1, "c": 1, "d": 1, "e": 1}}]}"#,
        )],
        &Limits {
            font_glyphs: 3,
            ..Limits::untrusted()
        },
    );
    let f = font(&res);
    assert!(!f.has('a'));
    assert!(
        f.warnings.iter().any(|w| w.contains("more glyphs")),
        "{:?}",
        f.warnings
    );
}

#[test]
fn unihex_size_overrides_are_bounded() {
    let overrides = vec![r#"{"from": "A", "to": "B", "left": 0, "right": 3}"#; 2000].join(",");
    let json = format!(
        r#"{{"providers": [{{"type": "unihex", "hex_file": "minecraft:font/u.zip", "size_overrides": [{overrides}]}}]}}"#
    );
    let u = zip(&[(
        "u.hex",
        b"0041:3C3C3C3C3C3C3C3C3C3C3C3C3C3C3C3C
",
    )]);
    let (res, _t) = pack(
        &[
            ("assets/minecraft/font/default.json", json.as_bytes()),
            ("assets/minecraft/font/u.zip", &u),
        ],
        &Limits::untrusted(),
    );
    let f = font(&res);
    assert!(!f.has('A'));
    assert!(
        f.warnings.iter().any(|w| w.contains("size overrides")),
        "{:?}",
        f.warnings
    );
}

#[test]
fn a_unihex_zip_inside_the_zip_is_read_under_the_same_limits() {
    let started = Instant::now();
    // The inner zip says its .hex file inflates to 3 GiB.
    let mut bomb = entry("u.hex", b"0041:00000000000000000000000000000000\n");
    bomb.size = Some(3 << 30);
    let bomb = zip_with(&[bomb], None);
    let lines: String = (0..2000)
        .map(|c| format!("{:04X}:{}\n", 0x4E00 + c, "18".repeat(16)))
        .collect();
    let many = zip(&[("m.hex", lines.as_bytes())]);
    let sides = zip(&[("s.hex", b"0041:FFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFF\n")]);
    let (res, _t) = pack(
        &[
            ("assets/minecraft/font/default.json", br#"{"providers": [
                {"type": "unihex", "hex_file": "minecraft:font/bomb.zip", "size_overrides": []},
                {"type": "unihex", "hex_file": "minecraft:font/many.zip", "size_overrides": []},
                {"type": "unihex", "hex_file": "minecraft:font/sides.zip", "size_overrides": [{"from": "A", "to": "A", "left": -2147483648, "right": 2147483647}]}
            ]}"#),
            ("assets/minecraft/font/bomb.zip", &bomb),
            ("assets/minecraft/font/many.zip", &many),
            ("assets/minecraft/font/sides.zip", &sides),
        ],
        &Limits {
            font_glyphs: 1000,
            ..Limits::untrusted()
        },
    );
    let f = font(&res);
    assert!(
        f.warnings
            .iter()
            .any(|w| w.contains("bomb.zip") && w.contains("larger than a file may be")),
        "{:?}",
        f.warnings
    );
    assert!(
        f.warnings.iter().any(|w| w.contains("more glyphs")),
        "{:?}",
        f.warnings
    );
    // Sides cut to a glyph's columns: at most 34 wide.
    let a = f.glyph('A');
    assert!(a.picture.unwrap().width <= 34 && a.advance < 20.0, "{a:?}");
    assert!(f.texels('A').unwrap().width <= 34);
    quick(started);
}

#[test]
fn a_budget_is_shared_by_every_reader_of_a_pack() {
    let limits = Limits {
        load_bytes: 1 << 20,
        ..Limits::untrusted()
    };
    let budget = std::sync::Arc::new(Budget::default());
    let z = Zip::from_vec_untrusted(zip(&[("a", &vec![0; 600 << 10])]), &limits, budget.clone())
        .unwrap();
    assert!(z.read(0).is_ok());
    assert!(z.read(0).is_err());
    assert_eq!(budget.used().0, 600 << 10);
}

// Found by the review of the merged branch.

#[test]
fn a_zip64_offset_near_the_end_of_memory_is_an_error_not_a_panic() {
    // The directory's offset is 0xFFFFFFFF: the zip64 extra field gives the real one.
    for at in [u64::MAX, u64::MAX - 3, u64::MAX - 40, 1 << 62] {
        let mut e = entry("assets/minecraft/a.txt", b"x");
        e.offset = Some(0xFFFF_FFFF);
        e.extra = [
            1u16.to_le_bytes().to_vec(),
            8u16.to_le_bytes().to_vec(),
            at.to_le_bytes().to_vec(),
        ]
        .concat();
        let z = zip_with(&[entry("pack.mcmeta", META), e], None);
        assert!(
            Zip::from_vec_untrusted(z.clone(), &Limits::untrusted(), Default::default()).is_err()
        );
        // A trusted reader opens it lazily, and its read fails.
        let trusted = Zip::from_vec(z).unwrap();
        let i = trusted.find("assets/minecraft/a.txt").unwrap();
        assert!(trusted.read(i).is_err());
    }
}

#[test]
fn advances_are_clamped_so_that_a_line_stays_finite() {
    let (res, _t) = pack(
        &[
            (
                "assets/minecraft/font/default.json",
                br#"{"providers": [
                    {"type": "space", "advances": {"a": 3.0e38, "b": -3.0e38, "c": 4}},
                    {"type": "bitmap", "file": "minecraft:font/x.png", "ascent": -2147483648, "height": 2147483647, "chars": ["d"]}
                ]}"#,
            ),
            ("assets/minecraft/textures/font/x.png", &png(8, 8, 0xFFFF_FFFF)),
        ],
        &Limits::untrusted(),
    );
    let f = font(&res);
    assert!(f.warnings.is_empty(), "{:?}", f.warnings);
    assert_eq!(f.glyph('c').advance, 4.0);
    for c in ['a', 'b', 'd'] {
        assert!(f.glyph(c).advance.abs() <= 16384.0, "{c}: {:?}", f.glyph(c));
    }
    let line: f32 = "a".repeat(10_000).chars().map(|c| f.glyph(c).advance).sum();
    assert!(line.is_finite());
    let p = f.glyph('d').picture.unwrap();
    assert!(
        [p.right, p.top, p.bottom]
            .iter()
            .all(|v| v.abs() <= 3.0 * 16384.0),
        "{p:?}"
    );
}

#[test]
fn a_font_of_thousands_of_providers_looks_characters_up() {
    // 16 000 providers, the character in the last: asked of every provider in turn,
    // 100 000 glyphs would be 1.6 billion lookups.
    let providers: Vec<String> = (0..16_000)
        .map(|i| {
            let c = char::from_u32(0x4E00 + i).unwrap();
            format!(r#"{{"type": "space", "advances": {{"{c}": 1}}}}"#)
        })
        .collect();
    let json = format!(
        r#"{{"providers": [{}, {{"type": "space", "advances": {{"z": 7, "{}": 2}}}}]}}"#,
        providers.join(","),
        char::from_u32(0x4E00).unwrap()
    );
    let (res, _t) = pack(
        &[("assets/minecraft/font/default.json", json.as_bytes())],
        &Limits::untrusted(),
    );
    let f = font(&res);
    // The first provider that has a character gives its glyph.
    assert_eq!(f.glyph('\u{4E00}').advance, 1.0);
    assert_eq!(f.glyph('z').advance, 7.0);
    assert!(f.has('z') && !f.has('y'));
    assert_eq!(f.glyph('y'), ruthenium_assets::font::Glyph::MISSING);
    let started = Instant::now();
    let mut sum = 0.0;
    for _ in 0..100_000 {
        sum += f.glyph('z').advance + f.glyph('y').advance;
    }
    assert!(sum > 0.0);
    assert!(
        started.elapsed() < Duration::from_secs(10),
        "{:?}",
        started.elapsed()
    );
}

#[test]
fn texture_slots_are_bounded_and_followed_once() {
    // 300 slots: more than a model may have.
    let many: Vec<String> = (0..300).map(|i| format!(r#""s{i}": "block/x""#)).collect();
    let many = format!(
        r##"{{"textures": {{{}}}, "elements": [{{"from": [0, 0, 0], "to": [16, 16, 16], "faces": {{"north": {{"texture": "#s0"}}}}}}]}}"##,
        many.join(",")
    );
    let (res, _t) = pack(
        &[
            ("assets/minecraft/atlases/blocks.json", BLOCKS_ATLAS),
            (
                "assets/minecraft/textures/block/x.png",
                &png(16, 16, 0xFFFF_FFFF),
            ),
            (
                "assets/minecraft/blockstates/stone.json",
                br#"{"variants": {"": {"model": "block/many"}}}"#,
            ),
            ("assets/minecraft/models/block/many.json", many.as_bytes()),
        ],
        &Limits::untrusted(),
    );
    let a = blocks(&res, 16384).unwrap();
    let models = BlockModels::bake(&res, &a);
    assert!(
        models.warnings.iter().any(|w| w.contains("texture slots")),
        "{:?}",
        models.warnings
    );
}

#[test]
fn a_filter_that_does_not_read_is_ignored_as_vanilla_ignores_it() {
    let long = "a".repeat(2000);
    for path in [long.as_str(), "a{1000}{1000}", "a{50000}", "(unclosed"] {
        let meta = format!(
            r#"{{"pack": {{"description": ""}}, "filter": {{"block": [{{"path": "{path}"}}]}}}}"#
        );
        let (p, _t) = open(
            &zip(&[("pack.mcmeta", meta.as_bytes())]),
            &Limits::untrusted(),
        )
        .unwrap();
        assert!(p.filter().is_none(), "{path}");
    }
    let many = format!(
        r#"{{"pack": {{"description": ""}}, "filter": {{"block": [{}]}}}}"#,
        [r#"{"path": "a"}"#; 3].join(",")
    );
    let limits = Limits {
        pack_filters: 2,
        ..Limits::untrusted()
    };
    let (p, _t) = open(&zip(&[("pack.mcmeta", many.as_bytes())]), &limits).unwrap();
    assert!(p.filter().is_none());
    // One that reads filters, every pattern at once.
    let fine = r#"{"pack": {"description": ""}, "filter": {"block": [{"path": "^a/"}, {"namespace": "x", "path": "b$"}]}}"#;
    let (p, _t) = open(&zip(&[("pack.mcmeta", fine.as_bytes())]), &limits).unwrap();
    let f = p.filter().unwrap();
    assert!(f.filters_path("a/x") && f.filters_path("q/b") && !f.filters_path("q/c"));
    // An atlas's filter source with too long a pattern: that definition is skipped.
    let source = format!(
        r#"{{"sources": [{{"type": "directory", "source": "block", "prefix": "block/"}}, {{"type": "filter", "pattern": {{"path": "{long}"}}}}]}}"#
    );
    let (res, _t) = pack(
        &[
            ("assets/minecraft/atlases/blocks.json", source.as_bytes()),
            (
                "assets/minecraft/textures/block/x.png",
                &png(16, 16, 0xFFFF_FFFF),
            ),
        ],
        &Limits::untrusted(),
    );
    let a = blocks(&res, 16384).unwrap();
    assert!(a.sprite(&id("block/x")).is_none());
    assert!(
        a.warnings.iter().any(|w| w.contains("longer than")),
        "{:?}",
        a.warnings
    );
}

#[test]
fn missing_models_share_one_part_and_parts_and_variants_are_bounded() {
    // Three thousand names of models that are not there, under four rotations.
    let variants: Vec<String> = (0..3000)
        .map(|i| format!(r#"{{"model": "block/absent{i}", "y": {}}}"#, 90 * (i % 4)))
        .collect();
    let def = format!(r#"{{"variants": {{"": [{}]}}}}"#, variants.join(","));
    let (res, _t) = pack(
        &[
            ("assets/minecraft/atlases/blocks.json", BLOCKS_ATLAS),
            ("assets/minecraft/blockstates/stone.json", def.as_bytes()),
        ],
        &Limits::untrusted(),
    );
    let a = blocks(&res, 16384).unwrap();
    let models = BlockModels::bake(&res, &a);
    assert!(models.part_count() <= 5, "{}", models.part_count());
    // Real models, more of them than may be baked.
    let names: Vec<String> = (0..20)
        .map(|i| format!("assets/minecraft/models/block/m{i}.json"))
        .collect();
    let list: Vec<String> = (0..20)
        .map(|i| format!(r#"{{"model": "block/m{i}"}}"#))
        .collect();
    let def = format!(r#"{{"variants": {{"": [{}]}}}}"#, list.join(","));
    let mut files: Vec<(&str, &[u8])> = vec![
        ("assets/minecraft/atlases/blocks.json", BLOCKS_ATLAS),
        ("assets/minecraft/blockstates/stone.json", def.as_bytes()),
    ];
    files.extend(names.iter().map(|n| (n.as_str(), cube_model())));
    let (res, _t) = pack(
        &files,
        &Limits {
            baked_parts: 10,
            ..Limits::untrusted()
        },
    );
    let a = blocks(&res, 16384).unwrap();
    let models = BlockModels::bake(&res, &a);
    assert!(models.part_count() <= 12, "{}", models.part_count());
    assert!(
        models.warnings.iter().any(|w| w.contains("more parts")),
        "{:?}",
        models.warnings
    );
    // Variants over all the files of a bake.
    let (res, _t) = pack(
        &files,
        &Limits {
            baked_variants: 10,
            ..Limits::untrusted()
        },
    );
    let models = BlockModels::bake(&res, &a);
    assert!(
        models
            .warnings
            .iter()
            .any(|w| w.contains("more variants than may be baked")),
        "{:?}",
        models.warnings
    );
}

#[test]
fn the_metadata_check_reads_pack_mcmeta_alone() {
    let u = Limits::untrusted();
    let check = |bytes: &[u8]| {
        let t = Temp::new("check.zip", bytes);
        Pack::check_untrusted(&t.0, &u)
    };
    assert!(check(&zip(&[("pack.mcmeta", META)])).is_ok());
    assert!(matches!(
        check(&zip(&[("x", b"x")])),
        Err(PackError::NoMetadata(_))
    ));
    assert!(matches!(
        check(&zip(&[("pack.mcmeta", br#"{"pack": {}}"#)])),
        Err(PackError::NoMetadata(_))
    ));
    assert!(matches!(
        check(&zip(&[("pack.mcmeta", b"{")])),
        Err(PackError::BadMetadata(..))
    ));
    // The last of two copies, as the archive's lookup gives.
    let two = zip(&[("pack.mcmeta", b"{"), ("pack.mcmeta", META)]);
    assert!(check(&two).is_ok());
    // A pack.mcmeta that claims to inflate to more than a JSON file may be.
    let mut big = entry("pack.mcmeta", META);
    big.size = Some(5 << 20);
    let e = check(&zip_with(&[big], None)).unwrap_err().to_string();
    assert!(e.contains("larger than a file may be"), "{e}");
    // Entries that overlap, or a broken filter, are not looked at: the full opening
    // refuses the first (the reload then fails) and ignores the second.
    let mut entries = vec![
        entry("pack.mcmeta", META),
        entry("assets/minecraft/a.txt", &[0; 100]),
    ];
    let mut again = entry("assets/minecraft/b.txt", &[0; 100]);
    again.offset = Some(0);
    entries.push(again);
    let overlapping = zip_with(&entries, None);
    assert!(check(&overlapping).is_ok());
    assert!(open(&overlapping, &u).is_err());
    // Over its limits, the check fails as the opening does.
    assert!(check(&[0xFF; 100]).is_err());
    let limits = Limits {
        pack_bytes: 10,
        ..u
    };
    let t = Temp::new("check-big.zip", &zip(&[("pack.mcmeta", META)]));
    assert!(Pack::check_untrusted(&t.0, &limits).is_err());
}

// The second review.

#[test]
fn the_filter_patterns_of_a_stack_are_bounded() {
    let meta = |n: usize| {
        let blocks = vec![r#"{"path": "^nothing/"}"#; n].join(",");
        format!(r#"{{"pack": {{"description": ""}}, "filter": {{"block": [{blocks}]}}}}"#)
    };
    let u = Limits::untrusted();
    // Nine patterns: more than a pack may have; eight read.
    let (p, _t) = open(&zip(&[("pack.mcmeta", meta(9).as_bytes())]), &u).unwrap();
    assert!(p.filter().is_none());
    let (a, _ta) = open(&zip(&[("pack.mcmeta", meta(8).as_bytes())]), &u).unwrap();
    let (b, _tb) = open(&zip(&[("pack.mcmeta", meta(8).as_bytes())]), &u).unwrap();
    let (c, _tc) = open(&zip(&[("pack.mcmeta", meta(1).as_bytes())]), &u).unwrap();
    assert_eq!(a.filter().unwrap().patterns(), 8);
    // Sixteen in all: the third pack goes without its filter.
    let res = Resources::new(vec![a, b, c]);
    let kept: Vec<usize> = res
        .packs()
        .iter()
        .map(|p| p.filter().map_or(0, |f| f.patterns()))
        .collect();
    assert_eq!(kept, [8, 8, 0]);
}

/// A pack whose load is cancelled: with the flag set every lookup finds nothing, reads
/// fail, and every builder returns at once.
#[test]
fn a_cancelled_load_finds_nothing_and_builds_nothing() {
    let (res, _t) = pack(
        &[
            ("assets/minecraft/atlases/blocks.json", BLOCKS_ATLAS),
            (
                "assets/minecraft/textures/block/x.png",
                &png(16, 16, 0xFFFF_FFFF),
            ),
            ("assets/minecraft/models/block/m.json", cube_model()),
            (
                "assets/minecraft/blockstates/stone.json",
                br#"{"variants": {"": {"model": "block/m"}}}"#,
            ),
            (
                "assets/minecraft/items/stone.json",
                br#"{"model": {"type": "model", "model": "block/m"}}"#,
            ),
            (
                "assets/minecraft/font/default.json",
                br#"{"providers": [{"type": "space", "advances": {"a": 1}}]}"#,
            ),
        ],
        &Limits::untrusted(),
    );
    let flag = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    res.packs()[0].cancel_with(flag.clone());
    let a = blocks(&res, 16384).unwrap();
    assert!(a.sprite(&id("block/x")).is_some() && !res.cancelled());
    let x = res.get(&id("textures/block/x.png")).unwrap();
    flag.store(true, std::sync::atomic::Ordering::Relaxed);
    assert!(res.cancelled());
    assert!(res.get(&id("textures/block/x.png")).is_none());
    assert!(res.stack(&id("textures/block/x.png")).is_empty());
    assert!(res.list("textures", |_| true).is_empty());
    assert!(x.read().is_err() && x.image(16384).is_err());
    assert!(matches!(blocks(&res, 16384), Err(AtlasError::Cancelled)));
    let models = BlockModels::bake(&res, &a);
    assert!(
        models.warnings.iter().any(|w| w.contains("cancelled")),
        "{:?}",
        models.warnings
    );
    let items = ItemModels::bake(&res, &a, &a, None);
    assert_eq!(items.len(), 0);
    // Definitions already read when the flag is set: the bake stops at the next one.
    let definitions = vec![(
        "minecraft:stone".to_owned(),
        r#"{"model": {"type": "model", "model": "block/m"}}"#.to_owned(),
    )];
    let items = ItemModels::bake_from(&res, &definitions, &a, &a, None);
    assert!(
        items.warnings.iter().any(|w| w.contains("cancelled")),
        "{:?}",
        items.warnings
    );
    assert!(!font(&res).has('a'));
}

/// A worst-case atlas (sixty pictures of a million texels) cancelled from another thread
/// while it loads stops within a bounded time.
#[test]
fn a_cancelled_atlas_stops_while_it_loads() {
    let big = png(1024, 1024, 0xFF80_8080);
    let names: Vec<String> = (0..60)
        .map(|i| format!("assets/minecraft/textures/block/b{i}.png"))
        .collect();
    let mut files: Vec<(&str, &[u8])> =
        vec![("assets/minecraft/atlases/blocks.json", BLOCKS_ATLAS)];
    files.extend(names.iter().map(|n| (n.as_str(), &big[..])));
    let (res, _t) = pack(&files, &Limits::untrusted());
    let flag = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    res.packs()[0].cancel_with(flag.clone());
    let setter = {
        let flag = flag.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(20));
            flag.store(true, std::sync::atomic::Ordering::Relaxed);
            Instant::now()
        })
    };
    let built = blocks(&res, 16384);
    let ended = Instant::now();
    let set = setter.join().unwrap();
    assert!(
        matches!(built, Err(AtlasError::Cancelled)),
        "{:?}",
        built.map(|a| a.width)
    );
    // At most one picture a thread after the flag.
    assert!(ended.saturating_duration_since(set) < Duration::from_secs(5));
}

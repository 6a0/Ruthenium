//! A small pack that has a little of everything the client loads, its bytes changed at
//! random (fixed seeds) a few thousand times: inside its files, and in the zip itself.
//! Everything the client loads from it is loaded; nothing may panic, and every case ends.

use ruthenium_assets::atlas::{self, AtlasOptions};
use ruthenium_assets::item::ItemModels;
use ruthenium_assets::{AtlasAnimations, BlockModels, Font, Id, Image, Limits, Pack, Resources};

/// The 64-bit LCG the golden generators use.
struct Lcg(u64);

impl Lcg {
    fn next(&mut self, n: u64) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (self.0 >> 33) % n.max(1)
    }
}

fn png(w: u32, h: u32, seed: u32) -> Vec<u8> {
    let mut i = Image::new(w, h);
    for (n, p) in i.pixels.iter_mut().enumerate() {
        let v = (n as u32).wrapping_mul(2654435761) ^ seed;
        *p = if v.is_multiple_of(5) {
            0
        } else {
            v | 0x8000_0000
        };
    }
    i.encode_png()
}

fn stored_zip(name: &str, data: &[u8]) -> Vec<u8> {
    zip(&[(name.to_owned(), data.to_vec())])
}

/// A zip of these files, deflated.
fn zip(files: &[(String, Vec<u8>)]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut central = Vec::new();
    for (name, contents) in files {
        let data = miniz_oxide::deflate::compress_to_vec(contents, 6);
        let offset = out.len() as u32;
        let header = |out: &mut Vec<u8>| {
            out.extend_from_slice(&8u16.to_le_bytes());
            out.extend_from_slice(&[0; 8]);
            out.extend_from_slice(&(data.len() as u32).to_le_bytes());
            out.extend_from_slice(&(contents.len() as u32).to_le_bytes());
            out.extend_from_slice(&(name.len() as u16).to_le_bytes());
        };
        out.extend_from_slice(&0x0403_4b50u32.to_le_bytes());
        out.extend_from_slice(&[20, 0, 0, 0]);
        header(&mut out);
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(name.as_bytes());
        out.extend_from_slice(&data);
        central.extend_from_slice(&0x0201_4b50u32.to_le_bytes());
        central.extend_from_slice(&[20, 0, 20, 0, 0, 0]);
        header(&mut central);
        central.extend_from_slice(&[0; 12]);
        central.extend_from_slice(&offset.to_le_bytes());
        central.extend_from_slice(name.as_bytes());
    }
    let cd = out.len() as u32;
    out.extend_from_slice(&central);
    out.extend_from_slice(&0x0605_4b50u32.to_le_bytes());
    out.extend_from_slice(&[0; 4]);
    out.extend_from_slice(&(files.len() as u16).to_le_bytes());
    out.extend_from_slice(&(files.len() as u16).to_le_bytes());
    out.extend_from_slice(&(central.len() as u32).to_le_bytes());
    out.extend_from_slice(&cd.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out
}

/// The files of the pack: a little of everything.
fn files() -> Vec<(String, Vec<u8>)> {
    let t = |s: &str| s.as_bytes().to_vec();
    let a = "assets/minecraft/";
    let hex: String = (0x41..0x60)
        .map(|c| format!("{c:04X}:{}\n", "3C".repeat(16)))
        .collect();
    vec![
        (
            "pack.mcmeta".into(),
            t(
                r#"{"pack": {"description": "fuzz", "min_format": 97, "max_format": 100}, "filter": {"block": [{"namespace": "minecraft", "path": "nothing"}]}, "overlays": {"entries": [{"directory": "over", "min_format": 97, "max_format": 100}]}}"#,
            ),
        ),
        (
            format!("{a}atlases/blocks.json"),
            t(
                r#"{"sources": [{"type": "directory", "source": "block", "prefix": "block/"}, {"type": "single", "resource": "entity/x"}, {"type": "unstitch", "resource": "block/stone", "divisor_x": 16, "divisor_y": 16, "regions": [{"sprite": "block/cut", "x": 0, "y": 0, "width": 16, "height": 16}]}, {"type": "paletted_permutations", "textures": ["block/stone"], "palette_key": "block/key", "permutations": {"red": "block/red"}}, {"type": "filter", "pattern": {"path": "nothing"}}]}"#,
            ),
        ),
        (
            format!("{a}atlases/items.json"),
            t(r#"{"sources": [{"type": "directory", "source": "item", "prefix": "item/"}]}"#),
        ),
        (
            format!("{a}atlases/gui.json"),
            t(r#"{"sources": [{"type": "directory", "source": "gui/sprites", "prefix": ""}]}"#),
        ),
        (format!("{a}textures/block/stone.png"), png(16, 16, 1)),
        (format!("{a}textures/block/lava.png"), png(16, 64, 2)),
        (
            format!("{a}textures/block/lava.png.mcmeta"),
            t(
                r#"{"animation": {"frametime": 2, "interpolate": true, "frames": [0, {"index": 2, "time": 3}, 1, 3]}, "texture": {"mipmap_strategy": "cutout", "alpha_cutoff_bias": 0.2}}"#,
            ),
        ),
        (format!("{a}textures/entity/x.png"), png(16, 16, 3)),
        (format!("{a}textures/palettes/block/key.png"), png(4, 1, 4)),
        (format!("{a}textures/palettes/block/red.png"), png(4, 1, 5)),
        (format!("{a}textures/item/stick.png"), png(16, 16, 6)),
        (
            format!("{a}textures/gui/sprites/button.png"),
            png(20, 20, 7),
        ),
        (
            format!("{a}textures/gui/sprites/button.png.mcmeta"),
            t(
                r#"{"gui": {"scaling": {"type": "nine_slice", "width": 200, "height": 20, "border": {"left": 3, "top": 3, "right": 3, "bottom": 4}}}}"#,
            ),
        ),
        (format!("{a}textures/gui/sprites/tile.png"), png(8, 8, 8)),
        (
            format!("{a}textures/gui/sprites/tile.png.mcmeta"),
            t(r#"{"gui": {"scaling": {"type": "tile", "width": 8, "height": 8}}}"#),
        ),
        (
            format!("{a}blockstates/stone.json"),
            t(
                r#"{"variants": {"": [{"model": "block/stone"}, {"model": "block/stone", "y": 90, "weight": 3}]}}"#,
            ),
        ),
        (
            format!("{a}blockstates/oak_fence.json"),
            t(
                r#"{"multipart": [{"apply": {"model": "block/post"}}, {"when": {"north": "true"}, "apply": {"model": "block/side", "uvlock": true}}, {"when": {"OR": [{"east": "true"}, {"west": "true|false"}]}, "apply": {"model": "block/side", "y": 90}}]}"#,
            ),
        ),
        (
            format!("{a}blockstates/redstone_wire.json"),
            t(
                r#"{"multipart": [{"when": {"AND": [{"north": "side|up"}, {"power": "!0"}]}, "apply": {"model": "block/post"}}]}"#,
            ),
        ),
        (
            format!("{a}models/block/cube.json"),
            t(
                r##"{"textures": {"particle": "#all"}, "elements": [{"from": [0, 0, 0], "to": [16, 16, 16], "faces": {"down": {"texture": "#all", "cullface": "down"}, "up": {"texture": "#all"}, "north": {"texture": "#all", "uv": [0, 0, 16, 16], "rotation": 90}, "south": {"texture": "#all", "tintindex": 0}, "west": {"texture": "#all"}, "east": {"texture": "#all"}}}]}"##,
            ),
        ),
        (
            format!("{a}models/block/stone.json"),
            t(r#"{"parent": "block/cube", "textures": {"all": "block/stone"}}"#),
        ),
        (
            format!("{a}models/block/post.json"),
            t(
                r##"{"textures": {"t": "block/lava"}, "ambientocclusion": false, "elements": [{"from": [6, 0, 6], "to": [10, 16, 10], "rotation": {"origin": [8, 8, 8], "axis": "y", "angle": 22.5, "rescale": true}, "shade_direction_override": "up", "light_emission": 4, "faces": {"north": {"texture": "#t"}, "up": {"texture": "#t"}}}]}"##,
            ),
        ),
        (
            format!("{a}models/block/side.json"),
            t(
                r##"{"parent": "block/post", "elements": [{"from": [7, 12, 0], "to": [9, 15, 9], "rotation": {"origin": [8, 8, 8], "x": 10, "y": 20, "z": 30}, "faces": {"north": {"texture": "#t"}}}]}"##,
            ),
        ),
        (
            format!("{a}items/stick.json"),
            t(
                r#"{"model": {"type": "select", "property": "display_context", "cases": [{"when": ["gui", "fixed"], "model": {"type": "model", "model": "item/stick", "tints": [{"type": "constant", "value": [1, 0.5, 0]}]}}], "fallback": {"type": "range_dispatch", "property": "damage", "entries": [{"threshold": 0.5, "model": {"type": "composite", "models": [{"type": "model", "model": "block/stone"}]}}], "fallback": {"type": "condition", "property": "broken", "on_true": {"type": "special", "base": "item/stick", "model": {"type": "shield"}}, "on_false": {"type": "model", "model": "item/stick", "transformation": {"scale": [2, 2, 2]}}}}}, "swap_animation_scale": 2}"#,
            ),
        ),
        (
            format!("{a}models/item/stick.json"),
            t(
                r#"{"parent": "item/generated", "textures": {"layer0": "item/stick", "layer1": "block/lava"}, "display": {"gui": {"rotation": [30, 45, 0], "scale": [0.6, 0.6, 0.6]}}}"#,
            ),
        ),
        (
            format!("{a}models/item/generated.json"),
            t(r#"{"parent": "builtin/generated", "gui_light": "front"}"#),
        ),
        (
            format!("{a}font/default.json"),
            t(
                r#"{"providers": [{"type": "space", "advances": {" ": 4, "‌": 0}}, {"type": "bitmap", "file": "minecraft:font/ascii.png", "ascent": 7, "chars": ["AB", "CD"]}, {"type": "reference", "id": "minecraft:other"}, {"type": "unihex", "hex_file": "minecraft:font/u.zip", "size_overrides": [{"from": "E", "to": "F", "left": 1, "right": 6}]}]}"#,
            ),
        ),
        (
            format!("{a}font/other.json"),
            t(
                r#"{"providers": [{"type": "space", "filter": {"uniform": false}, "advances": {"x": 2}}]}"#,
            ),
        ),
        (format!("{a}textures/font/ascii.png"), png(16, 16, 9)),
        (
            format!("{a}font/u.zip"),
            stored_zip("u.hex", hex.as_bytes()),
        ),
        (
            format!("{a}lang/en_us.json"),
            t(r#"{"a.b": "A %s B", "c": "C"}"#),
        ),
        (
            format!("{a}equipment/iron.json"),
            t(r#"{"layers": {"humanoid": [{"texture": "minecraft:iron"}]}}"#),
        ),
        (
            format!("{a}textures/entity/equipment/humanoid/iron.png"),
            png(64, 32, 10),
        ),
        (format!("{a}textures/colormap/grass.png"), png(256, 256, 11)),
        (
            "over/assets/minecraft/textures/block/stone.png".into(),
            png(16, 16, 12),
        ),
    ]
}

/// What the client loads from a stack, as the world's assets and the menus load it.
fn load_everything(res: &Resources) {
    let opts = AtlasOptions {
        max_size: 4096,
        ..AtlasOptions::default()
    };
    let mut atlases = atlas::build_all(res, &opts);
    if let (Ok(blocks), Ok(items)) = (&atlases[1], &atlases[2]) {
        let models = BlockModels::bake(res, blocks);
        let mut parts = Vec::new();
        for state in [0, 1, 100, 5000] {
            models.parts(state, 42, &mut parts);
            models.is_empty(state);
        }
        let items = ItemModels::bake(res, blocks, items, None);
        let _ = items.counts();
    }
    for a in atlases.iter_mut().flatten() {
        let mut anim = AtlasAnimations::take(a);
        let (mut rects, mut texels) = (Vec::new(), Vec::new());
        for _ in 0..5 {
            anim.tick();
            let mut at = 0;
            while let Some(s) = anim.next_stale(at) {
                anim.compose(s, &mut rects, &mut texels);
                anim.written(s);
                at = s + 1;
            }
        }
        for s in a.sprites.values() {
            let _ = (s.uv(a.width, a.height), s.gui);
        }
    }
    let font = Font::load(res, &Id::minecraft("default").unwrap());
    for c in ['A', 'B', 'E', 'x', ' ', '\u{4e00}'] {
        let _ = (font.glyph(c), font.texels(c));
    }
    for advance in 0..10 {
        let _ = font.of_advance(advance);
    }
    if let Some(r) = res.get(&Id::minecraft("lang/en_us.json").unwrap()) {
        let _ = ruthenium_assets::lang::read(&r);
    }
    for (_, r) in res.list("equipment", |p| p.ends_with(".json")) {
        let _ = r.json();
    }
    for (_, r) in res.list("textures", |p| p.ends_with(".png")) {
        let _ = (
            r.image(4096),
            r.metadata().map(|m| (m.animation(), m.texture(), m.gui())),
        );
    }
}

fn open(bytes: &[u8], dir: &std::path::Path, case: u64) -> Option<Resources> {
    let path = dir.join(format!("{case}.zip"));
    std::fs::write(&path, bytes).unwrap();
    let pack = Pack::open_untrusted(&path, &Limits::untrusted());
    // The cheap check before a reload passes whatever the full opening takes.
    let checked = Pack::check_untrusted(&path, &Limits::untrusted());
    if pack.is_ok() {
        assert!(checked.is_ok(), "case {case}: {checked:?}");
    }
    let _ = std::fs::remove_file(&path);
    Some(Resources::new(vec![pack.ok()?]))
}

/// Changes a few bytes, inserts or cuts some, or writes a number a reader might trust.
fn mutate(bytes: &mut Vec<u8>, rng: &mut Lcg) {
    for _ in 0..1 + rng.next(4) {
        if bytes.is_empty() {
            bytes.push(0);
        }
        let at = rng.next(bytes.len() as u64) as usize;
        match rng.next(6) {
            0 => bytes[at] = rng.next(256) as u8,
            1 => bytes[at] ^= 1 << rng.next(8),
            2 => {
                let n =
                    [0u32, 1, 0x7FFF_FFFF, 0xFFFF_FFFF, 0x8000_0000, 65535][rng.next(6) as usize];
                let end = (at + 4).min(bytes.len());
                bytes[at..end].copy_from_slice(&n.to_le_bytes()[..end - at]);
            }
            3 => bytes.truncate(at),
            4 => {
                let digit = b"0123456789-e.[{\"]}"[rng.next(18) as usize];
                bytes.insert(at, digit);
            }
            _ => {
                let len = rng.next(16) as usize;
                let end = (at + len).min(bytes.len());
                bytes.drain(at..end);
            }
        }
    }
}

#[test]
fn the_pack_itself_loads() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    let res = open(&zip(&files()), dir, 0).expect("the pack opens");
    let opts = AtlasOptions::default();
    let blocks = atlas::build(&res, atlas::atlas_def("blocks").unwrap(), &opts).unwrap();
    assert!(blocks.warnings.is_empty(), "{:?}", blocks.warnings);
    for s in [
        "block/stone",
        "block/lava",
        "block/cut",
        "block/stone_red",
        "entity/x",
    ] {
        assert!(blocks.sprite(&Id::minecraft(s).unwrap()).is_some(), "{s}");
    }
    let font = Font::load(&res, &Id::minecraft("default").unwrap());
    assert!(font.warnings.is_empty(), "{:?}", font.warnings);
    assert!(font.has('A') && font.has('E') && font.has('x'));
    let models = BlockModels::bake(&res, &blocks);
    assert!(models.warnings.is_empty(), "{:?}", models.warnings);
    load_everything(&res);
}

/// Cases per test; each changes the pack's files or its zip. A case takes some 15 ms
/// unoptimised.
const CASES: u64 = if cfg!(debug_assertions) { 1000 } else { 4000 };

#[test]
fn files_changed_at_random_load_without_a_panic() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    let base = files();
    let mut rng = Lcg(0x5EED_F11E);
    for case in 0..CASES {
        let mut f = base.clone();
        for _ in 0..1 + rng.next(3) {
            let i = rng.next(f.len() as u64) as usize;
            mutate(&mut f[i].1, &mut rng);
        }
        if let Some(res) = open(&zip(&f), dir, case) {
            load_everything(&res);
        }
    }
}

#[test]
fn zips_changed_at_random_load_without_a_panic() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    let base = zip(&files());
    let mut rng = Lcg(0x0123_4567_89AB);
    let mut opened = 0;
    for case in 0..CASES {
        let mut z = base.clone();
        // Mostly the directory and the end record, where the numbers are.
        let tail = rng.next(2) == 0;
        if tail {
            let from = z.len() - z.len() / 8;
            let mut end = z.split_off(from);
            mutate(&mut end, &mut rng);
            z.extend(end);
        } else {
            mutate(&mut z, &mut rng);
        }
        if let Some(res) = open(&z, dir, case) {
            opened += 1;
            load_everything(&res);
        }
    }
    // Most changes leave a pack that opens, with some of its files broken.
    assert!(opened > CASES / 10, "{opened}");
}

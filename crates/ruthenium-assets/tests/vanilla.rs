//! Checks against the player's real 26.3 install. Skipped unless
//! `RUTHENIUM_GAME_DIR` names an imported game folder (`<data>/game`, as
//! written by `ruthenium prepare`), because the game files are never in the
//! repository.
//!
//! `RUTHENIUM_MIP_GOLDEN` may also name a file of mip levels written by
//! vanilla's own loader and `MipmapGenerator` (a local, uncommitted dev
//! harness): every level of every sprite in it must match bit for bit.

use std::path::PathBuf;

use ruthenium_assets::atlas::{self, AtlasOptions};
use ruthenium_assets::meta::MipmapStrategy;
use ruthenium_assets::{Id, Image, Pack, Resources, mipmap};

fn vanilla() -> Option<Resources> {
    let dir = PathBuf::from(std::env::var_os("RUTHENIUM_GAME_DIR")?);
    let version = dir.join("versions/26.3");
    let index = dir.join("assets/indexes/34.json");
    let pack = Pack::vanilla(
        &version.join("client.jar"),
        &index,
        &dir.join("assets/objects"),
    )
    .expect("vanilla pack");
    Some(Resources::new(vec![pack]))
}

/// Atlas sizes vanilla 26.3 logs at start-up (`Created: WxHxM`) with default
/// options and no resource packs.
const VANILLA_SIZES: [(&str, u32, u32, u32); 12] = [
    ("particles", 512, 256, 0),
    ("celestials", 256, 128, 0),
    ("decorated_pot", 128, 128, 0),
    ("paintings", 512, 256, 0),
    ("shield_patterns", 1024, 512, 0),
    ("banner_patterns", 1024, 512, 0),
    ("blocks", 2048, 2048, 4),
    ("items", 1024, 512, 0),
    ("gui", 1024, 1024, 0),
    ("map_decorations", 128, 64, 0),
    ("chests", 512, 512, 0),
    ("shulker_boxes", 512, 512, 0),
];

#[test]
fn atlas_sizes_match_vanilla() {
    let Some(res) = vanilla() else {
        eprintln!("RUTHENIUM_GAME_DIR not set; skipped");
        return;
    };
    let start = std::time::Instant::now();
    let atlases = atlas::build_all(&res, &AtlasOptions::default());
    eprintln!("12 atlases in {:.0?}", start.elapsed());
    for a in atlases {
        let a = a.expect("atlas builds");
        for w in &a.warnings {
            eprintln!("{}: {w}", a.def.name);
        }
        let (_, w, h, m) = VANILLA_SIZES.iter().find(|s| s.0 == a.def.name).unwrap();
        assert_eq!(
            (a.width, a.height, a.max_level),
            (*w, *h, *m),
            "{}",
            a.def.name
        );
        assert_eq!(a.levels.len() as u32, m + 1);
        assert!(a.sprites.contains_key(&atlas::missing_id()));
    }
}

struct Reader(Vec<u8>, usize);

impl Reader {
    fn take(&mut self, n: usize) -> &[u8] {
        self.1 += n;
        &self.0[self.1 - n..self.1]
    }
    fn u16(&mut self) -> u16 {
        u16::from_be_bytes(self.take(2).try_into().unwrap())
    }
    fn u32(&mut self) -> u32 {
        u32::from_be_bytes(self.take(4).try_into().unwrap())
    }
    fn utf(&mut self) -> String {
        let n = self.u16() as usize;
        String::from_utf8(self.take(n).to_vec()).unwrap()
    }
    fn done(&self) -> bool {
        self.1 >= self.0.len()
    }
}

#[test]
fn mips_match_vanilla() {
    let (Some(res), Some(golden)) = (vanilla(), std::env::var_os("RUTHENIUM_MIP_GOLDEN")) else {
        eprintln!("RUTHENIUM_GAME_DIR or RUTHENIUM_MIP_GOLDEN not set; skipped");
        return;
    };
    let mut r = Reader(std::fs::read(golden).unwrap(), 0);
    let s2l: Vec<u16> = (0..256).map(|_| r.u16()).collect();
    let l2s = r.take(1024).to_vec();
    assert_eq!(
        &s2l[..],
        &mipmap::srgb_to_linear_table()[..],
        "sRGB to linear table"
    );
    assert_eq!(
        &l2s[..],
        &mipmap::linear_to_srgb_table()[..],
        "linear to sRGB table"
    );
    let (mut checked, mut failed) = (0, Vec::new());
    while !r.done() {
        let file = r.utf();
        let sprite = r.utf();
        let levels = r.u32();
        let strategy = match r.utf().as_str() {
            "auto" => MipmapStrategy::Auto,
            "mean" => MipmapStrategy::Mean,
            "cutout" => MipmapStrategy::Cutout,
            "strict_cutout" => MipmapStrategy::StrictCutout,
            "dark_cutout" => MipmapStrategy::DarkCutout,
            s => panic!("strategy {s}"),
        };
        let bias = f32::from_bits(r.u32());
        let expected: Vec<Image> = (0..r.u32())
            .map(|_| {
                let (w, h) = (r.u32(), r.u32());
                Image {
                    width: w,
                    height: h,
                    pixels: (0..w * h).map(|_| r.u32()).collect(),
                }
            })
            .collect();
        let resource = res
            .get(&Id::minecraft(&file).unwrap())
            .unwrap_or_else(|| panic!("{file}"));
        let meta = resource
            .metadata()
            .unwrap()
            .texture()
            .unwrap()
            .unwrap_or_default();
        assert_eq!(
            (meta.mipmap_strategy, meta.alpha_cutoff_bias),
            (strategy, bias),
            "{file} metadata"
        );
        let image = Image::decode_png(&resource.read().unwrap()).unwrap();
        let t = image.transparency();
        let got = mipmap::generate(&sprite, image, levels, strategy, bias, t);
        if got != expected {
            let level = got.iter().zip(&expected).position(|(a, b)| a != b);
            failed.push(format!("{file} (level {level:?})"));
        }
        checked += 1;
    }
    eprintln!("{checked} sprites checked");
    assert!(
        failed.is_empty(),
        "{} of {checked} differ: {:?}",
        failed.len(),
        &failed[..failed.len().min(20)]
    );
}

/// Bakes every block state's model from the real game and checks facts about a few
/// well-known blocks.
#[test]
fn block_models_bake_from_the_game() {
    use ruthenium_assets::model::{BlockModels, Layer, ModelPart};
    use ruthenium_data::v26_3::Block;
    use ruthenium_data::v26_3::blocks::STATE_COUNT;

    let Some(res) = vanilla() else {
        eprintln!("RUTHENIUM_GAME_DIR not set; skipped");
        return;
    };
    let blocks = atlas::build(
        &res,
        atlas::atlas_def("blocks").unwrap(),
        &AtlasOptions::default(),
    )
    .expect("the block atlas builds");
    let start = std::time::Instant::now();
    let models = BlockModels::bake(&res, &blocks);
    eprintln!(
        "{} parts for {STATE_COUNT} states in {:.0?}",
        models.part_count(),
        start.elapsed()
    );
    for w in &models.warnings {
        eprintln!("{w}");
    }
    assert!(
        models.warnings.is_empty(),
        "the game's own files bake cleanly"
    );

    let state = |name: &str, values: &[(&str, &str)]| {
        let info = Block::from_name(name).unwrap().info();
        values.iter().fold(info.default_state, |s, (p, v)| {
            info.with_value(s, p, v).unwrap()
        })
    };
    let parts = |s: u32, seed: i64| {
        let mut out: Vec<&ModelPart> = Vec::new();
        models.parts(s, seed, &mut out);
        out
    };
    let sprite = |name: &str| {
        let s = blocks.sprite(&Id::minecraft(name).unwrap()).unwrap();
        let (x, y) = ((s.x + s.padding) as f32, (s.y + s.padding) as f32);
        (x, y, x + s.width as f32, y + s.height as f32)
    };
    let inside = |uv: [f32; 2], s: (f32, f32, f32, f32)| {
        uv[0] >= s.0 && uv[0] <= s.2 && uv[1] >= s.1 && uv[1] <= s.3
    };

    // Stone: one of two variants, six culled solid faces of the stone sprite.
    let stone = parts(state("minecraft:stone", &[]), 0);
    assert_eq!(stone.len(), 1);
    assert!(stone[0].unculled().is_empty());
    for d in 0..6 {
        let q = &stone[0].culled(d)[0];
        assert_eq!(
            (usize::from(q.direction), q.layer, q.tint_index),
            (d, Layer::Solid, -1)
        );
        assert!(q.uvs.iter().all(|uv| inside(*uv, sprite("block/stone"))));
    }
    assert!(stone[0].ambient_occlusion);

    // Air has a model without elements.
    assert!(models.is_empty(state("minecraft:air", &[])));
    assert!(parts(state("minecraft:air", &[]), 0)[0].all().is_empty());

    // A grass block: the top is tinted, and each side has a tinted overlay over it.
    let grass = parts(state("minecraft:grass_block", &[("snowy", "false")]), 3);
    assert_eq!(grass[0].culled(1).len(), 1);
    assert_eq!(grass[0].culled(1)[0].tint_index, 0);
    assert_eq!(grass[0].culled(0)[0].tint_index, -1);
    for d in 2..6 {
        let side = grass[0].culled(d);
        assert_eq!(side.len(), 2);
        assert_eq!((side[0].tint_index, side[0].layer), (-1, Layer::Solid));
        assert_eq!((side[1].tint_index, side[1].layer), (0, Layer::Cutout));
    }
    // Its four variants turn the top's texture: every one is picked by some seed.
    let mut tops: Vec<[u32; 2]> = (0..64)
        .map(|seed| {
            parts(
                state("minecraft:grass_block", &[("snowy", "false")]),
                seed * 104_729,
            )[0]
            .culled(1)[0]
                .uvs[0]
                .map(f32::to_bits)
        })
        .collect();
    tops.sort_unstable();
    tops.dedup();
    assert_eq!(tops.len(), 4);

    // Short grass: two crossed planes, drawn from both sides, never culled.
    let tuft = parts(state("minecraft:short_grass", &[]), 0);
    assert_eq!(tuft[0].unculled().len(), 4);
    assert_eq!(tuft[0].all().len(), 4);
    assert!(
        tuft[0]
            .unculled()
            .iter()
            .all(|q| q.layer == Layer::Cutout && q.tint_index == 0)
    );
    assert!(!tuft[0].ambient_occlusion);

    // Leaves and a spawner's cage have holes; stained glass and ice are partly
    // transparent, and plain glass, whose texture only has holes, is made translucent
    // by its model ("force_translucent").
    let layer = |name: &str| parts(state(name, &[]), 0)[0].all()[0].layer;
    assert_eq!(layer("minecraft:oak_leaves"), Layer::Cutout);
    assert_eq!(layer("minecraft:spawner"), Layer::Cutout);
    assert_eq!(layer("minecraft:glass"), Layer::Translucent);
    assert_eq!(layer("minecraft:red_stained_glass"), Layer::Translucent);
    assert_eq!(layer("minecraft:ice"), Layer::Translucent);
    assert_eq!(layer("minecraft:oak_planks"), Layer::Solid);

    // A log lying along x: its end faces look west and east.
    let log = parts(state("minecraft:oak_log", &[("axis", "x")]), 0);
    for d in [4, 5] {
        let q = &log[0].culled(d)[0];
        assert!(
            q.uvs
                .iter()
                .all(|uv| inside(*uv, sprite("block/oak_log_top")))
        );
    }
    assert!(
        log[0].culled(1)[0]
            .uvs
            .iter()
            .all(|uv| inside(*uv, sprite("block/oak_log")))
    );

    // A bottom slab: its top is half way up and culled with nothing.
    let slab = parts(state("minecraft:oak_slab", &[("type", "bottom")]), 0);
    assert_eq!(slab[0].unculled().len(), 1);
    assert!(slab[0].unculled()[0].positions.iter().all(|p| p[1] == 0.5));
    assert_eq!(slab[0].unculled()[0].direction, 1);

    // A fence with two arms: the post and one part per arm.
    let fence = parts(
        state(
            "minecraft:oak_fence",
            &[("north", "true"), ("east", "true")],
        ),
        0,
    );
    assert_eq!(fence.len(), 3);
    // The east arm is the north arm's model turned: it reaches x = 1, not z = 0.
    let reach = |p: &ModelPart, axis: usize, to: f32| {
        p.all()
            .iter()
            .any(|q| q.positions.iter().any(|v| v[axis] == to))
    };
    assert!(reach(fence[1], 2, 0.0) && !reach(fence[1], 0, 1.0));
    assert!(reach(fence[2], 0, 1.0) && !reach(fence[2], 2, 0.0));

    // A torch gives off light by its state, not by its model; a few blocks' models say
    // it themselves.
    let lit = (0..STATE_COUNT as u32)
        .filter(|s| {
            parts(*s, 0)
                .iter()
                .any(|p| p.all().iter().any(|q| q.light_emission > 0))
        })
        .count();
    eprintln!("{lit} states with a light-emitting element");

    // Every state bakes to something consistent.
    // (A multipart state none of whose conditions holds has no part at all.)
    let (mut quads, mut empty, mut translucent) = (0usize, 0usize, 0usize);
    for s in 0..STATE_COUNT as u32 {
        let list = parts(s, 0);
        let n: usize = list.iter().map(|p| p.all().len()).sum();
        assert_eq!(n == 0, models.is_empty(s), "state {s}");
        empty += usize::from(n == 0);
        quads += n;
        for q in list.iter().flat_map(|p| p.all()) {
            translucent += usize::from(q.layer == Layer::Translucent);
            assert!(q.direction < 6);
            for (p, uv) in q.positions.iter().zip(q.uvs) {
                assert!(
                    p.iter().all(|c| (-1.0..=2.0).contains(c)),
                    "state {s}: {p:?}"
                );
                assert!(
                    uv[0] >= 0.0
                        && uv[0] <= blocks.width as f32
                        && uv[1] >= 0.0
                        && uv[1] <= blocks.height as f32,
                    "state {s}: {uv:?}"
                );
            }
        }
    }
    eprintln!("{quads} quads in all, {translucent} translucent; {empty} states draw nothing");
}

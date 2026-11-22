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

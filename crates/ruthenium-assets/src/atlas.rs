//! Sprite atlases: sources, sprite loading, stitching and the finished
//! texels of every mip level.
//!
//! Follows 26.3's `client/resources/model/sprite/AtlasManager.java`,
//! `client/renderer/texture/atlas/**`, `SpriteLoader.java`,
//! `SpriteContents.java` and `TextureAtlas.java`. Vanilla composes atlases
//! on the GPU (a clamp-to-edge blit of each sprite level into its padded
//! slot); the CPU composition here produces the same texels.

use std::collections::{BTreeMap, HashMap};
use std::fmt;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};

use rayon::prelude::*;
use regex::Regex;
use serde_json::Value;

use crate::id::Id;
use crate::image::{Image, Transparency, alpha};
use crate::limits::Limits;
use crate::meta::{GuiScaling, Metadata, MipmapStrategy};
use crate::mipmap;
use crate::resources::{Resource, Resources};
use crate::stitch;

/// One of vanilla's atlases.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AtlasDef {
    /// `atlases/<name>.json`.
    pub name: &'static str,
    /// The texture id vanilla registers it under, `textures/atlas/<file>`.
    pub texture: &'static str,
    /// Only the block atlas has mip levels.
    pub mipmapped: bool,
    /// Sprites carry `gui` metadata (nine-slice, tile, stretch).
    pub gui: bool,
}

const fn def(name: &'static str, texture: &'static str, mipmapped: bool, gui: bool) -> AtlasDef {
    AtlasDef {
        name,
        texture,
        mipmapped,
        gui,
    }
}

/// The 12 atlases, in `AtlasManager.KNOWN_ATLASES` order.
pub const ATLASES: [AtlasDef; 12] = [
    def(
        "banner_patterns",
        "textures/atlas/banner_patterns.png",
        false,
        false,
    ),
    def("blocks", "textures/atlas/blocks.png", true, false),
    def("items", "textures/atlas/items.png", false, false),
    def("chests", "textures/atlas/chest.png", false, false),
    def(
        "decorated_pot",
        "textures/atlas/decorated_pot.png",
        false,
        false,
    ),
    def("gui", "textures/atlas/gui.png", false, true),
    def(
        "map_decorations",
        "textures/atlas/map_decorations.png",
        false,
        false,
    ),
    def("paintings", "textures/atlas/paintings.png", false, false),
    def("particles", "textures/atlas/particles.png", false, false),
    def(
        "shield_patterns",
        "textures/atlas/shield_patterns.png",
        false,
        false,
    ),
    def(
        "shulker_boxes",
        "textures/atlas/shulker_boxes.png",
        false,
        false,
    ),
    def("celestials", "textures/atlas/celestials.png", false, false),
];

pub fn atlas_def(name: &str) -> Option<&'static AtlasDef> {
    ATLASES.iter().find(|d| d.name == name)
}

/// Settings that change atlas contents.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AtlasOptions {
    /// The `mipmapLevels` option (0-4, default 4); used by the block atlas.
    pub mip_levels: u32,
    /// The device's largest 2D image size.
    pub max_size: u32,
    /// `maxAnisotropyBit` when texture filtering is anisotropic, else 0.
    pub anisotropy_bit: u32,
}

impl Default for AtlasOptions {
    fn default() -> AtlasOptions {
        AtlasOptions {
            mip_levels: 4,
            max_size: 16384,
            anisotropy_bit: 0,
        }
    }
}

/// The id of the missing sprite every atlas has.
pub fn missing_id() -> Id {
    Id::minecraft("missingno").unwrap()
}

/// `textures/<id>.png`.
fn texture_file(id: &Id) -> Id {
    id.wrap("textures/", ".png")
}

/// An image decoded on first use and shared by several sprites (unstitch
/// regions, palette permutations).
#[derive(Debug)]
struct Shared<'a> {
    resource: Resource<'a>,
    image: OnceLock<Option<Image>>,
}

impl Shared<'_> {
    fn get(&self, max_side: u32) -> Option<&Image> {
        self.image
            .get_or_init(|| self.resource.image(max_side).ok())
            .as_ref()
    }
}

/// What loading an atlas's sprites is held to.
struct Load<'l> {
    limits: &'l Limits,
    /// The largest side a sprite's picture may have: the atlas's.
    max_side: u32,
    /// Texels of every sprite loaded so far, frames and all, against the atlas's area.
    texels: AtomicU64,
    /// Some sprite did not fit in what the atlas may hold.
    full: AtomicBool,
}

impl Load<'_> {
    /// Counts a sprite's picture; false (and the atlas is full) if the sprites so far
    /// would cover more than the whole atlas, which then could never be stitched.
    fn take(&self, image: &Image) -> bool {
        let n = u64::from(image.width) * u64::from(image.height);
        let area = u64::from(self.max_side) * u64::from(self.max_side);
        let fits = self
            .texels
            .try_update(Ordering::Relaxed, Ordering::Relaxed, |used| {
                used.checked_add(n).filter(|&t| t <= area)
            })
            .is_ok();
        if !fits {
            self.full.store(true, Ordering::Relaxed);
        }
        fits
    }
}

/// A palette for `paletted_permutations`: `opaque key colour -> colour`.
type PaletteMap = Result<HashMap<u32, u32>, String>;

#[derive(Debug)]
enum Loader<'a> {
    File(Resource<'a>),
    Region {
        image: Arc<Shared<'a>>,
        region: [f64; 4],
        divisor: [f64; 2],
    },
    Paletted {
        image: Arc<Shared<'a>>,
        palette: Arc<OnceLock<PaletteMap>>,
        key: Arc<Option<Resource<'a>>>,
        target: Option<Resource<'a>>,
    },
    Missing,
}

/// A sprite source from an atlas definition.
#[derive(Debug)]
enum Source {
    Directory {
        source: String,
        prefix: String,
    },
    Single {
        resource: Id,
        sprite: Option<Id>,
    },
    Filter {
        namespace: Option<Regex>,
        path: Option<Regex>,
    },
    Unstitch {
        resource: Id,
        regions: Vec<(Id, [f64; 4])>,
        divisor: [f64; 2],
    },
    Paletted {
        textures: Vec<Id>,
        palette_key: Id,
        permutations: Vec<(String, Id)>,
        separator: String,
    },
}

fn parse_id(v: Option<&Value>, what: &str) -> Result<Id, String> {
    let s = v
        .and_then(Value::as_str)
        .ok_or_else(|| format!("missing {what}"))?;
    Id::parse(s).ok_or_else(|| format!("bad {what} {s:?}"))
}

fn parse_str(v: Option<&Value>, what: &str) -> Result<String, String> {
    v.and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| format!("missing {what}"))
}

fn parse_f64(v: Option<&Value>, what: &str, default: Option<f64>) -> Result<f64, String> {
    match (v, default) {
        (Some(v), _) => v.as_f64().ok_or_else(|| format!("{what}: not a number")),
        (None, Some(d)) => Ok(d),
        (None, None) => Err(format!("missing {what}")),
    }
}

fn parse_sources(json: &Value) -> Result<Vec<Source>, String> {
    let list = json
        .get("sources")
        .and_then(Value::as_array)
        .ok_or("no \"sources\" list")?;
    list.iter().map(parse_source).collect()
}

fn parse_source(v: &Value) -> Result<Source, String> {
    let ty = parse_id(v.get("type"), "type")?;
    if ty.namespace() != "minecraft" {
        return Err(format!("unknown source type {ty}"));
    }
    Ok(match ty.path() {
        "directory" => Source::Directory {
            source: parse_str(v.get("source"), "source")?,
            prefix: parse_str(v.get("prefix"), "prefix")?,
        },
        "single" => Source::Single {
            resource: parse_id(v.get("resource"), "resource")?,
            sprite: v
                .get("sprite")
                .map(|s| parse_id(Some(s), "sprite"))
                .transpose()?,
        },
        "filter" => {
            let pattern = v
                .get("pattern")
                .and_then(Value::as_object)
                .ok_or("missing pattern")?;
            let re = |k: &str| -> Result<Option<Regex>, String> {
                pattern
                    .get(k)
                    .map(|p| {
                        Regex::new(p.as_str().ok_or_else(|| format!("{k}: not a string"))?)
                            .map_err(|e| e.to_string())
                    })
                    .transpose()
            };
            Source::Filter {
                namespace: re("namespace")?,
                path: re("path")?,
            }
        }
        "unstitch" => {
            let regions = v
                .get("regions")
                .and_then(Value::as_array)
                .filter(|r| !r.is_empty())
                .ok_or("regions: missing or empty")?
                .iter()
                .map(|r| {
                    Ok((
                        parse_id(r.get("sprite"), "sprite")?,
                        [
                            parse_f64(r.get("x"), "x", None)?,
                            parse_f64(r.get("y"), "y", None)?,
                            parse_f64(r.get("width"), "width", None)?,
                            parse_f64(r.get("height"), "height", None)?,
                        ],
                    ))
                })
                .collect::<Result<_, String>>()?;
            Source::Unstitch {
                resource: parse_id(v.get("resource"), "resource")?,
                regions,
                divisor: [
                    parse_f64(v.get("divisor_x"), "divisor_x", Some(1.0))?,
                    parse_f64(v.get("divisor_y"), "divisor_y", Some(1.0))?,
                ],
            }
        }
        "paletted_permutations" => Source::Paletted {
            textures: v
                .get("textures")
                .and_then(Value::as_array)
                .ok_or("missing textures")?
                .iter()
                .map(|t| parse_id(Some(t), "texture"))
                .collect::<Result<_, _>>()?,
            palette_key: parse_id(v.get("palette_key"), "palette_key")?,
            permutations: v
                .get("permutations")
                .and_then(Value::as_object)
                .ok_or("missing permutations")?
                .iter()
                .map(|(k, p)| Ok((k.clone(), parse_id(Some(p), "permutation")?)))
                .collect::<Result<_, String>>()?,
            separator: v
                .get("separator")
                .and_then(Value::as_str)
                .unwrap_or("_")
                .to_owned(),
        },
        other => return Err(format!("unknown source type minecraft:{other}")),
    })
}

/// Runs every source of the atlas's definition files (all packs, lowest
/// first) into one id -> loader map. A definition file that fails to parse
/// is skipped, as in vanilla.
fn collect<'a>(
    res: &'a Resources,
    def: &AtlasDef,
    warnings: &mut Vec<String>,
) -> Result<HashMap<Id, Loader<'a>>, AtlasError> {
    let Some(file) = Id::minecraft(&format!("atlases/{}.json", def.name)) else {
        return Ok(HashMap::new());
    };
    let limits = res.limits();
    let mut sprites: HashMap<Id, Loader<'a>> = HashMap::new();
    for r in res.stack(&file) {
        let sources = r.json().and_then(|j| {
            let n = j
                .get("sources")
                .and_then(Value::as_array)
                .map_or(0, Vec::len);
            if n > r.limits().atlas_sources {
                return Err(format!(
                    "{n} sources is more than an atlas definition may have ({})",
                    r.limits().atlas_sources
                ));
            }
            parse_sources(&j)
        });
        let sources = match sources {
            Ok(s) => s,
            Err(e) => {
                warnings.push(format!("{file} in pack {}: {e}", r.pack().name()));
                continue;
            }
        };
        for source in sources {
            run_source(res, source, &mut sprites, warnings);
            if sprites.len() > limits.atlas_sprites {
                return Err(AtlasError::TooManySprites {
                    atlas: def.name,
                    max: limits.atlas_sprites,
                });
            }
        }
    }
    // The missing sprite is the atlas's own (vanilla puts it in after every source).
    sprites.remove(&missing_id());
    Ok(sprites)
}

fn run_source<'a>(
    res: &'a Resources,
    source: Source,
    sprites: &mut HashMap<Id, Loader<'a>>,
    warnings: &mut Vec<String>,
) {
    match source {
        Source::Directory { source, prefix } => {
            let dir = format!("textures/{source}");
            for (file, r) in res.list(&dir, |p| p.ends_with(".png")) {
                let path = &file.path()[dir.len() + 1..file.path().len() - 4];
                sprites.insert(file.with_path(format!("{prefix}{path}")), Loader::File(r));
            }
        }
        Source::Single { resource, sprite } => match res.get(&texture_file(&resource)) {
            Some(r) => {
                sprites.insert(sprite.unwrap_or(resource), Loader::File(r));
            }
            None => warnings.push(format!("missing sprite: {}", texture_file(&resource))),
        },
        Source::Filter { namespace, path } => {
            let hit = |re: &Option<Regex>, s: &str| re.as_ref().is_none_or(|r| r.is_match(s));
            sprites.retain(|id, _| !(hit(&namespace, id.namespace()) && hit(&path, id.path())));
        }
        Source::Unstitch {
            resource,
            regions,
            divisor,
        } => match res.get(&texture_file(&resource)) {
            Some(r) => {
                let image = Arc::new(Shared {
                    resource: r,
                    image: OnceLock::new(),
                });
                for (sprite, region) in regions {
                    sprites.insert(
                        sprite,
                        Loader::Region {
                            image: image.clone(),
                            region,
                            divisor,
                        },
                    );
                }
            }
            None => warnings.push(format!("missing sprite: {}", texture_file(&resource))),
        },
        Source::Paletted {
            textures,
            palette_key,
            permutations,
            separator,
        } => {
            let palette_file = |id: &Id| id.wrap("textures/palettes/", ".png");
            let key = Arc::new(res.get(&palette_file(&palette_key)));
            let palettes: Vec<(String, Arc<OnceLock<PaletteMap>>, Option<Resource<'a>>)> =
                permutations
                    .into_iter()
                    .map(|(suffix, p)| {
                        (
                            suffix,
                            Arc::new(OnceLock::new()),
                            res.get(&palette_file(&p)),
                        )
                    })
                    .collect();
            let max = res.limits().atlas_sprites;
            for texture in textures {
                if sprites.len() > max {
                    // The atlas fails as soon as this source is done.
                    return;
                }
                let Some(r) = res.get(&texture_file(&texture)) else {
                    warnings.push(format!("unable to find texture {}", texture_file(&texture)));
                    continue;
                };
                let image = Arc::new(Shared {
                    resource: r,
                    image: OnceLock::new(),
                });
                for (suffix, palette, target) in &palettes {
                    sprites.insert(
                        texture.wrap("", &format!("{separator}{suffix}")),
                        Loader::Paletted {
                            image: image.clone(),
                            palette: palette.clone(),
                            key: key.clone(),
                            target: target.clone(),
                        },
                    );
                }
            }
        }
    }
}

fn load_palette(r: Option<&Resource<'_>>, max_side: u32) -> Result<Vec<u32>, String> {
    let r = r.ok_or("palette image not found")?;
    Ok(r.image(max_side)?.pixels)
}

fn build_palette(
    key: Option<&Resource<'_>>,
    target: Option<&Resource<'_>>,
    max_side: u32,
) -> PaletteMap {
    let (key, target) = (
        load_palette(key, max_side)?,
        load_palette(target, max_side)?,
    );
    if key.len() != target.len() {
        return Err(format!(
            "palette sizes differ: {} != {}",
            key.len(),
            target.len()
        ));
    }
    Ok(key
        .into_iter()
        .zip(target)
        .filter(|(k, _)| alpha(*k) != 0)
        .map(|(k, t)| (k | 0xFF00_0000, t))
        .collect())
}

/// Recolours one texel (`client/resources/palette/PaletteMapping.java`).
fn apply_palette(map: &HashMap<u32, u32>, c: u32) -> u32 {
    let a = alpha(c);
    if a == 0 {
        return c;
    }
    let rgb = c | 0xFF00_0000;
    let t = map.get(&rgb).copied().unwrap_or(rgb);
    (a * alpha(t) / 255) << 24 | (t & 0x00FF_FFFF)
}

/// Frame timing of an animated sprite.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Animation {
    /// `(frame index, ticks)`, in play order.
    pub frames: Vec<(u32, u32)>,
    /// Frames per row of the source image.
    pub per_row: u32,
    pub interpolate: bool,
}

impl Animation {
    /// Where frame `index` starts in the sprite image, in frame units.
    pub fn frame_pos(&self, index: u32) -> (u32, u32) {
        (index % self.per_row, index / self.per_row)
    }
}

/// A loaded sprite before stitching.
#[derive(Debug)]
struct Contents {
    id: Id,
    width: u32,
    height: u32,
    image: Image,
    animation: Option<Animation>,
    strategy: MipmapStrategy,
    alpha_cutoff_bias: f32,
    transparency: Transparency,
    gui: Option<GuiScaling>,
    blur: bool,
    clamp: bool,
}

impl Contents {
    fn plain(id: Id, image: Image) -> Contents {
        Contents {
            id,
            width: image.width,
            height: image.height,
            transparency: image.transparency(),
            image,
            animation: None,
            strategy: MipmapStrategy::Auto,
            alpha_cutoff_bias: 0.0,
            gui: None,
            blur: false,
            clamp: false,
        }
    }
}

fn load_file(id: Id, r: &Resource<'_>, gui: bool, load: &Load<'_>) -> Result<Contents, String> {
    let meta: Metadata = r.metadata()?;
    let animation = meta.animation()?;
    let max_frames = load
        .limits
        .animation_frames
        .min(r.limits().animation_frames);
    if let Some(list) = animation.as_ref().and_then(|a| a.frames.as_ref())
        && list.len() > max_frames
    {
        return Err(format!(
            "{} frames is more than an animation may have ({max_frames})",
            list.len()
        ));
    }
    let texture = meta.texture()?.unwrap_or_default();
    let gui = if gui {
        Some(meta.gui()?.unwrap_or_default())
    } else {
        None
    };
    let image = r.image(load.max_side)?;
    if !load.take(&image) {
        return Err("the atlas's sprites cover more than it may hold".into());
    }
    let (w, h) = match &animation {
        Some(a) => a.frame_size(image.width, image.height),
        None => (image.width, image.height),
    };
    if image.width % w != 0 || image.height % h != 0 {
        return Err(format!(
            "image size {}x{} is not a multiple of frame size {w}x{h}",
            image.width, image.height
        ));
    }
    if let Some(a) = &animation
        && a.frames.is_none()
        && u64::from(image.width / w) * u64::from(image.height / h) > max_frames as u64
    {
        return Err(format!(
            "{}x{} frames of {w}x{h} is more than an animation may have ({max_frames})",
            image.width / w,
            image.height / h
        ));
    }
    let animation = animation.and_then(|a| {
        let per_row = image.width / w;
        let total = per_row * (image.height / h);
        let frames: Vec<(u32, u32)> = match &a.frames {
            None => (0..total).map(|i| (i, a.frametime)).collect(),
            Some(list) => list
                .iter()
                .map(|f| (f.index, f.time.unwrap_or(a.frametime)))
                .filter(|&(index, time)| time > 0 && index < total)
                .collect(),
        };
        (frames.len() > 1).then_some(Animation {
            frames,
            per_row,
            interpolate: a.interpolate,
        })
    });
    Ok(Contents {
        id,
        width: w,
        height: h,
        transparency: image.transparency(),
        image,
        animation,
        strategy: texture.mipmap_strategy,
        alpha_cutoff_bias: texture.alpha_cutoff_bias,
        gui,
        blur: texture.blur,
        clamp: texture.clamp,
    })
}

/// Java's `Mth.floor(double)`.
fn floor_i(v: f64) -> i64 {
    v.floor() as i64
}

fn load(id: Id, loader: Loader<'_>, gui: bool, load: &Load<'_>) -> Result<Contents, String> {
    match loader {
        Loader::Missing => Ok(Contents::plain(id, Image::missing(16, 16))),
        Loader::File(r) => load_file(id, &r, gui, load),
        Loader::Region {
            image,
            region,
            divisor,
        } => {
            let img = image.get(load.max_side).ok_or("unreadable image")?;
            let (sx, sy) = (
                f64::from(img.width) / divisor[0],
                f64::from(img.height) / divisor[1],
            );
            let [x, y, w, h] = [
                region[0] * sx,
                region[1] * sy,
                region[2] * sx,
                region[3] * sy,
            ]
            .map(floor_i);
            let fits = |v: i64| u32::try_from(v).ok();
            let sizes = |v: i64| u32::try_from(v).ok().filter(|&v| v > 0);
            let sub = (|| img.sub_image(fits(x)?, fits(y)?, sizes(w)?, sizes(h)?))()
                .ok_or("region is empty or outside the image")?;
            if !load.take(&sub) {
                return Err("the atlas's sprites cover more than it may hold".into());
            }
            Ok(Contents::plain(id, sub))
        }
        Loader::Paletted {
            image,
            palette,
            key,
            target,
        } => {
            let img = image.get(load.max_side).ok_or("unreadable image")?;
            let map = palette
                .get_or_init(|| {
                    build_palette(key.as_ref().as_ref(), target.as_ref(), load.max_side)
                })
                .as_ref()?;
            if !load.take(img) {
                return Err("the atlas's sprites cover more than it may hold".into());
            }
            let mut out = img.clone();
            for p in &mut out.pixels {
                *p = apply_palette(map, *p);
            }
            Ok(Contents::plain(id, out))
        }
    }
}

/// A sprite's place in an atlas.
#[derive(Debug, Clone)]
pub struct Sprite {
    /// Top-left of the padded slot.
    pub x: u32,
    pub y: u32,
    /// Texels of padding on each side.
    pub padding: u32,
    /// Frame size.
    pub width: u32,
    pub height: u32,
    pub transparency: Transparency,
    pub blur: bool,
    pub clamp: bool,
    /// GUI atlas only.
    pub gui: Option<GuiScaling>,
    /// Index into [`Atlas::animated`].
    pub animated: Option<usize>,
}

impl Sprite {
    /// UV rectangle `[u0, v0, u1, v1]` in an atlas of `w` x `h`, computed as
    /// vanilla computes it (`TextureAtlasSprite` constructor).
    pub fn uv(&self, w: u32, h: u32) -> [f32; 4] {
        let (x, y) = (self.x + self.padding, self.y + self.padding);
        [
            x as f32 / w as f32,
            y as f32 / h as f32,
            (x + self.width) as f32 / w as f32,
            (y + self.height) as f32 / h as f32,
        ]
    }
}

/// What an animated sprite needs to play: its frames at every mip level.
#[derive(Debug)]
pub struct Animated {
    pub id: Id,
    pub animation: Animation,
    /// The whole sprite image (every frame) per mip level.
    pub levels: Vec<Image>,
}

/// A finished atlas.
#[derive(Debug)]
pub struct Atlas {
    pub def: AtlasDef,
    pub width: u32,
    pub height: u32,
    /// Highest mip level (0 = none).
    pub max_level: u32,
    /// Texels of level 0..=max_level, `width >> level` by `height >> level`.
    pub levels: Vec<Image>,
    pub sprites: BTreeMap<Id, Sprite>,
    pub animated: Vec<Animated>,
    /// Sprites dropped or sources skipped, for the log.
    pub warnings: Vec<String>,
}

impl Atlas {
    pub fn sprite(&self, id: &Id) -> Option<&Sprite> {
        self.sprites.get(id)
    }

    /// The missing sprite, which every atlas has.
    pub fn missing(&self) -> &Sprite {
        &self.sprites[&missing_id()]
    }

    /// Sprite positions in the format of vanilla's atlas dump (F3 + S),
    /// sorted by id: `<id>\tx=<x>\ty=<y>\tw=<w>\th=<h>`.
    pub fn dump_index(&self) -> String {
        let mut out = String::new();
        let mut ids: Vec<&Id> = self.sprites.keys().collect();
        ids.sort_by_key(|id| id.to_string());
        for id in ids {
            let s = &self.sprites[id];
            out.push_str(&format!(
                "{id}\tx={}\ty={}\tw={}\th={}\n",
                s.x, s.y, s.width, s.height
            ));
        }
        out
    }
}

#[derive(Debug)]
pub enum AtlasError {
    /// A sprite cannot fit in the device's largest texture (or the largest atlas a pack
    /// may make).
    TooLarge {
        atlas: &'static str,
        sprite: Id,
        max: u32,
    },
    /// The sprites together cover more than the largest atlas.
    TooManyTexels { atlas: &'static str, max: u32 },
    /// The definitions list more sprites than an atlas may have.
    TooManySprites { atlas: &'static str, max: usize },
}

impl fmt::Display for AtlasError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AtlasError::TooLarge { atlas, sprite, max } => {
                write!(f, "atlas {atlas}: {sprite} does not fit in {max}x{max}")
            }
            AtlasError::TooManyTexels { atlas, max } => {
                write!(f, "atlas {atlas}: its sprites cover more than {max}x{max}")
            }
            AtlasError::TooManySprites { atlas, max } => {
                write!(f, "atlas {atlas}: more than {max} sprites")
            }
        }
    }
}

impl std::error::Error for AtlasError {}

/// Writes one level of a sprite (frame `frame`) into its padded slot,
/// repeating the edge texels into the padding.
fn blit(
    dst: &mut Image,
    src: &Image,
    frame: (u32, u32),
    size: (u32, u32),
    at: (u32, u32),
    pad: u32,
) {
    let (w, h) = size;
    let (fx, fy) = (frame.0 * w, frame.1 * h);
    for dy in 0..h + 2 * pad {
        let sy = dy.saturating_sub(pad).min(h - 1);
        for dx in 0..w + 2 * pad {
            let sx = dx.saturating_sub(pad).min(w - 1);
            dst.set(at.0 + dx, at.1 + dy, src.get(fx + sx, fy + sy));
        }
    }
}

/// Loads, stitches and composes one atlas from the resource stack.
pub fn build(res: &Resources, def: &AtlasDef, opts: &AtlasOptions) -> Result<Atlas, AtlasError> {
    let mut warnings = Vec::new();
    let loaders = collect(res, def, &mut warnings)?;
    let mut jobs: Vec<(Id, Loader<'_>)> = vec![(missing_id(), Loader::Missing)];
    jobs.extend(loaders);
    let max_side = opts.max_size.min(res.limits().atlas_side);
    let load_state = Load {
        limits: res.limits(),
        max_side,
        texels: AtomicU64::new(0),
        full: AtomicBool::new(false),
    };
    let results: Vec<Result<Contents, (Id, String)>> = jobs
        .into_par_iter()
        .map(|(id, loader)| load(id.clone(), loader, def.gui, &load_state).map_err(|e| (id, e)))
        .collect();
    if load_state.full.load(Ordering::Relaxed) {
        return Err(AtlasError::TooManyTexels {
            atlas: def.name,
            max: max_side,
        });
    }
    let mut sprites = Vec::with_capacity(results.len());
    for r in results {
        match r {
            Ok(c) => sprites.push(c),
            Err((id, e)) => warnings.push(format!("sprite {id} dropped: {e}")),
        }
    }

    let requested = if def.mipmapped { opts.mip_levels } else { 0 };
    let levels = stitch::mip_levels(requested, sprites.iter().map(|s| (s.width, s.height)));
    if levels < requested {
        warnings.push(format!(
            "{}: mip levels lowered from {requested} to {levels}",
            def.name
        ));
    }
    let padding = stitch::padding(levels, opts.anisotropy_bit);
    let sizes: Vec<(u32, u32)> = sprites.iter().map(|s| (s.width, s.height)).collect();
    let layout = stitch::stitch(
        &sizes,
        |a, b| sprites[a].id.cmp(&sprites[b].id),
        levels,
        padding,
        max_side,
        max_side,
    )
    .map_err(|stitch::DoesNotFit(i)| AtlasError::TooLarge {
        atlas: def.name,
        sprite: sprites.get(i).map_or_else(missing_id, |s| s.id.clone()),
        max: max_side,
    })?;

    // Mip levels per sprite, over the whole image (every frame).
    let mips: Vec<Vec<Image>> = sprites
        .par_iter_mut()
        .map(|s| {
            let image = std::mem::replace(&mut s.image, Image::new(0, 0));
            mipmap::generate(
                s.id.path(),
                image,
                levels,
                s.strategy,
                s.alpha_cutoff_bias,
                s.transparency,
            )
        })
        .collect();

    let mut out_levels: Vec<Image> = (0..=levels)
        .map(|l| Image::new(layout.width >> l, layout.height >> l))
        .collect();
    let mut placed = BTreeMap::new();
    let mut animated = Vec::new();
    for ((s, m), p) in sprites.into_iter().zip(mips).zip(&layout.placements) {
        let first = s
            .animation
            .as_ref()
            .map_or((0, 0), |a| a.frame_pos(a.frames[0].0));
        for (l, dst) in out_levels.iter_mut().enumerate() {
            blit(
                dst,
                &m[l],
                first,
                (s.width >> l, s.height >> l),
                (p.x >> l, p.y >> l),
                padding >> l,
            );
        }
        let anim_index = s.animation.map(|animation| {
            animated.push(Animated {
                id: s.id.clone(),
                animation,
                levels: m,
            });
            animated.len() - 1
        });
        placed.insert(
            s.id,
            Sprite {
                x: p.x,
                y: p.y,
                padding,
                width: s.width,
                height: s.height,
                transparency: s.transparency,
                blur: s.blur,
                clamp: s.clamp,
                gui: s.gui,
                animated: anim_index,
            },
        );
    }
    Ok(Atlas {
        def: *def,
        width: layout.width,
        height: layout.height,
        max_level: levels,
        levels: out_levels,
        sprites: placed,
        animated,
        warnings,
    })
}

/// Builds every atlas, in parallel.
pub fn build_all(res: &Resources, opts: &AtlasOptions) -> Vec<Result<Atlas, AtlasError>> {
    ATLASES.par_iter().map(|d| build(res, d, opts)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::image::argb;

    /// An opaque grey texel.
    fn grey(v: u32) -> u32 {
        argb(255, v, v, v)
    }

    #[test]
    fn palette_mapping() {
        let map: HashMap<u32, u32> = [(grey(10), 0x80FF0000)].into_iter().collect();
        // Mapped: alpha multiplies (int), colour replaced.
        assert_eq!(
            apply_palette(&map, grey(10) & 0x00FF_FFFF | 0xC000_0000),
            (0xC0 * 0x80 / 255) << 24 | 0xFF0000
        );
        // Unmapped colours keep their rgb; alpha times 255/255.
        assert_eq!(apply_palette(&map, 0xFF123456), 0xFF123456);
        // Transparent texels are untouched.
        assert_eq!(apply_palette(&map, 0x00ABCDEF), 0x00ABCDEF);
    }

    #[test]
    fn blit_repeats_edges() {
        let mut src = Image::new(2, 2);
        src.pixels = vec![1, 2, 3, 4];
        let mut dst = Image::new(4, 4);
        blit(&mut dst, &src, (0, 0), (2, 2), (0, 0), 1);
        assert_eq!(dst.pixels, [1, 1, 2, 2, 1, 1, 2, 2, 3, 3, 4, 4, 3, 3, 4, 4]);
    }

    #[test]
    fn sources_parse() {
        let json: Value = serde_json::from_str(
            r#"{"sources": [
                {"type": "directory", "source": "block", "prefix": "block/"},
                {"type": "minecraft:single", "resource": "entity/bell/bell_body"},
                {"type": "filter", "pattern": {"path": "^block/a"}},
                {"type": "unstitch", "resource": "x", "regions": [{"sprite": "y", "x": 0, "y": 0, "width": 8, "height": 8}], "divisor_x": 16}
            ]}"#,
        )
        .unwrap();
        let s = parse_sources(&json).unwrap();
        assert_eq!(s.len(), 4);
        assert!(matches!(
            &s[3],
            Source::Unstitch {
                divisor: [16.0, 1.0],
                ..
            }
        ));
        let bad: Value = serde_json::from_str(r#"{"sources": [{"type": "nope"}]}"#).unwrap();
        assert!(parse_sources(&bad).is_err());
    }
}

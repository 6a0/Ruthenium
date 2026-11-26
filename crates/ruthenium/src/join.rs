//! Joining a server from the menus: who joins (the account's session, or in development
//! builds an offline name), with what (the imported game's language and data packs), and
//! the words the connecting and disconnected screens use.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::thread::JoinHandle;

use ruthenium_assets::atlas::AtlasOptions;
use ruthenium_assets::item::ItemModels;
use ruthenium_assets::zip::Zip;
use ruthenium_assets::{AtlasAnimations, BlockModels, Font, Id, Image, Resources};
use ruthenium_net::connection::{Auth, Closed, ConnectOptions, Identity, Local, Reason, Stage};
use ruthenium_protocol::packets::{ClientInformation, KnownPack};
use ruthenium_protocol::text::StyledText;
use ruthenium_render::block_entity::equipment::{self, EquipmentSet};
use ruthenium_render::block_entity::model::ModelSet;
use ruthenium_render::block_entity::renderer::{self as block_entity, BlockEntityAssets};
use ruthenium_render::terrain::{BlockTable, ColourMaps};
use ruthenium_render::{MOON_PHASES, SkyImage, SkyImages, WorldAssets};
use ruthenium_sim::blocks::Blocks;
use ruthenium_sim::enchantment::Enchantments;
use ruthenium_sim::entity::EntityTypes;
use ruthenium_sim::item::Items;
use ruthenium_sim::session::{DimensionType, Physics, Session, SessionOptions};
use ruthenium_sim::use_on::Transformers;

use crate::account::Accounts;
use crate::skins::SkinLoader;

/// The brand Ruthenium sends (PLAN D6).
pub const BRAND: &str = "ruthenium";

/// Present in development builds that can join offline-mode servers, and only there: CI
/// checks that release binaries don't contain it (PLAN D3).
#[cfg(feature = "dev-offline")]
pub const DEV_OFFLINE_MARKER: &str = "RUTHENIUM-DEV-OFFLINE-BUILD";

const LANG: &str = "assets/minecraft/lang/en_us.json";
const LANG_PATH: &str = "lang/en_us.json";
const DATAPACKS: &str = "data/minecraft/datapacks/";

/// What joining needs from the imported game.
#[derive(Debug, Default)]
pub struct GameData {
    /// English translations, for the servers' messages.
    pub lang: HashMap<String, String>,
    /// Vanilla's data packs at the game's version: `core` and the feature packs in the
    /// jar (`ServerPacksSource.createVanillaTrustedRepository`).
    pub known_packs: Vec<KnownPack>,
    /// What movement needs: the exported block physics and vanilla's dimension heights.
    pub physics: Option<Physics>,
    /// Every block state's sounds (steps, breaking, placing), for the client's own.
    pub block_sounds: Option<Arc<ruthenium_sim::sound::BlockSounds>>,
}

impl GameData {
    /// Reads the imported game in `game`; empty without it.
    pub fn load(game: &Path) -> GameData {
        let version = ruthenium_prepare::supported_version();
        let Some(imported) = ruthenium_prepare::import::open(version, game) else {
            return GameData::default();
        };
        let Ok(jar) = Zip::open(&imported.client_jar) else {
            return GameData::default();
        };
        let lang = jar
            .find(LANG)
            .and_then(|i| jar.read(i).ok())
            .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
            .and_then(|v| match v {
                serde_json::Value::Object(o) => Some(
                    o.into_iter()
                        .filter_map(|(k, v)| Some((k, v.as_str()?.to_owned())))
                        .collect(),
                ),
                _ => None,
            })
            .unwrap_or_default();
        let id = ruthenium_data::v26_3::version::ID;
        let mut known_packs = vec![KnownPack::vanilla("core", id)];
        let mut features: Vec<String> = jar
            .with_prefix(DATAPACKS)
            .filter_map(|i| {
                let rest = jar.name(i).strip_prefix(DATAPACKS)?;
                let (pack, file) = rest.split_once('/')?;
                (file == "pack.mcmeta").then(|| pack.to_owned())
            })
            .collect();
        features.sort();
        features.dedup();
        known_packs.extend(features.iter().map(|f| KnownPack::vanilla(f, id)));
        let exports = ruthenium_prepare::exporters::exports_dir(&imported);
        let physics = match Blocks::load(&exports) {
            Ok(blocks) => {
                // Without entity types the player still moves; it just doesn't see
                // other entities (an export from before they were added, say).
                let entity_types = EntityTypes::load(&exports).unwrap_or_else(|e| {
                    eprintln!(
                        "[ruthenium] no entity types ({e}); run `ruthenium prepare`. Other entities won't push the player"
                    );
                    EntityTypes::default()
                });
                // Without items nothing in the hands can be used.
                let reports = ruthenium_prepare::datagen::reports_dir(&imported);
                let items = Items::load(&exports, &reports).unwrap_or_else(|e| {
                    eprintln!(
                        "[ruthenium] no item data ({e}); run `ruthenium prepare`. Items can't be used"
                    );
                    Items::default()
                });
                Some(Physics {
                    blocks: Arc::new(blocks),
                    entity_types: Arc::new(entity_types),
                    dimension_types: dimension_types(&jar),
                    items: Arc::new(items),
                    block_transformers: Arc::new(block_transformers(&jar)),
                    enchantments: Arc::new(enchantments(&jar)),
                    biomes: biomes(&jar),
                    registries: Arc::new(local_registries(&jar)),
                })
            }
            Err(e) => {
                eprintln!("[ruthenium] no block physics ({e}); run `ruthenium prepare`");
                None
            }
        };
        let block_sounds = ruthenium_sim::sound::BlockSounds::load(&exports)
            .map(Arc::new)
            .map_err(|e| {
                eprintln!("[ruthenium] no block sounds ({e}); run `ruthenium prepare`");
            })
            .ok();
        GameData {
            lang,
            known_packs,
            physics,
            block_sounds,
        }
    }
}

/// Vanilla's definitions of what a level looks like, as the jar's data pack has them:
/// dimension types, timelines, world clocks, and each biome's attributes, effects and
/// climate (a biome's world generation is left out). A server that names these without sending
/// them means these.
fn local_registries(jar: &Zip) -> ruthenium_sim::environment::LocalRegistries {
    use ruthenium_sim::environment::{BIOME, DIMENSION_TYPE, TIMELINE, WORLD_CLOCK};
    let mut out = ruthenium_sim::environment::LocalRegistries::new();
    for (registry, dir) in [
        (DIMENSION_TYPE, "data/minecraft/dimension_type/"),
        (TIMELINE, "data/minecraft/timeline/"),
        (WORLD_CLOCK, "data/minecraft/world_clock/"),
        (BIOME, "data/minecraft/worldgen/biome/"),
    ] {
        let entries = out.entry(registry.to_owned()).or_default();
        for i in jar.with_prefix(dir) {
            let Some(name) = jar
                .name(i)
                .strip_prefix(dir)
                .and_then(|n| n.strip_suffix(".json"))
            else {
                continue;
            };
            let Some(mut v) = jar
                .read(i)
                .ok()
                .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
            else {
                continue;
            };
            if registry == BIOME
                && let Some(o) = v.as_object_mut()
            {
                o.retain(|k, _| {
                    matches!(
                        k.as_str(),
                        "attributes" | "effects" | "temperature" | "downfall" | "has_precipitation"
                    )
                });
            }
            entries.insert(format!("minecraft:{name}"), v);
        }
    }
    out
}

/// Vanilla's dimension types from the jar's data pack: heights and fast lava, by name.
fn dimension_types(jar: &Zip) -> HashMap<String, DimensionType> {
    const DIR: &str = "data/minecraft/dimension_type/";
    jar.with_prefix(DIR)
        .filter_map(|i| {
            let file = jar.name(i).strip_prefix(DIR)?.strip_suffix(".json")?;
            let v: serde_json::Value = serde_json::from_slice(&jar.read(i).ok()?).ok()?;
            let n = |k: &str| v.get(k)?.as_i64().and_then(|n| i32::try_from(n).ok());
            let fast_lava = v["attributes"][DimensionType::FAST_LAVA]
                .as_bool()
                .unwrap_or(false);
            let t = DimensionType {
                min_y: n("min_y")?,
                height: n("height")?,
                fast_lava,
                has_skylight: v.get("has_skylight")?.as_bool()?,
                has_ceiling: v.get("has_ceiling")?.as_bool()?,
            };
            Some((format!("minecraft:{file}"), t))
        })
        .collect()
}

/// Vanilla's biomes' climates from the jar's data pack, by name.
fn biomes(jar: &Zip) -> HashMap<String, ruthenium_sim::biome::Climate> {
    const DIR: &str = "data/minecraft/worldgen/biome/";
    jar.with_prefix(DIR)
        .filter_map(|i| {
            let file = jar.name(i).strip_prefix(DIR)?.strip_suffix(".json")?;
            let v: serde_json::Value = serde_json::from_slice(&jar.read(i).ok()?).ok()?;
            Some((
                format!("minecraft:{file}"),
                ruthenium_sim::biome::Climate::from_json(&v)?,
            ))
        })
        .collect()
}

/// Vanilla's block transformers from the jar's data pack (`data/minecraft/block_transformer`).
fn block_transformers(jar: &Zip) -> Transformers {
    const DIR: &str = "data/minecraft/block_transformer/";
    let files: Vec<(String, std::borrow::Cow<'_, [u8]>)> = jar
        .with_prefix(DIR)
        .filter_map(|i| Some((jar.name(i).to_owned(), jar.read(i).ok()?)))
        .collect();
    Transformers::from_json_files(files)
}

/// Vanilla's enchantments from the jar's data pack (`data/minecraft/enchantment`).
fn enchantments(jar: &Zip) -> Enchantments {
    const DIR: &str = "data/minecraft/enchantment/";
    let files: Vec<(String, std::borrow::Cow<'_, [u8]>)> = jar
        .with_prefix(DIR)
        .filter_map(|i| Some((jar.name(i).to_owned(), jar.read(i).ok()?)))
        .collect();
    Enchantments::from_json_files(files)
}

/// Where the game's own data pack has its block tags. The mesher reads two: `bars`
/// (which bars and panes leave out faces against each other) and `blocks_fluid_flow`
/// (what a fluid's flow, and with it the turn of its texture, takes for a wall).
const BLOCK_TAGS: &str = "data/minecraft/tags/block/";

/// Adds the blocks of a block tag to `out`, with those of the tags it names.
fn block_tag(jar: &Zip, name: &str, out: &mut std::collections::HashSet<String>, depth: u32) {
    // No tag of the game's is nested this deep: one that names itself ends here.
    if depth > 16 {
        return;
    }
    let Some(tag) = jar
        .find(&format!("{BLOCK_TAGS}{name}.json"))
        .and_then(|i| jar.read(i).ok())
        .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
    else {
        return;
    };
    let values = tag.get("values").and_then(|v| v.as_array());
    for v in values.into_iter().flatten() {
        // An entry is a name, or a name with a note that it may be missing.
        let Some(id) = v.as_str().or_else(|| v.get("id")?.as_str()) else {
            continue;
        };
        match id.strip_prefix('#') {
            Some(inner) => block_tag(
                jar,
                inner.strip_prefix("minecraft:").unwrap_or(inner),
                out,
                depth + 1,
            ),
            None => {
                out.insert(id.to_owned());
            }
        }
    }
}
/// The colour maps biomes take their grass and foliage colours from. The grass map's
/// default colour tints grass until biomes do.
const GRASS_MAP: &str = "textures/colormap/grass.png";
const FOLIAGE_MAP: &str = "textures/colormap/foliage.png";
const DRY_FOLIAGE_MAP: &str = "textures/colormap/dry_foliage.png";
/// The sky's pictures: the sun, the folder of the moon's phases, and the End's sky.
const SUN: &str = "textures/environment/celestial/sun.png";
const MOON_DIR: &str = "textures/environment/celestial/moon/";
const END_SKY: &str = "textures/environment/end_sky.png";
/// The clouds' cells, and the pictures of rain and snow.
const CLOUDS: &str = "textures/environment/clouds.png";
const RAIN: &str = "textures/environment/rain.png";
const SNOW: &str = "textures/environment/snow.png";
/// What darkens the corners of the frame in the dark.
const VIGNETTE: &str = "textures/misc/vignette.png";
/// What is before a player's eyes in water.
const UNDERWATER: &str = "textures/misc/underwater.png";

/// Builds what drawing a world needs from the imported game in `game`: the block atlas
/// with its mip levels, and every block state's baked model and drawing rules.
/// `max_texture` is the largest texture the GPU takes.
pub fn load_world_assets(game: &Path, max_texture: u32) -> Result<WorldAssets, String> {
    let started = std::time::Instant::now();
    let version = ruthenium_prepare::supported_version();
    let imported = ruthenium_prepare::import::open(version, game)
        .ok_or("the game files are not imported; run `ruthenium prepare`")?;
    // The vanilla pack the sound files and the menus read too.
    let resources = crate::game_files::vanilla(game)?;
    let def = ruthenium_assets::atlas::atlas_def("blocks").expect("a known atlas");
    let options = AtlasOptions {
        max_size: max_texture,
        ..AtlasOptions::default()
    };
    let mut atlas = ruthenium_assets::atlas::build(&resources, def, &options)
        .map_err(|e| format!("the block atlas: {e}"))?;
    let stitched = started.elapsed();
    let models = BlockModels::bake(&*resources, &atlas);
    for w in &models.warnings {
        eprintln!("[ruthenium] block models: {w}");
    }
    // The pixel the game reads for a temperature of 0.5 and a downfall of 1
    // (`GrassColor.getDefaultColor`); its own stand-in where the map is missing.
    // Each picture as large as the GPU takes at most (and as its pack's limits allow).
    let colour_map = |file: &str| {
        resources
            .get(&Id::minecraft(file).expect("a valid id"))
            .and_then(|r| r.image(max_texture).ok())
    };
    let grass_map = colour_map(GRASS_MAP);
    let grass_default = grass_map
        .as_ref()
        .filter(|map| map.width == 256 && map.height == 256)
        .map_or(0xFFFF_00FF, |map| map.get(127, 127));
    // What is held: every item definition's models, with the atlas of the items' own
    // pictures.
    let items_started = std::time::Instant::now();
    let items = ruthenium_assets::atlas::atlas_def("items")
        .and_then(|def| {
            ruthenium_assets::atlas::build(&resources, def, &options)
                .inspect_err(|e| eprintln!("[ruthenium] no item atlas: {e}"))
                .ok()
        })
        .map(|item_atlas| {
            let models = ItemModels::bake(&resources, &atlas, &item_atlas, grass_map.as_ref());
            for w in &models.warnings {
                eprintln!("[ruthenium] item models: {w}");
            }
            let (meshes, quads) = models.counts();
            eprintln!(
                "[ruthenium] {} item definitions with {meshes} models of {quads} quads, item atlas {}x{}, in {:.0?}",
                models.len(),
                item_atlas.width,
                item_atlas.height,
                items_started.elapsed(),
            );
            block_entity::ItemAssets {
                models,
                atlas: SkyImage {
                    width: item_atlas.width,
                    height: item_atlas.height,
                    rgba: item_atlas.levels[0].to_rgba8(),
                },
            }
        });
    // The sun, the moon and the End's sky, as bytes of red, green, blue and alpha.
    let sky_image = |file: &str| {
        colour_map(file).map(|image| SkyImage {
            width: image.width,
            height: image.height,
            rgba: image
                .pixels
                .iter()
                .flat_map(|p| [(p >> 16) as u8, (p >> 8) as u8, *p as u8, (p >> 24) as u8])
                .collect(),
        })
    };
    let sky = SkyImages {
        sun: sky_image(SUN),
        moon: MOON_PHASES.map(|phase| sky_image(&format!("{MOON_DIR}{phase}.png"))),
        end_sky: sky_image(END_SKY),
    };
    let underwater = sky_image(UNDERWATER);
    let pixels = |map: Option<Image>| map.map_or(Vec::new(), |m| m.pixels);
    let colour_maps = ColourMaps {
        grass: pixels(grass_map),
        foliage: pixels(colour_map(FOLIAGE_MAP)),
        dry_foliage: pixels(colour_map(DRY_FOLIAGE_MAP)),
    };
    let jar = Zip::open(&imported.client_jar).ok();
    let tags: Vec<(&str, std::collections::HashSet<String>)> = ["bars", "blocks_fluid_flow"]
        .into_iter()
        .map(|name| {
            let mut blocks = std::collections::HashSet::new();
            if let Some(jar) = &jar {
                block_tag(jar, name, &mut blocks, 0);
            }
            (name, blocks)
        })
        .collect();
    let exports = ruthenium_prepare::exporters::exports_dir(&imported);
    let mut block_entities = block_entity_assets(&exports, &resources, &sky_image)
        .inspect_err(|e| eprintln!("[ruthenium] no block entities: {e}; run `ruthenium prepare`"))
        .ok();
    if let Some(b) = &mut block_entities {
        b.items = items;
    }
    let fluids = ruthenium_render::terrain::FluidModels::from_atlas(&atlas);
    let table = BlockTable::load(&exports, models, fluids, grass_default, &|tag, block| {
        tags.iter()
            .any(|(name, blocks)| *name == tag && blocks.contains(block))
    })
    .map_err(|e| format!("{e}; run `ruthenium prepare`"))?;
    let animations = AtlasAnimations::take(&mut atlas);
    let atlas_levels: Vec<Vec<u8>> = atlas.levels.iter().map(Image::to_rgba8).collect();
    eprintln!(
        "[ruthenium] world assets in {:.0?}: block atlas {}x{} with {} levels ({:.1} MiB, {:.0?}), {} of {} block states drawn from {} model parts",
        started.elapsed(),
        atlas.width,
        atlas.height,
        atlas_levels.len(),
        atlas_levels.iter().map(Vec::len).sum::<usize>() as f64 / f64::from(1 << 20),
        stitched,
        table.drawn_states(),
        ruthenium_data::v26_3::blocks::STATE_COUNT,
        table.part_count(),
    );
    // The GUI's sprites, which the HUD is made of.
    let gui = ruthenium_assets::atlas::atlas_def("gui").and_then(|def| {
        ruthenium_assets::atlas::build(&resources, def, &options)
            .inspect_err(|e| eprintln!("[ruthenium] no GUI atlas, so no HUD: {e}"))
            .ok()
    });
    // The cracks in a block being broken, a picture for each stage.
    let cracks = (0..ruthenium_render::CRACK_STAGES)
        .map(|stage| {
            sky_image(&format!("textures/block/destroy_stage_{stage}.png")).unwrap_or(SkyImage {
                width: 0,
                height: 0,
                rgba: Vec::new(),
            })
        })
        .collect();
    Ok(WorldAssets {
        table: Arc::new(table),
        atlas_width: atlas.width,
        atlas_height: atlas.height,
        atlas_levels,
        animations,
        underwater,
        colour_maps,
        sky,
        clouds: sky_image(CLOUDS),
        rain: sky_image(RAIN),
        snow: sky_image(SNOW),
        block_entities,
        cracks,
        gui,
        vignette: sky_image(VIGNETTE),
        containers: ruthenium_ui::hud::BACKGROUNDS
            .iter()
            .filter_map(|n| {
                let image = sky_image(&format!("textures/gui/container/{n}.png"))?;
                Some(((*n).to_owned(), image))
            })
            .collect(),
    })
}

/// The models block entities are drawn from, and their pictures (read by `image`, from a
/// file's path in the pack).
fn block_entity_assets(
    exports: &Path,
    resources: &Resources,
    image: &dyn Fn(&str) -> Option<SkyImage>,
) -> Result<BlockEntityAssets, String> {
    let file = exports.join(ruthenium_prepare::exporters::MODEL_LAYERS);
    let bytes = std::fs::read(&file).map_err(|e| format!("{}: {e}", file.display()))?;
    let models = ModelSet::new(&ruthenium_prepare::exporters::read_model_layers(&bytes)?)?;
    let mut paths = block_entity::texture_paths();
    for folder in [block_entity::BANNER_DIR, block_entity::SHIELD_DIR] {
        let dir = format!("textures/entity/{folder}");
        paths.extend(
            resources
                .list(&dir, |p| p.ends_with(".png"))
                .keys()
                .filter(|id| id.namespace() == "minecraft")
                .filter_map(|id| {
                    let path = id.path().strip_prefix("textures/entity/")?;
                    Some(path.strip_suffix(".png")?.to_owned())
                }),
        );
    }
    let textures = paths
        .into_iter()
        .filter_map(|path| {
            let picture = image(&format!("textures/entity/{path}.png"))?;
            Some((path, picture))
        })
        .collect();
    // What is worn: every equipment asset, the pictures of its layers, and the trims with
    // their palettes.
    let listed = |dir: &str, suffix: &str| -> Vec<(Id, String)> {
        resources
            .list(dir, |p| p.ends_with(suffix))
            .keys()
            .filter(|id| id.namespace() == "minecraft")
            .filter_map(|id| {
                let below = id.path().strip_prefix(dir)?.strip_prefix('/')?;
                Some((id.clone(), below.strip_suffix(suffix)?.to_owned()))
            })
            .collect()
    };
    let json = |id: &Id| -> Option<serde_json::Value> { resources.get(id)?.json().ok() };
    let equipment_files: Vec<(String, serde_json::Value)> = listed("equipment", ".json")
        .into_iter()
        .filter_map(|(id, name)| Some((name, json(&id)?)))
        .collect();
    let equipment = EquipmentSet::new(equipment_files.iter().map(|(n, v)| (n.as_str(), v)));
    let mut textures: Vec<(String, SkyImage)> = textures;
    for dir in [
        equipment::HUMANOID_DIR,
        equipment::LEGGINGS_DIR,
        equipment::WINGS_DIR,
    ] {
        for (_, name) in listed(&format!("textures/entity/{dir}"), ".png") {
            let path = format!("{dir}/{name}");
            if let Some(picture) = image(&format!("textures/entity/{path}.png")) {
                textures.push((path, picture));
            }
        }
    }
    let mut trims = Vec::new();
    for kind in ["humanoid", "humanoid_leggings"] {
        let dir = format!("textures/trims/entity/{kind}");
        for (id, name) in listed(&dir, ".png") {
            let Some(picture) = image(id.path()) else {
                continue;
            };
            // The palette its colours are of, from its metadata.
            let palette = Id::minecraft(&format!("{}.mcmeta", id.path()))
                .and_then(|meta| json(&meta))
                .and_then(|m| {
                    let id = m.get("palette")?.get("base_palette")?.as_str()?.to_owned();
                    Some(id.strip_prefix("minecraft:").unwrap_or(&id).to_owned())
                });
            trims.push((format!("trims/entity/{kind}/{name}"), picture, palette));
        }
    }
    let palettes = listed("textures/palettes", ".png")
        .into_iter()
        .filter_map(|(id, name)| Some((name, image(id.path())?)))
        .collect();
    let font = Font::load(resources, &Id::minecraft("default").expect("a valid id"));
    for w in &font.warnings {
        eprintln!("[ruthenium] {w}");
    }
    let lang = resources
        .get(&Id::minecraft(LANG_PATH).expect("a valid id"))
        .and_then(|r| ruthenium_assets::lang::read(&r).ok())
        .unwrap_or_default();
    Ok(BlockEntityAssets {
        models,
        textures,
        font,
        lang,
        shadow: image("textures/misc/shadow.png"),
        equipment,
        trims,
        palettes,
        glint: [
            image("textures/misc/enchanted_glint_armor.png"),
            image("textures/misc/enchanted_glint_item.png"),
        ],
        items: None,
    })
}

/// The world's drawing assets on their way: built on a thread from the first join on.
#[derive(Debug, Default)]
enum WorldLoad {
    #[default]
    NotStarted,
    Loading(JoinHandle<Result<WorldAssets, String>>),
    /// Handed over, or failed (said once).
    Done,
}

/// Starts joining servers: loads the game data on a thread meanwhile.
#[derive(Debug)]
pub struct Joiner {
    data: Data,
    game: Option<PathBuf>,
    world: WorldLoad,
    skins: Option<SkinLoader>,
    /// For the services' discovery document (online logins) and the skins.
    cache_dir: Option<PathBuf>,
    /// Where the sessions' sounds go.
    sound: ruthenium_audio::SoundHandle,
    /// The signed-in profile's chat key pair, with the access token it is fetched with
    /// (kept across sessions, as vanilla keeps it for the game's run).
    #[cfg(not(feature = "dev-offline"))]
    profile_keys: Option<(String, Arc<ProfileKeySource>)>,
}

#[derive(Debug)]
enum Data {
    Loading(JoinHandle<GameData>),
    Ready(Arc<GameData>),
}

impl Joiner {
    /// `game`: the imported game files; `cache_dir`: for the services' discovery document
    /// and the skins.
    pub fn new(game: Option<PathBuf>, cache_dir: Option<PathBuf>) -> Joiner {
        let data = match game.clone() {
            Some(dir) => match std::thread::Builder::new()
                .name("game-data".into())
                .spawn(move || GameData::load(&dir))
            {
                Ok(h) => Data::Loading(h),
                Err(_) => Data::Ready(Arc::default()),
            },
            None => Data::Ready(Arc::default()),
        };
        Joiner {
            data,
            game,
            world: WorldLoad::NotStarted,
            skins: None,
            cache_dir,
            sound: ruthenium_audio::SoundHandle::off(),
            #[cfg(not(feature = "dev-offline"))]
            profile_keys: None,
        }
    }

    /// The game data, if it has loaded (without waiting for it).
    pub fn data_if_ready(&mut self) -> Option<Arc<GameData>> {
        if matches!(&self.data, Data::Loading(h) if !h.is_finished()) {
            return None;
        }
        Some(self.data())
    }

    /// Where the sessions it starts play their sounds.
    pub fn set_sound(&mut self, sound: ruthenium_audio::SoundHandle) {
        self.sound = sound;
    }

    /// Starts building the world's drawing assets on a thread, the first time it is
    /// called (when a world is first entered).
    pub fn start_world_assets(&mut self, max_texture: u32) {
        if !matches!(self.world, WorldLoad::NotStarted) {
            return;
        }
        self.world = match self.game.clone().map(|game| {
            std::thread::Builder::new()
                .name("world-assets".into())
                .spawn(move || load_world_assets(&game, max_texture))
        }) {
            Some(Ok(thread)) => WorldLoad::Loading(thread),
            _ => {
                eprintln!(
                    "[ruthenium] the world is not drawn: there are no game files to draw it from"
                );
                WorldLoad::Done
            }
        };
    }

    /// What loads the skins of players, once a world is entered.
    pub fn skins(&mut self) -> &SkinLoader {
        self.skins
            .get_or_insert_with(|| SkinLoader::new(self.game.clone(), self.cache_dir.clone()))
    }

    /// The world's drawing assets once they are built: handed over once.
    pub fn take_world_assets(&mut self) -> Option<WorldAssets> {
        if !matches!(&self.world, WorldLoad::Loading(t) if t.is_finished()) {
            return None;
        }
        let WorldLoad::Loading(thread) = std::mem::replace(&mut self.world, WorldLoad::Done) else {
            unreachable!()
        };
        match thread.join() {
            Ok(Ok(assets)) => Some(assets),
            Ok(Err(e)) => {
                eprintln!("[ruthenium] the world is not drawn: {e}");
                None
            }
            Err(_) => {
                eprintln!("[ruthenium] the world is not drawn: its assets' thread panicked");
                None
            }
        }
    }

    /// The game data, waiting for it if it is still loading.
    pub fn data(&mut self) -> Arc<GameData> {
        if let Data::Loading(_) = &self.data {
            let Data::Loading(h) = std::mem::replace(&mut self.data, Data::Ready(Arc::default()))
            else {
                unreachable!()
            };
            self.data = Data::Ready(Arc::new(h.join().unwrap_or_default()));
        }
        match &self.data {
            Data::Ready(d) => d.clone(),
            Data::Loading(_) => unreachable!(),
        }
    }

    /// Starts a session with the server at `address` (as typed), or says why it can't.
    pub fn join(&mut self, address: &str, accounts: &Accounts) -> Result<Session, String> {
        let (identity, auth, local_only) = self.who(accounts)?;
        let profile_keys = self.profile_keys(accounts);
        let data = self.data();
        Session::start(SessionOptions {
            connect: ConnectOptions {
                address: address.to_owned(),
                identity,
                auth,
                brand: BRAND.into(),
                information: ClientInformation::default(),
                local_only,
            },
            known_packs: data.known_packs.clone(),
            physics: data.physics.clone(),
            sound: ruthenium_sim::sound::GameSounds::new(
                self.sound.clone(),
                data.block_sounds.clone(),
            ),
            profile_keys,
        })
        .map_err(|e| format!("couldn't start the connection: {e}"))
    }

    /// Development builds sign no chat.
    #[cfg(feature = "dev-offline")]
    fn profile_keys(
        &mut self,
        _: &Accounts,
    ) -> Option<Arc<dyn ruthenium_sim::chat::ProfileKeySource>> {
        None
    }

    /// The signed-in account's chat key pair (`Minecraft.getProfileKeyPairManager`): the
    /// same while the account's access token is.
    #[cfg(not(feature = "dev-offline"))]
    fn profile_keys(
        &mut self,
        accounts: &Accounts,
    ) -> Option<Arc<dyn ruthenium_sim::chat::ProfileKeySource>> {
        let session = accounts.session()?;
        if !self
            .profile_keys
            .as_ref()
            .is_some_and(|(token, _)| *token == session.access_token)
        {
            let cache = self.cache_dir.as_deref().map(|d| d.join("discovery.json"));
            let signed_in = session.clone();
            let fetch = Arc::new(move || {
                let mut auth = ruthenium_auth::Auth::new();
                if let Some(c) = &cache {
                    auth = auth.with_discovery_cache(c);
                }
                auth.certificates(&signed_in)
            });
            let keys = ProfileKeySource(ruthenium_auth::ProfileKeys::new(fetch));
            self.profile_keys = Some((session.access_token.clone(), Arc::new(keys)));
        }
        let keys: Arc<dyn ruthenium_sim::chat::ProfileKeySource> =
            self.profile_keys.as_ref()?.1.clone();
        Some(keys)
    }

    /// Development builds join as an offline player, only on this machine or the local
    /// network: `RUTHENIUM_OFFLINE_NAME`, else the account's name, else "Developer".
    #[cfg(feature = "dev-offline")]
    fn who(&self, accounts: &Accounts) -> Result<(Identity, Auth, bool), String> {
        let name = std::env::var("RUTHENIUM_OFFLINE_NAME")
            .ok()
            .or_else(|| accounts.session().map(|s| s.profile.name.clone()))
            .or_else(|| accounts.list().first().map(|a| a.name.clone()))
            .unwrap_or_else(|| "Developer".into());
        if !valid_name(&name) {
            return Err(format!("{name:?} isn't a valid player name"));
        }
        eprintln!("[ruthenium] {DEV_OFFLINE_MARKER}: joining offline as {name}");
        let id = ruthenium_net::crypt::offline_uuid(&name);
        Ok((Identity { name, id }, Auth::Offline, true))
    }

    /// Release builds join as the signed-in account, with its session.
    #[cfg(not(feature = "dev-offline"))]
    fn who(&self, accounts: &Accounts) -> Result<(Identity, Auth, bool), String> {
        let Some(session) = accounts.session() else {
            return Err(match accounts.status() {
                crate::account::AccountStatus::Checking(_) => {
                    "Still signing in, try again in a moment".into()
                }
                crate::account::AccountStatus::Failed { reason, .. } => {
                    format!("Not signed in: {reason}")
                }
                _ => "Sign in with a Microsoft account to play on servers".into(),
            });
        };
        let id = u128::from_str_radix(&session.profile.id, 16)
            .map_err(|_| "the account's profile id isn't a UUID".to_owned())?;
        let join = ruthenium_auth::ServerJoin::new(
            session,
            self.cache_dir
                .as_deref()
                .map(|d| d.join("discovery.json"))
                .as_deref(),
        );
        Ok((
            Identity {
                name: session.profile.name.clone(),
                id,
            },
            Auth::Online(Arc::new(Join(join))),
            false,
        ))
    }
}

/// A name a server accepts (`StringUtil.isValidPlayerName`): at most 16 characters,
/// printable ASCII without spaces.
#[cfg(feature = "dev-offline")]
fn valid_name(name: &str) -> bool {
    !name.is_empty() && name.len() <= 16 && name.bytes().all(|b| (33..127).contains(&b))
}

#[cfg(not(feature = "dev-offline"))]
struct Join(ruthenium_auth::ServerJoin);

/// The account's key pairs, as sessions take them.
#[cfg(not(feature = "dev-offline"))]
#[derive(Debug)]
struct ProfileKeySource(ruthenium_auth::ProfileKeys);

#[cfg(not(feature = "dev-offline"))]
impl ruthenium_sim::chat::ProfileKeySource for ProfileKeySource {
    fn prepare(&self) {
        self.0.prepare();
    }

    fn poll(&self) -> ruthenium_sim::chat::KeyPoll {
        use ruthenium_sim::chat::{KeyPoll, ProfileKeys};
        match self.0.poll() {
            None => KeyPoll::Pending,
            Some(pair) => KeyPoll::Ready(pair.map(|p| {
                Arc::new(ProfileKeys {
                    private_key: p.private_key.clone(),
                    public_key: p.public_key.clone(),
                    key_signature: p.key_signature.clone(),
                    expires_at: p.expires_at,
                    refresh_after: p.refresh_after,
                })
            })),
        }
    }

    fn should_refresh(&self) -> bool {
        self.0.should_refresh()
    }
}

#[cfg(not(feature = "dev-offline"))]
impl ruthenium_net::connection::SessionJoin for Join {
    fn join(&self, server_hash: &str) -> Result<(), String> {
        self.0.join(server_hash).map_err(|e| e.to_string())
    }
}

/// The connecting screen's line for `stage` (vanilla's `connect.*`).
pub fn stage_text(stage: Stage) -> &'static str {
    match stage {
        Stage::Connecting => "Connecting to the server...",
        Stage::Authorizing => "Logging in...",
        Stage::Encrypting => "Encrypting...",
        Stage::Negotiating => "Negotiating...",
        Stage::Joining => "Joining world...",
    }
}

/// The disconnected screen's title and reason.
pub fn closed_text(c: &Closed, lang: &HashMap<String, String>) -> (String, String) {
    let title = if c.during_login {
        "Couldn't connect to the server"
    } else {
        "Connection lost"
    };
    let reason = match &c.reason {
        // Vanilla reads it leniently; what doesn't parse shows as it is.
        Reason::Json(j) => match serde_json::from_str::<serde_json::Value>(j) {
            Ok(v) => StyledText::from_json_in(&v, lang).plain(),
            Err(_) => j.clone(),
        },
        Reason::Text(t) => StyledText::from_nbt_in(t, lang).plain(),
        Reason::Local(l) => l.to_string(),
    };
    (title.into(), reason)
}

/// A text component from the server (a death message, say) as plain text, translated
/// with the game's `en_us.json`.
pub fn plain_text(t: &ruthenium_protocol::nbt::Tag, lang: &HashMap<String, String>) -> String {
    StyledText::from_nbt_in(t, lang).plain()
}

/// Whether the session ended because the player left (no screen to show).
pub fn left_on_purpose(c: &Closed) -> bool {
    c.reason == Reason::Local(Local::Closed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ruthenium_protocol::nbt::Tag;

    #[test]
    fn reasons_read_as_text() {
        let mut lang = HashMap::new();
        lang.insert(
            "multiplayer.disconnect.not_whitelisted".to_owned(),
            "You are not white-listed on this server!".to_owned(),
        );
        let c = Closed {
            during_login: true,
            reason: Reason::Json(
                r#"{"translate":"multiplayer.disconnect.not_whitelisted"}"#.into(),
            ),
        };
        assert_eq!(
            closed_text(&c, &lang),
            (
                "Couldn't connect to the server".into(),
                "You are not white-listed on this server!".into()
            )
        );
        let c = Closed {
            during_login: false,
            reason: Reason::Text(Tag::String("Server closed".into())),
        };
        assert_eq!(closed_text(&c, &lang).1, "Server closed");
        assert!(left_on_purpose(&Closed {
            during_login: false,
            reason: Reason::Local(Local::Closed)
        }));
    }
}

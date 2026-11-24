//! A small made-up game for the terrain tests: a handful of block states with models,
//! face shapes and rules of the kinds the real game has, and a level to put them in.
//! Nothing here comes from the game's files.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use ruthenium_assets::atlas::{ATLASES, Atlas, Sprite, missing_id};
use ruthenium_assets::image::Transparency;
use ruthenium_assets::model::ModelFiles;
use ruthenium_assets::{BlockModels, Id, Image};
use ruthenium_data::v26_3::Block;
use ruthenium_data::v26_3::blocks::STATE_COUNT;
use ruthenium_prepare::exporters::{
    BlockBehaviours, BlockPhysics, BlockRender, OffsetData, ShapeData, SkipRule, StateBehaviour,
    StatePhysics, StateRender, TintLayer, WorldTint,
};
use ruthenium_protocol::packets::play::{LIGHT_BYTES, LightData};
use ruthenium_sim::scene::{Dims, Neighbourhood, Scene, ScenePublisher, SectionKey};
use ruthenium_sim::world::Level;

use super::table::BlockTable;
use super::table::tests::boxed;

/// The tint the made-up grass has: 0xAARRGGBB.
pub(crate) const GRASS_TINT: u32 = 0xFF40_8020;

/// A state of a real block name, by property values.
pub(crate) fn state(name: &str, values: &[(&str, &str)]) -> u32 {
    let info = Block::from_name(name).unwrap().info();
    values.iter().fold(info.default_state, |s, (p, v)| {
        info.with_value(s, p, v).unwrap()
    })
}

pub(crate) fn stone() -> u32 {
    state("minecraft:stone", &[])
}

/// A cube with holes in its texture that hides nothing and leaves out faces against
/// itself.
pub(crate) fn glass() -> u32 {
    state("minecraft:glass", &[])
}

/// A cube whose texture is partly transparent.
pub(crate) fn ice() -> u32 {
    state("minecraft:ice", &[])
}

pub(crate) fn slab() -> u32 {
    state("minecraft:oak_slab", &[("type", "bottom")])
}

/// Two crossed planes, tinted, moved about by its position.
pub(crate) fn grass() -> u32 {
    state("minecraft:short_grass", &[])
}

/// A cube that gives off light 15.
pub(crate) fn glowstone() -> u32 {
    state("minecraft:glowstone", &[])
}

/// A cube lit fully whatever the light.
pub(crate) fn magma() -> u32 {
    state("minecraft:magma_block", &[])
}

/// A 64 x 16 atlas of four 16 x 16 sprites: the missing one, `a` (opaque), `holes`
/// (its left half fully transparent) and `ice` (half transparent everywhere).
fn atlas() -> Atlas {
    let mut level = Image::new(64, 16);
    for y in 0..16 {
        for x in 0..64 {
            let alpha = match x {
                32..40 => 0,
                48.. => 128,
                _ => 255,
            };
            level.set(x, y, alpha << 24 | 0x80_8080);
        }
    }
    let sprite = |x: u32, transparent, translucent| Sprite {
        x,
        y: 0,
        padding: 0,
        width: 16,
        height: 16,
        transparency: Transparency {
            transparent,
            translucent,
        },
        blur: false,
        clamp: false,
        gui: None,
        animated: None,
    };
    let id = |path: &str| Id::minecraft(path).unwrap();
    Atlas {
        def: ATLASES[1],
        width: 64,
        height: 16,
        max_level: 0,
        levels: vec![level],
        sprites: BTreeMap::from([
            (missing_id(), sprite(0, false, false)),
            (id("block/a"), sprite(16, false, false)),
            (id("block/holes"), sprite(32, true, false)),
            (id("block/ice"), sprite(48, false, true)),
        ]),
        animated: Vec::new(),
        warnings: Vec::new(),
    }
}

#[derive(Default)]
struct Files {
    models: HashMap<String, String>,
    states: HashMap<String, Vec<String>>,
}

impl ModelFiles for Files {
    fn model(&self, id: &Id) -> Option<String> {
        self.models.get(&id.to_string()).cloned()
    }

    fn blockstates(&self, block: &Id) -> Vec<String> {
        self.states
            .get(&block.to_string())
            .cloned()
            .unwrap_or_default()
    }
}

fn cube(texture: &str) -> String {
    let faces: Vec<String> = ["down", "up", "north", "south", "west", "east"]
        .iter()
        .map(|f| format!(r##""{f}": {{"texture": "#t", "cullface": "{f}"}}"##))
        .collect();
    format!(
        r##"{{"textures": {{"t": "block/{texture}"}}, "elements": [{{"from": [0, 0, 0],
        "to": [16, 16, 16], "faces": {{{}}}}}]}}"##,
        faces.join(", ")
    )
}

const SLAB: &str = r##"{"textures": {"t": "block/a"}, "elements": [{"from": [0, 0, 0],
    "to": [16, 8, 16], "faces": {
    "down": {"texture": "#t", "cullface": "down"}, "up": {"texture": "#t"},
    "north": {"texture": "#t", "cullface": "north"},
    "south": {"texture": "#t", "cullface": "south"},
    "west": {"texture": "#t", "cullface": "west"},
    "east": {"texture": "#t", "cullface": "east"}}}]}"##;

const CROSS: &str = r##"{"ambientocclusion": false, "textures": {"t": "block/holes"},
    "elements": [
    {"from": [0.8, 0, 8], "to": [15.2, 16, 8],
     "rotation": {"origin": [8, 8, 8], "axis": "y", "angle": 45, "rescale": true},
     "faces": {"north": {"texture": "#t", "tintindex": 0},
               "south": {"texture": "#t", "tintindex": 0}}},
    {"from": [8, 0, 0.8], "to": [8, 16, 15.2],
     "rotation": {"origin": [8, 8, 8], "axis": "y", "angle": 45, "rescale": true},
     "faces": {"west": {"texture": "#t", "tintindex": 0},
               "east": {"texture": "#t", "tintindex": 0}}}]}"##;

/// The made-up game's block table.
pub(crate) fn table() -> Arc<BlockTable> {
    let mut files = Files::default();
    let mut block = |name: &str, model: &str, json: String| {
        files
            .models
            .insert(format!("minecraft:block/{model}"), json);
        files.states.insert(
            format!("minecraft:{name}"),
            vec![format!(
                r#"{{"variants": {{"": {{"model": "block/{model}"}}}}}}"#
            )],
        );
    };
    block("stone", "cube_a", cube("a"));
    block("glowstone", "cube_a", cube("a"));
    block("magma_block", "cube_a", cube("a"));
    block("glass", "cube_holes", cube("holes"));
    block("ice", "cube_ice", cube("ice"));
    block("oak_slab", "slab", SLAB.into());
    block("short_grass", "cross", CROSS.into());
    let atlas = atlas();
    let models = BlockModels::bake(&files, &atlas);

    let plain = StatePhysics {
        shape: 0,
        flags: 0,
        friction: 0.6,
        speed_factor: 1.0,
        jump_factor: 1.0,
        bounce: 0.0,
        fluid: 0,
        sturdy: 0,
        offset: None,
        outline: 0,
        interaction_shape: 0,
        interaction: 0,
        destroy_time: 0.0,
        light_dampening: 0,
        light_emission: 0,
        light_shape: false,
        occlusion: [0; 6],
    };
    let mut physics = BlockPhysics {
        shapes: vec![ShapeData {
            coords: [vec![0.0], vec![0.0], vec![0.0]],
            full: vec![],
        }],
        states: vec![plain; STATE_COUNT],
    };
    let mut behaviour = BlockBehaviours {
        instruments: Vec::new(),
        behaviours: Vec::new(),
        blocks: Vec::new(),
        states: vec![StateBehaviour::default(); STATE_COUNT],
    };
    // Face shapes: nothing, the whole face, and the lower half of a side.
    let mut render = BlockRender {
        shapes: vec![
            physics.shapes[0].clone(),
            boxed([0.0; 3], [1.0; 3]),
            boxed([0.0; 3], [1.0, 0.5, 1.0]),
        ],
        states: vec![
            StateRender {
                flags: 0,
                skip: SkipRule::None,
                occlusion: [0; 6],
                tints: Vec::new(),
                seed_offset: [0; 3],
            };
            STATE_COUNT
        ],
    };
    let full = StateBehaviour::COLLISION_FULL_BLOCK;
    for (id, occlusion, collision_full) in [
        (stone(), [1; 6], true),
        (glowstone(), [1; 6], true),
        (magma(), [1; 6], true),
        (glass(), [0; 6], true),
        (ice(), [1; 6], true),
        (slab(), [1, 0, 2, 2, 2, 2], false),
        (grass(), [0; 6], false),
    ] {
        let r = &mut render.states[id as usize];
        r.flags = StateRender::MODEL;
        r.occlusion = occlusion;
        if collision_full {
            behaviour.states[id as usize].flags |= full;
        }
    }
    for id in [glass(), ice()] {
        render.states[id as usize].skip = SkipRule::SameBlock;
    }
    physics.states[glowstone() as usize].light_emission = 15;
    render.states[magma() as usize].flags |= StateRender::EMISSIVE;
    render.states[grass() as usize].tints = vec![TintLayer {
        hand: None,
        world: WorldTint::Grass,
        dy: 0,
    }];
    physics.states[grass() as usize].offset = Some(OffsetData {
        vertical: false,
        max_horizontal: 0.25,
        max_vertical: 0.2,
        collision: false,
        outline: true,
    });
    Arc::new(
        BlockTable::new(&physics, &behaviour, &render, models, GRASS_TINT, &|_| {
            false
        })
        .unwrap(),
    )
}

/// A level four sections high (y -16 to 47; sections -1 to 2) and what it publishes.
pub(crate) struct World {
    pub level: Level,
    pub publisher: ScenePublisher,
    pub scene: Scene,
}

impl World {
    /// With the chunks `chunks` loaded, all air and with light 15 from the sky and none
    /// from blocks.
    pub fn new(chunks: &[(i32, i32)]) -> World {
        let mut level = Level::new(-16, 64);
        level.track_changes(4096);
        let (mut publisher, scene) = Scene::new(4096);
        publisher.begin_level(Dims::of(&level));
        let mut world = World {
            level,
            publisher,
            scene,
        };
        for &(x, z) in chunks {
            world.load(x, z);
        }
        world
    }

    pub fn load(&mut self, x: i32, z: i32) {
        self.level.add_empty_chunk(x, z);
        for section in -1..3 {
            self.light(x, section, z, 15, 0);
        }
    }

    pub fn set(&mut self, x: i32, y: i32, z: i32, state: u32) {
        self.level.set_block(x, y, z, state);
    }

    /// Gives every block of a section the same sky and block light, as the server
    /// would send it.
    pub fn light(&mut self, x: i32, section: i32, z: i32, sky: u8, block: u8) {
        // Light sections are counted from one below the level's lowest section.
        let bit = 1u64 << (section + 2);
        self.level.update_light(
            x,
            z,
            &LightData {
                sky_mask: vec![bit],
                block_mask: vec![bit],
                empty_sky_mask: Vec::new(),
                empty_block_mask: Vec::new(),
                sky: vec![Arc::new([sky | sky << 4; LIGHT_BYTES])],
                block: vec![Arc::new([block | block << 4; LIGHT_BYTES])],
            },
        );
    }

    pub fn publish(&mut self) {
        self.publisher.publish_level(&mut self.level);
    }

    /// Publishes, and pins a section.
    pub fn pin(&mut self, section: SectionKey) -> Neighbourhood {
        self.publish();
        self.scene
            .pin(section)
            .expect("the section's chunk is loaded")
    }
}

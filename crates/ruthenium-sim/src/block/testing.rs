//! A small level with the prepared game's blocks, tags and items, for the ports'
//! tests. Without `RUTHENIUM_GAME_DIR` (the prepared game folder) there is none and
//! the tests that need it skip.

use std::path::PathBuf;
use std::sync::OnceLock;

use super::{Pos, state};
use crate::blocks::{BlockTags, Blocks};
use crate::direction::Direction;
use crate::entity::Entities;
use crate::interact::{GameMode, UseKeys, World};
use crate::inventory::Hand;
use crate::item::{ItemTags, Items, Stack};
use crate::pick::{self, BlockHit, HitResult};
use crate::player::LocalPlayer;
use crate::predict::Predictions;
use crate::world::Level;

struct Game {
    blocks: Blocks,
    tags: BlockTags,
    item_tags: ItemTags,
    items: Items,
}

fn game() -> Option<&'static Game> {
    static GAME: OnceLock<Option<Game>> = OnceLock::new();
    GAME.get_or_init(|| {
        let dir = PathBuf::from(std::env::var_os("RUTHENIUM_GAME_DIR")?);
        let exports = dir.join("versions/26.3/exports");
        Some(Game {
            blocks: Blocks::load(&exports).expect("block exports"),
            tags: jar_tags(&dir),
            item_tags: jar_item_tags(&dir),
            items: Items::load(&exports, &dir.join("versions/26.3/generated/reports"))
                .expect("item exports"),
        })
    })
    .as_ref()
}

/// Every tag of one registry in the jar's data pack (`block`, `item`), nested tags
/// expanded: (tag, members).
fn jar_tag_lists(game: &std::path::Path, registry: &str) -> Vec<(String, Vec<String>)> {
    let dir = format!("data/minecraft/tags/{registry}/");
    let jar = ruthenium_assets::zip::Zip::open(&game.join("versions/26.3/client.jar")).unwrap();
    fn expand(jar: &ruthenium_assets::zip::Zip, dir: &str, tag: &str, out: &mut Vec<String>) {
        let Some(i) = jar.find(&format!("{dir}{tag}.json")) else {
            return;
        };
        let v: serde_json::Value = serde_json::from_slice(&jar.read(i).unwrap()).unwrap();
        for n in v["values"].as_array().unwrap() {
            let n = n.as_str().or_else(|| n["id"].as_str()).unwrap();
            match n.strip_prefix("#minecraft:") {
                Some(nested) => expand(jar, dir, nested, out),
                None => out.push(n.to_owned()),
            }
        }
    }
    let names: Vec<String> = jar
        .with_prefix(&dir)
        .filter_map(|i| {
            Some(
                jar.name(i)
                    .strip_prefix(dir.as_str())?
                    .strip_suffix(".json")?
                    .to_owned(),
            )
        })
        .collect();
    names
        .into_iter()
        .map(|tag| {
            let mut members = Vec::new();
            expand(&jar, &dir, &tag, &mut members);
            (format!("minecraft:{tag}"), members)
        })
        .collect()
}

fn jar_tags(game: &std::path::Path) -> BlockTags {
    let mut tags = BlockTags::new();
    for (tag, members) in jar_tag_lists(game, "block") {
        tags.set(&tag, members.iter().map(String::as_str));
    }
    tags
}

fn jar_item_tags(game: &std::path::Path) -> ItemTags {
    let mut tags = ItemTags::new();
    for (tag, members) in jar_tag_lists(game, "item") {
        tags.set(&tag, members.iter().map(String::as_str));
    }
    tags
}

/// A level of air chunks from -32 to 31 on x and z, a player standing at
/// `player.position` (0.5, -60, 0.5 at first), and the client's prediction state.
#[allow(dead_code)]
pub(crate) struct Fixture {
    pub level: Level,
    pub predictions: Predictions,
    pub entities: Entities,
    pub fluid_shapes: pick::FluidShapes,
    pub player: LocalPlayer,
    pub game_mode: GameMode,
    game: &'static Game,
}

#[allow(dead_code)]
impl Fixture {
    /// `None` (skip the test) without the prepared game.
    pub fn new() -> Option<Fixture> {
        let game = game()?;
        let mut level = Level::new(-64, 384);
        for x in -2..2 {
            for z in -2..2 {
                level.add_empty_chunk(x, z);
            }
        }
        let mut player = LocalPlayer::new();
        player.position = [0.5, -60.0, 0.5];
        Some(Fixture {
            level,
            predictions: Predictions::new(),
            entities: Entities::new(0, false),
            fluid_shapes: pick::FluidShapes::default(),
            player,
            game_mode: GameMode::default(),
            game,
        })
    }

    pub fn blocks(&self) -> &'static Blocks {
        &self.game.blocks
    }

    pub fn tags(&self) -> &'static BlockTags {
        &self.game.tags
    }

    /// A state by its printed form (`minecraft:oak_stairs[facing=east]`); panics on a
    /// typo.
    pub fn state(text: &str) -> u32 {
        state::parse(text).unwrap_or_else(|| panic!("no state {text}"))
    }

    /// Sets a block (as the server would: no prediction, no shape updates).
    pub fn set(&mut self, pos: Pos, text: &str) {
        self.level
            .set_block(pos[0], pos[1], pos[2], Fixture::state(text));
    }

    /// The block at `pos`, printed.
    pub fn get(&self, pos: Pos) -> String {
        state::describe(self.level.block(pos[0], pos[1], pos[2]))
    }

    /// A stone floor at y = -61 from -8 to 8.
    pub fn floor(&mut self) {
        for x in -8..=8 {
            for z in -8..=8 {
                self.set([x, -61, z], "minecraft:stone");
            }
        }
    }

    /// A stack of an item by name.
    pub fn stack(&self, name: &str, count: i32) -> Stack {
        let id = ruthenium_data::v26_3::registries::Item::from_name(name)
            .unwrap_or_else(|| panic!("no item {name}"));
        self.game.items.new_stack(id.id() as i32, count)
    }

    /// Puts a stack in a hand.
    pub fn hold(&mut self, hand: Hand, name: &str, count: i32) {
        let s = self.stack(name, count);
        self.player.inventory.set_hand(hand, s);
    }

    /// The world the actions see.
    pub fn world(&mut self) -> World<'_> {
        World::new(
            &mut self.level,
            &mut self.predictions,
            &self.game.blocks,
            &self.game.tags,
            &self.entities,
            &self.fluid_shapes,
        )
        .with_items(&self.game.items)
        .with_item_tags(&self.game.item_tags)
    }

    /// One use click (`startUseItem`) with the crosshair on `face` of the block at
    /// `pos`, at `at` (the hit's location, in the block's own coordinates 0..1).
    /// Returns the packets it sent.
    pub fn click(&mut self, pos: Pos, face: Direction, at: [f64; 3]) -> Vec<Vec<u8>> {
        let hit = HitResult::Block(BlockHit {
            location: [
                f64::from(pos[0]) + at[0],
                f64::from(pos[1]) + at[1],
                f64::from(pos[2]) + at[2],
            ],
            direction: face,
            pos,
            inside: false,
            miss: false,
            world_border: false,
        });
        let mut out = Vec::new();
        let game = self.game;
        let mut w = World::new(
            &mut self.level,
            &mut self.predictions,
            &game.blocks,
            &game.tags,
            &self.entities,
            &self.fluid_shapes,
        )
        .with_items(&game.items)
        .with_item_tags(&game.item_tags);
        let mut keys = UseKeys {
            use_clicks: 1,
            ..UseKeys::default()
        };
        self.game_mode.right_click_delay = 0;
        self.game_mode
            .handle_keybinds(&mut self.player, &mut w, &hit, &mut keys, &mut out);
        out
    }

    /// One tick of the attack key held on `face` of the block at `pos` (its middle),
    /// with `clicks` presses of it this tick. Returns the packets it sent.
    pub fn attack_face(&mut self, pos: Pos, face: Direction, clicks: u32) -> Vec<Vec<u8>> {
        let n = face.normal();
        let hit = HitResult::Block(BlockHit {
            location: [
                f64::from(pos[0]) + 0.5 + f64::from(n[0]) * 0.5,
                f64::from(pos[1]) + 0.5 + f64::from(n[1]) * 0.5,
                f64::from(pos[2]) + 0.5 + f64::from(n[2]) * 0.5,
            ],
            direction: face,
            pos,
            inside: false,
            miss: false,
            world_border: false,
        });
        let mut out = Vec::new();
        let game = self.game;
        let mut w = World::new(
            &mut self.level,
            &mut self.predictions,
            &game.blocks,
            &game.tags,
            &self.entities,
            &self.fluid_shapes,
        )
        .with_items(&game.items)
        .with_item_tags(&game.item_tags);
        let mut keys = UseKeys {
            attack_down: true,
            attack_clicks: clicks,
            ..UseKeys::default()
        };
        self.game_mode
            .handle_keybinds(&mut self.player, &mut w, &hit, &mut keys, &mut out);
        out
    }

    /// Clicks the middle of a face.
    pub fn click_face(&mut self, pos: Pos, face: Direction) -> Vec<Vec<u8>> {
        let n = face.normal();
        let at = [
            0.5 + f64::from(n[0]) * 0.5,
            0.5 + f64::from(n[1]) * 0.5,
            0.5 + f64::from(n[2]) * 0.5,
        ];
        self.click(pos, face, at)
    }
}

#[test]
fn faces_of_shapes() {
    let Some(f) = Fixture::new() else {
        return;
    };
    let b = f.blocks();
    let slab = Fixture::state("minecraft:oak_slab[type=bottom]");
    assert!(b.collision_face_full(slab, Direction::Down));
    assert!(!b.collision_face_full(slab, Direction::Up));
    assert!(!b.collision_face_full(slab, Direction::North));
    let stone = Fixture::state("minecraft:stone");
    assert!(
        Direction::ALL
            .iter()
            .all(|&d| b.collision_face_full(stone, d))
    );
    let fence = Fixture::state("minecraft:oak_fence");
    assert!(!b.collision_face_full(fence, Direction::Up));
    let top = Fixture::state("minecraft:oak_stairs[half=top,facing=north]");
    assert!(b.collision_face_full(top, Direction::Up));
    assert!(!b.collision_face_full(top, Direction::Down));
}

#[test]
fn a_stone_block_goes_on_the_clicked_face_and_the_stack_shrinks() {
    let Some(mut f) = Fixture::new() else {
        return;
    };
    f.floor();
    f.player.position = [0.5, -60.0, 3.5];
    f.player.x_rot = 60.0;
    f.hold(Hand::Main, "minecraft:stone", 5);
    let sent = f.click_face([0, -61, 0], Direction::Up);
    assert_eq!(f.get([0, -60, 0]), "minecraft:stone");
    assert_eq!(f.player.hand(Hand::Main).count, 4);
    assert_eq!(f.predictions.pending(), 1);
    assert_eq!(sent.len(), 1, "one use_item_on");
    // Into the player: refused, nothing used.
    f.player.position = [0.9, -60.0, 0.5];
    f.click_face([1, -61, 0], Direction::Up);
    assert_eq!(f.get([1, -60, 0]), "minecraft:air");
    assert_eq!(f.player.hand(Hand::Main).count, 4);
}

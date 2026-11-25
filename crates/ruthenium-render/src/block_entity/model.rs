//! The models block entities are drawn from, made flat: one list of quads for all of them,
//! and per model its parts with their places in that list and in each other.

use ruthenium_prepare::exporters::{ModelFace, ModelLayers, ModelPart};

use super::affine::Affine;

/// A skull's kind (`SkullBlock.Types`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Skull {
    Skeleton,
    WitherSkeleton,
    Player,
    Zombie,
    Creeper,
    Piglin,
    Dragon,
}

/// A model, from one of the game's model layers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Model {
    Chest,
    ChestLeft,
    ChestRight,
    Shulker,
    Skull(Skull),
    StandingBanner,
    StandingFlag,
    WallBanner,
    WallFlag,
    /// A player with wide arms, and one with thin ones.
    Player,
    PlayerSlim,
    /// A cape, at its place on a player's body.
    Cape,
    /// A piece of armour on a player with thin arms or not: on the head, the chest, the
    /// legs or the feet (0 to 3).
    Armour {
        slim: bool,
        piece: u8,
    },
    /// What is held that the game draws from models of its own.
    Shield,
    Trident,
}

impl Model {
    pub const ALL: [Model; 28] = [
        Model::Chest,
        Model::ChestLeft,
        Model::ChestRight,
        Model::Shulker,
        Model::Skull(Skull::Skeleton),
        Model::Skull(Skull::WitherSkeleton),
        Model::Skull(Skull::Player),
        Model::Skull(Skull::Zombie),
        Model::Skull(Skull::Creeper),
        Model::Skull(Skull::Piglin),
        Model::Skull(Skull::Dragon),
        Model::StandingBanner,
        Model::StandingFlag,
        Model::WallBanner,
        Model::WallFlag,
        Model::Player,
        Model::PlayerSlim,
        Model::Cape,
        Model::Armour {
            slim: false,
            piece: 0,
        },
        Model::Armour {
            slim: false,
            piece: 1,
        },
        Model::Armour {
            slim: false,
            piece: 2,
        },
        Model::Armour {
            slim: false,
            piece: 3,
        },
        Model::Armour {
            slim: true,
            piece: 0,
        },
        Model::Armour {
            slim: true,
            piece: 1,
        },
        Model::Armour {
            slim: true,
            piece: 2,
        },
        Model::Armour {
            slim: true,
            piece: 3,
        },
        Model::Shield,
        Model::Trident,
    ];

    /// The game's model layer.
    pub fn layer(self) -> &'static str {
        match self {
            Model::Chest => "minecraft:chest#main",
            Model::ChestLeft => "minecraft:double_chest_left#main",
            Model::ChestRight => "minecraft:double_chest_right#main",
            Model::Shulker => "minecraft:shulker_box#main",
            Model::Skull(Skull::Skeleton) => "minecraft:skeleton_skull#main",
            Model::Skull(Skull::WitherSkeleton) => "minecraft:wither_skeleton_skull#main",
            Model::Skull(Skull::Player) => "minecraft:player_head#main",
            Model::Skull(Skull::Zombie) => "minecraft:zombie_head#main",
            Model::Skull(Skull::Creeper) => "minecraft:creeper_head#main",
            Model::Skull(Skull::Piglin) => "minecraft:piglin_head#main",
            Model::Skull(Skull::Dragon) => "minecraft:dragon_skull#main",
            Model::StandingBanner => "minecraft:standing_banner#main",
            Model::StandingFlag => "minecraft:standing_banner#flag",
            Model::WallBanner => "minecraft:wall_banner#main",
            Model::WallFlag => "minecraft:wall_banner#flag",
            Model::Player => "minecraft:player#main",
            Model::PlayerSlim => "minecraft:player_slim#main",
            Model::Cape => "minecraft:player#cape",
            Model::Shield => "minecraft:shield#main",
            Model::Trident => "minecraft:trident#main",
            Model::Armour { slim, piece } => {
                const WIDE: [&str; 4] = [
                    "minecraft:player#helmet",
                    "minecraft:player#chestplate",
                    "minecraft:player#leggings",
                    "minecraft:player#boots",
                ];
                const SLIM: [&str; 4] = [
                    "minecraft:player_slim#helmet",
                    "minecraft:player_slim#chestplate",
                    "minecraft:player_slim#leggings",
                    "minecraft:player_slim#boots",
                ];
                (if slim { SLIM } else { WIDE })[usize::from(piece.min(3))]
            }
        }
    }

    fn index(self) -> usize {
        Model::ALL
            .iter()
            .position(|m| *m == self)
            .expect("every model is in the list")
    }
}

/// How far a model's moving parts have moved.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Pose {
    Rest,
    /// A chest's lid: its openness, 0 to 1.
    Chest(f32),
    /// A shulker box's lid: its progress, 0 to 1.
    Shulker(f32),
    /// A banner's cloth: its phase, 0 to 1.
    Flag(f32),
    /// A dragon's jaw or a piglin's ears: the ticks they have moved for.
    Skull(f32),
}

/// A part of a model.
#[derive(Debug, Clone, PartialEq)]
struct Part {
    name: String,
    /// The part it is carried by: an earlier one of the model.
    parent: Option<usize>,
    /// In sixteenths of a block.
    position: [f32; 3],
    /// Radians about x, y and z.
    rotation: [f32; 3],
    scale: [f32; 3],
    /// It is of a skin's second layer.
    over: bool,
    first_quad: u32,
    quads: u32,
}

/// Where a part is put on what carries it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Moved {
    /// In sixteenths of a block.
    pub position: [f32; 3],
    /// Radians about x, y and z.
    pub rotation: [f32; 3],
    /// Done to the part before it is turned so.
    pub then: Affine,
}

/// A part to draw: its quads, and its place in the model's space (in blocks).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PartDraw {
    pub first_quad: u32,
    pub quads: u32,
    pub transform: Affine,
    /// It is of a skin's second layer.
    pub over: bool,
}

/// Every model's parts and the quads of all of them.
#[derive(Debug, Clone, PartialEq)]
pub struct ModelSet {
    /// Positions in sixteenths of a block, in the part's own space.
    quads: Vec<ModelFace>,
    /// By the model's place in [`Model::ALL`].
    models: Vec<Vec<Part>>,
}

impl ModelSet {
    /// The models from the prepared game's layers; the name of one that is missing as
    /// the error.
    pub fn new(layers: &ModelLayers) -> Result<ModelSet, String> {
        let mut set = ModelSet {
            quads: Vec::new(),
            models: Vec::new(),
        };
        for model in Model::ALL {
            let root = layers
                .get(model.layer())
                .ok_or_else(|| format!("no model layer {}", model.layer()))?;
            let mut parts = Vec::new();
            set.add("", root, None, &mut parts);
            set.models.push(parts);
        }
        Ok(set)
    }

    fn add(&mut self, name: &str, from: &ModelPart, parent: Option<usize>, to: &mut Vec<Part>) {
        let index = to.len();
        to.push(Part {
            name: name.to_owned(),
            parent,
            position: from.position,
            rotation: from.rotation,
            scale: from.scale,
            over: super::avatar::is_layer(name),
            first_quad: self.quads.len() as u32,
            quads: from.faces.len() as u32,
        });
        self.quads.extend_from_slice(&from.faces);
        for (name, child) in &from.children {
            self.add(name, child, Some(index), to);
        }
    }

    pub fn quads(&self) -> &[ModelFace] {
        &self.quads
    }

    /// Adds a model's parts that have quads to `out`, posed.
    pub fn draw(&self, model: Model, pose: Pose, out: &mut Vec<PartDraw>) {
        self.draw_by(model, out, |name, part| {
            move_part(pose, name, &mut part.position, &mut part.rotation);
            true
        });
    }

    /// Where the part `name` of a model is in the model's space (in blocks), each part
    /// moved by `moves`: what the part carries, or holds, is placed by it.
    pub fn part_transform(
        &self,
        model: Model,
        name: &str,
        mut moves: impl FnMut(&str, &mut Moved),
    ) -> Option<Affine> {
        let mut found = None;
        self.place(
            model,
            |part, m| {
                moves(part, m);
                true
            },
            |part, transform| {
                if part.name == name && found.is_none() {
                    found = Some(transform);
                }
            },
        );
        found
    }

    /// The same, with each part moved by `moves`, which says whether it and what it
    /// carries are shown. Parts are given in the model's order, each after the one that
    /// carries it.
    pub fn draw_by(
        &self,
        model: Model,
        out: &mut Vec<PartDraw>,
        moves: impl FnMut(&str, &mut Moved) -> bool,
    ) {
        self.place(model, moves, |part, transform| {
            if part.quads > 0 {
                out.push(PartDraw {
                    first_quad: part.first_quad,
                    quads: part.quads,
                    transform,
                    over: part.over,
                });
            }
        });
    }

    /// Places a model's parts that are shown, each given to `placed` with where it is.
    fn place(
        &self,
        model: Model,
        mut moves: impl FnMut(&str, &mut Moved) -> bool,
        mut each: impl FnMut(&Part, Affine),
    ) {
        /// As many parts as are placed without asking for memory.
        const FEW: usize = 24;
        let parts = &self.models[model.index()];
        // None for those not shown.
        let mut few = [None; FEW];
        let mut many = Vec::new();
        let placed: &mut [Option<Affine>] = if parts.len() <= FEW {
            &mut few[..parts.len()]
        } else {
            many.resize(parts.len(), None);
            &mut many
        };
        for (i, part) in parts.iter().enumerate() {
            let mut moved = Moved {
                position: part.position,
                rotation: part.rotation,
                then: Affine::IDENTITY,
            };
            let shown = moves(&part.name, &mut moved);
            let parent = part.parent.map_or(Some(Affine::IDENTITY), |p| placed[p]);
            let Some(parent) = parent.filter(|_| shown) else {
                continue;
            };
            let mut transform = parent.translate(moved.position.map(|c| c / 16.0));
            let [x, y, z] = moved.rotation;
            if z != 0.0 {
                transform = transform.rotate_z(z);
            }
            if y != 0.0 {
                transform = transform.rotate_y(y);
            }
            if x != 0.0 {
                transform = transform.rotate_x(x);
            }
            if moved.then != Affine::IDENTITY {
                transform = transform.then_inner(moved.then);
            }
            if part.scale != [1.0; 3] {
                transform = transform.scale(part.scale);
            }
            placed[i] = Some(transform);
            each(part, transform);
        }
    }
}

/// Moves the part `name` of a model posed so.
fn move_part(pose: Pose, name: &str, position: &mut [f32; 3], rotation: &mut [f32; 3]) {
    use std::f32::consts::{FRAC_PI_2, PI, TAU};
    match (pose, name) {
        (Pose::Chest(openness), "lid" | "lock") => {
            let shut = 1.0 - openness;
            let eased = 1.0 - shut * shut * shut;
            rotation[0] = -(eased * FRAC_PI_2);
        }
        (Pose::Shulker(progress), "lid") => {
            *position = [0.0, 24.0 - progress * 0.5 * 16.0, 0.0];
            rotation[1] = (270.0 * progress).to_radians();
        }
        (Pose::Flag(phase), "flag") => {
            let wave = ruthenium_javamath::mth::cos(f64::from(TAU * phase));
            rotation[0] = (-0.0125 + 0.01 * wave) * PI;
        }
        (Pose::Skull(ticks), "jaw") => {
            rotation[0] = ((ticks * PI * 0.2).sin() + 1.0) * 0.2;
        }
        (Pose::Skull(ticks), "left_ear") => {
            rotation[2] = -((ticks * PI * 0.2 * 1.2).cos() + 2.5) * 0.2;
        }
        (Pose::Skull(ticks), "right_ear") => {
            rotation[2] = ((ticks * PI * 0.2).cos() + 2.5) * 0.2;
        }
        _ => {}
    }
}

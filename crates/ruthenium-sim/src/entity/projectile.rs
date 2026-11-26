//! Projectiles as vanilla's client moves them between the server's updates: thrown
//! items (`ThrowableProjectile`: ender pearls, snowballs, eggs, potions, experience
//! bottles) and `AbstractHurtingProjectile`s (wind charges, fireballs, wither skulls).
//! The client ticks them itself, as it ticks every entity: their motion, what they hit
//! on the way (they stop where they hit; what a hit does is the server's), and the fluid
//! they are in. Wind charges and fireballs can be picked by the crosshair, so where the
//! client has them decides what a click is on.
//!
//! The server's updates set the position (projectiles have no interpolation) and the
//! motion (`lerpMotion`); the client goes on from there.
//!
//! Not run: arrows and tridents (`AbstractArrow`, whose flight steps through piercing
//! hits), fishing hooks, llama spit, shulker bullets and firework rockets keep the
//! server's position between updates; rotations aren't kept up (nothing drawn reads
//! them); the world border isn't a hit (`clipIncludingBorder`); bubble columns on
//! the first tick and the blocks' own effects (`applyEffectsFromBlocks`) do nothing.

use ruthenium_javamath::mth;
use ruthenium_prepare::exporters::EntityTypeData;

use super::{Entities, TickContext, add, length_sqr};
use crate::fluid::FluidInteraction;
use crate::player::LocalPlayer;
use crate::shape::Aabb;

/// How a projectile type flies on the client.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Flight {
    /// `ThrowableProjectile.tick`: gravity, then the drag (0.99; 0.8 in water).
    Throwable { gravity: f64 },
    /// `AbstractHurtingProjectile.tick`: the acceleration along the motion, then the
    /// inertia (in water, the liquid's). A wither skull's inertia is 0.73 while it is
    /// dangerous.
    Hurting {
        inertia: f32,
        liquid_inertia: f32,
        acceleration: f64,
        wither_skull: bool,
    },
}

impl Flight {
    /// A type's flight, by its class chain (`getDefaultGravity`, `getInertia`,
    /// `getLiquidInertia` and the client's `accelerationPower`, which the server
    /// doesn't send: 0.1, 0 for wind charges).
    pub(crate) fn of(d: &EntityTypeData) -> Option<Flight> {
        if d.is("ThrowableProjectile") {
            let gravity = if d.is("ThrownExperienceBottle") {
                0.07
            } else if d.is("AbstractThrownPotion") {
                0.05
            } else {
                0.03
            };
            Some(Flight::Throwable { gravity })
        } else if d.is("AbstractWindCharge") {
            Some(Flight::Hurting {
                inertia: 1.0,
                liquid_inertia: 1.0,
                acceleration: 0.0,
                wither_skull: false,
            })
        } else if d.is("AbstractHurtingProjectile") {
            Some(Flight::Hurting {
                inertia: 0.95,
                liquid_inertia: 0.8,
                acceleration: 0.1,
                wither_skull: d.is("WitherSkull"),
            })
        } else {
            None
        }
    }
}

/// What the client keeps of a flying projectile.
#[derive(Debug, Clone, Default)]
pub(crate) struct ProjectileState {
    /// `getOwner`: the entity the add packet named, if the client had it.
    pub(crate) owner: Option<i32>,
    /// `leftOwner`: it has been clear of its owner once (the client starts without).
    left_owner: bool,
    /// `wasTouchingWater`, from the last tick's fluids.
    in_water: bool,
    fluids: FluidInteraction,
    /// A wither skull's `DATA_DANGEROUS`.
    pub(crate) dangerous: bool,
}

/// `Vec3.normalize`.
fn normalize(v: [f64; 3]) -> [f64; 3] {
    let d = length_sqr(v).sqrt();
    if d < 9.999999747378752E-6 {
        [0.0; 3]
    } else {
        [v[0] / d, v[1] / d, v[2] / d]
    }
}

fn scale(v: [f64; 3], s: f64) -> [f64; 3] {
    [v[0] * s, v[1] * s, v[2] * s]
}

fn distance_sqr(a: [f64; 3], b: [f64; 3]) -> f64 {
    length_sqr([a[0] - b[0], a[1] - b[1], a[2] - b[2]])
}

impl Entities {
    /// One client tick of projectile `id` (after `commonTick`).
    pub(super) fn projectile_tick(
        &mut self,
        id: i32,
        cx: &TickContext<'_, '_>,
        player: &LocalPlayer,
    ) {
        let Some(e) = self.entities.get(&id) else {
            return;
        };
        let (Some(flight), Some(st)) = (e.info.flight, e.projectile.as_deref()) else {
            return;
        };
        let mut delta = e.delta;
        match flight {
            Flight::Throwable { gravity } => {
                // `applyGravity`, then `applyInertia`.
                if !e.no_gravity {
                    delta = add(delta, [0.0, -gravity, 0.0]);
                }
                let inertia: f32 = if st.in_water { 0.8 } else { 0.99 };
                delta = scale(delta, f64::from(inertia));
            }
            Flight::Hurting {
                inertia,
                liquid_inertia,
                acceleration,
                wither_skull,
            } => {
                let inertia = if st.in_water {
                    liquid_inertia
                } else if wither_skull && st.dangerous {
                    0.73
                } else {
                    inertia
                };
                let pushed = add(delta, scale(normalize(delta), acceleration));
                delta = scale(pushed, f64::from(inertia));
            }
        }
        // `ProjectileUtil.getHitResultOnMoveVector`: the blocks' collision shapes along
        // the move, then the entities it would meet before that.
        let from = e.position;
        let block = crate::pick::clip_collider(cx.env, from, add(from, delta));
        let to = if block.miss {
            add(from, delta)
        } else {
            block.location
        };
        let search = e.bounding_box.expand_towards(delta).inflate([1.0; 3]);
        // `computeMargin`.
        let margin = f64::from(((e.tick_count - 2) as f32 / 20.0).clamp(0.0, 0.3));
        let mut nearest = f64::MAX;
        let mut hit = None;
        for target in self.projectile_targets(id, &search, cx, player) {
            let b = target.inflate([margin; 3]);
            if let Some(at) = crate::pick::clip_box(&b, from, to) {
                let d = distance_sqr(from, at);
                if d < nearest {
                    nearest = d;
                    hit = Some(at);
                }
            }
        }
        let new_position = match hit {
            Some(at) => at,
            None if !block.miss => block.location,
            None => add(from, delta),
        };
        let Some(e) = self.entities.get_mut(&id) else {
            return;
        };
        e.delta = delta;
        e.set_pos(new_position);
        // `Projectile.tick`: `checkLeftOwner` where it now is; then `baseTick`'s fluids,
        // whose current pushes it (`isPushedByFluid`).
        let left = self.outside_owner_range(id, player, cx);
        let Some(e) = self.entities.get_mut(&id) else {
            return;
        };
        let eye = (
            mth::floor(e.position[0]),
            e.position[1] + f64::from(e.info.eye_height),
            mth::floor(e.position[2]),
        );
        let b = e.bounding_box;
        let fast_lava = cx.env.level.fast_lava();
        let Some(st) = e.projectile.as_deref_mut() else {
            return;
        };
        if !st.left_owner {
            st.left_owner = left;
        }
        st.fluids.update(cx.env, &b, eye, false);
        st.in_water = st.fluids.is_in(true);
        if st.in_water {
            st.fluids.apply_current(true, &mut e.delta, 0.014);
        }
        if st.fluids.is_in(false) {
            let scale = if fast_lava {
                0.007
            } else {
                0.0023333333333333335
            };
            st.fluids.apply_current(false, &mut e.delta, scale);
        }
        self.moved(id);
    }

    /// The boxes of the entities projectile `id` may hit in `search` (`canHitEntity`): alive and
    /// pickable (`canBeHitByProjectile`), not riding with its owner until it has left
    /// it; a fireball's or wind charge's not going through everything; a wind charge's
    /// not another wind charge or an end crystal. The local player is among them.
    fn projectile_targets(
        &self,
        id: i32,
        search: &Aabb,
        cx: &TickContext<'_, '_>,
        player: &LocalPlayer,
    ) -> Vec<Aabb> {
        let e = &self.entities[&id];
        let st = e.projectile.as_deref();
        let owner = st.and_then(|s| s.owner);
        let left = st.is_some_and(|s| s.left_owner);
        let hurting = matches!(e.info.flight, Some(Flight::Hurting { .. }));
        let wind = e.info.wind_charge;
        let local = self.local_id;
        let root_of = |i: i32| self.root_vehicle(i);
        let owner_root = owner.map(root_of);
        let allowed = |root: i32| owner_root.is_none() || left || owner_root != Some(root);
        let mut out = Vec::new();
        for t in self.in_box(search) {
            if t.id == id || !allowed(root_of(t.id)) {
                continue;
            }
            let alive = !t.removed && (!t.info.living || t.health > 0.0);
            let can = alive
                && t.is_pickable(cx.players)
                && t.info.key != "minecraft:interaction"
                && !(hurting && t.no_physics)
                && !(wind && (t.info.wind_charge || t.info.key == "minecraft:end_crystal"));
            if can {
                out.push(t.bounding_box);
            }
        }
        // The local player (`LivingEntity.isAlive`; a spectator is neither pickable nor
        // solid to fireballs).
        if let Some(local) = local {
            let b = player.bounding_box();
            let root = root_of(local);
            if b.intersects(search)
                && player.is_alive()
                && player.health > 0.0
                && !player.spectator
                && allowed(root)
            {
                out.push(b);
            }
        }
        out
    }

    /// `isOutsideOwnerCollisionRange`: no pickable entity of the owner's vehicle stack
    /// meets the projectile's box grown along its motion and by 1.
    fn outside_owner_range(&self, id: i32, player: &LocalPlayer, cx: &TickContext<'_, '_>) -> bool {
        let e = &self.entities[&id];
        let Some(owner) = e.projectile.as_deref().and_then(|s| s.owner) else {
            return true;
        };
        let reach = e.bounding_box.expand_towards(e.delta).inflate([1.0; 3]);
        let root = self.root_vehicle(owner);
        let mut stack = vec![root];
        let mut i = 0;
        while i < stack.len() {
            let next = if Some(stack[i]) == self.local_id {
                self.local_passengers.clone()
            } else {
                self.entities
                    .get(&stack[i])
                    .map(|v| v.passengers.clone())
                    .unwrap_or_default()
            };
            stack.extend(next);
            i += 1;
        }
        let area = |b: &Aabb| b.intersects(&reach);
        !stack.into_iter().any(|m| {
            if Some(m) == self.local_id {
                !player.spectator && area(&player.bounding_box())
            } else {
                self.entities.get(&m).is_some_and(|v| {
                    !v.removed
                        && v.is_pickable(cx.players)
                        && !v.is_spectator(cx.players)
                        && area(&v.bounding_box)
                })
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::super::{Local, PlayerInfos, Teams};
    use super::*;
    use crate::blocks::{BlockTags, Blocks};
    use crate::player::Env;
    use crate::world::Level;
    use ruthenium_prepare::exporters::{BlockPhysics, ShapeData, StatePhysics};
    use ruthenium_protocol::packets::play;

    const PLAYER: i32 = 1;
    const SNOWBALL: i32 = 0;
    const ZOMBIE: i32 = 1;
    const WIND_CHARGE: i32 = 2;
    const STONE: u32 = 1;
    const WATER: u32 = 2;

    fn data(id: u32, key: &str, classes: &[&str], size: (f32, f32)) -> EntityTypeData {
        EntityTypeData {
            id,
            key: key.into(),
            width: size.0,
            height: size.1,
            eye_height: size.1 * 0.85,
            fixed: false,
            update_interval: 10,
            classes: classes.iter().map(|c| (*c).to_owned()).collect(),
            fields: vec![("LivingEntity.DATA_HEALTH_ID".to_owned(), 9, 0)],
            baby: None,
            passenger_points: vec![[0.0, f64::from(size.1), 0.0]],
            vehicle_points: vec![[0.0; 3]],
            attributes: Vec::new(),
        }
    }

    fn types() -> super::super::EntityTypes {
        super::super::EntityTypes::new(&[
            data(
                0,
                "minecraft:snowball",
                &[
                    "Snowball",
                    "ThrowableItemProjectile",
                    "ThrowableProjectile",
                    "Projectile",
                    "Entity",
                ],
                (0.25, 0.25),
            ),
            data(
                1,
                "minecraft:zombie",
                &["Zombie", "Mob", "LivingEntity", "Entity"],
                (0.6, 1.95),
            ),
            data(
                2,
                "minecraft:wind_charge",
                &[
                    "WindCharge",
                    "AbstractWindCharge",
                    "AbstractHurtingProjectile",
                    "Projectile",
                    "Entity",
                ],
                (0.3125, 0.3125),
            ),
        ])
    }

    /// Air, stone and still water, one chunk at the origin.
    fn world() -> (Level, Blocks, BlockTags) {
        let state = |shape: u32, fluid: u8| StatePhysics {
            shape,
            flags: 0,
            friction: 0.6,
            speed_factor: 1.0,
            jump_factor: 1.0,
            bounce: 0.0,
            fluid,
            sturdy: 0,
            offset: None,
            outline: shape,
            interaction_shape: 0,
            interaction: 0,
            destroy_time: 0.0,
            light_dampening: 0,
            light_emission: 0,
            light_shape: false,
            occlusion: [0; 6],
        };
        let blocks = Blocks::new(&BlockPhysics {
            shapes: vec![
                ShapeData {
                    coords: [vec![0.0, 1.0], vec![0.0, 1.0], vec![0.0, 1.0]],
                    full: vec![false],
                },
                ShapeData {
                    coords: [vec![0.0, 1.0], vec![0.0, 1.0], vec![0.0, 1.0]],
                    full: vec![true],
                },
            ],
            states: vec![
                state(0, 0),
                state(1, 0),
                state(
                    0,
                    ruthenium_prepare::exporters::StatePhysics::FLUID_WATER | 8 << 4,
                ),
            ],
        });
        let mut level = Level::new(-64, 384);
        level.add_empty_chunk(0, 0);
        (level, blocks, BlockTags::new())
    }

    fn add(
        e: &mut Entities,
        id: i32,
        entity_type: i32,
        position: [f64; 3],
        movement: [f64; 3],
        owner: i32,
    ) {
        let p = play::AddEntity {
            entity_id: id,
            uuid: 1000 + id as u128,
            entity_type,
            position,
            movement,
            x_rot: 0,
            y_rot: 0,
            y_head_rot: 0,
            data: owner,
        };
        e.add(&p, &types(), &PlayerInfos::default());
    }

    fn tick(e: &mut Entities, level: &Level, blocks: &Blocks, player: &mut LocalPlayer) {
        let tags = BlockTags::new();
        let env = Env {
            level,
            blocks,
            tags: &tags,
            colliders: &[],
            others: None,
        };
        let players = PlayerInfos::default();
        let cx = TickContext {
            env: &env,
            teams: &Teams::default(),
            players: &players,
            local: Local {
                name: "Developer",
                spectator: false,
                alive: true,
            },
            tick_rate: 20.0,
            runs_normally: true,
            game_time: 0,
        };
        e.tick(&cx, player, |_, _, _| {});
    }

    fn player_at(p: [f64; 3]) -> LocalPlayer {
        let mut player = LocalPlayer::new();
        player.entity_id = PLAYER;
        player.position = p;
        player
    }

    /// `ThrowableProjectile.tick` in the air: gravity, then the drag, then the move.
    #[test]
    fn a_snowball_falls_and_slows() {
        let (level, blocks, _) = world();
        let mut e = Entities::new(PLAYER, false);
        e.chunk_loaded(0, 0);
        add(&mut e, 5, SNOWBALL, [0.5, -50.0, 0.5], [0.5, 0.2, 0.0], 0);
        let mut p = player_at([8.5, -60.0, 8.5]);
        tick(&mut e, &level, &blocks, &mut p);
        let s = e.get(5).unwrap();
        let d = [
            0.5 * f64::from(0.99f32),
            (0.2 - 0.03) * f64::from(0.99f32),
            0.0,
        ];
        assert_eq!(s.delta, d);
        assert_eq!(s.position, [0.5 + d[0], -50.0 + d[1], 0.5 + d[2]]);
    }

    /// A projectile stops where it meets a block's collision shape and stays there (the
    /// server removes it).
    #[test]
    fn a_projectile_stops_at_the_block_it_hits() {
        let (mut level, blocks, _) = world();
        level.set_block(3, -50, 0, STONE);
        let mut e = Entities::new(PLAYER, false);
        e.chunk_loaded(0, 0);
        add(
            &mut e,
            5,
            WIND_CHARGE,
            [0.5, -49.5, 0.5],
            [1.0, 0.0, 0.0],
            0,
        );
        let mut p = player_at([8.5, -60.0, 8.5]);
        for _ in 0..3 {
            tick(&mut e, &level, &blocks, &mut p);
        }
        let s = e.get(5).unwrap();
        assert_eq!(s.position, [3.0, -49.5, 0.5]);
        // A wind charge's box hangs 0.15 below it.
        assert_eq!(s.bounding_box.min[1], -49.5 - 0.15000000596046448);
        // From the face, `VoxelShape.clip` finds its first step (a thousandth of the
        // move) inside the block: it creeps in by that much each tick.
        tick(&mut e, &level, &blocks, &mut p);
        assert_eq!(e.get(5).unwrap().position, [3.001, -49.5, 0.5]);
    }

    /// In water the drag is 0.8 from the tick after it went in (`wasTouchingWater`).
    #[test]
    fn water_slows_a_thrown_snowball_from_the_next_tick() {
        let (mut level, blocks, _) = world();
        for y in -60..-50 {
            level.set_block(8, y, 8, WATER);
        }
        let mut e = Entities::new(PLAYER, false);
        e.chunk_loaded(0, 0);
        // (Away from the chunk's edge: fluids count only where the chunks around are.)
        add(&mut e, 5, SNOWBALL, [8.5, -51.0, 8.5], [0.0, -0.1, 0.0], 0);
        let mut p = player_at([2.5, -60.0, 2.5]);
        tick(&mut e, &level, &blocks, &mut p);
        let first = e.get(5).unwrap().delta;
        assert_eq!(first[1], (-0.1 - 0.03) * f64::from(0.99f32));
        tick(&mut e, &level, &blocks, &mut p);
        let second = e.get(5).unwrap().delta;
        assert_eq!(second[1], (first[1] - 0.03) * f64::from(0.8f32));
    }

    /// What a projectile hits: the nearest box on the way, grown by the margin; not
    /// its owner (nor what rides with it) until it has been clear of it once.
    #[test]
    fn a_projectile_hits_the_nearest_entity_but_not_its_owner_at_first() {
        let (level, blocks, _) = world();
        let mut p = player_at([8.5, -60.0, 8.5]);
        // Not owned: the zombie in the way stops it at its box.
        let mut e = Entities::new(PLAYER, false);
        e.chunk_loaded(0, 0);
        add(&mut e, 7, ZOMBIE, [1.6, -50.0, 0.5], [0.0; 3], 0);
        add(&mut e, 5, SNOWBALL, [0.5, -49.0, 0.5], [1.0, 0.03, 0.0], 0);
        e.entities.get_mut(&7).unwrap().health = 20.0;
        tick(&mut e, &level, &blocks, &mut p);
        // (`EntityDimensions.makeBoundingBox`: half the width as a float.)
        assert_eq!(e.get(5).unwrap().position[0], 1.6 - f64::from(0.3f32));
        // Owned by the zombie, next to it: it flies through.
        let mut e = Entities::new(PLAYER, false);
        e.chunk_loaded(0, 0);
        add(&mut e, 7, ZOMBIE, [1.6, -50.0, 0.5], [0.0; 3], 0);
        e.entities.get_mut(&7).unwrap().health = 20.0;
        add(&mut e, 5, SNOWBALL, [0.5, -49.0, 0.5], [1.0, 0.03, 0.0], 7);
        tick(&mut e, &level, &blocks, &mut p);
        assert!(e.get(5).unwrap().position[0] > 1.4);
        // A dead zombie isn't hit either.
        let mut e = Entities::new(PLAYER, false);
        e.chunk_loaded(0, 0);
        add(&mut e, 7, ZOMBIE, [1.6, -50.0, 0.5], [0.0; 3], 0);
        e.entities.get_mut(&7).unwrap().health = 0.0;
        add(&mut e, 5, SNOWBALL, [0.5, -49.0, 0.5], [1.0, 0.03, 0.0], 0);
        tick(&mut e, &level, &blocks, &mut p);
        assert!(e.get(5).unwrap().position[0] > 1.4);
    }

    /// One thrown by the local player doesn't hit it before leaving it; a wind charge
    /// never hits another.
    #[test]
    fn the_local_player_and_wind_charges() {
        let (level, blocks, _) = world();
        let mut p = player_at([1.5, -50.0, 0.5]);
        let mut e = Entities::new(PLAYER, false);
        e.chunk_loaded(0, 0);
        add(
            &mut e,
            5,
            SNOWBALL,
            [0.5, -49.0, 0.5],
            [1.0, 0.03, 0.0],
            PLAYER,
        );
        tick(&mut e, &level, &blocks, &mut p);
        assert!(e.get(5).unwrap().position[0] > 1.4);
        let mut e = Entities::new(PLAYER, false);
        e.chunk_loaded(0, 0);
        add(&mut e, 5, SNOWBALL, [0.5, -49.0, 0.5], [1.0, 0.03, 0.0], 0);
        tick(&mut e, &level, &blocks, &mut p);
        assert_eq!(e.get(5).unwrap().position[0], 1.5 - f64::from(0.3f32));
        let mut p = player_at([8.5, -60.0, 8.5]);
        let mut e = Entities::new(PLAYER, false);
        e.chunk_loaded(0, 0);
        add(&mut e, 6, WIND_CHARGE, [1.6, -49.0, 0.5], [0.0; 3], 0);
        add(
            &mut e,
            5,
            WIND_CHARGE,
            [0.5, -49.0, 0.5],
            [1.0, 0.0, 0.0],
            0,
        );
        tick(&mut e, &level, &blocks, &mut p);
        assert_eq!(e.get(5).unwrap().position[0], 1.5);
    }

    /// The margin grows a hit's boxes by a twentieth a tick from the third, to 0.3; a
    /// projectile with no gravity doesn't fall.
    #[test]
    fn the_margin_grows_and_no_gravity_holds() {
        let (level, blocks, _) = world();
        let mut p = player_at([8.5, -60.0, 8.5]);
        let mut e = Entities::new(PLAYER, false);
        e.chunk_loaded(0, 0);
        add(&mut e, 7, ZOMBIE, [5.6, -50.0, 0.5], [0.0; 3], 0);
        e.entities.get_mut(&7).unwrap().health = 20.0;
        add(
            &mut e,
            5,
            WIND_CHARGE,
            [0.5, -49.0, 0.5],
            [1.0, 0.0, 0.0],
            0,
        );
        for _ in 0..5 {
            tick(&mut e, &level, &blocks, &mut p);
        }
        // The fifth tick: (5 - 2) / 20 = 0.15 more each side.
        let margin = f64::from((5 - 2) as f32 / 20.0);
        assert_eq!(
            e.get(5).unwrap().position[0],
            5.6 - f64::from(0.3f32) - margin
        );
        let mut e = Entities::new(PLAYER, false);
        e.chunk_loaded(0, 0);
        add(&mut e, 5, SNOWBALL, [0.5, -49.0, 0.5], [0.0; 3], 0);
        e.entities.get_mut(&5).unwrap().no_gravity = true;
        tick(&mut e, &level, &blocks, &mut p);
        assert_eq!(e.get(5).unwrap().position, [0.5, -49.0, 0.5]);
    }

    /// Once clear of its owner, a projectile can hit it.
    #[test]
    fn a_projectile_that_left_its_owner_can_hit_it() {
        let (level, blocks, _) = world();
        let mut p = player_at([8.5, -60.0, 8.5]);
        let mut e = Entities::new(PLAYER, false);
        e.chunk_loaded(0, 0);
        add(&mut e, 7, ZOMBIE, [0.5, -50.0, 0.5], [0.0; 3], 0);
        e.entities.get_mut(&7).unwrap().health = 20.0;
        add(
            &mut e,
            5,
            WIND_CHARGE,
            [1.5, -49.0, 0.5],
            [1.0, 0.0, 0.0],
            7,
        );
        for _ in 0..3 {
            tick(&mut e, &level, &blocks, &mut p);
        }
        assert_eq!(e.get(5).unwrap().position[0], 4.5);
        // Back the way it came: the owner's box stops it now.
        e.set_motion(5, [-4.0, 0.0, 0.0]);
        tick(&mut e, &level, &blocks, &mut p);
        let margin = f64::from((4 - 2) as f32 / 20.0);
        assert_eq!(
            e.get(5).unwrap().position[0],
            0.5 + f64::from(0.3f32) + margin
        );
    }
}

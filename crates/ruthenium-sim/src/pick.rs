//! What the crosshair points at (`HitResult`), as vanilla computes it: rays through
//! the blocks' outline shapes (`BlockGetter.clip` with `ClipContext.Block.OUTLINE`),
//! optionally stopping at fluids, and the entities along the view
//! (`ProjectileUtil.getEntityHitResult`), put together as `LocalPlayer.pick` does.
//!
//! Every step keeps vanilla's arithmetic: the ray's walk through the block grid
//! (`traverseBlocks`), the shape test point a thousandth of the way in, the boxes a
//! shape splits into (`forAllBoxes`) and the order they are tried in (it decides
//! ties), the float direction guess of misses.
//!
//! An item with an `attack_range` (a spear) aims first with its own reach
//! ([`closest_hit`]), as `LocalPlayer.raycastHitResult` does ([`raycast`]).
//!
//! Not here: the ender dragon's parts (the dragon itself is never picked; its parts
//! aren't tracked).

use std::cell::Cell;

use ruthenium_javamath::mth;
use ruthenium_prepare::exporters::ShapeData;

use crate::entity::{Entities, PlayerInfos};
use crate::fluid::FluidState;
use crate::item::AttackRange;
use crate::player::Env;
use crate::shape::{Aabb, Axis, Placed, Shape};

pub use crate::direction::Direction;

/// `BlockHitResult`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BlockHit {
    pub location: [f64; 3],
    /// The face hit (for a miss, the direction the ray came from, roughly).
    pub direction: Direction,
    pub pos: [i32; 3],
    /// The ray started inside the shape.
    pub inside: bool,
    /// Nothing was hit (`HitResult.Type.MISS`): `location` is where the ray ended.
    pub miss: bool,
    /// The world border stopped the ray (`isWorldBorderHit`).
    pub world_border: bool,
}

impl BlockHit {
    /// `BlockHitResult.miss`.
    pub fn miss(location: [f64; 3], direction: Direction, pos: [i32; 3]) -> BlockHit {
        BlockHit {
            location,
            direction,
            pos,
            inside: false,
            miss: true,
            world_border: false,
        }
    }
}

/// `HitResult`: a block (or a miss) or an entity.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum HitResult {
    /// A block, or a miss when [`BlockHit::miss`] is set.
    Block(BlockHit),
    /// `EntityHitResult`: the entity's id and where the ray met it.
    Entity { id: i32, location: [f64; 3] },
}

impl HitResult {
    pub fn is_miss(&self) -> bool {
        matches!(self, HitResult::Block(b) if b.miss)
    }

    pub fn location(&self) -> [f64; 3] {
        match self {
            HitResult::Block(b) => b.location,
            HitResult::Entity { location, .. } => *location,
        }
    }
}

/// `ClipContext.Fluid`: which fluids stop the ray.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FluidMode {
    None,
    SourceOnly,
    Any,
    Water,
}

impl FluidMode {
    /// `canPick`.
    pub fn can_pick(self, f: FluidState) -> bool {
        match self {
            FluidMode::None => false,
            FluidMode::SourceOnly => f.is_source(),
            FluidMode::Any => !f.is_empty(),
            FluidMode::Water => f.fluid.is_water(),
        }
    }
}

/// `FlowingFluid.shapes`: vanilla caches each fluid state's shape the first time a
/// ray asks for it, with the height the fluid had *there* (a full block under the
/// same fluid, else its own height), and keeps it for the rest of the game, wherever
/// the state is met later. Keep one for the whole client and pass it to every
/// [`clip`] to match.
#[derive(Debug, Clone)]
pub struct FluidShapes {
    /// By the fluid state's byte (kind, falling, amount): the cached height.
    heights: Box<[Cell<Option<f32>>; 256]>,
}

impl Default for FluidShapes {
    fn default() -> FluidShapes {
        FluidShapes {
            heights: Box::new(std::array::from_fn(|_| Cell::new(None))),
        }
    }
}

impl FluidShapes {
    pub fn new() -> FluidShapes {
        FluidShapes::default()
    }

    /// The height of the fluid state's shape (`getShape`: `box(0, 0, 0, 1, height, 1)`;
    /// the amount is never 9, so always the cached box).
    fn height(&self, env: &Env<'_>, pos: [i32; 3], f: FluidState) -> f32 {
        let key = f.fluid as usize | usize::from(f.falling) << 3 | usize::from(f.amount & 15) << 4;
        let cell = &self.heights[key];
        match cell.get() {
            Some(h) => h,
            None => {
                let h = env.fluid_height(pos, f);
                cell.set(Some(h));
                h
            }
        }
    }
}

fn sub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn length_sqr(v: [f64; 3]) -> f64 {
    v[0] * v[0] + v[1] * v[1] + v[2] * v[2]
}

/// `Vec3.distanceToSqr`: `a.distanceToSqr(b)`.
fn distance_sqr(a: [f64; 3], b: [f64; 3]) -> f64 {
    length_sqr(sub(b, a))
}

/// `BlockPos.containing`.
fn containing(p: [f64; 3]) -> [i32; 3] {
    [mth::floor(p[0]), mth::floor(p[1]), mth::floor(p[2])]
}

/// `Entity.calculateViewVector`: the unit vector a rotation looks along, in floats
/// with the game's sine table.
pub fn view_vector(x_rot: f32, y_rot: f32) -> [f64; 3] {
    let real_x_rot = x_rot * 0.017453292_f32;
    let real_y_rot = -y_rot * 0.017453292_f32;
    let y_cos = mth::cos(f64::from(real_y_rot));
    let y_sin = mth::sin(f64::from(real_y_rot));
    let x_cos = mth::cos(f64::from(real_x_rot));
    let x_sin = mth::sin(f64::from(real_x_rot));
    [
        f64::from(y_sin * x_cos),
        f64::from(-x_sin),
        f64::from(y_cos * x_cos),
    ]
}

/// `Mth.lerp` of each axis from `old` (`xo`, `yo`, `zo`) to `now`
/// (`getPosition(partialTicks)`).
pub fn lerp_position(partial: f32, old: [f64; 3], now: [f64; 3]) -> [f64; 3] {
    let t = f64::from(partial);
    [
        mth::lerp_f64(t, old[0], now[0]),
        mth::lerp_f64(t, old[1], now[1]),
        mth::lerp_f64(t, old[2], now[2]),
    ]
}

/// `Entity.getXRot(partialTicks)`: the pitch lerped from last tick's (`xRotO`), in
/// floats; exactly the current one at 1. The local player looks along this and
/// [`lerp_y_rot`] (`getViewXRot`, `getViewYRot`).
pub fn lerp_x_rot(partial: f32, old: f32, now: f32) -> f32 {
    if partial == 1.0 {
        now
    } else {
        mth::lerp(partial, old, now)
    }
}

/// `Entity.getYRot(partialTicks)`: the yaw lerped the short way round from last
/// tick's (`yRotO`); exactly the current one at 1.
pub fn lerp_y_rot(partial: f32, old: f32, now: f32) -> f32 {
    if partial == 1.0 {
        now
    } else {
        mth::rot_lerp(partial, old, now)
    }
}

/// `Entity.getEyePosition(partialTicks)`: the lerped position raised by the eye
/// height.
pub fn eye_position(partial: f32, old: [f64; 3], now: [f64; 3], eye_height: f32) -> [f64; 3] {
    let p = lerp_position(partial, old, now);
    [p[0], p[1] + f64::from(eye_height), p[2]]
}

/// `AABB.clipPoint` along axis a (b and c the others): where the ray meets the
/// plane `a = point`, if sooner than `scale` and within the face (with the
/// tolerance).
#[allow(clippy::too_many_arguments)]
fn clip_point(
    scale: &mut f64,
    direction: Option<Direction>,
    da: f64,
    db: f64,
    dc: f64,
    point: f64,
    min_b: f64,
    max_b: f64,
    min_c: f64,
    max_c: f64,
    new_direction: Direction,
    from_a: f64,
    from_b: f64,
    from_c: f64,
) -> Option<Direction> {
    let s = (point - from_a) / da;
    let pb = from_b + s * db;
    let pc = from_c + s * dc;
    if 0.0 < s
        && s < *scale
        && min_b - 1.0E-7 < pb
        && pb < max_b + 1.0E-7
        && min_c - 1.0E-7 < pc
        && pc < max_c + 1.0E-7
    {
        *scale = s;
        Some(new_direction)
    } else {
        direction
    }
}

/// `AABB.getDirection`: the face of `b` the ray from `from` along `d` enters
/// first, if before `scale` (which it shortens), else `direction` as it was.
fn get_direction(
    b: &Aabb,
    from: [f64; 3],
    scale: &mut f64,
    mut direction: Option<Direction>,
    d: [f64; 3],
) -> Option<Direction> {
    let ([x0, y0, z0], [x1, y1, z1]) = (b.min, b.max);
    let [fx, fy, fz] = from;
    let [dx, dy, dz] = d;
    if dx > 1.0E-7 {
        direction = clip_point(
            scale,
            direction,
            dx,
            dy,
            dz,
            x0,
            y0,
            y1,
            z0,
            z1,
            Direction::West,
            fx,
            fy,
            fz,
        );
    } else if dx < -1.0E-7 {
        direction = clip_point(
            scale,
            direction,
            dx,
            dy,
            dz,
            x1,
            y0,
            y1,
            z0,
            z1,
            Direction::East,
            fx,
            fy,
            fz,
        );
    }
    if dy > 1.0E-7 {
        direction = clip_point(
            scale,
            direction,
            dy,
            dz,
            dx,
            y0,
            z0,
            z1,
            x0,
            x1,
            Direction::Down,
            fy,
            fz,
            fx,
        );
    } else if dy < -1.0E-7 {
        direction = clip_point(
            scale,
            direction,
            dy,
            dz,
            dx,
            y1,
            z0,
            z1,
            x0,
            x1,
            Direction::Up,
            fy,
            fz,
            fx,
        );
    }
    if dz > 1.0E-7 {
        direction = clip_point(
            scale,
            direction,
            dz,
            dx,
            dy,
            z0,
            x0,
            x1,
            y0,
            y1,
            Direction::North,
            fz,
            fx,
            fy,
        );
    } else if dz < -1.0E-7 {
        direction = clip_point(
            scale,
            direction,
            dz,
            dx,
            dy,
            z1,
            x0,
            x1,
            y0,
            y1,
            Direction::South,
            fz,
            fx,
            fy,
        );
    }
    direction
}

/// `AABB.clip(from, to)`: where the segment enters the box, if it does (not when it
/// starts inside).
pub fn clip_box(b: &Aabb, from: [f64; 3], to: [f64; 3]) -> Option<[f64; 3]> {
    let mut scale = 1.0;
    let d = sub(to, from);
    get_direction(b, from, &mut scale, None, d)?;
    Some([
        from[0] + scale * d[0],
        from[1] + scale * d[1],
        from[2] + scale * d[2],
    ])
}

/// `VoxelShape.clip`: a shape placed at `pos`, hit by the segment. A ray starting in
/// a full cell (tested a thousandth of the way along) hits there, "inside", facing
/// back along the ray; else the first face of the shape's boxes it enters.
fn clip_shape(shape: &Placed<'_>, from: [f64; 3], to: [f64; 3], pos: [i32; 3]) -> Option<BlockHit> {
    if shape.shape.is_empty() {
        return None;
    }
    let diff = sub(to, from);
    if length_sqr(diff) < 1.0E-7 {
        return None;
    }
    let test = [
        from[0] + diff[0] * 0.001,
        from[1] + diff[1] * 0.001,
        from[2] + diff[2] * 0.001,
    ];
    let cell = |axis: Axis| {
        shape.find_index_local(axis, test[axis as usize] - f64::from(pos[axis as usize]))
    };
    if shape
        .shape
        .is_full_wide(cell(Axis::X), cell(Axis::Y), cell(Axis::Z))
    {
        return Some(BlockHit {
            location: test,
            direction: Direction::approximate_nearest(diff).opposite(),
            pos,
            inside: true,
            miss: false,
            world_border: false,
        });
    }
    // `AABB.clip(toAabbs(), from, to, pos)`.
    let mut scale = 1.0;
    let mut direction = None;
    let block = [f64::from(pos[0]), f64::from(pos[1]), f64::from(pos[2])];
    for b in shape.shape.boxes() {
        let local = Aabb::new(
            [
                shape.local(Axis::X, b[0]),
                shape.local(Axis::Y, b[1]),
                shape.local(Axis::Z, b[2]),
            ],
            [
                shape.local(Axis::X, b[3]),
                shape.local(Axis::Y, b[4]),
                shape.local(Axis::Z, b[5]),
            ],
        );
        direction = get_direction(&local.moved(block), from, &mut scale, direction, diff);
    }
    Some(BlockHit {
        location: [
            from[0] + scale * diff[0],
            from[1] + scale * diff[1],
            from[2] + scale * diff[2],
        ],
        direction: direction?,
        pos,
        inside: false,
        miss: false,
        world_border: false,
    })
}

/// `Mth.frac`.
fn frac(v: f64) -> f64 {
    v - mth::lfloor(v) as f64
}

/// `Mth.sign`.
fn sign(v: f64) -> i32 {
    if v == 0.0 {
        0
    } else if v > 0.0 {
        1
    } else {
        -1
    }
}

/// `BlockGetter.traverseBlocks`: the blocks the segment passes through, in order,
/// until `visit` returns a hit.
fn traverse(
    from: [f64; 3],
    to: [f64; 3],
    mut visit: impl FnMut([i32; 3]) -> Option<BlockHit>,
    miss: impl FnOnce() -> BlockHit,
) -> BlockHit {
    // `Vec3.equals` compares with `Double.compare`.
    if (0..3).all(|i| from[i].total_cmp(&to[i]).is_eq()) {
        return miss();
    }
    let to_x = mth::lerp_f64(-1.0E-7, to[0], from[0]);
    let to_y = mth::lerp_f64(-1.0E-7, to[1], from[1]);
    let to_z = mth::lerp_f64(-1.0E-7, to[2], from[2]);
    let from_x = mth::lerp_f64(-1.0E-7, from[0], to[0]);
    let from_y = mth::lerp_f64(-1.0E-7, from[1], to[1]);
    let from_z = mth::lerp_f64(-1.0E-7, from[2], to[2]);
    let mut x = mth::floor(from_x);
    let mut y = mth::floor(from_y);
    let mut z = mth::floor(from_z);
    if let Some(hit) = visit([x, y, z]) {
        return hit;
    }
    let (dx, dy, dz) = (to_x - from_x, to_y - from_y, to_z - from_z);
    let (sx, sy, sz) = (sign(dx), sign(dy), sign(dz));
    let delta = |s: i32, d: f64| {
        if s == 0 { f64::MAX } else { f64::from(s) / d }
    };
    let (t_delta_x, t_delta_y, t_delta_z) = (delta(sx, dx), delta(sy, dy), delta(sz, dz));
    let start = |s: i32, f: f64| if s > 0 { 1.0 - frac(f) } else { frac(f) };
    let mut tx = t_delta_x * start(sx, from_x);
    let mut ty = t_delta_y * start(sy, from_y);
    let mut tz = t_delta_z * start(sz, from_z);
    while tx <= 1.0 || ty <= 1.0 || tz <= 1.0 {
        if tx < ty {
            if tx < tz {
                x += sx;
                tx += t_delta_x;
            } else {
                z += sz;
                tz += t_delta_z;
            }
        } else if ty < tz {
            y += sy;
            ty += t_delta_y;
        } else {
            z += sz;
            tz += t_delta_z;
        }
        if let Some(hit) = visit([x, y, z]) {
            return hit;
        }
    }
    miss()
}

/// `BlockGetter.clip` with `ClipContext.Block.OUTLINE`: the first block outline (or
/// fluid, as `fluid` says) the segment from `from` to `to` hits, else a miss at
/// `to`. `held_item` is the main-hand item of the entity the context is for
/// (`minecraft:air` for none; see [`crate::blocks::Blocks::outline`]);
/// `fluid_shapes` is the client's [`FluidShapes`] cache (unused with
/// [`FluidMode::None`]).
pub fn clip(
    env: &Env<'_>,
    from: [f64; 3],
    to: [f64; 3],
    fluid: FluidMode,
    held_item: &str,
    fluid_shapes: &FluidShapes,
) -> BlockHit {
    clip_blocks(env, from, to, fluid, held_item, Some(fluid_shapes))
}

/// [`clip`], with no fluid cache needed for [`FluidMode::None`].
fn clip_blocks(
    env: &Env<'_>,
    from: [f64; 3],
    to: [f64; 3],
    fluid: FluidMode,
    held_item: &str,
    fluid_shapes: Option<&FluidShapes>,
) -> BlockHit {
    traverse(
        from,
        to,
        |pos| {
            let state = env.level.block(pos[0], pos[1], pos[2]);
            let block = clip_with_interaction_override(env, from, to, pos, state, held_item);
            let fluid_state = env.blocks.fluid(state);
            let liquid = if let Some(shapes) = fluid_shapes
                && fluid.can_pick(fluid_state)
            {
                let height = shapes.height(env, pos, fluid_state);
                let shape = Shape::new(&ShapeData {
                    coords: [vec![0.0, 1.0], vec![0.0, f64::from(height)], vec![0.0, 1.0]],
                    full: vec![true],
                });
                let placed = Placed {
                    shape: &shape,
                    offset: None,
                    pos: [f64::from(pos[0]), f64::from(pos[1]), f64::from(pos[2])],
                };
                clip_shape(&placed, from, to, pos)
            } else {
                None
            };
            let block_distance = block.map_or(f64::MAX, |b| distance_sqr(from, b.location));
            let liquid_distance = liquid.map_or(f64::MAX, |b| distance_sqr(from, b.location));
            if block_distance <= liquid_distance {
                block
            } else {
                liquid
            }
        },
        || {
            let delta = sub(from, to);
            BlockHit::miss(to, Direction::approximate_nearest(delta), containing(to))
        },
    )
}

/// `clipWithInteractionOverride`: the outline's hit, facing as the interaction
/// shape's hit when that one is nearer.
fn clip_with_interaction_override(
    env: &Env<'_>,
    from: [f64; 3],
    to: [f64; 3],
    pos: [i32; 3],
    state: u32,
    held_item: &str,
) -> Option<BlockHit> {
    let mut hit = clip_shape(&env.blocks.outline(state, pos, held_item)?, from, to, pos)?;
    if let Some(shape) = env.blocks.interaction_shape(state, pos)
        && let Some(over) = clip_shape(&shape, from, to, pos)
        && length_sqr(sub(over.location, from)) < length_sqr(sub(hit.location, from))
    {
        hit.direction = over.direction;
    }
    Some(hit)
}

/// The entity the crosshair looks from: the local player, or the entity a spectator
/// looks through.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Camera {
    /// The camera entity's id (never picked itself).
    pub id: i32,
    /// `getEyePosition(partialTicks)` (see [`eye_position`]).
    pub eye: [f64; 3],
    /// `getViewVector(partialTicks)` (see [`view_vector`]; the local player looks
    /// along its lerped rotation).
    pub view: [f64; 3],
    /// Its bounding box now.
    pub bounding_box: Aabb,
}

/// `ProjectileUtil.getEntityHitResult(except, from, to, box, CAN_BE_PICKED,
/// maxValue)`: the nearest pickable entity whose box (grown by its pick radius) the
/// segment enters within `max_distance_sqr`. One whose box holds `from` wins at
/// distance zero; one in the camera's own vehicle stack only takes over a hit at
/// zero, never shortening the search.
pub fn entity_hit(
    entities: &Entities,
    players: &PlayerInfos,
    except: i32,
    from: [f64; 3],
    to: [f64; 3],
    search: &Aabb,
    max_distance_sqr: f64,
) -> Option<(i32, [f64; 3])> {
    let mut nearest = max_distance_sqr;
    let mut hovered: Option<(i32, [f64; 3])> = None;
    let except_root = entities.root_vehicle(except);
    for e in entities.in_box(search) {
        if e.id == except || !e.is_pickable(players) {
            continue;
        }
        let r = f64::from(e.pick_radius());
        let b = e.bounding_box.inflate([r, r, r]);
        let clip_point = clip_box(&b, from, to);
        let inside = (0..3).all(|i| from[i] >= b.min[i] && from[i] < b.max[i]);
        if inside {
            if nearest >= 0.0 && e.can_be_picked_from_inside() {
                hovered = Some((e.id, clip_point.unwrap_or(from)));
                nearest = 0.0;
            }
        } else if let Some(location) = clip_point {
            let dd = distance_sqr(from, location);
            if dd < nearest || nearest == 0.0 {
                if entities.root_vehicle(e.id) == except_root {
                    if nearest == 0.0 {
                        hovered = Some((e.id, location));
                    }
                } else {
                    hovered = Some((e.id, location));
                    nearest = dd;
                }
            }
        }
    }
    hovered
}

/// `LocalPlayer.filterHitResult`: a hit beyond `range` becomes a miss where it was.
pub fn filter_hit_result(hit: HitResult, from: [f64; 3], range: f64) -> HitResult {
    let location = hit.location();
    // `closerThan`.
    if distance_sqr(location, from) < range * range {
        return hit;
    }
    let direction = Direction::approximate_nearest(sub(location, from));
    HitResult::Block(BlockHit::miss(location, direction, containing(location)))
}

/// What an item's attack range is aimed from (`AttackRange.getClosesetHit`'s attacker):
/// unlike the rest of the pick, the eyes as they are and the head's rotation, which for
/// the local player has the yaw of its last tick.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Attacker {
    pub range: AttackRange,
    /// `isCreative`.
    pub creative: bool,
    /// `getEyePosition()`, not interpolated.
    pub eye: [f64; 3],
    /// `getHeadLookAngle`.
    pub head_look: [f64; 3],
    /// `getKnownMovement`: moving along the look reaches that much further.
    pub movement: [f64; 3],
}

/// `CollisionGetter.clipIncludingBorder` (outlines, no fluids): a ray from inside the
/// world border that ends outside it hits the border.
fn clip_including_border(env: &Env<'_>, from: [f64; 3], to: [f64; 3], held_item: &str) -> BlockHit {
    let hit = clip_blocks(env, from, to, FluidMode::None, held_item, None);
    let border = &env.level.border;
    let end = hit.location;
    if !border.is_within_bounds(from[0], from[2]) || border.is_within_bounds(end[0], end[2]) {
        return hit;
    }
    let location = border.clamp_to_bounds(end);
    BlockHit {
        location,
        direction: Direction::approximate_nearest(sub(end, from)),
        pos: containing(location),
        inside: false,
        miss: false,
        world_border: true,
    }
}

/// `AttackRange.getClosesetHit` (`ProjectileUtil.getHitEntitiesAlong` with
/// `CAN_BE_PICKED` and outlines): of the entities along the head's look between the
/// range's ends, the one hit nearest to `camera.eye`; with none, the block the look
/// hits from the eyes (a miss without one). A block nearer than the range's start hides
/// everything.
pub fn closest_hit(
    env: &Env<'_>,
    entities: &Entities,
    players: &PlayerInfos,
    camera: &Camera,
    attacker: &Attacker,
    held_item: &str,
) -> HitResult {
    let (eye, look, m) = (attacker.eye, attacker.head_look, attacker.movement);
    let along = |d: f64| [0, 1, 2].map(|i| eye[i] + look[i] * d);
    let from = along(f64::from(attacker.range.min_range(attacker.creative)));
    let moving = m[0] * look[0] + m[1] * look[1] + m[2] * look[2];
    let mut to = along(f64::from(attacker.range.max_range(attacker.creative)) + moving.max(0.0));
    let block = clip_including_border(env, eye, to, held_item);
    if !block.miss {
        to = block.location;
        if distance_sqr(eye, to) < distance_sqr(eye, from) {
            return HitResult::Block(block);
        }
    }
    // `AABB.ofSize` around the start.
    let margin = f64::from(attacker.range.hitbox_margin);
    let half = margin / 2.0;
    let search = Aabb::new(from.map(|v| v - half), from.map(|v| v + half))
        .expand_towards(sub(to, from))
        .inflate([1.0; 3]);
    let mut closest: Option<(i32, [f64; 3])> = None;
    let mut closest_distance = f64::MAX;
    for e in entities.in_box(&search) {
        if e.id == camera.id || !e.is_pickable(players) {
            continue;
        }
        let b = &e.bounding_box;
        let Some(location) = hit_with_margin(env, b, from, to, margin, held_item) else {
            continue;
        };
        let distance = distance_sqr(camera.eye, location);
        if distance < closest_distance {
            closest_distance = distance;
            closest = Some((e.id, location));
        }
    }
    match closest {
        Some((id, location)) => HitResult::Entity { id, location },
        None => HitResult::Block(block),
    }
}

/// `ProjectileUtil.getManyEntityHitResult` for one entity's box (`includeFromEntity`,
/// `projectSurfaceHitLocation`): hit at `from` when that is inside it, else where the
/// segment enters it; else, when the segment only enters the box grown by the margin,
/// where the line from there to the box's middle enters the box, unless a block is in
/// the way.
fn hit_with_margin(
    env: &Env<'_>,
    b: &Aabb,
    from: [f64; 3],
    to: [f64; 3],
    margin: f64,
    held_item: &str,
) -> Option<[f64; 3]> {
    if (0..3).all(|i| from[i] >= b.min[i] && from[i] < b.max[i]) {
        return Some(from);
    }
    if let Some(exact) = clip_box(b, from, to) {
        return Some(exact);
    }
    if margin <= 0.0 {
        return None;
    }
    let outside = clip_box(&b.inflate([margin; 3]), from, to)?;
    let mut towards = [0, 1, 2].map(|i| mth::lerp_f64(0.5, b.min[i], b.max[i]));
    let block = clip_including_border(env, outside, towards, held_item);
    if !block.miss {
        towards = block.location;
    }
    clip_box(b, outside, towards)
}

/// `LocalPlayer.raycastHitResult`: what the active item's attack range hits, if it has
/// one (a block only within the block range), else `LocalPlayer.pick`.
#[allow(clippy::too_many_arguments)]
pub fn raycast(
    env: &Env<'_>,
    entities: &Entities,
    players: &PlayerInfos,
    camera: &Camera,
    attacker: Option<&Attacker>,
    held_item: &str,
    block_range: f64,
    entity_range: f64,
) -> HitResult {
    if let Some(attacker) = attacker {
        let mut hit = closest_hit(env, entities, players, camera, attacker, held_item);
        if matches!(hit, HitResult::Block(_)) {
            hit = filter_hit_result(hit, camera.eye, block_range);
        }
        if !hit.is_miss() {
            return hit;
        }
    }
    pick(
        env,
        entities,
        players,
        camera,
        held_item,
        block_range,
        entity_range,
    )
}

/// `LocalPlayer.pick`: the block outline the camera looks at within the longer of the two ranges, and an
/// entity before it if there is one; the result is a miss past its own range.
/// `held_item` is the camera entity's main-hand item (`minecraft:air` for none).
#[allow(clippy::too_many_arguments)]
pub fn pick(
    env: &Env<'_>,
    entities: &Entities,
    players: &PlayerInfos,
    camera: &Camera,
    held_item: &str,
    block_range: f64,
    entity_range: f64,
) -> HitResult {
    let mut max_distance = block_range.max(entity_range);
    let mut max_distance_sqr = max_distance * max_distance;
    let from = camera.eye;
    let view = camera.view;
    // `Entity.pick(maxDistance, partialTicks, false)`.
    let reach = [
        from[0] + view[0] * max_distance,
        from[1] + view[1] * max_distance,
        from[2] + view[2] * max_distance,
    ];
    let block = clip_blocks(env, from, reach, FluidMode::None, held_item, None);
    let block_distance_sqr = distance_sqr(block.location, from);
    if !block.miss {
        max_distance_sqr = block_distance_sqr;
        max_distance = max_distance_sqr.sqrt();
    }
    let to = [
        from[0] + view[0] * max_distance,
        from[1] + view[1] * max_distance,
        from[2] + view[2] * max_distance,
    ];
    let search = camera
        .bounding_box
        .expand_towards([
            view[0] * max_distance,
            view[1] * max_distance,
            view[2] * max_distance,
        ])
        .inflate([1.0, 1.0, 1.0]);
    match entity_hit(
        entities,
        players,
        camera.id,
        from,
        to,
        &search,
        max_distance_sqr,
    ) {
        Some((id, location)) if distance_sqr(location, from) < block_distance_sqr => {
            filter_hit_result(HitResult::Entity { id, location }, from, entity_range)
        }
        _ => filter_hit_result(HitResult::Block(block), from, block_range),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::blocks::{BlockTags, Blocks, offset};
    use crate::entity::EntityTypes;
    use crate::world::Level;
    use ruthenium_prepare::exporters::{
        BlockPhysics, EntityTypeData, OffsetData, ShapeData, StatePhysics,
    };
    use ruthenium_protocol::packets::play::{self, DataValue};

    const FULL: u32 = 1;
    const SLAB: u32 = 2;
    const WATER: u32 = 3;
    const FLOWING: u32 = 4;
    const FLOWER: u32 = 5;
    /// A bottom slab outline with a full-block interaction shape.
    const BOWL: u32 = 6;

    fn shape(x: [f64; 2], y: [f64; 2], z: [f64; 2]) -> ShapeData {
        ShapeData {
            coords: [x.to_vec(), y.to_vec(), z.to_vec()],
            full: vec![true],
        }
    }

    fn blocks() -> Blocks {
        let state = |outline, fluid, interaction_shape, offset| StatePhysics {
            shape: outline,
            flags: 0,
            friction: 0.6,
            speed_factor: 1.0,
            jump_factor: 1.0,
            bounce: 0.0,
            fluid,
            sturdy: 0,
            offset,
            outline,
            interaction_shape,
            interaction: 0,
            destroy_time: 0.0,
            light_dampening: 0,
            occlusion: [0, 0],
        };
        let water = StatePhysics::FLUID_WATER | 8 << 4;
        let flowing = StatePhysics::FLUID_FLOWING_WATER | 4 << 4;
        let flower_offset = OffsetData {
            vertical: false,
            max_horizontal: 0.25,
            max_vertical: 0.2,
            collision: true,
            outline: true,
        };
        Blocks::new(&BlockPhysics {
            shapes: vec![
                ShapeData {
                    full: vec![false],
                    ..shape([0.0, 1.0], [0.0, 1.0], [0.0, 1.0])
                },
                shape([0.0, 1.0], [0.0, 1.0], [0.0, 1.0]),
                shape([0.0, 1.0], [0.0, 0.5], [0.0, 1.0]),
                shape([0.3125, 0.6875], [0.0, 0.625], [0.3125, 0.6875]),
            ],
            states: vec![
                state(0, 0, 0, None),
                state(1, 0, 0, None),
                state(2, 0, 0, None),
                state(0, water, 0, None),
                state(0, flowing, 0, None),
                state(3, 0, 0, Some(flower_offset)),
                state(2, 0, 1, None),
            ],
        })
    }

    fn level(blocks: &[([i32; 3], u32)]) -> Level {
        let mut level = Level::new(-64, 384);
        for cx in -1..=1 {
            for cz in -1..=1 {
                level.add_empty_chunk(cx, cz);
            }
        }
        for &(p, s) in blocks {
            level.set_block(p[0], p[1], p[2], s);
        }
        level
    }

    fn clip_in(level: &Level, from: [f64; 3], to: [f64; 3], fluid: FluidMode) -> BlockHit {
        clip_cached(level, from, to, fluid, &FluidShapes::new())
    }

    fn clip_cached(
        level: &Level,
        from: [f64; 3],
        to: [f64; 3],
        fluid: FluidMode,
        shapes: &FluidShapes,
    ) -> BlockHit {
        let blocks = blocks();
        let tags = BlockTags::new();
        let env = Env {
            level,
            blocks: &blocks,
            tags: &tags,
            colliders: &[],
            others: None,
        };
        clip(&env, from, to, fluid, "minecraft:air", shapes)
    }

    const BLOCK: [i32; 3] = [2, -60, 2];

    #[test]
    fn rays_hit_each_face_of_a_full_block() {
        let level = level(&[(BLOCK, FULL)]);
        let center = [2.5, -59.5, 2.5];
        for d in Direction::ALL {
            let n = d.normal().map(f64::from);
            // 4.5 out, 3.5 through: the face is half way, exactly.
            let from = [0, 1, 2].map(|i| center[i] + n[i] * 4.5);
            let to = [0, 1, 2].map(|i| center[i] - n[i] * 3.5);
            let hit = clip_in(&level, from, to, FluidMode::None);
            let face = [0, 1, 2].map(|i| center[i] + n[i] * 0.5);
            assert_eq!(
                hit,
                BlockHit {
                    location: face,
                    direction: d,
                    pos: BLOCK,
                    inside: false,
                    miss: false,
                    world_border: false,
                },
                "{d:?}"
            );
        }
        assert_eq!(Direction::East.id(), 5);
        assert_eq!(Direction::from_id(2), Some(Direction::North));
    }

    #[test]
    fn a_slab_is_hit_on_its_top_and_passed_over() {
        let level = level(&[(BLOCK, SLAB)]);
        let hit = clip_in(
            &level,
            [2.5, -55.0, 2.5],
            [2.5, -63.0, 2.5],
            FluidMode::None,
        );
        assert_eq!(hit.location, [2.5, -59.5, 2.5]);
        assert_eq!(
            (hit.direction, hit.pos, hit.miss),
            (Direction::Up, BLOCK, false)
        );
        // Above the slab's top, a ray across goes on to its end.
        let over = clip_in(
            &level,
            [0.5, -59.25, 2.5],
            [5.5, -59.25, 2.5],
            FluidMode::None,
        );
        assert!(over.miss);
    }

    #[test]
    fn a_ray_from_inside_hits_where_it_starts() {
        let level = level(&[(BLOCK, FULL)]);
        let hit = clip_in(
            &level,
            [2.5, -59.5, 2.5],
            [2.5, -59.5, 10.5],
            FluidMode::None,
        );
        assert_eq!(
            hit,
            BlockHit {
                location: [2.5, -59.5, 2.5 + 8.0 * 0.001],
                direction: Direction::North,
                pos: BLOCK,
                inside: true,
                miss: false,
                world_border: false,
            }
        );
    }

    #[test]
    fn a_miss_ends_at_the_end_facing_back() {
        let level = level(&[]);
        let to = [4.5, -50.5, -1.5];
        let hit = clip_in(&level, [0.5, -50.5, 0.5], to, FluidMode::None);
        // Back along (-4, 0, 2): mostly west.
        assert_eq!(hit, BlockHit::miss(to, Direction::West, [4, -51, -2]));
        // No length at all: a miss there too.
        let same = clip_in(&level, to, to, FluidMode::None);
        assert!(same.miss && same.location == to);
    }

    #[test]
    fn the_nearer_block_hides_the_other() {
        let level = level(&[(BLOCK, FULL), ([4, -60, 2], FULL)]);
        let hit = clip_in(
            &level,
            [6.5, -59.5, 2.5],
            [-1.5, -59.5, 2.5],
            FluidMode::None,
        );
        assert_eq!((hit.pos, hit.direction), ([4, -60, 2], Direction::East));
        let hit = clip_in(
            &level,
            [0.5, -59.5, 2.5],
            [8.5, -59.5, 2.5],
            FluidMode::None,
        );
        assert_eq!((hit.pos, hit.direction), (BLOCK, Direction::West));
        assert_eq!(hit.location, [2.0, -59.5, 2.5]);
    }

    #[test]
    fn offset_outlines_move_with_the_block() {
        let pos = [5, -60, 7];
        let level = level(&[(pos, FLOWER)]);
        let o = offset(pos, false, 0.25, 0.2);
        let x = 5.0 + 0.5 + o[0];
        let z = 7.0 + 0.5 + o[2];
        let hit = clip_in(&level, [x, -55.0, z], [x, -63.0, z], FluidMode::None);
        assert_eq!(
            (hit.pos, hit.direction, hit.miss),
            (pos, Direction::Up, false)
        );
        assert_eq!(hit.location, [x, -59.375, z]);
        // Just outside the moved box: a miss.
        let edge = 5.0 + 0.6875 + o[0] + 0.01;
        let miss = clip_in(&level, [edge, -55.0, z], [edge, -63.0, z], FluidMode::None);
        assert!(miss.miss);
    }

    #[test]
    fn the_interaction_shape_can_turn_the_face() {
        let level = level(&[(BLOCK, BOWL)]);
        // Down and east: the full interaction box is entered on its west side before
        // the slab's top.
        let hit = clip_in(
            &level,
            [1.0, -59.0, 2.5],
            [3.0, -59.8, 2.5],
            FluidMode::None,
        );
        assert_eq!(hit.direction, Direction::West);
        assert!((hit.location[0] - 2.25).abs() < 1e-12 && (hit.location[1] + 59.5).abs() < 1e-12);
        // Straight down both are hit on top: up either way.
        let down = clip_in(
            &level,
            [2.5, -55.0, 2.5],
            [2.5, -63.0, 2.5],
            FluidMode::None,
        );
        assert_eq!((down.direction, down.location[1]), (Direction::Up, -59.5));
    }

    #[test]
    fn fluids_stop_rays_as_the_mode_says() {
        let source = [2, -60, 8];
        let flowing = [4, -60, 8];
        let level = level(&[(source, WATER), (flowing, FLOWING)]);
        let down = |p: [i32; 3], mode| {
            let (x, z) = (f64::from(p[0]) + 0.5, f64::from(p[2]) + 0.5);
            clip_in(&level, [x, -55.0, z], [x, -63.0, z], mode)
        };
        assert!(down(source, FluidMode::None).miss);
        let hit = down(source, FluidMode::SourceOnly);
        assert_eq!(
            (hit.pos, hit.direction, hit.miss),
            (source, Direction::Up, false)
        );
        let top = -60.0 + f64::from(8.0f32 / 9.0);
        assert!((hit.location[1] - top).abs() < 1e-12);
        assert!(down(flowing, FluidMode::SourceOnly).miss);
        for mode in [FluidMode::Any, FluidMode::Water] {
            let hit = down(flowing, mode);
            let top = -60.0 + f64::from(4.0f32 / 9.0);
            assert!(!hit.miss && (hit.location[1] - top).abs() < 1e-12);
        }
    }

    #[test]
    fn a_fluid_shape_keeps_the_height_it_was_first_met_with() {
        // A source under a source: its shape fills the block. Met first by a ray across
        // it, that full box is what every water source has afterwards.
        let lone = [6, -60, 2];
        let level = level(&[([2, -60, 8], WATER), ([2, -59, 8], WATER), (lone, WATER)]);
        let shapes = FluidShapes::new();
        let across = clip_cached(
            &level,
            [0.5, -59.75, 8.5],
            [5.5, -59.75, 8.5],
            FluidMode::Any,
            &shapes,
        );
        assert_eq!(
            (across.pos, across.direction),
            ([2, -60, 8], Direction::West)
        );
        let down = |shapes: &FluidShapes| {
            clip_cached(
                &level,
                [6.5, -55.0, 2.5],
                [6.5, -63.0, 2.5],
                FluidMode::Any,
                shapes,
            )
        };
        assert_eq!(down(&shapes).location[1], -59.0);
        let fresh = down(&FluidShapes::new()).location[1];
        assert!((fresh - (-60.0 + f64::from(8.0f32 / 9.0))).abs() < 1e-12);
    }

    #[test]
    fn view_vectors_and_eyes() {
        assert_eq!(view_vector(0.0, 0.0), [0.0, 0.0, 1.0]);
        let east = view_vector(0.0, -90.0);
        assert!((east[0] - 1.0).abs() < 1e-6 && east[2].abs() < 1e-6);
        let down = view_vector(90.0, 0.0);
        assert!((down[1] + 1.0).abs() < 1e-6);
        // Exactly the float table's values.
        let v = view_vector(30.0, 45.0);
        let (xr, yr) = (30.0f32 * 0.017453292, -45.0f32 * 0.017453292);
        let (xc, xs) = (mth::cos(f64::from(xr)), mth::sin(f64::from(xr)));
        let (yc, ys) = (mth::cos(f64::from(yr)), mth::sin(f64::from(yr)));
        assert_eq!(v, [f64::from(ys * xc), f64::from(-xs), f64::from(yc * xc)]);
        let eye = eye_position(0.5, [0.0, 10.0, 0.0], [1.0, 11.0, 2.0], 1.62);
        assert_eq!(eye, [0.5, 10.5 + f64::from(1.62f32), 1.0]);
        assert_eq!(lerp_y_rot(0.5, 170.0, -170.0), 180.0);
        assert_eq!(lerp_x_rot(0.5, 10.0, 20.0), 15.0);
        assert_eq!(lerp_y_rot(1.0, 170.0, -170.0), -170.0);
        assert_eq!(Direction::approximate_nearest([0.0; 3]), Direction::North);
        assert_eq!(
            Direction::approximate_nearest([0.0, -1e-9, 0.0]),
            Direction::Down
        );
    }

    // Entities.

    const PLAYER_ID: i32 = 1;
    const ZOMBIE: i32 = 0;
    const ARMOR_STAND: i32 = 1;
    const ARROW: i32 = 2;
    const ITEM: i32 = 3;
    const FIREBALL: i32 = 4;

    fn types() -> EntityTypes {
        let data =
            |id: u32, key: &str, classes: &[&str], size: (f32, f32), fields: &[(&str, u8)]| {
                EntityTypeData {
                    id,
                    key: key.into(),
                    width: size.0,
                    height: size.1,
                    eye_height: size.1 * 0.85,
                    fixed: false,
                    update_interval: 3,
                    classes: classes.iter().map(|c| (*c).to_owned()).collect(),
                    fields: fields
                        .iter()
                        .map(|(n, i)| ((*n).to_owned(), *i, 0))
                        .collect(),
                    baby: None,
                    passenger_points: vec![[0.0, f64::from(size.1), 0.0]],
                    vehicle_points: vec![[0.0; 3]],
                    attributes: Vec::new(),
                }
            };
        EntityTypes::new(&[
            data(
                0,
                "minecraft:zombie",
                &["Zombie", "Mob", "LivingEntity", "Entity"],
                (0.6, 1.95),
                &[],
            ),
            data(
                1,
                "minecraft:armor_stand",
                &["ArmorStand", "LivingEntity", "Entity"],
                (0.5, 1.975),
                &[("ArmorStand.DATA_CLIENT_FLAGS", 15)],
            ),
            data(
                2,
                "minecraft:arrow",
                &["Arrow", "AbstractArrow", "Projectile", "Entity"],
                (0.5, 0.5),
                &[("AbstractArrow.IN_GROUND", 10)],
            ),
            data(
                3,
                "minecraft:item",
                &["ItemEntity", "Entity"],
                (0.25, 0.25),
                &[],
            ),
            data(
                4,
                "minecraft:fireball",
                &[
                    "LargeFireball",
                    "Fireball",
                    "AbstractHurtingProjectile",
                    "Projectile",
                    "Entity",
                ],
                (1.0, 1.0),
                &[],
            ),
        ])
    }

    fn add(e: &mut Entities, id: i32, entity_type: i32, position: [f64; 3]) {
        let p = play::AddEntity {
            entity_id: id,
            uuid: 1000 + id as u128,
            entity_type,
            position,
            movement: [0.0; 3],
            x_rot: 0,
            y_rot: 0,
            y_head_rot: 0,
            data: 0,
        };
        e.add(&p, &types(), &PlayerInfos::default());
    }

    /// The player at (0.5, -60, 0.5) looking east, eyes at 1.62.
    fn camera() -> Camera {
        let feet = [0.5, -60.0, 0.5];
        Camera {
            id: PLAYER_ID,
            eye: [0.5, -60.0 + f64::from(1.62f32), 0.5],
            view: [1.0, 0.0, 0.0],
            bounding_box: Aabb::new(
                [feet[0] - 0.3, feet[1], feet[2] - 0.3],
                [feet[0] + 0.3, feet[1] + 1.8, feet[2] + 0.3],
            ),
        }
    }

    fn pick_in(level: &Level, entities: &Entities, camera: &Camera) -> HitResult {
        let blocks = blocks();
        let tags = BlockTags::new();
        let env = Env {
            level,
            blocks: &blocks,
            tags: &tags,
            colliders: &[],
            others: None,
        };
        pick(
            &env,
            entities,
            &PlayerInfos::default(),
            camera,
            "minecraft:air",
            4.5,
            3.0,
        )
    }

    /// An entity hit, its location within rounding of `near`.
    fn assert_entity(hit: HitResult, id: i32, near: [f64; 3]) {
        let HitResult::Entity { id: got, location } = hit else {
            panic!("expected entity {id}, got {hit:?}");
        };
        assert_eq!(got, id);
        assert!(
            (0..3).all(|i| (location[i] - near[i]).abs() < 1e-6),
            "{location:?} vs {near:?}"
        );
    }

    fn entities() -> Entities {
        let mut e = Entities::new(PLAYER_ID, false);
        e.chunk_loaded(0, 0);
        e
    }

    #[test]
    fn the_nearest_entity_is_picked() {
        let level = level(&[]);
        let mut e = entities();
        add(&mut e, 6, ZOMBIE, [3.0, -60.0, 0.5]);
        add(&mut e, 5, ZOMBIE, [2.0, -60.0, 0.5]);
        let eye = camera().eye;
        assert_entity(pick_in(&level, &e, &camera()), 5, [1.7, eye[1], 0.5]);
        // The second only once the first is gone.
        e.remove(5);
        assert_entity(pick_in(&level, &e, &camera()), 6, [2.7, eye[1], 0.5]);
    }

    #[test]
    fn an_entity_around_the_eyes_is_picked_there() {
        let level = level(&[]);
        let c = camera();
        let mut e = entities();
        add(&mut e, 6, ZOMBIE, [2.0, -60.0, 0.5]);
        add(&mut e, 5, ZOMBIE, [0.6, -60.0, 0.5]);
        // The ray starts in its box and never enters it: the hit is at the eyes.
        assert_eq!(
            pick_in(&level, &e, &c),
            HitResult::Entity {
                id: 5,
                location: c.eye,
            }
        );
        // Vanilla's quirk: an entity met after one around the eyes takes over (the
        // search was at distance zero), so the order the level lists them in counts.
        let mut e = entities();
        add(&mut e, 5, ZOMBIE, [0.6, -60.0, 0.5]);
        add(&mut e, 6, ZOMBIE, [2.0, -60.0, 0.5]);
        assert_entity(pick_in(&level, &e, &c), 6, [1.7, c.eye[1], 0.5]);
    }

    #[test]
    fn entities_past_their_range_are_misses() {
        let level = level(&[]);
        let mut e = entities();
        // 3.2 away: inside the block range, past the entity range of 3.
        add(&mut e, 5, ZOMBIE, [4.0, -60.0, 0.5]);
        let eye = camera().eye;
        let HitResult::Block(miss) = pick_in(&level, &e, &camera()) else {
            panic!("an entity out of range");
        };
        assert!(miss.miss && (miss.location[0] - 3.7).abs() < 1e-6);
        assert_eq!((miss.direction, miss.pos), (Direction::East, [3, -59, 0]));
        // Nothing at all: a miss at the block range's end, facing back.
        e.remove(5);
        let hit = pick_in(&level, &e, &camera());
        assert!(hit.is_miss());
        assert_eq!(hit.location(), [5.0, eye[1], 0.5]);
    }

    #[test]
    fn unpickable_entities_are_looked_through() {
        let level = level(&[]);
        let mut e = entities();
        let types = types();
        add(&mut e, 5, ITEM, [1.5, -58.5, 0.5]);
        add(&mut e, 6, ARMOR_STAND, [2.0, -60.0, 0.5]);
        e.set_data(6, &[(15, DataValue::Byte(16))], &types);
        add(&mut e, 7, ARROW, [2.5, -58.5, 0.5]);
        e.set_data(7, &[(10, DataValue::Boolean(true))], &types);
        assert!(pick_in(&level, &e, &camera()).is_miss());
        // Out of the ground an arrow still isn't (it isn't a redirectable projectile).
        e.set_motion(7, [0.1, 0.0, 0.0]);
        assert!(!e.get(7).unwrap().in_ground);
        assert!(pick_in(&level, &e, &camera()).is_miss());
        // An armor stand that isn't a marker is.
        e.set_data(6, &[(15, DataValue::Byte(0))], &types);
        let eye = camera().eye;
        assert_entity(pick_in(&level, &e, &camera()), 6, [1.75, eye[1], 0.5]);
    }

    #[test]
    fn redirectable_projectiles_are_picked_with_a_wider_box() {
        let level = level(&[]);
        let mut e = entities();
        // A fireball whose box is below the eyes, but not its box grown by 1.
        add(&mut e, 5, FIREBALL, [2.5, -60.0, 0.5]);
        let eye = camera().eye;
        assert_entity(pick_in(&level, &e, &camera()), 5, [1.0, eye[1], 0.5]);
    }

    #[test]
    fn a_block_in_front_hides_the_entity() {
        let mut e = entities();
        add(&mut e, 5, ZOMBIE, [2.5, -60.0, 0.5]);
        let level = level(&[([1, -59, 0], FULL)]);
        let HitResult::Block(hit) = pick_in(&level, &e, &camera()) else {
            panic!("the block is nearer");
        };
        assert!(!hit.miss && (hit.location[0] - 1.0).abs() < 1e-12);
        assert_eq!((hit.direction, hit.pos), (Direction::West, [1, -59, 0]));
        // Behind the entity, the block doesn't matter.
        let level = level_behind();
        assert!(matches!(
            pick_in(&level, &e, &camera()),
            HitResult::Entity { id: 5, .. }
        ));
    }

    fn level_behind() -> Level {
        level(&[([3, -59, 0], FULL)])
    }

    /// [`closest_hit`] with a spear-like range of 2 to 6 and a margin of 0.5.
    fn closest_in(level: &Level, entities: &Entities, camera: &Camera, eye: [f64; 3]) -> HitResult {
        let blocks = blocks();
        let tags = BlockTags::new();
        let env = Env {
            level,
            blocks: &blocks,
            tags: &tags,
            colliders: &[],
            others: None,
        };
        let attacker = Attacker {
            range: AttackRange {
                min_reach: 2.0,
                max_reach: 6.0,
                hitbox_margin: 0.5,
                ..AttackRange::DEFAULT
            },
            creative: false,
            eye,
            head_look: [1.0, 0.0, 0.0],
            movement: [0.0; 3],
        };
        closest_hit(
            &env,
            entities,
            &PlayerInfos::default(),
            camera,
            &attacker,
            "minecraft:air",
        )
    }

    #[test]
    fn a_block_nearer_than_the_attack_range_hides_everything() {
        let level = level(&[([1, -59, 0], FULL)]);
        let mut e = entities();
        // Between the block and the range's start, 2 out from the eyes at x 0.5.
        add(&mut e, 5, ZOMBIE, [2.5, -60.0, 0.5]);
        let eye = camera().eye;
        let HitResult::Block(hit) = closest_in(&level, &e, &camera(), eye) else {
            panic!("the block is nearer");
        };
        assert_eq!((hit.pos, hit.miss), ([1, -59, 0], false));
    }

    #[test]
    fn attack_ranges_pass_over_what_cant_be_picked() {
        let level = level(&[]);
        let mut e = entities();
        add(&mut e, 5, ITEM, [3.5, -58.5, 0.5]);
        let eye = camera().eye;
        assert!(closest_in(&level, &e, &camera(), eye).is_miss());
        add(&mut e, 6, ZOMBIE, [5.0, -60.0, 0.5]);
        assert_entity(
            closest_in(&level, &e, &camera(), eye),
            6,
            [4.7, eye[1], 0.5],
        );
    }

    #[test]
    fn the_nearest_to_the_eyes_is_attacked() {
        let level = level(&[]);
        let mut e = entities();
        let eye = camera().eye;
        // One straight ahead, and one a little nearer to the eyes off to the side, met
        // by the margin (but farther from the range's start).
        add(&mut e, 5, ZOMBIE, [3.45, -60.0, 0.5]);
        add(&mut e, 6, ZOMBIE, [3.35, -60.0, 1.25]);
        assert!(matches!(
            closest_in(&level, &e, &camera(), eye),
            HitResult::Entity { id: 6, .. }
        ));
        // It's the interpolated eyes the distance is from.
        let far = Camera {
            eye: [6.0, eye[1], 0.5],
            ..camera()
        };
        add(&mut e, 7, ZOMBIE, [5.5, -60.0, 0.5]);
        assert_entity(closest_in(&level, &e, &far, eye), 7, [5.2, eye[1], 0.5]);
    }

    #[test]
    fn the_cameras_own_passengers_are_skipped() {
        let level = level(&[]);
        let mut e = entities();
        add(&mut e, 5, ZOMBIE, [2.0, -60.0, 0.5]);
        add(&mut e, 6, ZOMBIE, [3.0, -60.0, 0.5]);
        e.set_passengers(PLAYER_ID, &[5]);
        assert!(matches!(
            pick_in(&level, &e, &camera()),
            HitResult::Entity { id: 6, .. }
        ));
    }
}

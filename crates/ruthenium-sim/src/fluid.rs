//! Fluids as movement meets them: each block state's fluid, its height and its current
//! (`FluidState`, `FlowingFluid`), and what an entity's box is in
//! (`EntityFluidInteraction`).
//!
//! The fluid tags are vanilla's (`#water` is water and flowing water, `#lava` lava
//! and flowing lava); servers can't change them in practice, so they aren't read from
//! the tags packet.

use ruthenium_javamath::mth;
use ruthenium_prepare::exporters::StatePhysics;

use crate::player::Env;
use crate::shape::Aabb;

/// `Direction` ordinals.
pub const DOWN: usize = 0;
pub const NORTH: usize = 2;
pub const SOUTH: usize = 3;
pub const WEST: usize = 4;
pub const EAST: usize = 5;

/// `Direction.Plane.HORIZONTAL` in its order (north, east, south, west): ordinal and
/// step.
const HORIZONTAL: [(usize, [i32; 3]); 4] = [
    (NORTH, [0, 0, -1]),
    (EAST, [1, 0, 0]),
    (SOUTH, [0, 0, 1]),
    (WEST, [-1, 0, 0]),
];

/// A fluid type (`Fluids`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fluid {
    Empty,
    Water,
    FlowingWater,
    Lava,
    FlowingLava,
}

impl Fluid {
    /// In `#minecraft:water`.
    pub fn is_water(self) -> bool {
        matches!(self, Fluid::Water | Fluid::FlowingWater)
    }

    /// In `#minecraft:lava`.
    pub fn is_lava(self) -> bool {
        matches!(self, Fluid::Lava | Fluid::FlowingLava)
    }

    /// `Fluid.isSame`: source and flowing of one fluid are the same.
    pub fn is_same(self, other: Fluid) -> bool {
        self == other || self.is_water() && other.is_water() || self.is_lava() && other.is_lava()
    }
}

/// A block's fluid state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FluidState {
    pub fluid: Fluid,
    /// `getAmount`: 8 for sources, 1 to 8 for flowing fluid.
    pub amount: u8,
    /// The `falling` property.
    pub falling: bool,
}

impl FluidState {
    pub const EMPTY: FluidState = FluidState {
        fluid: Fluid::Empty,
        amount: 0,
        falling: false,
    };

    /// From the export's byte.
    pub fn from_bits(b: u8) -> FluidState {
        let fluid = match b & 7 {
            StatePhysics::FLUID_WATER => Fluid::Water,
            StatePhysics::FLUID_FLOWING_WATER => Fluid::FlowingWater,
            StatePhysics::FLUID_LAVA => Fluid::Lava,
            StatePhysics::FLUID_FLOWING_LAVA => Fluid::FlowingLava,
            _ => return FluidState::EMPTY,
        };
        FluidState {
            fluid,
            amount: b >> 4,
            falling: b & 8 != 0,
        }
    }

    pub fn is_empty(self) -> bool {
        self.fluid == Fluid::Empty
    }

    pub fn is_source(self) -> bool {
        matches!(self.fluid, Fluid::Water | Fluid::Lava)
    }

    /// `getOwnHeight`: the amount in ninths, as a float.
    pub fn own_height(self) -> f32 {
        if self.is_empty() {
            0.0
        } else {
            f32::from(self.amount) / 9.0
        }
    }
}

impl Env<'_> {
    /// `getFluidState` at a position: none outside the level or where no chunk is.
    pub fn fluid(&self, p: [i32; 3]) -> FluidState {
        self.blocks.fluid(self.level.block(p[0], p[1], p[2]))
    }

    /// `FluidState.getHeight`: the whole block when the same fluid is above.
    pub fn fluid_height(&self, p: [i32; 3], f: FluidState) -> f32 {
        if f.is_empty() {
            return 0.0;
        }
        if f.fluid.is_same(self.fluid([p[0], p[1] + 1, p[2]]).fluid) {
            1.0
        } else {
            f.own_height()
        }
    }

    /// `FluidState.getHeightForCamera`: a source under a block whose bottom face is
    /// sturdy fills the block.
    pub fn fluid_height_for_camera(&self, p: [i32; 3], f: FluidState) -> f32 {
        if f.is_source() {
            let above = self.level.block(p[0], p[1] + 1, p[2]);
            if self.blocks.sturdy(above, DOWN) {
                return 1.0;
            }
        }
        self.fluid_height(p, f)
    }

    /// `FlowingFluid.getFlow`: which way the fluid at `p` pushes, normalised.
    pub fn fluid_flow(&self, p: [i32; 3], f: FluidState) -> [f64; 3] {
        if f.is_empty() {
            return [0.0; 3];
        }
        let affects_flow = |n: FluidState| n.is_empty() || n.fluid.is_same(f.fluid);
        let (mut flow_x, mut flow_z) = (0.0f64, 0.0f64);
        for (_, step) in HORIZONTAL {
            let n = [p[0] + step[0], p[1], p[2] + step[2]];
            let neighbour = self.fluid(n);
            if !affects_flow(neighbour) {
                continue;
            }
            let mut neighbour_height = neighbour.own_height();
            let mut distance = 0.0f32;
            if neighbour_height == 0.0 {
                let state = self.level.block(n[0], n[1], n[2]);
                if !self.tags.blocks_fluid_flow(state) {
                    let below = self.fluid([n[0], n[1] - 1, n[2]]);
                    if affects_flow(below) {
                        neighbour_height = below.own_height();
                        if neighbour_height > 0.0 {
                            distance = f.own_height() - (neighbour_height - 0.8888889);
                        }
                    }
                }
            } else if neighbour_height > 0.0 {
                distance = f.own_height() - neighbour_height;
            }
            if distance != 0.0 {
                flow_x += f64::from(step[0] as f32 * distance);
                flow_z += f64::from(step[2] as f32 * distance);
            }
        }
        let mut flow = [flow_x, 0.0, flow_z];
        if f.falling {
            for (direction, step) in HORIZONTAL {
                let n = [p[0] + step[0], p[1], p[2] + step[2]];
                if self.solid_face(n, direction, f)
                    || self.solid_face([n[0], n[1] + 1, n[2]], direction, f)
                {
                    let v = normalize(flow);
                    flow = [v[0], v[1] - 6.0, v[2]];
                    break;
                }
            }
        }
        normalize(flow)
    }

    /// `FlowingFluid.isSolidFace` for a horizontal direction.
    fn solid_face(&self, p: [i32; 3], direction: usize, f: FluidState) -> bool {
        let state = self.level.block(p[0], p[1], p[2]);
        if self.blocks.fluid(state).fluid.is_same(f.fluid) {
            return false;
        }
        !self.blocks.is_ice(state) && self.blocks.sturdy(state, direction)
    }

    /// `LevelReader.containsAnyLiquid`: a fluid in any block the box touches.
    pub fn contains_any_liquid(&self, b: &Aabb) -> bool {
        let lo = b.min.map(mth::floor);
        let hi = b.max.map(mth::floor);
        for x in lo[0]..=hi[0] {
            for y in lo[1]..=hi[1] {
                for z in lo[2]..=hi[2] {
                    if !self.fluid([x, y, z]).is_empty() {
                        return true;
                    }
                }
            }
        }
        false
    }
}

/// `Vec3.normalize`.
pub fn normalize(v: [f64; 3]) -> [f64; 3] {
    let d = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    if d < 9.999999747378752E-6 {
        [0.0; 3]
    } else {
        [v[0] / d, v[1] / d, v[2] / d]
    }
}

/// What one fluid type's blocks reach (`EntityFluidInteraction.Tracker`).
#[derive(Debug, Clone, Copy, PartialEq)]
struct Tracker {
    fluid: Fluid,
    height: f64,
    eyes_inside: bool,
}

/// The current of one fluid tag (`CurrentAccumulator`).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
struct Current {
    height: f64,
    sum: [f64; 3],
    count: i32,
}

impl Current {
    /// `CurrentAccumulator.applyTo` for a player: the average flow, scaled, with a
    /// minimum push when nearly still.
    fn apply_to(&self, delta: &mut [f64; 3], scale: f64) {
        let s = self.sum;
        if self.count == 0 || s[0] * s[0] + s[1] * s[1] + s[2] * s[2] < 9.999999747378752E-6 {
            return;
        }
        let inv = 1.0 / f64::from(self.count);
        let mut impulse = [s[0] * inv * scale, s[1] * inv * scale, s[2] * inv * scale];
        let length =
            (impulse[0] * impulse[0] + impulse[1] * impulse[1] + impulse[2] * impulse[2]).sqrt();
        if delta[0].abs() < 0.003 && delta[2].abs() < 0.003 && length < 0.0045000000000000005 {
            let n = normalize(impulse);
            impulse = [
                n[0] * 0.0045000000000000005,
                n[1] * 0.0045000000000000005,
                n[2] * 0.0045000000000000005,
            ];
        }
        for i in 0..3 {
            delta[i] += impulse[i];
        }
    }
}

/// `EntityFluidInteraction`: how deep in each fluid an entity's box is, whether its
/// eyes are under the surface, and the current pushing it.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct FluidInteraction {
    trackers: Vec<Tracker>,
    water: Current,
    lava: Current,
}

impl FluidInteraction {
    /// `update`: `entity` is the entity's bounding box, `eye` its block x, eye y and
    /// block z. Whether any fluid was found.
    pub fn update(
        &mut self,
        env: &Env<'_>,
        entity: &Aabb,
        eye: (i32, f64, i32),
        ignore_current: bool,
    ) -> bool {
        self.trackers.retain_mut(|t| {
            if t.height == 0.0 && !t.eyes_inside {
                return false;
            }
            t.height = 0.0;
            t.eyes_inside = false;
            true
        });
        self.water = Current::default();
        self.lava = Current::default();
        let b = entity.inflate([-0.001; 3]);
        let (x0, y0, z0) = (
            mth::floor(b.min[0]),
            mth::floor(b.min[1]),
            mth::floor(b.min[2]),
        );
        let (x1, y1, z1) = (
            mth::ceil(b.max[0]) - 1,
            mth::ceil(b.max[1]) - 1,
            mth::ceil(b.max[2]) - 1,
        );
        // hasFluidAndLoaded: every chunk around loaded (sections without fluid only
        // skip the search, which would find nothing in them anyway).
        for cz in (z0 - 1) >> 4..=(z1 + 1) >> 4 {
            for cx in (x0 - 1) >> 4..=(x1 + 1) >> 4 {
                if !env.level.has_chunk(cx, cz) {
                    return false;
                }
            }
        }
        let entity_y = entity.min[1];
        let (eye_x, eye_y, eye_z) = eye;
        let mut last: Option<Fluid> = None;
        let mut tracker = 0usize;
        let mut current: Option<bool> = None; // Some(true): water's, Some(false): lava's
        for x in x0..=x1 {
            for y in y0..=y1 {
                for z in z0..=z1 {
                    let p = [x, y, z];
                    let f = env.fluid(p);
                    if f.is_empty() {
                        continue;
                    }
                    let bottom = f64::from(y);
                    let top = bottom + f64::from(env.fluid_height(p, f));
                    if top < b.min[1] {
                        continue;
                    }
                    if last != Some(f.fluid) {
                        last = Some(f.fluid);
                        tracker = match self.trackers.iter().position(|t| t.fluid == f.fluid) {
                            Some(i) => i,
                            None => {
                                self.trackers.push(Tracker {
                                    fluid: f.fluid,
                                    height: 0.0,
                                    eyes_inside: false,
                                });
                                self.trackers.len() - 1
                            }
                        };
                        if !ignore_current {
                            current = Some(f.fluid.is_water());
                        }
                    }
                    let t = &mut self.trackers[tracker];
                    if x == eye_x && z == eye_z && eye_y >= bottom {
                        let camera_top = bottom + f64::from(env.fluid_height_for_camera(p, f));
                        if eye_y <= camera_top {
                            t.eyes_inside = true;
                        }
                    }
                    t.height = (top - entity_y).max(t.height);
                    let height = t.height;
                    if let Some(water) = current {
                        let mut flow = env.fluid_flow(p, f);
                        let c = if water {
                            &mut self.water
                        } else {
                            &mut self.lava
                        };
                        c.height = height.max(c.height);
                        if c.height < 0.4 {
                            flow = [flow[0] * c.height, flow[1] * c.height, flow[2] * c.height];
                        }
                        for i in 0..3 {
                            c.sum[i] += flow[i];
                        }
                        c.count += 1;
                    }
                }
            }
        }
        last.is_some()
    }

    /// `getFluidHeight`: the deepest of a tag's fluids (water's when `water`, else
    /// lava's).
    pub fn height(&self, water: bool) -> f64 {
        let mut h = 0.0;
        for t in &self.trackers {
            let is = if water {
                t.fluid.is_water()
            } else {
                t.fluid.is_lava()
            };
            if t.height > h && is {
                h = t.height;
            }
        }
        h
    }

    /// `isInFluid`.
    pub fn is_in(&self, water: bool) -> bool {
        self.height(water) > 0.0
    }

    /// `isEyeInFluid`.
    pub fn eye_in(&self, water: bool) -> bool {
        self.trackers.iter().any(|t| {
            t.eyes_inside
                && if water {
                    t.fluid.is_water()
                } else {
                    t.fluid.is_lava()
                }
        })
    }

    /// `applyCurrentTo`.
    pub fn apply_current(&self, water: bool, delta: &mut [f64; 3], scale: f64) {
        if water {
            self.water.apply_to(delta, scale);
        } else {
            self.lava.apply_to(delta, scale);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fluid_bytes() {
        assert_eq!(FluidState::from_bits(0), FluidState::EMPTY);
        let s = FluidState::from_bits(1 | 8 << 4);
        assert!(s.is_source() && s.fluid.is_water() && !s.falling);
        assert_eq!(s.own_height(), 8.0 / 9.0);
        let f = FluidState::from_bits(2 | 8 | 3 << 4);
        assert!(!f.is_source() && f.falling && f.amount == 3);
        assert!(f.fluid.is_same(Fluid::Water) && !f.fluid.is_same(Fluid::Lava));
    }
}

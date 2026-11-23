//! The world border as the client keeps it (vanilla's `WorldBorder` on the
//! `ClientLevel`): a square around a centre, standing or moving from one size to
//! another a tick at a time, set by the server's border packets.
//!
//! Entities near it collide with it as with a block (`collectColliders`, its shape
//! everything outside the blocks it covers); the local player can't break, use or
//! interact with what lies outside it, a piston can't push there, and a bed's head
//! can't go there. Its warnings (the red vignette) and the damage outside it are the
//! renderer's and the server's.

use ruthenium_prepare::exporters::ShapeData;
use ruthenium_protocol::packets::play::Clientbound;

use crate::shape::{Aabb, Shape};

/// `WorldBorder.MAX_SIZE`: the size of a border nobody set.
pub const MAX_SIZE: f64 = 5.9999968E7;

/// `Mth.clamp(double, double, double)`.
fn clamp(v: f64, min: f64, max: f64) -> f64 {
    if v < min { min } else { v.min(max) }
}

/// `Mth.lerp(double, double, double)`.
fn lerp(delta: f64, a: f64, b: f64) -> f64 {
    a + delta * (b - a)
}

/// `BorderExtent`: a standing size, or one moving towards another.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Extent {
    /// `StaticBorderExtent`.
    Static { size: f64 },
    /// `MovingBorderExtent`: `lerp_progress` ticks to go of `lerp_duration`.
    Moving {
        from: f64,
        to: f64,
        lerp_duration: f64,
        lerp_progress: i64,
        size: f64,
        previous_size: f64,
    },
}

impl Extent {
    /// `MovingBorderExtent`'s constructor (`calculateSize` with all the ticks to go).
    fn moving(from: f64, to: f64, duration: i64) -> Extent {
        let mut e = Extent::Moving {
            from,
            to,
            lerp_duration: duration as f64,
            lerp_progress: duration,
            size: 0.0,
            previous_size: 0.0,
        };
        let size = e.calculate_size();
        if let Extent::Moving {
            size: s,
            previous_size,
            ..
        } = &mut e
        {
            *s = size;
            *previous_size = size;
        }
        e
    }

    /// `calculateSize`: how far along the move is, the target once it's done.
    fn calculate_size(&self) -> f64 {
        match *self {
            Extent::Static { size } => size,
            Extent::Moving {
                from,
                to,
                lerp_duration,
                lerp_progress,
                ..
            } => {
                let progress = (lerp_duration - lerp_progress as f64) / lerp_duration;
                if progress < 1.0 {
                    lerp(progress, from, to)
                } else {
                    to
                }
            }
        }
    }

    /// The size the bounds use without a partial tick: a moving border's size of the
    /// tick before (`Mth.lerp(0, previousSize, size)`).
    fn bounds_size(&self) -> f64 {
        match *self {
            Extent::Static { size } => size,
            Extent::Moving {
                size,
                previous_size,
                ..
            } => lerp(0.0, previous_size, size),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct WorldBorder {
    center: [f64; 2],
    absolute_max_size: i32,
    /// How many seconds ahead of a shrinking border its warning shows.
    pub warning_time: i32,
    /// How many blocks from the border its warning shows.
    pub warning_blocks: i32,
    extent: Extent,
}

impl Default for WorldBorder {
    /// A new client level's border (`new WorldBorder()`'s fields: the server's
    /// settings only come with its packets).
    fn default() -> WorldBorder {
        WorldBorder {
            center: [0.0; 2],
            absolute_max_size: 29999984,
            warning_time: 15,
            warning_blocks: 5,
            extent: Extent::Static { size: MAX_SIZE },
        }
    }
}

impl WorldBorder {
    /// `getMinX`, `getMinZ`, `getMaxX`, `getMaxZ` (no partial tick).
    pub fn min_x(&self) -> f64 {
        self.bound(0, -1.0)
    }

    pub fn min_z(&self) -> f64 {
        self.bound(1, -1.0)
    }

    pub fn max_x(&self) -> f64 {
        self.bound(0, 1.0)
    }

    pub fn max_z(&self) -> f64 {
        self.bound(1, 1.0)
    }

    fn bound(&self, axis: usize, side: f64) -> f64 {
        let half = self.extent.bounds_size() / 2.0;
        let edge = if side < 0.0 {
            self.center[axis] - half
        } else {
            self.center[axis] + half
        };
        let max = f64::from(self.absolute_max_size);
        clamp(edge, -max, max)
    }

    /// `getCenterX` and `getCenterZ`.
    pub fn center(&self) -> [f64; 2] {
        self.center
    }

    /// `getSize`.
    pub fn size(&self) -> f64 {
        match self.extent {
            Extent::Static { size } | Extent::Moving { size, .. } => size,
        }
    }

    /// `getLerpTarget`: where a moving border is going.
    pub fn lerp_target(&self) -> f64 {
        match self.extent {
            Extent::Static { size } => size,
            Extent::Moving { to, .. } => to,
        }
    }

    /// `getLerpTime`: ticks left of a move.
    pub fn lerp_time(&self) -> i64 {
        match self.extent {
            Extent::Static { .. } => 0,
            Extent::Moving { lerp_progress, .. } => lerp_progress,
        }
    }

    /// `setCenter`.
    pub fn set_center(&mut self, x: f64, z: f64) {
        self.center = [x, z];
    }

    /// `setSize`: standing at `size`.
    pub fn set_size(&mut self, size: f64) {
        self.extent = Extent::Static { size };
    }

    /// `lerpSizeBetween`: from one size to another over `ticks` (standing when they
    /// are the same).
    pub fn lerp_size_between(&mut self, from: f64, to: f64, ticks: i64) {
        self.extent = if from == to {
            Extent::Static { size: to }
        } else {
            Extent::moving(from, to, ticks)
        };
    }

    /// `setAbsoluteMaxSize`.
    pub fn set_absolute_max_size(&mut self, size: i32) {
        self.absolute_max_size = size;
    }

    /// The border packets' handlers (`handleInitializeBorder`, `handleSetBorderCenter`,
    /// ...); false for any other packet.
    pub fn handle(&mut self, p: &Clientbound) -> bool {
        match *p {
            Clientbound::InitializeBorder {
                center,
                old_size,
                new_size,
                lerp_time,
                absolute_max_size,
                warning_blocks,
                warning_time,
            } => {
                // `handleInitializeBorder`.
                self.set_center(center[0], center[1]);
                if lerp_time > 0 {
                    self.lerp_size_between(old_size, new_size, lerp_time);
                } else {
                    self.set_size(new_size);
                }
                self.set_absolute_max_size(absolute_max_size);
                self.warning_blocks = warning_blocks;
                self.warning_time = warning_time;
            }
            Clientbound::SetBorderCenter { center } => self.set_center(center[0], center[1]),
            Clientbound::SetBorderLerpSize {
                old_size,
                new_size,
                lerp_time,
            } => self.lerp_size_between(old_size, new_size, lerp_time),
            Clientbound::SetBorderSize { size } => self.set_size(size),
            Clientbound::SetBorderWarningDelay { warning_time } => self.warning_time = warning_time,
            Clientbound::SetBorderWarningDistance { warning_blocks } => {
                self.warning_blocks = warning_blocks
            }
            _ => return false,
        }
        true
    }

    /// `tick` (the level's, while the game runs): a moving border takes a step, and
    /// stands once it's there.
    pub fn tick(&mut self) {
        let Extent::Moving {
            lerp_progress,
            size,
            previous_size,
            ..
        } = &mut self.extent
        else {
            return;
        };
        *lerp_progress -= 1;
        *previous_size = *size;
        let next = self.extent.calculate_size();
        if let Extent::Moving {
            to,
            lerp_progress,
            size,
            ..
        } = &mut self.extent
        {
            *size = next;
            if *lerp_progress <= 0 {
                self.extent = Extent::Static { size: *to };
            }
        }
    }

    /// `isWithinBounds(x, z, margin)`.
    pub fn is_within_bounds_margin(&self, x: f64, z: f64, margin: f64) -> bool {
        x >= self.min_x() - margin
            && x < self.max_x() + margin
            && z >= self.min_z() - margin
            && z < self.max_z() + margin
    }

    /// `isWithinBounds(x, z)`.
    pub fn is_within_bounds(&self, x: f64, z: f64) -> bool {
        self.is_within_bounds_margin(x, z, 0.0)
    }

    /// `isWithinBounds(BlockPos)`: the block's corner.
    pub fn block_within_bounds(&self, pos: [i32; 3]) -> bool {
        self.is_within_bounds(f64::from(pos[0]), f64::from(pos[2]))
    }

    /// `isWithinBounds(AABB)`: both corners (the far one a hair inside).
    pub fn box_within_bounds(&self, b: &Aabb) -> bool {
        self.is_within_bounds(b.min[0], b.min[2])
            && self.is_within_bounds(
                b.max[0] - 9.999999747378752E-6,
                b.max[2] - 9.999999747378752E-6,
            )
    }

    /// `getDistanceToBorder(x, z)`: to the nearest side (negative outside).
    pub fn distance_to_border(&self, x: f64, z: f64) -> f64 {
        let from_north = z - self.min_z();
        let from_south = self.max_z() - z;
        let from_west = x - self.min_x();
        let from_east = self.max_x() - x;
        let min = from_west.min(from_east);
        let min = min.min(from_north);
        min.min(from_south)
    }

    /// `isInsideCloseToBorder`: an entity at `xz` moving through `area` is near enough
    /// the border, and inside it, to collide with it.
    pub fn is_inside_close_to_border(&self, xz: [f64; 2], area: &Aabb) -> bool {
        let xs = area.max[0] - area.min[0];
        let zs = area.max[2] - area.min[2];
        let bb_max = xs.abs().max(zs.abs()).max(1.0);
        self.distance_to_border(xz[0], xz[1]) < bb_max * 2.0
            && self.is_within_bounds_margin(xz[0], xz[1], bb_max)
    }

    /// `getCollisionShape`: everything outside the whole blocks the border covers,
    /// as `Shapes.join(INFINITY, box, ONLY_FIRST)` builds it.
    pub fn collision_shape(&self) -> Shape {
        let (x0, x1) = (self.min_x().floor(), self.max_x().ceil());
        let (z0, z1) = (self.min_z().floor(), self.max_z().ceil());
        let inf = f64::INFINITY;
        // `Shapes.box` of an empty box is `Shapes.empty()`: all of space is outside.
        if x1 - x0 < 1.0E-7 || z1 - z0 < 1.0E-7 {
            return Shape::new(&ShapeData {
                coords: [vec![-inf, inf], vec![-inf, inf], vec![-inf, inf]],
                full: vec![true],
            });
        }
        let mut full = Vec::with_capacity(9);
        for x in 0..3 {
            for z in 0..3 {
                full.push(!(x == 1 && z == 1));
            }
        }
        Shape::new(&ShapeData {
            coords: [
                vec![-inf, x0, x1, inf],
                vec![-inf, inf],
                vec![-inf, z0, z1, inf],
            ],
            full,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shape::{Axis, Placed};

    fn player_at(x: f64, z: f64) -> Aabb {
        Aabb::new([x - 0.3, 0.0, z - 0.3], [x + 0.3, 1.8, z + 0.3])
    }

    #[test]
    fn a_border_stops_the_player_at_its_edge() {
        let mut b = WorldBorder::default();
        b.set_center(0.5, 0.5);
        b.set_size(10.0);
        assert_eq!((b.min_x(), b.max_x()), (-4.5, 5.5));
        let shape = b.collision_shape();
        // The border's shape covers whole blocks: from x -5 to 6.
        let placed = Placed {
            shape: &shape,
            offset: None,
            pos: [0.0; 3],
        };
        let p = player_at(5.0, 0.5);
        assert!(b.is_inside_close_to_border([5.0, 0.5], &p.expand_towards([1.0, 0.0, 0.0])));
        assert_eq!(placed.collide(Axis::X, &p, 1.0), 6.0 - 5.3);
        assert_eq!(placed.collide(Axis::Y, &p, -1.0), -1.0);
        assert!(!placed.overlaps(&p));
        // Far from it, it isn't in the way.
        assert!(!b.is_inside_close_to_border([0.5, 0.5], &player_at(0.5, 0.5)));
        // Outside it, nor is it.
        assert!(!b.is_inside_close_to_border([9.0, 0.5], &player_at(9.0, 0.5)));
    }

    #[test]
    fn a_moving_border_steps_each_tick_and_its_bounds_lag_a_tick() {
        let mut b = WorldBorder::default();
        b.lerp_size_between(10.0, 6.0, 4);
        assert_eq!((b.size(), b.min_x()), (10.0, -5.0));
        b.tick();
        assert_eq!((b.size(), b.min_x()), (9.0, -5.0));
        b.tick();
        assert_eq!((b.size(), b.min_x()), (8.0, -4.5));
        b.tick();
        b.tick();
        assert_eq!((b.size(), b.min_x(), b.lerp_time()), (6.0, -3.0, 0));
        b.lerp_size_between(6.0, 6.0, 20);
        assert_eq!(b.lerp_time(), 0);
    }

    #[test]
    fn blocks_on_the_edge_are_inside_on_the_low_side_only() {
        let mut b = WorldBorder::default();
        b.set_size(4.0);
        assert!(b.block_within_bounds([-2, 0, 1]));
        assert!(!b.block_within_bounds([2, 0, 1]));
        assert!(!b.block_within_bounds([-3, 0, 1]));
        assert!(b.box_within_bounds(&Aabb::new([1.0, 0.0, 1.0], [2.0, 1.0, 2.0])));
    }
}

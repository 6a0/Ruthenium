//! Collision shapes as the game sees them (`world/phys/shapes`): a grid of coordinates
//! per axis and which cells are full, and the one operation movement needs,
//! `VoxelShape.collide`: how far a box can move along an axis before it touches the
//! shape. Ported to give the same doubles, epsilons and all.

use ruthenium_prepare::exporters::ShapeData;

/// `Shapes.EPSILON`, the tolerance throughout the collision code.
pub const EPSILON: f64 = 1.0E-7;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Axis {
    X = 0,
    Y = 1,
    Z = 2,
}

impl Axis {
    pub const ALL: [Axis; 3] = [Axis::X, Axis::Y, Axis::Z];
}

/// `AABB`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Aabb {
    pub min: [f64; 3],
    pub max: [f64; 3],
}

impl Aabb {
    /// Corners in any order, as `new AABB(...)` sorts them.
    pub fn new(a: [f64; 3], b: [f64; 3]) -> Aabb {
        Aabb {
            min: [a[0].min(b[0]), a[1].min(b[1]), a[2].min(b[2])],
            max: [a[0].max(b[0]), a[1].max(b[1]), a[2].max(b[2])],
        }
    }

    pub fn moved(&self, d: [f64; 3]) -> Aabb {
        Aabb {
            min: [self.min[0] + d[0], self.min[1] + d[1], self.min[2] + d[2]],
            max: [self.max[0] + d[0], self.max[1] + d[1], self.max[2] + d[2]],
        }
    }

    /// `expandTowards`: grows the box on the side the movement goes.
    pub fn expand_towards(&self, d: [f64; 3]) -> Aabb {
        let mut b = *self;
        for i in 0..3 {
            if d[i] < 0.0 {
                b.min[i] += d[i];
            } else if d[i] > 0.0 {
                b.max[i] += d[i];
            }
        }
        b
    }

    /// `inflate`.
    pub fn inflate(&self, d: [f64; 3]) -> Aabb {
        Aabb::new(
            [self.min[0] - d[0], self.min[1] - d[1], self.min[2] - d[2]],
            [self.max[0] + d[0], self.max[1] + d[1], self.max[2] + d[2]],
        )
    }

    /// `intersects` with another box, open on all sides.
    pub fn intersects(&self, o: &Aabb) -> bool {
        (0..3).all(|i| self.min[i] < o.max[i] && self.max[i] > o.min[i])
    }
}

/// A shape's grid: coordinates per axis and full cells (x outermost, z innermost).
#[derive(Debug, Clone, PartialEq)]
pub struct Shape {
    pub coords: [Vec<f64>; 3],
    sizes: [usize; 3],
    full: Vec<bool>,
    empty: bool,
}

impl Shape {
    pub fn new(data: &ShapeData) -> Shape {
        let sizes = [
            data.coords[0].len() - 1,
            data.coords[1].len() - 1,
            data.coords[2].len() - 1,
        ];
        Shape {
            coords: data.coords.clone(),
            sizes,
            empty: !data.full.iter().any(|&f| f),
            full: data.full.clone(),
        }
    }

    /// `Shapes.block()`: one full cell from 0 to 1.
    pub fn block() -> Shape {
        Shape::new(&ShapeData {
            coords: [vec![0.0, 1.0], vec![0.0, 1.0], vec![0.0, 1.0]],
            full: vec![true],
        })
    }

    pub fn is_empty(&self) -> bool {
        self.empty
    }

    /// Whether this is the shape of a full block (the game's `Shapes.block()`).
    pub fn is_block(&self) -> bool {
        self.sizes == [1, 1, 1]
            && self.full[0]
            && self.coords.iter().all(|c| c[0] == 0.0 && c[1] == 1.0)
    }

    pub fn size(&self, axis: Axis) -> usize {
        self.sizes[axis as usize]
    }

    /// `DiscreteVoxelShape.isFullWide`: out of range is not full.
    pub(crate) fn is_full_wide(&self, x: isize, y: isize, z: isize) -> bool {
        if x < 0 || y < 0 || z < 0 {
            return false;
        }
        let (x, y, z) = (x as usize, y as usize, z as usize);
        if x >= self.sizes[0] || y >= self.sizes[1] || z >= self.sizes[2] {
            return false;
        }
        self.full[(x * self.sizes[1] + y) * self.sizes[2] + z]
    }

    /// `forAllBoxes` with neighbours merged (`BitSetDiscreteVoxelShape.forAllBoxes`):
    /// the full cells as boxes of cell indices `[x1, y1, z1, x2, y2, z2]` (ends
    /// exclusive), in vanilla's order: y outermost, then x; each run along z grows
    /// along x, then along y, clearing the cells it takes.
    pub fn boxes(&self) -> Vec<[usize; 6]> {
        let [xs, ys, zs] = self.sizes;
        let mut full = self.full.clone();
        let index = |x: usize, y: usize, z: usize| (x * ys + y) * zs + z;
        // `isZStripFull`: cells start_z..end_z of the column at (x, y) are all full.
        let strip_full = |full: &[bool], start: usize, end: usize, x: usize, y: usize| {
            x < xs && y < ys && (start..end).all(|z| full[index(x, y, z)])
        };
        let clear_strip = |full: &mut [bool], start: usize, end: usize, x: usize, y: usize| {
            for z in start..end {
                full[index(x, y, z)] = false;
            }
        };
        let mut out = Vec::new();
        for y in 0..ys {
            for x in 0..xs {
                let mut start_z: Option<usize> = None;
                for z in 0..=zs {
                    if z < zs && full[index(x, y, z)] {
                        start_z.get_or_insert(z);
                    } else if let Some(start) = start_z.take() {
                        let mut end_x = x;
                        let mut end_y = y;
                        clear_strip(&mut full, start, z, x, y);
                        while strip_full(&full, start, z, end_x + 1, y) {
                            clear_strip(&mut full, start, z, end_x + 1, y);
                            end_x += 1;
                        }
                        // `isXZRectangleFull` one layer up.
                        while (x..=end_x).all(|cx| strip_full(&full, start, z, cx, end_y + 1)) {
                            for cx in x..=end_x {
                                clear_strip(&mut full, start, z, cx, end_y + 1);
                            }
                            end_y += 1;
                        }
                        out.push([x, y, start, end_x + 1, end_y + 1, z]);
                    }
                }
            }
        }
        out
    }
}

/// A shape placed in the world: its coordinates are `(base + offset) + position`, as
/// the game nests `OffsetDoubleList`s (`offset` only for blocks offset by position).
#[derive(Debug, Clone, Copy)]
pub struct Placed<'a> {
    pub shape: &'a Shape,
    pub offset: Option<[f64; 3]>,
    pub pos: [f64; 3],
}

impl Placed<'_> {
    /// A coordinate of the shape as the block gives it, before its position: offset
    /// for blocks offset by position (`getCoords` of the moved shape).
    pub fn local(&self, axis: Axis, i: usize) -> f64 {
        let base = self.shape.coords[axis as usize][i];
        match self.offset {
            Some(o) => base + o[axis as usize],
            None => base,
        }
    }

    /// `findIndex` against [`Placed::local`] coordinates (a coordinate relative to
    /// the block's position).
    pub fn find_index_local(&self, axis: Axis, coord: f64) -> isize {
        let mut from = 0usize;
        let mut len = self.shape.size(axis) + 1;
        while len > 0 {
            let half = len / 2;
            let middle = from + half;
            if coord < self.local(axis, middle) {
                len = half;
            } else {
                from = middle + 1;
                len -= half + 1;
            }
        }
        from as isize - 1
    }

    pub fn get(&self, axis: Axis, i: usize) -> f64 {
        let a = axis as usize;
        let base = self.shape.coords[a][i];
        match self.offset {
            Some(o) => (base + o[a]) + self.pos[a],
            None => base + self.pos[a],
        }
    }

    /// The coordinates along an axis (`getCoords`).
    pub fn coords(&self, axis: Axis) -> impl Iterator<Item = f64> + '_ {
        (0..self.shape.coords[axis as usize].len()).map(move |i| self.get(axis, i))
    }

    /// `findIndex`: the cell a coordinate falls in (-1 before the first, the size
    /// past the last), by `Mth.binarySearch`.
    fn find_index(&self, axis: Axis, coord: f64) -> isize {
        let mut from = 0usize;
        let mut len = self.shape.size(axis) + 1;
        while len > 0 {
            let half = len / 2;
            let middle = from + half;
            if coord < self.get(axis, middle) {
                len = half;
            } else {
                from = middle + 1;
                len -= half + 1;
            }
        }
        from as isize - 1
    }

    /// `VoxelShape.collide`: `distance` along `axis`, cut short where `moving` would
    /// enter the shape.
    pub fn collide(&self, axis: Axis, moving: &Aabb, mut distance: f64) -> f64 {
        if self.shape.is_empty() {
            return distance;
        }
        if distance.abs() < EPSILON {
            return 0.0;
        }
        // `AxisCycle.between(axis, X).inverse()`: a is the moving axis.
        let (a_axis, b_axis, c_axis) = match axis {
            Axis::X => (Axis::X, Axis::Y, Axis::Z),
            Axis::Y => (Axis::Y, Axis::Z, Axis::X),
            Axis::Z => (Axis::Z, Axis::X, Axis::Y),
        };
        let (ai, bi, ci) = (a_axis as usize, b_axis as usize, c_axis as usize);
        let max_a = moving.max[ai];
        let min_a = moving.min[ai];
        let a_min = self.find_index(a_axis, min_a + EPSILON);
        let a_max = self.find_index(a_axis, max_a - EPSILON);
        let b_min = self.find_index(b_axis, moving.min[bi] + EPSILON).max(0);
        let b_max = (self.find_index(b_axis, moving.max[bi] - EPSILON) + 1)
            .min(self.shape.size(b_axis) as isize);
        let c_min = self.find_index(c_axis, moving.min[ci] + EPSILON).max(0);
        let c_max = (self.find_index(c_axis, moving.max[ci] - EPSILON) + 1)
            .min(self.shape.size(c_axis) as isize);
        let a_size = self.shape.size(a_axis) as isize;
        let full = |a: isize, b: isize, c: isize| {
            let mut idx = [0isize; 3];
            idx[ai] = a;
            idx[bi] = b;
            idx[ci] = c;
            self.shape.is_full_wide(idx[0], idx[1], idx[2])
        };
        if distance > 0.0 {
            for a in a_max + 1..a_size {
                for b in b_min..b_max {
                    for c in c_min..c_max {
                        if full(a, b, c) {
                            let d = self.get(a_axis, a as usize) - max_a;
                            if d >= -EPSILON {
                                distance = distance.min(d);
                            }
                            return distance;
                        }
                    }
                }
            }
        } else if distance < 0.0 {
            let mut a = a_min - 1;
            while a >= 0 {
                for b in b_min..b_max {
                    for c in c_min..c_max {
                        if full(a, b, c) {
                            let d = self.get(a_axis, a as usize + 1) - min_a;
                            if d <= EPSILON {
                                distance = distance.max(d);
                            }
                            return distance;
                        }
                    }
                }
                a -= 1;
            }
        }
        distance
    }

    /// Whether a full cell overlaps `b` by more than the merge tolerance on every axis
    /// (`Shapes.joinIsNotEmpty(shape, Shapes.create(box), AND)`).
    pub fn overlaps(&self, b: &Aabb) -> bool {
        if self.shape.is_empty() {
            return false;
        }
        let s = self.shape.sizes;
        for x in 0..s[0] {
            let (x0, x1) = (self.get(Axis::X, x), self.get(Axis::X, x + 1));
            if x0.max(b.min[0]) >= x1.min(b.max[0]) - EPSILON {
                continue;
            }
            for y in 0..s[1] {
                let (y0, y1) = (self.get(Axis::Y, y), self.get(Axis::Y, y + 1));
                if y0.max(b.min[1]) >= y1.min(b.max[1]) - EPSILON {
                    continue;
                }
                for z in 0..s[2] {
                    let (z0, z1) = (self.get(Axis::Z, z), self.get(Axis::Z, z + 1));
                    if z0.max(b.min[2]) >= z1.min(b.max[2]) - EPSILON {
                        continue;
                    }
                    if self.shape.full[(x * s[1] + y) * s[2] + z] {
                        return true;
                    }
                }
            }
        }
        false
    }
}

/// `Shapes.collide`: the shapes in order, stopping once the distance is within the
/// tolerance of zero.
pub fn collide_all(axis: Axis, moving: &Aabb, shapes: &[Placed<'_>], mut distance: f64) -> f64 {
    for s in shapes {
        if distance.abs() < EPSILON {
            return 0.0;
        }
        distance = s.collide(axis, moving, distance);
    }
    distance
}

#[cfg(test)]
mod tests {
    use super::*;

    fn slab() -> Shape {
        Shape::new(&ShapeData {
            coords: [vec![0.0, 1.0], vec![0.0, 0.5], vec![0.0, 1.0]],
            full: vec![true],
        })
    }

    fn player_at(x: f64, y: f64, z: f64) -> Aabb {
        Aabb::new([x - 0.3, y, z - 0.3], [x + 0.3, y + 1.8, z + 0.3])
    }

    #[test]
    fn falling_onto_a_block_stops_on_its_top() {
        let block = Shape::block();
        let below = Placed {
            shape: &block,
            offset: None,
            pos: [0.0, -61.0, 0.0],
        };
        let p = player_at(0.5, -59.5, 0.5);
        assert_eq!(below.collide(Axis::Y, &p, -1.0), -0.5);
        // Already on it: no movement down, any distance up.
        let on = player_at(0.5, -60.0, 0.5);
        assert_eq!(below.collide(Axis::Y, &on, -0.0784), 0.0);
        assert_eq!(below.collide(Axis::Y, &on, 0.42), 0.42);
        // Beside it (touching only at the edge): no effect.
        let beside = player_at(1.3, -59.5, 0.5);
        assert_eq!(below.collide(Axis::Y, &beside, -1.0), -1.0);
    }

    #[test]
    fn walking_into_a_slab_and_wall() {
        let s = slab();
        let slab_at = Placed {
            shape: &s,
            offset: None,
            pos: [2.0, -60.0, 0.0],
        };
        let p = player_at(1.5, -60.0, 0.5);
        // The slab's side is 0.2 ahead of the player's front at x 1.8.
        assert!((slab_at.collide(Axis::X, &p, 0.5) - 0.2).abs() < 1e-12);
        // Standing on the slab, it doesn't block sideways moves.
        let on = player_at(2.5, -59.5, 0.5);
        assert_eq!(slab_at.collide(Axis::X, &on, 0.3), 0.3);
        assert!(slab_at.overlaps(&Aabb::new([2.0, -60.0, 0.0], [2.5, -59.6, 0.5])));
        assert!(!slab_at.overlaps(&Aabb::new([2.0, -59.5, 0.0], [2.5, -59.0, 0.5])));
    }

    #[test]
    fn boxes_merge_like_vanilla() {
        // Stairs facing east, bottom half: a slab plus the upper east quarter.
        let stairs = Shape::new(&ShapeData {
            coords: [vec![0.0, 0.5, 1.0], vec![0.0, 0.5, 1.0], vec![0.0, 1.0]],
            // (x, y, z): (0,0) (0,1) (1,0) (1,1) with z innermost.
            full: vec![true, false, true, true],
        });
        // The bottom layer merges along x; the upper east cell stays alone.
        assert_eq!(stairs.boxes(), [[0, 0, 0, 2, 1, 1], [1, 1, 0, 2, 2, 1]]);
        assert_eq!(Shape::block().boxes(), [[0, 0, 0, 1, 1, 1]]);
        // A full 2x2x2 grid is one box, grown along x then y.
        let cube = Shape::new(&ShapeData {
            coords: [
                vec![0.0, 0.5, 1.0],
                vec![0.0, 0.5, 1.0],
                vec![0.0, 0.5, 1.0],
            ],
            full: vec![true; 8],
        });
        assert_eq!(cube.boxes(), [[0, 0, 0, 2, 2, 2]]);
        // Two z runs in one column stay apart.
        let gaps = Shape::new(&ShapeData {
            coords: [vec![0.0, 1.0], vec![0.0, 1.0], vec![0.0, 0.25, 0.75, 1.0]],
            full: vec![true, false, true],
        });
        assert_eq!(gaps.boxes(), [[0, 0, 0, 1, 1, 1], [0, 0, 2, 1, 1, 3]]);
    }

    #[test]
    fn tiny_distances_are_zero() {
        let block = Shape::block();
        let b = Placed {
            shape: &block,
            offset: None,
            pos: [5.0, 0.0, 0.0],
        };
        let p = player_at(0.0, 0.0, 0.0);
        assert_eq!(b.collide(Axis::X, &p, 5e-8), 0.0);
        assert_eq!(collide_all(Axis::X, &p, &[b], 1.0), 1.0);
        assert!(Shape::block().is_block() && !slab().is_block());
    }
}

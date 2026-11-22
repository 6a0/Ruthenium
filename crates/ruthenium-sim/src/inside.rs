//! The blocks a movement passes through, in the game's order and with its step
//! numbers (`BlockGetter.forEachBlockIntersectedBetween`): what
//! `Entity.checkInsideBlocks` walks to apply the effects of the blocks an entity
//! moves through.

use std::collections::HashSet;

use ruthenium_javamath::mth;

use crate::pick::clip_box;
use crate::shape::Aabb;

/// `Direction.axisStepOrder`: y first, then the horizontal axis the movement goes
/// along more (x on a tie).
pub fn axis_step_order(d: [f64; 3]) -> [usize; 3] {
    if d[0].abs() < d[2].abs() {
        [1, 2, 0]
    } else {
        [1, 0, 2]
    }
}

/// `BlockPos.betweenCornersInDirection`: the blocks between two corners, from the
/// corner the direction starts at, along the axes in step order (the last fastest).
pub fn between_corners_in_direction(a: [i32; 3], b: [i32; 3], dir: [f64; 3]) -> Vec<[i32; 3]> {
    let min = [a[0].min(b[0]), a[1].min(b[1]), a[2].min(b[2])];
    let max = [a[0].max(b[0]), a[1].max(b[1]), a[2].max(b[2])];
    let start: [i32; 3] = std::array::from_fn(|i| if dir[i] >= 0.0 { min[i] } else { max[i] });
    let step: [i32; 3] = std::array::from_fn(|i| if dir[i] >= 0.0 { 1 } else { -1 });
    let [first, second, third] = axis_step_order(dir);
    let mut out = Vec::new();
    for i in 0..=max[first] - min[first] {
        for j in 0..=max[second] - min[second] {
            for k in 0..=max[third] - min[third] {
                let mut p = start;
                p[first] += step[first] * i;
                p[second] += step[second] * j;
                p[third] += step[third] * k;
                out.push(p);
            }
        }
    }
    out
}

/// [`between_corners_in_direction`] over the blocks a box touches.
fn box_corners_in_direction(b: &Aabb, dir: [f64; 3]) -> Vec<[i32; 3]> {
    let lo = b.min.map(mth::floor);
    let hi = b.max.map(mth::floor);
    between_corners_in_direction(lo, hi, dir)
}

/// `Mth.clamp` for doubles.
fn clamp(v: f64, min: f64, max: f64) -> f64 {
    if v < min { min } else { v.min(max) }
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

/// `BlockGetter.getFurthestCorner`, as the game has it.
fn furthest_corner(d: [f64; 3]) -> [i32; 3] {
    let (xd, yd, zd) = (d[0].abs(), d[1].abs(), d[2].abs());
    let s = |v: f64| if v >= 0.0 { 1 } else { -1 };
    let (xs, ys, zs) = (s(d[0]), s(d[1]), s(d[2]));
    if xd <= yd && xd <= zd {
        [-xs, -zs, ys]
    } else if yd <= zd {
        [zs, -ys, -xs]
    } else {
        [-ys, xs, -zs]
    }
}

/// `BlockGetter.forEachBlockIntersectedBetween`: visits the blocks the box at
/// `target` touches had it travelled from `from` to `to`, each once, with the step it
/// is reached at; stops (false) when the visitor does.
pub fn for_each_block_intersected_between(
    from: [f64; 3],
    to: [f64; 3],
    target: &Aabb,
    visit: &mut dyn FnMut([i32; 3], i32) -> bool,
) -> bool {
    let travel = [to[0] - from[0], to[1] - from[1], to[2] - from[2]];
    let length_sqr = travel[0] * travel[0] + travel[1] * travel[1] + travel[2] * travel[2];
    if length_sqr < f64::from(1.0E-5f32 * 1.0E-5f32) {
        // BlockPos.betweenClosed: x fastest, then y, then z.
        let lo = target.min.map(mth::floor);
        let hi = target.max.map(mth::floor);
        for z in lo[2]..=hi[2] {
            for y in lo[1]..=hi[1] {
                for x in lo[0]..=hi[0] {
                    if !visit([x, y, z], 0) {
                        return false;
                    }
                }
            }
        }
        return true;
    }
    let mut visited = HashSet::new();
    // `move(travel.scale(-1.0))`: times -1 is negation.
    let back = target.moved([-travel[0], -travel[1], -travel[2]]);
    for p in box_corners_in_direction(&back, travel) {
        if !visit(p, 0) {
            return false;
        }
        visited.insert(p);
    }
    let Some(iterations) = collisions_along_travel(&mut visited, travel, target, visit) else {
        return false;
    };
    for p in box_corners_in_direction(target, travel) {
        if visited.insert(p) && !visit(p, iterations + 1) {
            return false;
        }
    }
    true
}

/// `BlockGetter.addCollisionsAlongTravel`: steps the box's leading corner through
/// the blocks along the travel, visiting the blocks the box covers at each; the
/// steps taken, or `None` when the visitor stopped.
fn collisions_along_travel(
    visited: &mut HashSet<[i32; 3]>,
    delta: [f64; 3],
    target: &Aabb,
    visit: &mut dyn FnMut([i32; 3], i32) -> bool,
) -> Option<i32> {
    let size: [f64; 3] = std::array::from_fn(|i| target.max[i] - target.min[i]);
    let corner = furthest_corner(delta);
    let center: [f64; 3] =
        std::array::from_fn(|i| target.min[i] + 0.5 * (target.max[i] - target.min[i]));
    let to_corner: [f64; 3] =
        std::array::from_fn(|i| center[i] + size[i] * 0.5 * f64::from(corner[i]));
    let from_corner: [f64; 3] = std::array::from_fn(|i| to_corner[i] - delta[i]);
    let mut cell: [i32; 3] = from_corner.map(mth::floor);
    let signs: [i32; 3] = delta.map(sign);
    let t_delta: [f64; 3] = std::array::from_fn(|i| {
        if signs[i] == 0 {
            f64::MAX
        } else {
            f64::from(signs[i]) / delta[i]
        }
    });
    let mut t: [f64; 3] = std::array::from_fn(|i| {
        t_delta[i]
            * if signs[i] > 0 {
                1.0 - frac(from_corner[i])
            } else {
                frac(from_corner[i])
            }
    });
    let mut iterations = 0;
    while t[0] <= 1.0 || t[1] <= 1.0 || t[2] <= 1.0 {
        let axis = if t[0] < t[1] {
            if t[0] < t[2] { 0 } else { 2 }
        } else if t[1] < t[2] {
            1
        } else {
            2
        };
        cell[axis] += signs[axis];
        t[axis] += t_delta[axis];
        let block = Aabb {
            min: cell.map(f64::from),
            max: cell.map(|c| f64::from(c) + 1.0),
        };
        let Some(hit) = clip_box(&block, from_corner, to_corner) else {
            continue;
        };
        iterations += 1;
        let opposite: [i32; 3] = std::array::from_fn(|i| {
            let c = f64::from(cell[i]);
            let h = clamp(
                hit[i],
                c + 9.999999747378752E-6,
                c + 1.0 - 9.999999747378752E-6,
            );
            mth::floor(h - size[i] * f64::from(corner[i]))
        });
        for p in between_corners_in_direction(cell, opposite, delta) {
            if visited.insert(p) && !visit(p, iterations) {
                return None;
            }
        }
    }
    Some(iterations)
}

/// `AABB.collidedAlongVector`: the box, moved by `vector` from where it is, starts
/// or ends in or passes through one of the boxes (grown by half its size).
pub fn collided_along_vector(b: &Aabb, vector: [f64; 3], boxes: &[Aabb]) -> bool {
    let from: [f64; 3] = std::array::from_fn(|i| b.min[i] + 0.5 * (b.max[i] - b.min[i]));
    let to: [f64; 3] = std::array::from_fn(|i| from[i] + vector[i]);
    let half: [f64; 3] = std::array::from_fn(|i| (b.max[i] - b.min[i]) * 0.5 - 1.0E-7);
    let contains = |a: &Aabb, p: [f64; 3]| (0..3).all(|i| p[i] >= a.min[i] && p[i] < a.max[i]);
    boxes.iter().any(|part| {
        let grown = part.inflate(half);
        contains(&grown, to) || contains(&grown, from) || clip_box(&grown, from, to).is_some()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn walk(from: [f64; 3], to: [f64; 3], target: Aabb) -> Vec<([i32; 3], i32)> {
        let mut out = Vec::new();
        for_each_block_intersected_between(from, to, &target, &mut |p, i| {
            out.push((p, i));
            true
        });
        out
    }

    #[test]
    fn standing_still_visits_the_blocks_the_box_touches() {
        let b = Aabb::new([0.2, 0.0, 0.2], [0.8, 1.8, 0.8]);
        let v = walk([0.5, 0.0, 0.5], [0.5, 0.0, 0.5], b);
        assert_eq!(v, vec![([0, 0, 0], 0), ([0, 1, 0], 0)]);
    }

    #[test]
    fn a_walk_visits_each_block_once_in_step_order() {
        // A thin box moving 2.5 blocks east along y = 0.5.
        let b = Aabb::new([2.9, 0.4, 0.4], [3.1, 0.6, 0.6]);
        let v = walk([0.5, 0.5, 0.5], [3.0, 0.5, 0.5], b);
        let cells: Vec<[i32; 3]> = v.iter().map(|(p, _)| *p).collect();
        assert_eq!(cells, vec![[0, 0, 0], [1, 0, 0], [2, 0, 0], [3, 0, 0]]);
        let steps: Vec<i32> = v.iter().map(|(_, i)| *i).collect();
        assert_eq!(steps, vec![0, 1, 2, 3]);
    }

    #[test]
    fn corners_go_from_where_the_direction_starts() {
        let c = between_corners_in_direction([0, 0, 0], [1, 0, 1], [-1.0, 0.0, 0.5]);
        assert_eq!(c, vec![[1, 0, 0], [1, 0, 1], [0, 0, 0], [0, 0, 1]]);
    }

    #[test]
    fn moving_through_a_box_collides_along_the_way() {
        let b = Aabb::new([0.0, 0.0, 0.0], [0.6, 1.8, 0.6]);
        let wall = [Aabb::new([2.0, 0.0, 0.0], [2.1, 1.0, 1.0])];
        assert!(collided_along_vector(&b, [3.0, 0.0, 0.0], &wall));
        assert!(!collided_along_vector(&b, [1.0, 0.0, 0.0], &wall));
    }
}

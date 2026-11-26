//! Quads grouped by the way they face, so that a frame can leave out the groups the
//! camera is behind.
//!
//! Every terrain pipeline culls back faces, so a quad seen from behind its plane draws
//! nothing. A quad that lies in a plane across an axis faces one of the six directions;
//! the mesher puts a layer's quads of each direction together ([`group`]), and keeps for
//! each direction the plane its quads are furthest back in ([`Facings::planes`]). When the
//! camera is behind that plane, every quad of the group faces away from it and the group
//! is not drawn ([`shown`]). That changes no pixel: the GPU would have culled each of
//! those triangles. Vanilla draws them all and lets the GPU cull them.
//!
//! Which way a quad faces is taken from its corners as packed, not from the model's
//! face, so a rotated element or a sloping fluid surface is never put in a direction's
//! group: it goes with the quads that are always drawn.

use crate::terrain_vertex::{VERTEX_BYTES, position_of};

use super::mesher::STEPS;

/// The groups of a layer, in the order they are packed: up, east, south, those facing
/// no one direction, north, west, down. Opposite directions are apart, and each
/// direction is next to those a camera above and to the south-east sees with it, so
/// that what is drawn of a section usually comes in one to three runs ([`runs`]).
pub const SLOTS: [Option<usize>; 7] = [Some(1), Some(5), Some(3), None, Some(2), Some(4), Some(0)];

/// A camera within this many blocks of a group's plane draws the group: what the GPU
/// computes from the same numbers is not trusted to agree to the last bit.
const MARGIN: f32 = 1.0 / 256.0;

/// How a mesh's solid and cutout quads are grouped.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Facings {
    /// Quads of each group ([`SLOTS`]) of the solid layer, then of the cutout layer.
    pub solid: [u32; 7],
    pub cutout: [u32; 7],
    /// For each direction (by ordinal), in blocks from the section's corner along its
    /// axis, the plane of its quads the camera has to be in front of to see any of
    /// them: the lowest for up, south and east, the highest for down, north and west.
    pub planes: [f32; 6],
}

impl Default for Facings {
    /// No quads.
    fn default() -> Facings {
        Facings {
            solid: [0; 7],
            cutout: [0; 7],
            planes: planes_in_blocks(&no_planes()),
        }
    }
}

/// The direction a packed quad faces, and the code of its plane along that direction's
/// axis; `None` if it lies in no plane across an axis, or its two triangles do not face
/// the same way.
pub fn classify(quad: &[u8]) -> Option<(usize, u16)> {
    let corner = |k: usize| -> [i64; 3] {
        let v = &quad[k * VERTEX_BYTES..];
        std::array::from_fn(|i| i64::from(u16::from_le_bytes([v[2 * i], v[2 * i + 1]])))
    };
    let c = [corner(0), corner(1), corner(2), corner(3)];
    let axis = (0..3).find(|&a| c.iter().all(|p| p[a] == c[0][a]))?;
    let (u, v) = ((axis + 1) % 3, (axis + 2) % 3);
    // The axis's part of the normal of the triangle a, b, c: (b - a) x (c - a).
    let normal = |a: [i64; 3], b: [i64; 3], d: [i64; 3]| {
        (b[u] - a[u]) * (d[v] - a[v]) - (b[v] - a[v]) * (d[u] - a[u])
    };
    // The two triangles the shared indices make of a quad: 0 1 2 and 2 3 0.
    let (first, second) = (normal(c[0], c[1], c[2]), normal(c[2], c[3], c[0]));
    let sign = if first >= 0 && second >= 0 && first + second > 0 {
        1
    } else if first <= 0 && second <= 0 && first + second < 0 {
        -1
    } else {
        return None;
    };
    let direction = STEPS.iter().position(|s| s[axis] == sign)?;
    Some((direction, c[0][axis] as u16))
}

/// Puts the quads of `layer` (packed, four vertices each) in the order of [`SLOTS`], each
/// group's in the order they were in; counts them into `counts` and widens `planes`
/// (codes, by direction) to take them in. `scratch` is only for the copy.
pub fn group(
    layer: &mut [u8],
    scratch: &mut Vec<u8>,
    counts: &mut [u32; 7],
    planes: &mut [i32; 6],
) {
    const QUAD: usize = 4 * VERTEX_BYTES;
    let slot_of = |d: Option<usize>| SLOTS.iter().position(|&s| s == d).unwrap_or(3);
    let mut slots: Vec<u8> = Vec::with_capacity(layer.len() / QUAD);
    *counts = [0; 7];
    for quad in layer.as_chunks::<QUAD>().0 {
        let facing = classify(quad);
        if let Some((d, code)) = facing {
            let code = i32::from(code);
            planes[d] = if STEPS[d].iter().sum::<i32>() > 0 {
                planes[d].min(code)
            } else {
                planes[d].max(code)
            };
        }
        let slot = slot_of(facing.map(|f| f.0));
        counts[slot] += 1;
        slots.push(slot as u8);
    }
    if counts.iter().filter(|&&n| n > 0).count() <= 1 {
        return;
    }
    let mut next = [0usize; 7];
    let mut at = 0;
    for (slot, &n) in counts.iter().enumerate() {
        next[slot] = at;
        at += n as usize;
    }
    scratch.clear();
    scratch.resize(layer.len(), 0);
    for (quad, &slot) in layer.as_chunks::<QUAD>().0.iter().zip(&slots) {
        let to = next[slot as usize];
        next[slot as usize] += 1;
        scratch[to * QUAD..(to + 1) * QUAD].copy_from_slice(quad);
    }
    layer.copy_from_slice(scratch);
}

/// What [`group`]'s planes start from: no quad of any direction yet.
pub fn no_planes() -> [i32; 6] {
    std::array::from_fn(|d| {
        if STEPS[d].iter().sum::<i32>() > 0 {
            i32::MAX
        } else {
            i32::MIN
        }
    })
}

/// The planes in blocks from the section's corner (a direction without quads gets one no
/// camera is behind).
pub fn planes_in_blocks(codes: &[i32; 6]) -> [f32; 6] {
    std::array::from_fn(|d| match codes[d] {
        i32::MAX => f32::NEG_INFINITY,
        i32::MIN => f32::INFINITY,
        c => position_of(c as u16),
    })
}

/// The directions whose quads a camera at `eye` (in blocks from the section's corner)
/// can see the front of, one bit per ordinal.
pub fn shown(planes: &[f32; 6], eye: [f32; 3]) -> u8 {
    let mut bits = 0;
    for (d, step) in STEPS.iter().enumerate() {
        let axis = step.iter().position(|&s| s != 0).unwrap_or(0);
        let seen = if step[axis] > 0 {
            eye[axis] >= planes[d] - MARGIN
        } else {
            eye[axis] <= planes[d] + MARGIN
        };
        if seen {
            bits |= 1 << d;
        }
    }
    bits
}

/// Hidden quads a run is drawn through rather than broken in two: the GPU culls a few
/// dozen faces in less time than another draw takes to record (RTX 4060: about 0.3 ns a
/// quad against 25 ns a draw).
pub const DRAWN_THROUGH: u32 = 64;

/// The runs of quads to draw of a layer grouped as `counts`, given the directions
/// `shown` lets through: (first quad, quads). Groups next to each other are one run, and
/// so are two drawn groups with no more than [`DRAWN_THROUGH`] hidden quads between them.
pub fn runs(counts: &[u32; 7], shown: u8) -> impl Iterator<Item = (u32, u32)> + '_ {
    let drawn =
        move |slot: usize| counts[slot] > 0 && SLOTS[slot].is_none_or(|d| shown & (1 << d) != 0);
    let mut slot = 0;
    let mut first = 0u32;
    std::iter::from_fn(move || {
        // Skip what is not drawn.
        while slot < 7 && !drawn(slot) {
            first += counts[slot];
            slot += 1;
        }
        if slot == 7 {
            return None;
        }
        let start = first;
        let mut end = first;
        while slot < 7 {
            if drawn(slot) {
                first += counts[slot];
                end = first;
                slot += 1;
                continue;
            }
            // Hidden quads up to the next drawn group: drawn through if few.
            let mut next = slot;
            let mut hidden = 0;
            while next < 7 && !drawn(next) {
                hidden += counts[next];
                next += 1;
            }
            if hidden > DRAWN_THROUGH {
                break;
            }
            first += hidden;
            slot = next;
        }
        Some((start, end - start))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::terrain_vertex::{Vertex, position_code};

    fn quad(corners: [[f32; 3]; 4]) -> Vec<u8> {
        let mut out = Vec::new();
        for c in corners {
            Vertex {
                position: c.map(|x| position_code(x).expect("in range")),
                block_light: 0,
                sky_light: 0,
                colour: [255; 4],
                uv: [0; 2],
            }
            .pack(&mut out);
        }
        out
    }

    /// A unit block's face at `at`, as the mesher's lone-block test sees them facing
    /// out: the corners of vanilla's `FaceInfo`, wound so that (1 - 0) x (2 - 0) points out.
    fn face(direction: usize, at: [f32; 3]) -> Vec<u8> {
        let [x, y, z] = at;
        let (x1, y1, z1) = (x + 1.0, y + 1.0, z + 1.0);
        let corners = match direction {
            0 => [[x, y, z1], [x, y, z], [x1, y, z], [x1, y, z1]],
            1 => [[x, y1, z], [x, y1, z1], [x1, y1, z1], [x1, y1, z]],
            2 => [[x1, y1, z], [x1, y, z], [x, y, z], [x, y1, z]],
            3 => [[x, y1, z1], [x, y, z1], [x1, y, z1], [x1, y1, z1]],
            4 => [[x, y1, z], [x, y, z], [x, y, z1], [x, y1, z1]],
            _ => [[x1, y1, z1], [x1, y, z1], [x1, y, z], [x1, y1, z]],
        };
        quad(corners)
    }

    /// Whether a camera at `eye` sees the front of a packed quad's first triangle, by the
    /// sign of its normal against the way to the eye, computed in f64.
    fn front(quad: &[u8], eye: [f32; 3]) -> bool {
        let p = |k: usize| -> [f64; 3] {
            let v = &quad[k * VERTEX_BYTES..];
            std::array::from_fn(|i| {
                f64::from(position_of(u16::from_le_bytes([v[2 * i], v[2 * i + 1]])))
            })
        };
        let (a, b, c) = (p(0), p(1), p(2));
        let (e, f) = (
            [b[0] - a[0], b[1] - a[1], b[2] - a[2]],
            [c[0] - a[0], c[1] - a[1], c[2] - a[2]],
        );
        let n = [
            e[1] * f[2] - e[2] * f[1],
            e[2] * f[0] - e[0] * f[2],
            e[0] * f[1] - e[1] * f[0],
        ];
        let to_eye = std::array::from_fn::<f64, 3, _>(|i| f64::from(eye[i]) - a[i]);
        n[0] * to_eye[0] + n[1] * to_eye[1] + n[2] * to_eye[2] > 0.0
    }

    #[test]
    fn a_blocks_faces_face_their_own_directions_in_their_own_planes() {
        for (d, step) in STEPS.iter().enumerate() {
            let (direction, code) = classify(&face(d, [3.0, 4.0, 5.0])).expect("facing");
            assert_eq!(direction, d);
            let axis = step.iter().position(|&s| s != 0).unwrap();
            let plane = [3.0, 4.0, 5.0][axis] + if step[axis] > 0 { 1.0 } else { 0.0 };
            assert_eq!(position_of(code), plane, "direction {d}");
        }
    }

    #[test]
    fn a_slanted_quad_and_a_folded_one_face_no_direction() {
        // A plant's cross: a diagonal plane.
        let cross = quad([
            [0.0, 1.0, 0.0],
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 1.0],
            [1.0, 1.0, 1.0],
        ]);
        assert_eq!(classify(&cross), None);
        // In one plane, but its triangles face away from each other.
        let bow = quad([
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0],
            [1.0, 0.0, 1.0],
        ]);
        assert_eq!(classify(&bow), None);
        // The same with one triangle larger than the other.
        let uneven = quad([
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0],
            [2.0, 0.0, 1.0],
        ]);
        assert_eq!(classify(&uneven), None);
        // A water surface sloping down to one side.
        let slope = quad([
            [0.0, 0.9, 0.0],
            [0.0, 0.9, 1.0],
            [1.0, 0.5, 1.0],
            [1.0, 0.5, 0.0],
        ]);
        assert_eq!(classify(&slope), None);
    }

    #[test]
    fn grouping_keeps_each_groups_quads_in_their_order_and_counts_them() {
        let made = [
            (4, 0.0),
            (1, 0.0),
            (9, 0.0),
            (4, 3.0),
            (0, 2.0),
            (1, 7.0),
            (9, 1.0),
            (5, 2.0),
        ];
        let mut layer = Vec::new();
        let cross = |at: f32| {
            quad([
                [at, 1.0, 0.0],
                [at, 0.0, 0.0],
                [at + 1.0, 0.0, 1.0],
                [at + 1.0, 1.0, 1.0],
            ])
        };
        for &(d, at) in &made {
            layer.extend(if d == 9 {
                cross(at)
            } else {
                face(d, [at, at, at])
            });
        }
        let original = layer.clone();
        let (mut counts, mut planes, mut scratch) = ([0; 7], no_planes(), Vec::new());
        group(&mut layer, &mut scratch, &mut counts, &mut planes);
        // up, east, south, none, north, west, down
        assert_eq!(counts, [2, 1, 0, 2, 0, 2, 1]);
        let q = 4 * VERTEX_BYTES;
        let old = |i: usize| &original[i * q..(i + 1) * q];
        let order = [1, 5, 7, 2, 6, 0, 3, 4];
        for (to, &from) in order.iter().enumerate() {
            assert_eq!(&layer[to * q..(to + 1) * q], old(from), "quad {to}");
        }
        let blocks = planes_in_blocks(&planes);
        assert_eq!(blocks[1], 1.0, "the lowest of the up faces' planes");
        assert_eq!(blocks[4], 3.0, "the highest of the west faces' planes");
        assert_eq!(blocks[5], 3.0);
        assert_eq!(blocks[0], 2.0);
        assert_eq!(blocks[2], f32::INFINITY, "no north faces: always shown");
        assert_eq!(blocks[3], f32::NEG_INFINITY, "no south faces: always shown");
    }

    #[test]
    fn two_groups_are_put_in_order_too() {
        let mut layer = face(0, [1.0, 1.0, 1.0]);
        layer.extend(face(1, [2.0, 2.0, 2.0]));
        let original = layer.clone();
        let (mut counts, mut planes, mut scratch) = ([0; 7], no_planes(), Vec::new());
        group(&mut layer, &mut scratch, &mut counts, &mut planes);
        assert_eq!(counts, [1, 0, 0, 0, 0, 0, 1]);
        let q = 4 * VERTEX_BYTES;
        assert_eq!(&layer[..q], &original[q..], "up first");
        assert_eq!(&layer[q..], &original[..q], "down last");
    }

    #[test]
    fn a_hidden_group_has_only_quads_seen_from_behind() {
        // Faces in every direction at many places, cameras all round and close to the
        // planes: whatever `shown` leaves out faces away from the camera.
        let mut state = 0x2545_f491_u32;
        let mut next = move || {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            state
        };
        let mut layer = Vec::new();
        for _ in 0..300 {
            let d = (next() % 6) as usize;
            let at = std::array::from_fn(|_| (next() % 15) as f32 + (next() % 4) as f32 / 4.0);
            layer.extend(face(d, at));
        }
        let (mut counts, mut planes, mut scratch) = ([0; 7], no_planes(), Vec::new());
        group(&mut layer, &mut scratch, &mut counts, &mut planes);
        let planes = planes_in_blocks(&planes);
        let mut hidden = 0;
        for _ in 0..2000 {
            // Whole and quarter blocks, and a hair either side of them (where the
            // planes are).
            let eye: [f32; 3] = std::array::from_fn(|_| {
                (next() % 40) as f32 - 12.0
                    + (next() % 4) as f32 / 4.0
                    + [0.0, 1e-3, -1e-3][(next() % 3) as usize]
            });
            let bits = shown(&planes, eye);
            let mut first = 0;
            for (slot, &n) in counts.iter().enumerate() {
                let drawn = SLOTS[slot].is_none_or(|d| bits & (1 << d) != 0);
                for i in first..first + n {
                    let q =
                        &layer[i as usize * 4 * VERTEX_BYTES..(i as usize + 1) * 4 * VERTEX_BYTES];
                    if !drawn {
                        hidden += 1;
                        assert!(!front(q, eye), "a quad facing {eye:?} left out");
                    }
                }
                first += n;
            }
        }
        assert!(
            hidden > 100_000,
            "the test hid too little to mean anything: {hidden}"
        );
    }

    #[test]
    fn runs_draw_through_a_few_hidden_quads_but_not_many() {
        let all = 0b11_1111;
        // Up and down drawn, everything between hidden.
        let ud = (1 << 1) | (1 << 0);
        let few = [100, 10, 10, 0, 10, 10, 100];
        assert_eq!(runs(&few, ud).collect::<Vec<_>>(), [(0, 240)]);
        let many = [100, 30, 30, 0, 30, 30, 100];
        assert_eq!(runs(&many, ud).collect::<Vec<_>>(), [(0, 100), (220, 100)]);
        // Hidden quads at the end are not drawn, however few.
        assert_eq!(
            runs(&[5, 1, 0, 0, 0, 0, 0], 1 << 1).collect::<Vec<_>>(),
            [(0, 5)]
        );
        // Nor at the start.
        assert_eq!(
            runs(&[1, 5, 0, 0, 0, 0, 0], 1 << 5).collect::<Vec<_>>(),
            [(1, 5)]
        );
        assert_eq!(runs(&many, all).collect::<Vec<_>>(), [(0, 320)]);
    }

    #[test]
    fn runs_join_neighbouring_groups_and_step_over_empty_ones() {
        let all = 0b11_1111;
        let counts = [3, 200, 0, 4, 100, 0, 5];
        assert_eq!(runs(&counts, all).collect::<Vec<_>>(), [(0, 312)]);
        // Up, east and south (empty) with the unassigned: one run.
        let ues = (1 << 1) | (1 << 5) | (1 << 3);
        assert_eq!(runs(&counts, ues).collect::<Vec<_>>(), [(0, 207)]);
        // Down, west (empty) and north: the unassigned, then north to down.
        let dwn = (1 << 0) | (1 << 4) | (1 << 2);
        assert_eq!(runs(&counts, dwn).collect::<Vec<_>>(), [(203, 109)]);
        // Up and north only: the five hidden quads after north are not drawn.
        let un = (1 << 1) | (1 << 2);
        assert_eq!(runs(&counts, un).collect::<Vec<_>>(), [(0, 3), (203, 104)]);
        assert_eq!(runs(&[0; 7], all).count(), 0);
        assert_eq!(runs(&[0, 4, 0, 0, 0, 0, 0], 0).count(), 0);
    }
}

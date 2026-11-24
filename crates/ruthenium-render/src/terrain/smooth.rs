//! Smooth lighting: the light and the shade of each corner of a quad, blended from the
//! blocks around the face it lies on, as 26.3's `BlockModelLighter` does it
//! (`prepareQuadAmbientOcclusion`). The mesher uses it for blocks that give off no light
//! and whose model has ambient occlusion, when Smooth Lighting is on.
//!
//! A corner's values are the mean of four places: the face's own (the block in front of
//! it, for a face on the block's boundary), the two beside it that meet at the corner, and
//! the one diagonally between those. A quad that covers only part of the face weighs the
//! four corners' values by where its own corners lie.

use super::mesher::STEPS;

/// What the lighter reads of the block at a place.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct Cell {
    /// Block and sky light there, 0 to 15, as `LightCoordsUtil.getLightCoords` gives them
    /// for the state there: both 15 for a state drawn at full light, else the block light
    /// not below what the state gives off.
    pub block: u8,
    pub sky: u8,
    pub flags: u8,
}

impl Cell {
    /// `getShadeBrightness` is 0.2 (else 1): a block with a full collision shape, and a
    /// few by their class.
    pub const DARK: u8 = 1;
    /// `isLightPermeable`: not solid to the eye, or it takes no light away.
    pub const PERMEABLE: u8 = 2;
    /// `isSolidRender`.
    pub const SOLID: u8 = 4;

    fn shade(self) -> f32 {
        if self.flags & Cell::DARK != 0 {
            0.2
        } else {
            1.0
        }
    }
}

/// The four directions beside a face, by the direction it looks in
/// (`BlockModelLighter.java:283-560`, each entry's first argument).
const SIDES: [[usize; 4]; 6] = [
    [4, 5, 2, 3],
    [5, 4, 2, 3],
    [1, 0, 5, 4],
    [4, 5, 0, 1],
    [1, 0, 2, 3],
    [0, 1, 2, 3],
];

/// Which vertex takes each of the four blends, by direction (`BlockModelLighter.java:586-591`).
const VERTEX: [[usize; 4]; 6] = [
    [0, 1, 2, 3],
    [2, 3, 0, 1],
    [3, 0, 1, 2],
    [0, 1, 2, 3],
    [3, 0, 1, 2],
    [1, 2, 3, 0],
];

/// For a quad over part of a face: by direction and blend slot, the four weights of the
/// blends as pairs of sizes to multiply (`BlockModelLighter.java:283-560`). A size is one
/// of the quad's bounds (0 to 5: least y, greatest y, least z, greatest z, least x,
/// greatest x) or one minus it (6 to 11).
const WEIGHTS: [[[usize; 8]; 4]; 6] = [
    // down
    [
        [10, 3, 10, 9, 4, 9, 4, 3],
        [10, 2, 10, 8, 4, 8, 4, 2],
        [11, 2, 11, 8, 5, 8, 5, 2],
        [11, 3, 11, 9, 5, 9, 5, 3],
    ],
    // up
    [
        [5, 3, 5, 9, 11, 9, 11, 3],
        [5, 2, 5, 8, 11, 8, 11, 2],
        [4, 2, 4, 8, 10, 8, 10, 2],
        [4, 3, 4, 9, 10, 9, 10, 3],
    ],
    // north
    [
        [1, 10, 1, 4, 7, 4, 7, 10],
        [1, 11, 1, 5, 7, 5, 7, 11],
        [0, 11, 0, 5, 6, 5, 6, 11],
        [0, 10, 0, 4, 6, 4, 6, 10],
    ],
    // south
    [
        [1, 10, 7, 10, 7, 4, 1, 4],
        [0, 10, 6, 10, 6, 4, 0, 4],
        [0, 11, 6, 11, 6, 5, 0, 5],
        [1, 11, 7, 11, 7, 5, 1, 5],
    ],
    // west
    [
        [1, 3, 1, 9, 7, 9, 7, 3],
        [1, 2, 1, 8, 7, 8, 7, 2],
        [0, 2, 0, 8, 6, 8, 6, 2],
        [0, 3, 0, 9, 6, 9, 6, 3],
    ],
    // east
    [
        [6, 3, 6, 9, 0, 9, 0, 3],
        [6, 2, 6, 8, 0, 8, 0, 2],
        [7, 2, 7, 8, 1, 8, 1, 2],
        [7, 3, 7, 9, 1, 9, 1, 3],
    ],
];

/// Where a quad lies in its block (`prepareQuadShape`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Face {
    min: [f32; 3],
    max: [f32; 3],
    /// It does not cover the whole face.
    pub partial: bool,
    /// It lies flat across its direction, on the block's boundary on that side or
    /// anywhere in a block whose collision shape is the full block: it is lit from the
    /// block in front.
    pub cubic: bool,
}

impl Face {
    pub fn of(positions: &[[f32; 3]; 4], direction: usize, collision_full: bool) -> Face {
        let (mut min, mut max) = ([32.0_f32; 3], [-32.0_f32; 3]);
        for p in positions {
            for a in 0..3 {
                min[a] = min[a].min(p[a]);
                max[a] = max[a].max(p[a]);
            }
        }
        let axis = [1, 1, 2, 2, 0, 0][direction];
        let partial = (0..3)
            .filter(|a| *a != axis)
            .any(|a| min[a] >= 1.0e-4 || max[a] <= 0.9999);
        let outside = if direction & 1 == 1 {
            max[axis] > 0.9999
        } else {
            min[axis] < 1.0e-4
        };
        Face {
            min,
            max,
            partial,
            cubic: min[axis] == max[axis] && (outside || collision_full),
        }
    }

    fn sizes(&self) -> [f32; 12] {
        let s = [
            self.min[1],
            self.max[1],
            self.min[2],
            self.max[2],
            self.min[0],
            self.max[0],
        ];
        std::array::from_fn(|i| if i < 6 { s[i] } else { 1.0 - s[i - 6] })
    }
}

/// What a quad's four vertices get.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct Lit {
    /// The grey each vertex's colour starts as, the direction's shade included.
    pub grey: [u8; 4],
    /// Block and sky light in sixteenths of a level, 0 to 240.
    pub light: [[u8; 2]; 4],
}

/// `LightCoordsUtil.smoothBlend`: the mean of three places' light and the centre's, in
/// sixteenths. Where the centre is lit over 2, a dark place counts as the centre, and
/// one without sky light takes the centre's.
fn blend(around: [Cell; 3], centre: Cell) -> [u8; 2] {
    let lit = centre.sky > 2 || centre.block > 2;
    let (mut block, mut sky) = (u32::from(centre.block), u32::from(centre.sky));
    for c in around {
        let (mut b, mut s) = (c.block, c.sky);
        if lit && s == 0 {
            if b == 0 {
                b = centre.block;
            }
            s = centre.sky;
        }
        block += u32::from(b);
        sky += u32::from(s);
    }
    [(block * 4) as u8, (sky * 4) as u8]
}

/// A quad's own light raises both lights of a vertex, counted in whole levels
/// (`LightCoordsUtil.lightCoordsWithEmission`): the fractions of a blend are lost.
pub(crate) fn with_emission(light: [u8; 2], emission: u8) -> [u8; 2] {
    if emission == 0 {
        return light;
    }
    light.map(|l| (l >> 4).max(emission) << 4)
}

/// Lights a quad of the block at the origin that looks in `direction`. `cell` gives the
/// block at a place relative to it, and `shade` is what the dimension's cardinal light
/// leaves of a face looking the quad's way.
pub(crate) fn smooth(
    face: &Face,
    direction: usize,
    shade: f32,
    mut cell: impl FnMut([i32; 3]) -> Cell,
) -> Lit {
    let add = |a: [i32; 3], b: [i32; 3]| [a[0] + b[0], a[1] + b[1], a[2] + b[2]];
    let ahead = STEPS[direction];
    let base = if face.cubic { ahead } else { [0; 3] };
    let steps = SIDES[direction].map(|d| STEPS[d]);
    let sides = steps.map(|s| cell(add(base, s)));
    // What is in front of each side decides whether the diagonals are looked at.
    let open = steps.map(|s| cell(add(add(base, s), ahead)).flags & Cell::PERMEABLE != 0);
    let mut diagonal = |i: usize, j: usize| {
        if open[i] || open[j] {
            cell(add(add(base, steps[i]), steps[j]))
        } else {
            // The first side's, for the second side's corners too.
            sides[0]
        }
    };
    let diagonals = [
        diagonal(0, 3),
        diagonal(0, 2),
        diagonal(1, 2),
        diagonal(1, 3),
    ];
    let own = cell([0; 3]);
    let front = cell(ahead);
    let centre = if face.cubic || front.flags & Cell::SOLID == 0 {
        front
    } else {
        own
    };
    let centre_shade = if face.cubic { front } else { own }.shade();
    // The four blends: a side of the second pair, one of the first, and their diagonal.
    let pairs = [(3, 0), (2, 0), (2, 1), (3, 1)];
    let shades: [f32; 4] = std::array::from_fn(|k| {
        let (a, b) = pairs[k];
        (sides[a].shade() + sides[b].shade() + diagonals[k].shade() + centre_shade) * 0.25
    });
    let lights: [[u8; 2]; 4] = std::array::from_fn(|k| {
        let (a, b) = pairs[k];
        blend([sides[a], sides[b], diagonals[k]], centre)
    });

    let mut out = Lit::default();
    let grey = |level: f32| ((level * 255.0).floor() as i32 & 0xFF) as f32;
    let scaled = |g: f32| ((g * shade) as i32).clamp(0, 255) as u8;
    if face.partial {
        let sizes = face.sizes();
        for (slot, pairs) in WEIGHTS[direction].iter().enumerate() {
            let w: [f32; 4] =
                std::array::from_fn(|k| sizes[pairs[2 * k]] * sizes[pairs[2 * k + 1]]);
            let level = shades[0] * w[0] + shades[1] * w[1] + shades[2] * w[2] + shades[3] * w[3];
            let channel = |c: usize| {
                let l = |k: usize| f32::from(lights[k][c]);
                (l(0) * w[0] + l(1) * w[1] + l(2) * w[2] + l(3) * w[3]) as i32 as u8
            };
            let v = VERTEX[direction][slot];
            out.grey[v] = scaled(grey(level.clamp(0.0, 1.0)));
            out.light[v] = [channel(0), channel(1)];
        }
    } else {
        for slot in 0..4 {
            let v = VERTEX[direction][slot];
            out.grey[v] = scaled(grey(shades[slot]));
            out.light[v] = lights[slot];
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    include!("smooth_golden.rs");

    /// The generator the golden values' program draws its cases from.
    struct Rng(u64);

    impl Rng {
        fn below(&mut self, n: u32) -> u32 {
            self.0 = self
                .0
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (self.0 >> 33) as u32 % n
        }
    }

    const SHADES: [[f32; 6]; 2] = [
        [0.5, 1.0, 0.8, 0.8, 0.6, 0.6],
        [0.9, 0.9, 0.8, 0.8, 0.6, 0.6],
    ];

    /// One made-up case: blocks and light in the 5 x 5 x 5 around a block, and a quad of
    /// it; what the vertices get.
    fn case(rng: &mut Rng) -> [u8; 12] {
        let kinds_n = PALETTE.len() as u32;
        let mut kinds = [0usize; 125];
        for k in &mut kinds {
            let r = rng.below(100);
            *k = match r {
                0..45 => 0,
                45..70 => 1,
                _ => 2 + rng.below(kinds_n - 2) as usize,
            };
        }
        let mut light = [(0u8, 0u8); 125];
        for l in &mut light {
            let r = rng.below(4);
            let block = if r == 0 { 0 } else { rng.below(16) as u8 };
            let sky = if r < 2 { 0 } else { rng.below(16) as u8 };
            *l = (block, sky);
        }
        kinds[62] = 1 + rng.below(kinds_n - 1) as usize;
        let direction = rng.below(6) as usize;
        let nether = rng.below(4) == 0;
        let shade_direction = if rng.below(5) == 0 {
            rng.below(6) as usize
        } else {
            direction
        };
        let emission = if rng.below(6) == 0 {
            rng.below(16) as u8
        } else {
            0
        };
        let shape = rng.below(7);
        let axis = [1, 1, 2, 2, 0, 0][direction];
        let ua = if axis == 0 { 1 } else { 0 };
        let va = if axis == 2 { 1 } else { 2 };
        let mut p = [[0.0_f32; 3]; 4];
        if shape == 5 {
            for corner in &mut p {
                for c in corner {
                    *c = rng.below(33) as f32 / 16.0 - 0.5;
                }
            }
        } else {
            let (mut u, mut v) = ([0.0_f32, 1.0], [0.0_f32, 1.0]);
            let outer = (direction & 1) as f32;
            let mut depth = outer;
            let mut rect = |rng: &mut Rng, n: u32, by: f32, less: f32| {
                u = [
                    rng.below(n) as f32 / by - less,
                    rng.below(n) as f32 / by - less,
                ];
                v = [
                    rng.below(n) as f32 / by - less,
                    rng.below(n) as f32 / by - less,
                ];
            };
            match shape {
                1 => depth = rng.below(17) as f32 / 16.0,
                2 => rect(rng, 17, 16.0, 0.0),
                3 => {
                    rect(rng, 1001, 1000.0, 0.0);
                    depth = rng.below(1001) as f32 / 1000.0;
                }
                4 => {
                    depth = 1.0 - outer;
                    if rng.below(2) == 0 {
                        rect(rng, 17, 16.0, 0.0);
                    }
                }
                6 => {
                    rect(rng, 49, 16.0, 1.0);
                    depth = rng.below(49) as f32 / 16.0 - 1.0;
                }
                _ => {}
            }
            let corners = [(u[0], v[0]), (u[0], v[1]), (u[1], v[1]), (u[1], v[0])];
            for (corner, (cu, cv)) in p.iter_mut().zip(corners) {
                corner[axis] = depth;
                corner[ua] = cu;
                corner[va] = cv;
            }
        }
        let face = Face::of(&p, direction, PALETTE[kinds[62]].0 & 16 != 0);
        let lit = smooth(
            &face,
            direction,
            SHADES[usize::from(nether)][shade_direction],
            |o| {
                assert!(o.iter().all(|c| c.abs() <= 2), "read at {o:?}");
                let i = (o[0] + 2 + 5 * (o[1] + 2) + 25 * (o[2] + 2)) as usize;
                let (flags, emits) = PALETTE[kinds[i]];
                let (block, sky) = light[i];
                if flags & 8 != 0 {
                    Cell {
                        block: 15,
                        sky: 15,
                        flags: flags & 7,
                    }
                } else {
                    Cell {
                        block: block.max(emits),
                        sky,
                        flags: flags & 7,
                    }
                }
            },
        );
        let mut out = [0; 12];
        for v in 0..4 {
            let l = with_emission(lit.light[v], emission);
            out[3 * v..3 * v + 3].copy_from_slice(&[lit.grey[v], l[0], l[1]]);
        }
        out
    }

    #[test]
    fn corners_are_lit_as_the_game_lights_them() {
        let mut rng = Rng(0x5275_7468_656e_6975);
        let mut hash = 0xcbf2_9ce4_8422_2325_u64;
        let mut wrong = Vec::new();
        for c in 0..CASES {
            let got = case(&mut rng);
            if let Some(want) = FIRST.get(c) {
                assert_eq!(got, *want, "case {c}");
            }
            for b in got {
                hash = (hash ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3);
            }
            if c % BATCH == BATCH - 1 {
                if hash != HASHES[c / BATCH] {
                    wrong.push(c / BATCH);
                }
                hash = 0xcbf2_9ce4_8422_2325;
            }
        }
        assert!(
            wrong.is_empty(),
            "{} batches differ, the first {}",
            wrong.len(),
            wrong[0]
        );
    }

    #[test]
    fn open_air_is_full_brightness_and_a_wall_beside_darkens_two_corners() {
        // The top of a block in the open: every place around is lit 15 by the sky.
        let open = |o: [i32; 3]| Cell {
            block: 0,
            sky: if o[1] >= 1 { 15 } else { 0 },
            flags: if o[1] >= 1 {
                Cell::PERMEABLE
            } else {
                Cell::DARK | Cell::SOLID
            },
        };
        let top = [
            [0.0, 1.0, 0.0],
            [0.0, 1.0, 1.0],
            [1.0, 1.0, 1.0],
            [1.0, 1.0, 0.0],
        ];
        let face = Face::of(&top, 1, true);
        assert!(face.cubic && !face.partial);
        let lit = smooth(&face, 1, 1.0, open);
        assert_eq!(lit.grey, [255; 4]);
        assert_eq!(lit.light, [[0, 240]; 4]);
        // A wall along the east: the two eastern corners lose light and brightness.
        let walled = |o: [i32; 3]| if o[0] >= 1 { open([0, 0, 0]) } else { open(o) };
        let lit = smooth(&face, 1, 1.0, walled);
        // Vertices 0 and 1 are the western ones (x 0).
        assert_eq!([lit.grey[0], lit.grey[1]], [255; 2]);
        assert!(lit.grey[2] < 255 && lit.grey[3] < 255);
        assert_eq!(lit.light[0], [0, 240]);
        // The dark places count as the centre where it is lit: no light is lost.
        assert_eq!(lit.light[2], [0, 240]);
    }

    #[test]
    fn a_quads_own_light_rounds_a_blend_down_to_whole_levels() {
        assert_eq!(with_emission([100, 37], 0), [100, 37]);
        assert_eq!(with_emission([100, 37], 3), [96, 48]);
        assert_eq!(with_emission([240, 0], 15), [240, 240]);
    }
}

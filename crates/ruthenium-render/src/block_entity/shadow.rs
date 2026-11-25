//! The shadows under entities: which blocks they lie on, and how dark they are there.

use ruthenium_javamath::mth;

/// Shadows reach no farther from their entity than this, in blocks.
const MAX_RADIUS: f32 = 32.0;
/// How far from the camera an entity has to be for its shadow to be gone, in blocks.
const GONE_AT: f64 = 16.0;
/// How much a shadow fades with each block it lies below its entity.
const FALLOFF: f32 = 0.5;
/// The light under which nothing casts a shadow.
const DARK: i32 = 3;

/// One block's piece of a shadow, from where its entity stands.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Piece {
    /// Its west, north, east and south edges.
    pub edges: [f32; 4],
    pub height: f32,
    /// How dark it is, 0 to 1.
    pub alpha: f32,
}

/// What a shadow may lie on: the block under a place.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Ground {
    /// The box around the block's shape: its west, bottom and north, then its east and
    /// south, in the block.
    pub bounds: [f32; 5],
    /// The light of the place over the block: the block light, or what the sky gives now
    /// if that is more (`getMaxLocalRawBrightness`).
    pub light: i32,
}

/// How strong the shadow of an entity is whose place (not blended between ticks) is this
/// far from the camera, squared: none from 16 blocks on.
pub(crate) fn power(away_squared: f64, strength: f32) -> f32 {
    ((1.0 - away_squared / (GONE_AT * GONE_AT)) * f64::from(strength)) as f32
}

/// `Lightmap.getBrightness`: how bright a light level is in a dimension with this ambient
/// light.
fn brightness(ambient: f32, light: i32) -> f32 {
    let v = light as f32 / 15.0;
    let curved = v / (4.0 - 3.0 * v);
    curved + ambient * (1.0 - curved)
}

/// Adds the pieces of the shadow of an entity at `at` (`EntityRenderer.extractShadow`):
/// one for every place around and below it whose `ground` takes a shadow, in the order
/// the game makes them.
pub(crate) fn pieces(
    at: [f64; 3],
    radius: f32,
    power: f32,
    ambient: f32,
    ground: impl Fn([i32; 3]) -> Option<Ground>,
    out: &mut Vec<Piece>,
) {
    // (Neither holds for what is not a number.)
    if !(radius > 0.0 && power > 0.0) {
        return;
    }
    let radius = radius.min(MAX_RADIUS);
    let reach = f64::from(radius);
    let (x0, x1) = (mth::floor(at[0] - reach), mth::floor(at[0] + reach));
    let (z0, z1) = (mth::floor(at[2] - reach), mth::floor(at[2] + reach));
    let depth = (power / FALLOFF - 1.0).min(radius);
    let (y0, y1) = (mth::floor(at[1] - f64::from(depth)), mth::floor(at[1]));
    for z in z0..=z1 {
        for x in x0..=x1 {
            for y in y0..=y1 {
                let Some(g) = ground([x, y, z]).filter(|g| g.light > DARK) else {
                    continue;
                };
                let here = power - (at[1] - f64::from(y)) as f32 * FALLOFF;
                let alpha = (here * 0.5 * brightness(ambient, g.light)).clamp(0.0, 1.0);
                let from = [x, y, z].map(f64::from);
                let from: [f32; 3] = std::array::from_fn(|i| (from[i] - at[i]) as f32);
                let [west, bottom, north, east, south] = g.bounds;
                out.push(Piece {
                    edges: [
                        from[0] + west,
                        from[2] + north,
                        from[0] + east,
                        from[2] + south,
                    ],
                    height: from[1] + bottom,
                    alpha,
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const WHOLE: [f32; 5] = [0.0, 0.0, 0.0, 1.0, 1.0];

    /// A floor whose top is at `y`, in full light.
    fn floor_at(y: i32) -> impl Fn([i32; 3]) -> Option<Ground> {
        move |pos| {
            (pos[1] == y).then_some(Ground {
                bounds: WHOLE,
                light: 15,
            })
        }
    }

    fn shadow(
        at: [f64; 3],
        radius: f32,
        power: f32,
        ground: impl Fn([i32; 3]) -> Option<Ground>,
    ) -> Vec<Piece> {
        let mut out = Vec::new();
        pieces(at, radius, power, 0.0, ground, &mut out);
        out
    }

    #[test]
    fn a_shadow_fades_with_the_distance_from_the_camera() {
        assert_eq!(power(0.0, 1.0), 1.0);
        assert_eq!(power(64.0, 1.0), 0.75);
        assert_eq!(power(256.0, 1.0), 0.0);
        assert!(power(400.0, 1.0) < 0.0);
        assert_eq!(power(64.0, 0.5), 0.375);
        assert!(shadow([0.5, 0.0, 0.5], 0.5, 0.0, floor_at(0)).is_empty());
        assert!(shadow([0.5, 0.0, 0.5], 0.0, 1.0, floor_at(0)).is_empty());
        assert!(shadow([0.5, 0.0, 0.5], f32::NAN, 1.0, floor_at(0)).is_empty());
    }

    #[test]
    fn a_shadow_lies_on_the_blocks_around_its_entity() {
        // In the middle of a block: that one and, as the game counts, the ones its far
        // edges touch.
        let on = shadow([0.5, 0.0, 0.5], 0.5, 1.0, floor_at(0));
        assert_eq!(on.len(), 4);
        // North first, west before east.
        assert_eq!(on[0].edges, [-0.5, -0.5, 0.5, 0.5]);
        assert_eq!(on[1].edges, [0.5, -0.5, 1.5, 0.5]);
        assert_eq!(on[2].edges, [-0.5, 0.5, 0.5, 1.5]);
        for p in &on {
            assert_eq!((p.height, p.alpha), (0.0, 0.5));
        }
        // On a corner: the four blocks that meet there, alike.
        let corner = shadow([1.0, 0.0, 1.0], 0.5, 1.0, floor_at(0));
        assert_eq!(corner.len(), 4);
        assert_eq!(corner[0].edges, [-1.0, -1.0, 0.0, 0.0]);
        assert_eq!(corner[3].edges, [0.0, 0.0, 1.0, 1.0]);
        // A wider one.
        assert_eq!(shadow([0.5, 0.0, 0.5], 1.5, 1.0, floor_at(0)).len(), 16);
        // Never wider than 32 blocks from its entity.
        let wide = shadow([0.5, 0.0, 0.5], 99.0, 1.0, floor_at(0));
        assert_eq!(wide.len(), 65 * 65);
    }

    #[test]
    fn a_shadow_is_looked_for_a_block_less_deep_than_it_reaches() {
        // One column of ground at every height.
        let column = |pos: [i32; 3]| {
            (pos[0] == 0 && pos[2] == 0).then_some(Ground {
                bounds: WHOLE,
                light: 15,
            })
        };
        let heights = |radius: f32, power: f32| -> Vec<f32> {
            shadow([0.5, 10.0, 0.5], radius, power, column)
                .iter()
                .map(|p| p.height)
                .collect()
        };
        // At full strength a block down, at one and a half two.
        assert_eq!(heights(3.0, 1.0), [-1.0, 0.0]);
        assert_eq!(heights(3.0, 1.5), [-2.0, -1.0, 0.0]);
        // And no deeper than it is wide.
        assert_eq!(heights(1.0, 1.5), [-1.0, 0.0]);
    }

    #[test]
    fn a_shadow_falls_as_far_as_it_is_strong() {
        // Half a block over the floor: weaker by a quarter.
        let over = shadow([0.5, 0.5, 0.5], 0.5, 1.0, floor_at(0));
        assert_eq!((over[0].height, over[0].alpha), (-0.5, 0.375));
        // A block and a half: out of the reach of full strength with this radius...
        assert!(shadow([0.5, 1.5, 0.5], 0.5, 1.0, floor_at(0)).is_empty());
        // ...but not of a larger one's.
        let large = shadow([0.5, 1.5, 0.5], 2.0, 1.0, floor_at(0));
        assert_eq!((large[0].height, large[0].alpha), (-1.5, 0.125));
        // Nor does a weak one reach down at all: only the block it stands in.
        assert!(shadow([0.5, 0.5, 0.5], 0.5, 0.4, floor_at(-1)).is_empty());
        let weak = shadow([0.5, 0.5, 0.5], 0.5, 0.4, floor_at(0));
        assert!((weak[0].alpha - (0.4 - 0.25) * 0.5).abs() < 1e-6);
        // Floors at two heights each take their piece.
        let both = shadow([0.5, 1.0, 0.5], 2.0, 1.0, |p| {
            floor_at(0)(p).or(floor_at(1)(p))
        });
        assert_eq!(both.len(), 2 * 25);
        assert_eq!((both[0].height, both[1].height), (-1.0, 0.0));
    }

    #[test]
    fn the_dark_and_the_dim_take_less() {
        let lit = |light| {
            move |pos: [i32; 3]| {
                (pos[1] == 0).then_some(Ground {
                    bounds: WHOLE,
                    light,
                })
            }
        };
        assert!(shadow([0.5, 0.0, 0.5], 0.5, 1.0, lit(3)).is_empty());
        let dim = shadow([0.5, 0.0, 0.5], 0.5, 1.0, lit(4));
        let curved = (4.0f32 / 15.0) / (4.0 - 3.0 * (4.0 / 15.0));
        assert_eq!(dim[0].alpha, 0.5 * curved);
        // The dimension's own light is added to what is missing.
        let mut out = Vec::new();
        pieces([0.5, 0.0, 0.5], 0.5, 1.0, 0.1, lit(4), &mut out);
        assert_eq!(out[0].alpha, 0.5 * (curved + 0.1 * (1.0 - curved)));
    }

    #[test]
    fn a_piece_is_as_wide_as_its_blocks_shape() {
        let slab = |pos: [i32; 3]| {
            (pos == [0, 0, 0]).then_some(Ground {
                bounds: [0.25, 0.5, 0.0, 0.75, 1.0],
                light: 15,
            })
        };
        let on = shadow([0.5, 0.0, 0.5], 0.5, 1.0, slab);
        assert_eq!(on.len(), 1);
        assert_eq!(on[0].edges, [-0.25, -0.5, 0.25, 0.5]);
        assert_eq!(on[0].height, 0.5);
    }
}

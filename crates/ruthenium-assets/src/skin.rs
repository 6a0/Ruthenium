//! Player skins as the game draws them: a picture of 64 by 64, made of an old one of 64 by
//! 32 if need be, opaque where it covers the body.

use crate::Image;
use crate::image::alpha;

const OPAQUE: u32 = 0xFF00_0000;

/// Parts of an old skin mirrored to make the arm and the leg it lacks: from x and y, by how
/// much, and their width and height.
const MIRRORED: [(u32, u32, i32, u32, u32, u32); 12] = [
    (4, 16, 16, 32, 4, 4),
    (8, 16, 16, 32, 4, 4),
    (0, 20, 24, 32, 4, 12),
    (4, 20, 16, 32, 4, 12),
    (8, 20, 8, 32, 4, 12),
    (12, 20, 16, 32, 4, 12),
    (44, 16, -8, 32, 4, 4),
    (48, 16, -8, 32, 4, 4),
    (40, 20, 0, 32, 4, 12),
    (44, 20, -8, 32, 4, 12),
    (48, 20, -16, 32, 4, 12),
    (52, 20, -8, 32, 4, 12),
];

/// A skin's picture made fit for drawing; `None` for what is no skin by its size.
pub fn process(skin: &Image) -> Option<Image> {
    let old = skin.height == 32;
    if skin.width != 64 || !(old || skin.height == 64) {
        return None;
    }
    let mut out = Image::new(64, 64);
    for y in 0..skin.height {
        for x in 0..64 {
            out.set(x, y, skin.get(x, y));
        }
    }
    if old {
        for (x, y, dx, dy, w, h) in MIRRORED {
            for j in 0..h {
                for i in 0..w {
                    let to = (x + w - 1 - i).wrapping_add_signed(dx);
                    out.set(to, y + dy + j, out.get(x + i, y + j));
                }
            }
        }
    }
    let opaque = |out: &mut Image, [x0, y0, x1, y1]: [u32; 4]| {
        for y in y0..y1 {
            for x in x0..x1 {
                out.set(x, y, out.get(x, y) | OPAQUE);
            }
        }
    };
    opaque(&mut out, [0, 0, 32, 16]);
    // An old skin whose hat covers everything has none.
    if old && (32..64).all(|x| (0..32).all(|y| alpha(out.get(x, y)) >= 128)) {
        for y in 0..32 {
            for x in 32..64 {
                out.set(x, y, out.get(x, y) & !OPAQUE);
            }
        }
    }
    opaque(&mut out, [0, 16, 64, 32]);
    opaque(&mut out, [16, 48, 48, 64]);
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::image::argb;

    /// Every texel another colour, half of them half clear.
    fn skin(height: u32) -> Image {
        let mut out = Image::new(64, height);
        for y in 0..height {
            for x in 0..64 {
                out.set(
                    x,
                    y,
                    argb(if x % 2 == 0 { 255 } else { 90 }, x * 4, y * 4, 7),
                );
            }
        }
        out
    }

    #[test]
    fn a_new_skin_is_opaque_where_it_covers_the_body() {
        let s = process(&skin(64)).unwrap();
        // The head, the body's row, the new arm and leg.
        for (x, y) in [(1, 1), (31, 15), (1, 16), (63, 31), (17, 48), (47, 63)] {
            assert_eq!(alpha(s.get(x, y)), 255, "{x} {y}");
            assert_eq!(s.get(x, y) & 0xFF_FFFF, skin(64).get(x, y) & 0xFF_FFFF);
        }
        // The hat, the second layers.
        for (x, y) in [
            (33, 1),
            (63, 15),
            (1, 33),
            (63, 47),
            (1, 48),
            (15, 63),
            (49, 50),
        ] {
            assert_eq!(alpha(s.get(x, y)), 90, "{x} {y}");
        }
    }

    #[test]
    fn an_old_skins_limbs_are_mirrored() {
        let from = skin(32);
        let s = process(&from).unwrap();
        let rgb = |c: u32| c & 0xFF_FFFF;
        // The top of the right leg becomes the left leg's, left to right.
        assert_eq!(rgb(s.get(20, 48)), rgb(from.get(7, 16)));
        assert_eq!(rgb(s.get(23, 51)), rgb(from.get(4, 19)));
        // Its outer side goes to the other side of the box.
        assert_eq!(rgb(s.get(24, 52)), rgb(from.get(3, 20)));
        // The arm: its outer side lands where it was across.
        assert_eq!(rgb(s.get(40, 63)), rgb(from.get(43, 31)));
        assert_eq!(rgb(s.get(32, 52)), rgb(from.get(51, 20)));
        assert_eq!(s.get(0, 40), 0);
        // The hat has clear texels: it stays.
        assert_eq!(alpha(s.get(32, 0)), 255);
    }

    #[test]
    fn an_old_skin_all_covered_by_its_hat_has_none() {
        let mut from = skin(32);
        for y in 0..32 {
            for x in 32..64 {
                from.set(x, y, argb(128, x, y, 1));
            }
        }
        let s = process(&from).unwrap();
        assert_eq!(alpha(s.get(40, 8)), 0);
        // The arms below the hat's row are the body's.
        assert_eq!(alpha(s.get(44, 20)), 255);
        from.set(63, 31, argb(127, 0, 0, 0));
        assert_eq!(alpha(process(&from).unwrap().get(40, 8)), 128);
        // A new skin's hat may cover everything.
        let mut new = Image::new(64, 64);
        for y in 0..64 {
            for x in 0..64 {
                new.set(x, y, argb(255, 1, 2, 3));
            }
        }
        assert_eq!(alpha(process(&new).unwrap().get(40, 8)), 255);
    }

    /// What the game's own code makes of three made-up skins (`PlayerGolden`): an old one,
    /// an old one all covered by its hat, and a new one.
    #[test]
    fn skins_are_made_as_the_game_makes_them() {
        let mut state = 31u64;
        let mut next = |n: u64| {
            state = state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            ((state >> 33) % n) as u32
        };
        let want = [
            0x3310_5a1c_4127_a3a0u64,
            0xe991_c9cd_617e_7a30,
            0x4abd_2ce9_9846_b263,
        ];
        for (k, want) in want.into_iter().enumerate() {
            let mut s = Image::new(64, if k == 2 { 64 } else { 32 });
            for y in 0..s.height {
                for x in 0..64 {
                    let a = if k == 1 { 128 + next(128) } else { next(256) };
                    s.set(x, y, a << 24 | next(1 << 24));
                }
            }
            let out = process(&s).unwrap();
            let mut hash = 0xcbf2_9ce4_8422_2325u64;
            for y in 0..64 {
                for x in 0..64 {
                    for b in out.get(x, y).to_le_bytes() {
                        hash = (hash ^ u64::from(b)).wrapping_mul(0x100_0000_01b3);
                    }
                }
            }
            assert_eq!(hash, want, "skin {k}");
        }
    }

    #[test]
    fn other_sizes_are_no_skins() {
        for (w, h) in [(64, 16), (32, 32), (128, 128), (64, 65)] {
            assert!(process(&Image::new(w, h)).is_none());
        }
    }
}

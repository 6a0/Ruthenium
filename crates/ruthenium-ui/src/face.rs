//! A player's face for the menus: the head's front from their skin with the hat layer
//! over it, as vanilla draws faces (`PlayerFaceExtractor`): the face itself opaque,
//! the hat at the same size. Enlarged with sharp pixels and given a mip chain, so the
//! renderer's linear sampling keeps it crisp at any menu scale.

use ruthenium_assets::Image;
use ruthenium_assets::image::{alpha, blue, green, red};

/// Texture pixels per skin pixel.
const ENLARGE: u32 = 8;
/// Side of the face's top level.
pub const FACE_SIZE: u32 = 8 * ENLARGE;

/// A face ready for the GPU.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Face {
    pub size: u32,
    /// RGBA8 (opaque, so premultiplied as it is), level 0 first, down to 1x1.
    pub levels: Vec<Vec<u8>>,
}

impl Face {
    /// The face from a skin PNG: 64 x 64, or a legacy 64 x 32 skin. Anything else is not
    /// a skin vanilla would load.
    pub fn from_skin(png: &[u8]) -> Option<Face> {
        let skin = Image::decode_png(png).ok()?;
        if skin.width != 64 || !(skin.height == 64 || skin.height == 32) {
            return None;
        }
        // A legacy skin whose hat layer has no transparency at all has no hat: vanilla
        // clears it (`SkinTextureDownloader`, Notch's transparency hack).
        let hat =
            skin.height == 64 || (32..64).any(|x| (0..32).any(|y| alpha(skin.get(x, y)) < 128));
        let mut face = [[0u8; 3]; 64];
        for y in 0..8 {
            for x in 0..8 {
                let base = skin.get(8 + x, 8 + y);
                let mut rgb = [red(base), green(base), blue(base)];
                if hat {
                    let over = skin.get(40 + x, 8 + y);
                    let a = alpha(over);
                    let top = [red(over), green(over), blue(over)];
                    for (c, t) in rgb.iter_mut().zip(top) {
                        *c = (t * a + *c * (255 - a) + 127) / 255;
                    }
                }
                face[(y * 8 + x) as usize] = rgb.map(|c| c as u8);
            }
        }
        let n = FACE_SIZE as usize;
        let mut top = Vec::with_capacity(n * n * 4);
        for y in 0..n {
            for x in 0..n {
                let [r, g, b] = face[(y / ENLARGE as usize) * 8 + x / ENLARGE as usize];
                top.extend_from_slice(&[r, g, b, 255]);
            }
        }
        let mut levels = vec![top];
        let mut side = n;
        while side > 1 {
            let prev = levels.last().expect("a level");
            let half = side / 2;
            let mut next = Vec::with_capacity(half * half * 4);
            for y in 0..half {
                for x in 0..half {
                    for c in 0..4 {
                        let at = |dx: usize, dy: usize| {
                            u32::from(prev[((2 * y + dy) * side + 2 * x + dx) * 4 + c])
                        };
                        next.push(((at(0, 0) + at(1, 0) + at(0, 1) + at(1, 1) + 2) / 4) as u8);
                    }
                }
            }
            levels.push(next);
            side = half;
        }
        Some(Face {
            size: FACE_SIZE,
            levels,
        })
    }
}

#[cfg(test)]
mod tests {
    use ruthenium_assets::image::argb;

    use super::*;

    fn skin(height: u32, hat: Option<u32>) -> Vec<u8> {
        let mut img = Image::new(64, height);
        for y in 8..16 {
            for x in 8..16 {
                img.set(x, y, argb(255, 200, 100, 50));
            }
            if let Some(c) = hat {
                for x in 40..48 {
                    img.set(x, y, c);
                }
            }
        }
        img.encode_png()
    }

    fn pixel(f: &Face, x: usize, y: usize) -> [u8; 4] {
        let i = (y * f.size as usize + x) * 4;
        f.levels[0][i..i + 4].try_into().unwrap()
    }

    #[test]
    fn the_hat_goes_over_the_face() {
        let f = Face::from_skin(&skin(64, None)).unwrap();
        assert_eq!(f.levels.len(), 7);
        assert_eq!(f.levels[6].len(), 4);
        assert_eq!(pixel(&f, 0, 0), [200, 100, 50, 255]);
        // A half-transparent hat mixes in.
        let f = Face::from_skin(&skin(64, Some(argb(128, 0, 0, 250)))).unwrap();
        let [r, _, b, a] = pixel(&f, 63, 63);
        assert!(
            (99..=101).contains(&r) && (148..=151).contains(&b) && a == 255,
            "{r} {b}"
        );
        // The 8 x 8 level is the face, pixel for pixel.
        assert_eq!(&f.levels[3][..4], &pixel(&f, 0, 0));
    }

    #[test]
    fn legacy_skins_with_an_opaque_hat_have_none() {
        let opaque_hat = Some(argb(255, 0, 0, 250));
        // A legacy skin whose whole hat area (32..64 x 0..32) is opaque.
        let mut legacy = Image::decode_png(&skin(32, opaque_hat)).unwrap();
        for y in 0..32 {
            for x in 32..64 {
                if alpha(legacy.get(x, y)) == 0 {
                    legacy.set(x, y, argb(255, 9, 9, 9));
                }
            }
        }
        let f = Face::from_skin(&legacy.encode_png()).unwrap();
        assert_eq!(pixel(&f, 10, 10), [200, 100, 50, 255]);
        // With some transparency there, the hat is kept.
        let f = Face::from_skin(&skin(32, opaque_hat)).unwrap();
        assert_eq!(pixel(&f, 10, 10), [0, 0, 250, 255]);
        // On a 64 x 64 skin the same hat counts.
        let f = Face::from_skin(&skin(64, opaque_hat)).unwrap();
        assert_eq!(pixel(&f, 10, 10), [0, 0, 250, 255]);
        let mut tall = Image::new(64, 128);
        tall.set(0, 0, argb(255, 1, 1, 1));
        assert_eq!(Face::from_skin(&tall.encode_png()), None);
        assert_eq!(Face::from_skin(b"not a png"), None);
    }
}

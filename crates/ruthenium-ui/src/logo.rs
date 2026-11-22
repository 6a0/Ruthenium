//! Ruthenium's logo mark (`branding/ruthenium.png`), prepared for the GPU: downscaled to
//! a power-of-two square with a full mip chain, in premultiplied alpha (so filtering never
//! darkens its edges), and a small straight-alpha copy for the window icon.

use image::RgbaImage;
use image::imageops::{self, FilterType};
use ruthenium_assets::Image;

/// The logo as committed.
pub const PNG: &[u8] = include_bytes!("../../../branding/ruthenium.png");

/// The logo's texture levels and window icon.
#[derive(Debug)]
pub struct Logo {
    /// Side of level 0.
    pub size: u32,
    /// RGBA8, premultiplied, level 0 first, down to 1x1.
    pub levels: Vec<Vec<u8>>,
    /// Side of the icon.
    pub icon_size: u32,
    /// RGBA8, straight alpha, for the window icon.
    pub icon: Vec<u8>,
}

fn decode() -> RgbaImage {
    let img = Image::decode_png(PNG).expect("the bundled logo decodes");
    RgbaImage::from_raw(img.width, img.height, img.to_rgba8()).expect("sizes match")
}

fn premultiply(img: &mut RgbaImage) {
    for p in img.pixels_mut() {
        let a = u32::from(p.0[3]);
        for v in &mut p.0[..3] {
            *v = ((u32::from(*v) * a + 127) / 255) as u8;
        }
    }
}

impl Logo {
    /// The logo at `size` (a power of two) with every mip level below, and an
    /// `icon_size` square icon.
    pub fn new(size: u32, icon_size: u32) -> Logo {
        assert!(size.is_power_of_two());
        let mut img = decode();
        let icon = imageops::resize(&img, icon_size, icon_size, FilterType::Triangle).into_raw();
        premultiply(&mut img);
        let mut level = imageops::resize(&img, size, size, FilterType::Triangle);
        let mut levels = vec![level.as_raw().clone()];
        while level.width() > 1 {
            let half = level.width() / 2;
            let mut next = RgbaImage::new(half, half);
            for y in 0..half {
                for x in 0..half {
                    let mut sum = [0u32; 4];
                    for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                        let p = level.get_pixel(2 * x + dx, 2 * y + dy).0;
                        for c in 0..4 {
                            sum[c] += u32::from(p[c]);
                        }
                    }
                    next.put_pixel(x, y, image::Rgba(sum.map(|s| ((s + 2) / 4) as u8)));
                }
            }
            levels.push(next.as_raw().clone());
            level = next;
        }
        Logo {
            size,
            levels,
            icon_size,
            icon,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mip_chain_down_to_one_texel() {
        let logo = Logo::new(64, 32);
        assert_eq!(logo.levels.len(), 7);
        for (i, l) in logo.levels.iter().enumerate() {
            let side = 64usize >> i;
            assert_eq!(l.len(), side * side * 4);
        }
        // Premultiplied: no channel exceeds alpha.
        for l in &logo.levels {
            assert!(
                l.as_chunks::<4>()
                    .0
                    .iter()
                    .all(|p| p[0] <= p[3] && p[1] <= p[3] && p[2] <= p[3])
            );
        }
        // The corners are transparent, the mark is mostly opaque somewhere.
        assert_eq!(logo.levels[0][3], 0);
        assert!(
            logo.levels[0]
                .as_chunks::<4>()
                .0
                .iter()
                .any(|p| p[3] == 255)
        );
        assert_eq!(logo.icon.len(), 32 * 32 * 4);
    }
}

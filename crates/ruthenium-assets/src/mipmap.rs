//! Sprite mip levels, built on the CPU exactly as 26.3 builds them
//! (`client/renderer/texture/MipmapGenerator.java`, `TextureUtil.java`
//! `solidify`/`fillEmptyAreasWithDarkColor`, `util/ARGB.java` linear means).
//!
//! Float maths uses `f32` in vanilla's evaluation order (Rust, like Java,
//! never fuses or reorders it), so results match bit for bit.

use std::collections::VecDeque;
use std::sync::LazyLock;

use crate::image::{Image, Transparency, alpha, argb, blue, green, red};
use crate::meta::MipmapStrategy;

/// Java's `Math.round(float)`: halves round up.
fn java_round(x: f32) -> i32 {
    (f64::from(x) + 0.5).floor() as i32
}

/// sRGB channel (0-255) to linear, scaled to 0-1023 (`ARGB.SRGB_TO_LINEAR`).
static SRGB_TO_LINEAR: LazyLock<[u16; 256]> = LazyLock::new(|| {
    std::array::from_fn(|i| {
        let x = i as f32 / 255.0;
        let lin = if x >= 0.04045 {
            ((f64::from(x) + 0.055) / 1.055).powf(2.4) as f32
        } else {
            x / 12.92
        };
        java_round(lin * 1023.0) as u16
    })
});

/// Linear (0-1023) to sRGB channel (`ARGB.LINEAR_TO_SRGB`).
static LINEAR_TO_SRGB: LazyLock<[u8; 1024]> = LazyLock::new(|| {
    std::array::from_fn(|i| {
        let x = i as f32 / 1023.0;
        let srgb = if x >= 0.0031308 {
            (1.055 * f64::from(x).powf(0.4166666666666667) - 0.055) as f32
        } else {
            12.92 * x
        };
        java_round(srgb * 255.0) as u8
    })
});

pub fn srgb_to_linear_table() -> &'static [u16; 256] {
    &SRGB_TO_LINEAR
}

pub fn linear_to_srgb_table() -> &'static [u8; 1024] {
    &LINEAR_TO_SRGB
}

fn linear_mean(c: [u32; 4]) -> u32 {
    let t = &*SRGB_TO_LINEAR;
    let sum: u32 = c.iter().map(|&v| u32::from(t[v as usize])).sum();
    u32::from(LINEAR_TO_SRGB[(sum / 4) as usize])
}

/// `ARGB.meanLinear`: alpha is a plain mean, colour a mean in linear light.
fn mean(c: [u32; 4]) -> u32 {
    argb(
        c.iter().map(|&p| alpha(p)).sum::<u32>() / 4,
        linear_mean(c.map(red)),
        linear_mean(c.map(green)),
        linear_mean(c.map(blue)),
    )
}

/// `MipmapGenerator.darkenedAlphaBlend`: transparent texels count as black.
fn darkened(c: [u32; 4]) -> u32 {
    let t = &*SRGB_TO_LINEAR;
    let lin = |v: u32| f32::from(t[v as usize]) / 1023.0;
    let (mut a, mut r, mut g, mut b) = (0.0f32, 0.0f32, 0.0f32, 0.0f32);
    for p in c {
        if alpha(p) != 0 {
            a += lin(alpha(p));
            r += lin(red(p));
            g += lin(green(p));
            b += lin(blue(p));
        }
    }
    let to = |v: f32| u32::from(LINEAR_TO_SRGB[(v / 4.0 * 1023.0).floor() as i32 as usize]);
    argb(to(a), to(r), to(g), to(b))
}

/// Alpha-0 texels take the colour of the nearest visible texel (by 4-way
/// steps), keeping alpha 0, so filtering never bleeds in black.
pub fn solidify(img: &mut Image) {
    let (w, h) = (img.width as i64, img.height as i64);
    let n = (w * h) as usize;
    if n == 0 {
        return;
    }
    let mut nearest = vec![0u32; n];
    let mut dist = vec![i32::MAX; n];
    // Use u32 indices (half the memory of usize on 64-bit) and only enqueue boundary
    // opaque pixels that touch at least one transparent pixel (preserving exact BFS order).
    let mut queue: VecDeque<u32> = VecDeque::new();
    // Seeded column by column, as vanilla does; ties resolve by this order.
    for x in 0..w {
        for y in 0..h {
            let i = (x + y * w) as usize;
            let c = img.pixels[i];
            if alpha(c) != 0 {
                dist[i] = 0;
                nearest[i] = c;
                let has_empty_neighbor = (x + 1 < w && alpha(img.pixels[i + 1]) == 0)
                    || (x > 0 && alpha(img.pixels[i - 1]) == 0)
                    || (y + 1 < h && alpha(img.pixels[i + w as usize]) == 0)
                    || (y > 0 && alpha(img.pixels[i - w as usize]) == 0);
                if has_empty_neighbor {
                    queue.push_back(i as u32);
                }
            }
        }
    }
    while let Some(i) = queue.pop_front() {
        let i = i as usize;
        let (x, y) = (i as i64 % w, i as i64 / w);
        for (dx, dy) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
            let (nx, ny) = (x + dx, y + dy);
            if nx >= 0 && ny >= 0 && nx < w && ny < h {
                let j = (nx + ny * w) as usize;
                if dist[j] > dist[i] + 1 {
                    dist[j] = dist[i] + 1;
                    nearest[j] = nearest[i];
                    queue.push_back(j as u32);
                }
            }
        }
    }
    for (i, p) in img.pixels.iter_mut().enumerate() {
        if alpha(*p) == 0 {
            *p = nearest[i] & 0x00FF_FFFF;
        }
    }
}

/// Alpha-0 texels become 3/4 of the darkest visible colour (lowest r+g+b,
/// first found column by column), with alpha 0.
pub fn fill_dark(img: &mut Image) {
    let mut darkest = 0xFFFF_FFFFu32;
    let mut min = u32::MAX;
    for x in 0..img.width {
        for y in 0..img.height {
            let c = img.get(x, y);
            if alpha(c) != 0 {
                let brightness = red(c) + green(c) + blue(c);
                if brightness < min {
                    min = brightness;
                    darkest = c;
                }
            }
        }
    }
    let fill = argb(
        0,
        3 * red(darkest) / 4,
        3 * green(darkest) / 4,
        3 * blue(darkest) / 4,
    );
    for p in &mut img.pixels {
        if alpha(*p) == 0 {
            *p = fill;
        }
    }
}

fn alpha_f(c: u32) -> f32 {
    alpha(c) as f32 / 255.0
}

/// Share of the image above `alpha_ref` under bilinear filtering, sampled
/// 4x4 per texel quad, with alpha scaled by `scale`.
fn coverage(img: &Image, alpha_ref: f32, scale: f32) -> f32 {
    let mut total = 0.0f32;
    let a = |x: u32, y: u32| (alpha_f(img.get(x, y)) * scale).clamp(0.0, 1.0);
    for y in 0..img.height.saturating_sub(1) {
        for x in 0..img.width.saturating_sub(1) {
            let (a00, a10, a01, a11) = (a(x, y), a(x + 1, y), a(x, y + 1), a(x + 1, y + 1));
            let mut texel = 0.0f32;
            for sy in 0..4 {
                let fy = (sy as f32 + 0.5) / 4.0;
                for sx in 0..4 {
                    let fx = (sx as f32 + 0.5) / 4.0;
                    let v = a00 * (1.0 - fx) * (1.0 - fy)
                        + a10 * fx * (1.0 - fy)
                        + a01 * (1.0 - fx) * fy
                        + a11 * fx * fy;
                    if v > alpha_ref {
                        texel += 1.0;
                    }
                }
            }
            total += texel / 16.0;
        }
    }
    // 0/0 (one texel wide or tall) is NaN, as in vanilla; the search below
    // then keeps a scale of 1.
    let cells = (img.width as i32 - 1) * (img.height as i32 - 1);
    total / cells as f32
}

/// Rescales alpha so the level keeps the coverage of level 0.
fn scale_to_coverage(img: &mut Image, desired: f32, alpha_ref: f32, bias: f32) {
    let (mut lo, mut hi) = (0.0f32, 4.0f32);
    let mut scale = 1.0f32;
    let mut best_scale = 1.0f32;
    let mut best_error = f32::MAX;
    for _ in 0..5 {
        let current = coverage(img, alpha_ref, scale);
        let error = (current - desired).abs();
        if error < best_error {
            best_error = error;
            best_scale = scale;
        }
        if current < desired {
            lo = scale;
        } else if current > desired {
            hi = scale;
        } else {
            break;
        }
        scale = (lo + hi) * 0.5;
    }
    for p in &mut img.pixels {
        let a = (alpha_f(*p) * best_scale + bias + 0.025).clamp(0.0, 1.0);
        *p = ((a * 255.0).floor() as i32 as u32) << 24 | (*p & 0x00FF_FFFF);
    }
}

/// Builds levels 0..=`levels` for one sprite image (all of its animation
/// frames at once, as vanilla does). Level 0 may be changed too: cutout
/// strategies fill transparent texels' colours, except for `item/` sprites.
pub fn generate(
    path: &str,
    base: Image,
    levels: u32,
    strategy: MipmapStrategy,
    alpha_cutoff_bias: f32,
    transparency: Transparency,
) -> Vec<Image> {
    let strategy = match strategy {
        MipmapStrategy::Auto if transparency.transparent => MipmapStrategy::Cutout,
        MipmapStrategy::Auto => MipmapStrategy::Mean,
        s => s,
    };
    let mut base = base;
    if !path.starts_with("item/") {
        match strategy {
            MipmapStrategy::Cutout | MipmapStrategy::StrictCutout => solidify(&mut base),
            MipmapStrategy::DarkCutout => fill_dark(&mut base),
            _ => {}
        }
    }
    let mut out = vec![base];
    if levels == 0 {
        return out;
    }
    let cutout = matches!(
        strategy,
        MipmapStrategy::Cutout | MipmapStrategy::StrictCutout | MipmapStrategy::DarkCutout
    );
    let alpha_ref = if strategy == MipmapStrategy::StrictCutout {
        0.3
    } else {
        0.5
    };
    let original = if cutout {
        coverage(&out[0], alpha_ref, 1.0)
    } else {
        0.0
    };
    for _ in 1..=levels {
        let last = out.last().unwrap();
        let mut next = Image::new(last.width >> 1, last.height >> 1);
        for y in 0..next.height {
            for x in 0..next.width {
                let c = [
                    last.get(2 * x, 2 * y),
                    last.get(2 * x + 1, 2 * y),
                    last.get(2 * x, 2 * y + 1),
                    last.get(2 * x + 1, 2 * y + 1),
                ];
                let v = if strategy == MipmapStrategy::DarkCutout {
                    darkened(c)
                } else {
                    mean(c)
                };
                next.set(x, y, v);
            }
        }
        if cutout {
            scale_to_coverage(&mut next, original, alpha_ref, alpha_cutoff_bias);
        }
        out.push(next);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tables_are_monotonic_and_end_to_end() {
        let s2l = srgb_to_linear_table();
        let l2s = linear_to_srgb_table();
        assert_eq!((s2l[0], s2l[255]), (0, 1023));
        assert_eq!((l2s[0], l2s[1023]), (0, 255));
        assert!(s2l.windows(2).all(|w| w[0] <= w[1]));
        assert!(l2s.windows(2).all(|w| w[0] <= w[1]));
    }

    #[test]
    fn mean_mip_of_flat_image_is_flat() {
        let img = Image {
            width: 4,
            height: 4,
            pixels: vec![0xFF336699; 16],
        };
        let t = img.transparency();
        let levels = generate("block/x", img, 2, MipmapStrategy::Auto, 0.0, t);
        assert_eq!(levels.len(), 3);
        assert_eq!((levels[2].width, levels[2].height), (1, 1));
        assert!(
            levels
                .iter()
                .all(|l| l.pixels.iter().all(|&p| p == 0xFF336699))
        );
    }

    #[test]
    fn solidify_spreads_nearest_colour() {
        let mut img = Image::new(3, 1);
        img.set(0, 0, 0xFF112233);
        img.set(2, 0, 0x80445566);
        solidify(&mut img);
        // The middle texel is one step from both; the first-seeded (x = 0) wins.
        assert_eq!(img.pixels, [0xFF112233, 0x00112233, 0x80445566]);
        // Item sprites skip it.
        let mut img = Image::new(2, 1);
        img.set(0, 0, 0xFF112233);
        let t = img.transparency();
        let out = generate("item/x", img.clone(), 0, MipmapStrategy::Auto, 0.0, t);
        assert_eq!(out[0], img);
    }

    #[test]
    fn dark_fill_uses_darkest_visible() {
        let mut img = Image::new(3, 1);
        img.set(0, 0, 0xFF808080);
        img.set(1, 0, 0xFF404040);
        fill_dark(&mut img);
        assert_eq!(img.get(2, 0), argb(0, 0x30, 0x30, 0x30));
    }

    #[test]
    fn cutout_keeps_alpha_coverage() {
        // A checkerboard of opaque and clear texels: mean alpha would be 50%,
        // which the coverage search pushes back to a 0/255-like split.
        let mut img = Image::new(8, 8);
        for y in 0..8 {
            for x in 0..8 {
                img.set(
                    x,
                    y,
                    if (x / 2 + y / 2) % 2 == 0 {
                        0xFFFFFFFF
                    } else {
                        0
                    },
                );
            }
        }
        let t = img.transparency();
        let levels = generate("block/x", img, 3, MipmapStrategy::Auto, 0.0, t);
        assert_eq!(levels.len(), 4);
        // The 1x1 level: NaN coverage keeps scale 1, so alpha = floor((a + 0.025) * 255).
        let a = alpha(levels[3].pixels[0]);
        assert!(a > 0, "{a}");
    }
}

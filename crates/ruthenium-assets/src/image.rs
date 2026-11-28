//! RGBA images as vanilla's `NativeImage` holds them after loading.

use std::fmt;

/// An 8-bit RGBA image. Pixels are row-major `0xAARRGGBB` words, the form
/// vanilla's pixel maths works on (`util/ARGB.java`).
#[derive(Clone, PartialEq, Eq)]
pub struct Image {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u32>,
}

impl fmt::Debug for Image {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Image({}x{})", self.width, self.height)
    }
}

pub fn alpha(c: u32) -> u32 {
    c >> 24
}
pub fn red(c: u32) -> u32 {
    (c >> 16) & 0xFF
}
pub fn green(c: u32) -> u32 {
    (c >> 8) & 0xFF
}
pub fn blue(c: u32) -> u32 {
    c & 0xFF
}
pub fn argb(a: u32, r: u32, g: u32, b: u32) -> u32 {
    (a & 0xFF) << 24 | (r & 0xFF) << 16 | (g & 0xFF) << 8 | (b & 0xFF)
}

/// Whether an image has fully transparent and partly transparent pixels
/// (`com/mojang/blaze3d/platform/Transparency.java`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Transparency {
    /// Some pixel has alpha 0.
    pub transparent: bool,
    /// Some pixel has alpha strictly between 0 and 255.
    pub translucent: bool,
}

impl Transparency {
    pub fn is_opaque(self) -> bool {
        !self.transparent && !self.translucent
    }
}

#[derive(Debug)]
pub struct ImageError(pub String);

impl fmt::Display for ImageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ImageError {}

/// The largest picture [`Image::decode_png`] takes on a side: twice the largest texture
/// any desktop GPU takes.
pub const DEFAULT_SIDE: u32 = 1 << 15;
/// The most texels [`Image::decode_png`] takes: 1 GiB decoded.
pub const DEFAULT_TEXELS: u64 = 1 << 28;

const PNG_SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', b'\r', b'\n', 0x1A, b'\n'];

impl Image {
    /// A transparent black image.
    pub fn new(width: u32, height: u32) -> Image {
        Image {
            width,
            height,
            pixels: vec![0; width as usize * height as usize],
        }
    }

    /// Decodes a PNG to RGBA the way vanilla's loader (stb_image) does:
    /// palette and grey images are expanded, `tRNS` becomes alpha, 16-bit
    /// channels keep their high byte, and gamma chunks are ignored. Only PNG
    /// is accepted.
    ///
    /// A picture larger than [`DEFAULT_SIDE`] on a side or [`DEFAULT_TEXELS`] in all is
    /// refused before anything is allocated for it: a PNG's header may claim any size.
    /// [`Self::decode_png_with`] takes other limits.
    pub fn decode_png(bytes: &[u8]) -> Result<Image, ImageError> {
        Image::decode_png_with(bytes, DEFAULT_SIDE, DEFAULT_TEXELS, &mut |_| true)
    }

    /// [`Self::decode_png`] for a picture no wider or taller than `max_side` and of no more
    /// than `max_texels`; `take` is asked for its texels before they are allocated (a
    /// load's budget) and refuses it by returning false.
    pub fn decode_png_with(
        bytes: &[u8],
        max_side: u32,
        max_texels: u64,
        take: &mut dyn FnMut(u64) -> bool,
    ) -> Result<Image, ImageError> {
        if !bytes.starts_with(&PNG_SIGNATURE) {
            return Err(ImageError("not a PNG file".into()));
        }
        let err = |e: png::DecodingError| ImageError(e.to_string());
        let mut decoder = png::Decoder::new(std::io::Cursor::new(bytes));
        // Vanilla reads pictures with STB, which checks neither a chunk's CRC nor the
        // image data's Adler-32: packs "protected" by item plugins break every CRC.
        decoder.ignore_checksums(true);
        decoder.set_ignore_text_chunk(true);
        decoder.set_ignore_iccp_chunk(true);
        decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
        let mut reader = decoder.read_info().map_err(err)?;
        let (w, h) = reader.info().size();
        let texels = u64::from(w) * u64::from(h);
        if w > max_side || h > max_side || texels > max_texels {
            return Err(ImageError(format!(
                "{w}x{h} is larger than a picture may be ({max_side} on a side, {max_texels} texels)"
            )));
        }
        if !take(texels) {
            return Err(ImageError(format!(
                "{w}x{h}: the pack's pictures have decoded to more than a load may"
            )));
        }
        let size = reader
            .output_buffer_size()
            .ok_or_else(|| ImageError("image too large".into()))?;
        let mut buf = vec![0u8; size];
        let info = reader.next_frame(&mut buf).map_err(err)?;
        let (w, h) = (info.width, info.height);
        let n = w as usize * h as usize;
        let mut pixels = Vec::with_capacity(n);
        let stride = info.line_size;
        let channels = match info.color_type {
            png::ColorType::Grayscale => 1,
            png::ColorType::GrayscaleAlpha => 2,
            png::ColorType::Rgb => 3,
            png::ColorType::Rgba => 4,
            png::ColorType::Indexed => return Err(ImageError("palette was not expanded".into())),
        };
        if info.bit_depth != png::BitDepth::Eight {
            return Err(ImageError("unexpected bit depth after expansion".into()));
        }
        if stride == 0 || buf.len() < stride * h as usize {
            return Err(ImageError("a frame smaller than its rows".into()));
        }
        for row in buf.chunks(stride).take(h as usize) {
            for p in row[..w as usize * channels].chunks_exact(channels) {
                let [r, g, b, a] = match *p {
                    [v] => [v, v, v, 255],
                    [v, a] => [v, v, v, a],
                    [r, g, b] => [r, g, b, 255],
                    [r, g, b, a] => [r, g, b, a],
                    _ => [0, 0, 0, 0],
                };
                pixels.push(argb(a.into(), r.into(), g.into(), b.into()));
            }
        }
        if pixels.len() != n {
            return Err(ImageError("a frame smaller than its rows".into()));
        }
        Ok(Image {
            width: w,
            height: h,
            pixels,
        })
    }

    /// Encodes as an 8-bit RGBA PNG (for dumps).
    pub fn encode_png(&self) -> Vec<u8> {
        let mut out = Vec::new();
        let mut enc = png::Encoder::new(&mut out, self.width, self.height);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        let mut w = enc.write_header().expect("writing to a Vec");
        w.write_image_data(&self.to_rgba8())
            .expect("writing to a Vec");
        w.finish().expect("writing to a Vec");
        out
    }

    /// Bytes in R, G, B, A order (`VK_FORMAT_R8G8B8A8_*`).
    pub fn to_rgba8(&self) -> Vec<u8> {
        self.pixels
            .iter()
            .flat_map(|&c| [red(c) as u8, green(c) as u8, blue(c) as u8, alpha(c) as u8])
            .collect()
    }

    pub fn get(&self, x: u32, y: u32) -> u32 {
        self.pixels[(y * self.width + x) as usize]
    }

    pub fn set(&mut self, x: u32, y: u32, c: u32) {
        self.pixels[(y * self.width + x) as usize] = c;
    }

    /// A copy of a rectangle, or `None` if it is not inside the image.
    pub fn sub_image(&self, x: u32, y: u32, width: u32, height: u32) -> Option<Image> {
        if x.checked_add(width)? > self.width || y.checked_add(height)? > self.height {
            return None;
        }
        let mut out = Image::new(width, height);
        for row in 0..height {
            let src = ((y + row) * self.width + x) as usize;
            let dst = (row * width) as usize;
            out.pixels[dst..dst + width as usize]
                .copy_from_slice(&self.pixels[src..src + width as usize]);
        }
        Some(out)
    }

    pub fn transparency(&self) -> Transparency {
        let mut t = Transparency::default();
        for &c in &self.pixels {
            match alpha(c) {
                0 => t.transparent = true,
                255 => {}
                _ => t.translucent = true,
            }
        }
        t
    }

    /// Vanilla's missing texture: magenta `#F800F8` and black quadrants
    /// (`client/renderer/texture/MissingTextureAtlasSprite.java`).
    pub fn missing(width: u32, height: u32) -> Image {
        let mut img = Image::new(width, height);
        for y in 0..height {
            for x in 0..width {
                let magenta = (y < height / 2) ^ (x < width / 2);
                img.set(x, y, if magenta { 0xFFF8_00F8 } else { 0xFF00_0000 });
            }
        }
        img
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn encode(
        w: u32,
        h: u32,
        color: png::ColorType,
        depth: png::BitDepth,
        data: &[u8],
        trns: Option<&[u8]>,
        palette: Option<&[u8]>,
    ) -> Vec<u8> {
        let mut out = Vec::new();
        let mut enc = png::Encoder::new(&mut out, w, h);
        enc.set_color(color);
        enc.set_depth(depth);
        if let Some(p) = palette {
            enc.set_palette(p.to_vec());
        }
        if let Some(t) = trns {
            enc.set_trns(t.to_vec());
        }
        let mut wr = enc.write_header().unwrap();
        wr.write_image_data(data).unwrap();
        wr.finish().unwrap();
        out
    }

    #[test]
    fn decodes_like_stb() {
        use png::{BitDepth::*, ColorType::*};
        let rgba = encode(2, 1, Rgba, Eight, &[1, 2, 3, 4, 5, 6, 7, 8], None, None);
        assert_eq!(
            Image::decode_png(&rgba).unwrap().pixels,
            [0x04010203, 0x08050607]
        );
        let grey = encode(2, 1, Grayscale, Eight, &[10, 200], None, None);
        assert_eq!(
            Image::decode_png(&grey).unwrap().pixels,
            [0xFF0A0A0A, 0xFFC8C8C8]
        );
        // 16-bit: the high byte.
        let g16 = encode(
            1,
            1,
            GrayscaleAlpha,
            Sixteen,
            &[0x12, 0x34, 0x80, 0xFF],
            None,
            None,
        );
        assert_eq!(Image::decode_png(&g16).unwrap().pixels, [0x80121212]);
        // Palette with tRNS: index 1 is half transparent.
        let pal = encode(
            2,
            1,
            Indexed,
            Eight,
            &[0, 1],
            Some(&[255, 128]),
            Some(&[255, 0, 0, 0, 0, 255]),
        );
        assert_eq!(
            Image::decode_png(&pal).unwrap().pixels,
            [0xFFFF0000, 0x800000FF]
        );
        // 1-bit grey scales 1 to 255.
        let bits = encode(2, 1, Grayscale, One, &[0b0100_0000], None, None);
        assert_eq!(
            Image::decode_png(&bits).unwrap().pixels,
            [0xFF000000, 0xFFFFFFFF]
        );
        assert!(Image::decode_png(b"GIF89a").is_err());
    }

    #[test]
    fn missing_texture_quadrants() {
        let m = Image::missing(16, 16);
        assert_eq!(m.get(0, 0), 0xFF00_0000);
        assert_eq!(m.get(8, 0), 0xFFF8_00F8);
        assert_eq!(m.get(0, 8), 0xFFF8_00F8);
        assert_eq!(m.get(15, 15), 0xFF00_0000);
    }
}

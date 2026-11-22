//! The starfield behind the menus: a square tile of stars baked once at start-up and drawn
//! one texel per pixel, repeating, over the background gradient.

/// Side of the tile, in texels.
pub const SIZE: u32 = 512;
/// Fixed, so the sky is the same at every launch.
pub const SEED: u64 = 0x5275_7468_656e_6975;
/// Texels per star.
const SPARSENESS: u32 = 420;
/// Stars at least this bright get a one-texel halo.
const HALO_FROM: f32 = 0.7;
const HALO: f32 = 0.3;
/// The colour range, from hot (blue-white) to cool (warm white).
const HOT: [f32; 3] = [0.78, 0.86, 1.0];
const COOL: [f32; 3] = [1.0, 0.91, 0.78];

/// SplitMix64: small, fast and good enough for scattering stars.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Uniform in [0, 1).
    fn unit(&mut self) -> f32 {
        (self.next() >> 40) as f32 / (1u64 << 24) as f32
    }

    fn below(&mut self, n: u32) -> u32 {
        (((self.next() >> 32) * u64::from(n)) >> 32) as u32
    }
}

/// The tile as RGBA8, rows top first. Colours are premultiplied by brightness (black is
/// empty sky), so the shader adds them to the gradient. Stars near an edge wrap around,
/// so the tile repeats without seams.
pub fn bake(seed: u64) -> Vec<u8> {
    let mut rgba = vec![0u8; (SIZE * SIZE * 4) as usize];
    for i in 0..SIZE * SIZE {
        rgba[i as usize * 4 + 3] = 255;
    }
    let mut rng = Rng(seed);
    let mut plot = |x: u32, y: u32, color: [f32; 3], brightness: f32| {
        let i = (((y % SIZE) * SIZE + x % SIZE) * 4) as usize;
        for c in 0..3 {
            let v = (color[c] * brightness * 255.0).round() as u8;
            rgba[i + c] = rgba[i + c].max(v);
        }
    };
    for _ in 0..SIZE * SIZE / SPARSENESS {
        let (x, y) = (rng.below(SIZE), rng.below(SIZE));
        // Mostly faint stars, a few bright ones.
        let brightness = 0.12 + 0.88 * rng.unit().powi(6);
        let t = rng.unit();
        let color: [f32; 3] = std::array::from_fn(|c| HOT[c] + (COOL[c] - HOT[c]) * t);
        plot(x, y, color, brightness);
        if brightness >= HALO_FROM {
            let halo = brightness * HALO;
            plot(x + 1, y, color, halo);
            plot(x + SIZE - 1, y, color, halo);
            plot(x, y + 1, color, halo);
            plot(x, y + SIZE - 1, color, halo);
        }
    }
    rgba
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_seed_same_sky() {
        assert_eq!(bake(SEED), bake(SEED));
        assert_ne!(bake(SEED), bake(SEED + 1));
    }

    #[test]
    fn mostly_empty_sky_with_a_few_bright_stars() {
        let sky = bake(SEED);
        assert_eq!(sky.len(), (SIZE * SIZE * 4) as usize);
        let texels: &[[u8; 4]] = sky.as_chunks::<4>().0;
        assert!(texels.iter().all(|t| t[3] == 255));
        let lit = texels.iter().filter(|t| t[..3] != [0, 0, 0]).count();
        let bright = texels
            .iter()
            .filter(|t| t[..3].iter().any(|&c| c > 200))
            .count();
        let n = texels.len();
        assert!(n / 1000 < lit && lit < n / 100, "{lit} lit texels of {n}");
        assert!(bright > 0 && bright < lit / 10, "{bright} bright of {lit}");
    }
}

//! The armour trims' pictures in their palettes, as far as they were asked for: kept up to
//! [`KEPT_TRIMS`] trims and [`KEPT_TRIM_TEXELS`] texels, the one drawn least lately let go
//! of past that (its texture handed back to be destroyed), and coloured at most
//! [`TRIM_TEXELS_PER_FRAME`] texels a frame (a large picture over several frames, as a
//! [`Colouring`]), so that a pack's large pictures or many trims at once can't stall a
//! frame.
//!
//! Vanilla puts every pattern in every material into one atlas (`armor_trims`): a pack's
//! pictures may be any size that atlas takes. So may ours, within the room of the largest
//! atlas a server pack may make (`Limits::trim_texels`).

use std::collections::HashMap;
use std::hash::Hash;

use super::equipment::{PaletteMapping, apply_palette};

/// Trims whose pictures are kept.
pub(crate) const KEPT_TRIMS: usize = 1024;
/// Texels of the coloured trims kept at once (on the GPU): what vanilla's trims atlas may
/// hold with a server's packs (`Limits::trim_texels`, 8192 x 8192, 256 MiB). One trim
/// larger than all of it is not drawn (no picture of a server's pack can be).
pub(crate) const KEPT_TRIM_TEXELS: usize =
    ruthenium_assets::Limits::untrusted().trim_texels as usize;
/// The largest palette a trim is coloured with (colours): vanilla's have 8.
pub(crate) const MAX_PALETTE_COLOURS: usize = 256;
/// Texels of trim pictures coloured in one frame (on the render thread): 64 of vanilla's
/// (64 by 32). A larger picture is coloured this much a frame until it is done.
pub(crate) const TRIM_TEXELS_PER_FRAME: usize = 512 * 256;

/// Whether a trim picture of `texels` coloured with `palettes` (RGBA) is made: within
/// [`KEPT_TRIM_TEXELS`] and [`MAX_PALETTE_COLOURS`].
pub(crate) fn trim_allowed(texels: usize, palettes: &[&[u8]]) -> bool {
    texels <= KEPT_TRIM_TEXELS && palettes.iter().all(|p| p.len() <= MAX_PALETTE_COLOURS * 4)
}

/// A large trim's picture being coloured a share a frame.
#[derive(Debug)]
pub(crate) struct Colouring<K> {
    pub key: K,
    pub width: u32,
    pub height: u32,
    mapping: Option<PaletteMapping>,
    /// The texels coloured so far (RGBA), from the top.
    out: Vec<u8>,
}

impl<K> Colouring<K> {
    pub fn new(key: K, width: u32, height: u32, mapping: Option<PaletteMapping>) -> Self {
        Colouring {
            key,
            width,
            height,
            mapping,
            out: Vec::new(),
        }
    }

    /// Colours up to `texels` more of `source` (the picture, RGBA); its texels once it
    /// is all done. A source of another size than the one begun with ends it, empty.
    pub fn step(&mut self, source: &[u8], texels: usize) -> Option<Vec<u8>> {
        let whole = self.width as usize * self.height as usize * 4;
        if source.len() != whole {
            return Some(Vec::new());
        }
        if self.out.capacity() < whole {
            self.out.reserve_exact(whole - self.out.len());
        }
        let start = self.out.len();
        let end = whole.min(start.saturating_add(texels.max(1).saturating_mul(4)));
        self.out.extend_from_slice(&source[start..end]);
        if let Some(m) = &self.mapping {
            apply_palette(&mut self.out[start..end], m);
        }
        (end == whole).then(|| std::mem::take(&mut self.out))
    }
}

/// See the module docs. `T` is a texture.
#[derive(Debug)]
pub(crate) struct TrimCache<K, T> {
    /// Each trim's texture (once made), when it was last drawn and its texels.
    kept: HashMap<K, (Option<T>, u64, usize)>,
    /// Texels of all the kept trims, made or on their way.
    kept_texels: usize,
    wanted: Vec<K>,
    uses: u64,
    gone: Vec<T>,
}

impl<K, T> Default for TrimCache<K, T> {
    fn default() -> Self {
        TrimCache {
            kept: HashMap::new(),
            kept_texels: 0,
            wanted: Vec::new(),
            uses: 0,
            gone: Vec::new(),
        }
    }
}

impl<K: Clone + Eq + Hash, T: Copy> TrimCache<K, T> {
    /// A trim to draw, of `texels`: its texture once it is made (`None` until then, and
    /// for one that can't be). One not kept is asked for; past [`KEPT_TRIMS`] trims or
    /// [`KEPT_TRIM_TEXELS`] texels, the ones used least lately go.
    pub fn get(&mut self, key: &K, texels: usize) -> Option<T> {
        self.uses += 1;
        if let Some((texture, used, _)) = self.kept.get_mut(key) {
            *used = self.uses;
            return *texture;
        }
        if texels > KEPT_TRIM_TEXELS {
            return None;
        }
        while (self.kept.len() >= KEPT_TRIMS
            || self.kept_texels.saturating_add(texels) > KEPT_TRIM_TEXELS)
            && let Some(oldest) = self
                .kept
                .iter()
                .min_by_key(|(_, (_, used, _))| *used)
                .map(|(k, _)| k.clone())
            && let Some((texture, _, size)) = self.kept.remove(&oldest)
        {
            self.kept_texels -= size;
            self.gone.extend(texture);
            self.wanted.retain(|k| *k != oldest);
        }
        self.kept.insert(key.clone(), (None, self.uses, texels));
        self.kept_texels += texels;
        self.wanted.push(key.clone());
        None
    }

    /// Whether the trim is kept (not let go of since it was asked for).
    pub fn is_kept(&self, key: &K) -> bool {
        self.kept.contains_key(key)
    }

    /// A trim whose texture couldn't be made for now: asked for again.
    pub fn again(&mut self, key: &K) {
        if self.kept.contains_key(key) && !self.wanted.contains(key) {
            self.wanted.push(key.clone());
        }
    }

    /// The trims to make this frame, oldest asked for first, within
    /// [`TRIM_TEXELS_PER_FRAME`] of `texels` (at least one).
    pub fn take_wanted(&mut self, texels: impl Fn(&K) -> usize) -> Vec<K> {
        let mut spent = 0;
        let mut n = 0;
        for key in &self.wanted {
            let size = texels(key);
            if n > 0 && spent + size > TRIM_TEXELS_PER_FRAME {
                break;
            }
            spent += size;
            n += 1;
        }
        self.wanted.drain(..n).collect()
    }

    /// A trim's texture is made: kept, or handed back to be destroyed if the trim was let
    /// go of meanwhile.
    pub fn made(&mut self, key: &K, texture: Option<T>) -> Option<T> {
        match self.kept.get_mut(key) {
            Some(slot) => {
                slot.0 = texture;
                None
            }
            None => texture,
        }
    }

    /// The textures let go of since the last call, to destroy.
    pub fn take_gone(&mut self) -> Vec<T> {
        std::mem::take(&mut self.gone)
    }

    /// Every texture it has, to destroy, and nothing kept any more.
    pub fn clear(&mut self) -> Vec<T> {
        let mut out = self.take_gone();
        out.extend(self.kept.drain().filter_map(|(_, (t, _, _))| t));
        self.kept_texels = 0;
        self.wanted.clear();
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Past the cap the trim drawn least lately goes, and its texture is handed back to
    /// be destroyed (not forgotten with the rest, as before).
    #[test]
    fn the_least_lately_drawn_trim_goes_with_its_texture() {
        let mut c: TrimCache<usize, u32> = TrimCache::default();
        for k in 0..KEPT_TRIMS {
            assert_eq!(c.get(&k, 1), None);
        }
        for k in c.take_wanted(|_| 1) {
            assert_eq!(c.made(&k, Some(k as u32 + 1000)), None);
        }
        assert_eq!(c.get(&0, 1), Some(1000));
        c.get(&KEPT_TRIMS, 1);
        // 0 was used last of all but the new one: 1 goes.
        assert_eq!(c.take_gone(), [1001]);
        assert_eq!(c.kept.len(), KEPT_TRIMS);
        assert_eq!(c.get(&0, 1), Some(1000));
        // One let go of while it was being made: its texture comes back at once.
        let mut c: TrimCache<usize, u32> = TrimCache::default();
        c.get(&5, 1);
        let make = c.take_wanted(|_| 1);
        c.clear();
        assert_eq!(c.made(&make[0], Some(7)), Some(7));
    }

    /// Pictures and palettes past the caps are not coloured.
    #[test]
    fn oversized_trims_are_not_made() {
        let eight = [0u8; 32];
        assert!(trim_allowed(64 * 32, &[&eight, &eight]));
        // As large as vanilla's trims atlas may be with a server's packs, not a fixed
        // size: a 4096 x 4096 picture (a server's largest) is coloured.
        assert!(trim_allowed(4096 * 4096, &[]));
        assert!(trim_allowed(KEPT_TRIM_TEXELS, &[]));
        assert!(!trim_allowed(KEPT_TRIM_TEXELS + 1, &[]));
        let huge = vec![0u8; (MAX_PALETTE_COLOURS + 1) * 4];
        assert!(!trim_allowed(64 * 32, &[&eight, &huge]));
    }

    /// A frame makes at most its share of texels, and always one.
    #[test]
    fn a_frame_colours_a_bounded_share() {
        let mut c: TrimCache<usize, u32> = TrimCache::default();
        for k in 0..100 {
            c.get(&k, 1);
        }
        let small = 64 * 32;
        let made = c.take_wanted(|_| small);
        assert_eq!(made.len(), TRIM_TEXELS_PER_FRAME / small);
        let made = c.take_wanted(|_| TRIM_TEXELS_PER_FRAME * 4);
        assert_eq!(made.len(), 1);
    }

    /// The kept trims take at most [`KEPT_TRIM_TEXELS`]: a large one makes room by
    /// letting the least lately drawn go (their textures handed back), and one larger
    /// than all the room is never asked for.
    #[test]
    fn the_kept_trims_take_at_most_their_room_in_texels() {
        let mut c: TrimCache<usize, u32> = TrimCache::default();
        let quarter = KEPT_TRIM_TEXELS / 4;
        for k in 0..4 {
            assert_eq!(c.get(&k, quarter), None);
        }
        for k in c.take_wanted(|_| 1) {
            c.made(&k, Some(k as u32 + 100));
        }
        assert_eq!(c.get(&0, quarter), Some(100));
        assert!(c.take_gone().is_empty());
        // Half of the room: the two drawn least lately (1 and 2) go.
        c.get(&9, 2 * quarter);
        let mut gone = c.take_gone();
        gone.sort_unstable();
        assert_eq!(gone, [101, 102]);
        assert_eq!(c.kept_texels, KEPT_TRIM_TEXELS);
        assert!(c.is_kept(&0) && c.is_kept(&3) && c.is_kept(&9));
        // Too large for all of it: not asked for, nothing let go of.
        assert_eq!(c.get(&10, KEPT_TRIM_TEXELS + 1), None);
        assert!(!c.is_kept(&10) && c.take_gone().is_empty());
        assert_eq!(c.clear().len(), 2);
        assert_eq!(c.kept_texels, 0);
    }

    /// A large picture is coloured a share a frame, and comes out as it would whole.
    #[test]
    fn a_large_trim_is_coloured_over_several_frames() {
        let (w, h) = (1024u32, 512u32);
        let picture: Vec<u8> = (0..w * h)
            .flat_map(|i| [(i % 3) as u8 * 10, 0, 0, 255])
            .collect();
        let base = [10, 0, 0, 255, 20, 0, 0, 255];
        let target = [1, 2, 3, 255, 4, 5, 6, 128];
        let whole = super::super::equipment::map_palette(&picture, &base, &target);
        let mapping = super::super::equipment::palette_mapping(&base, &target);
        let mut c = Colouring::new((), w, h, mapping);
        let mut frames = 0;
        let out = loop {
            frames += 1;
            if let Some(out) = c.step(&picture, TRIM_TEXELS_PER_FRAME) {
                break out;
            }
            assert!(c.out.len() <= frames * TRIM_TEXELS_PER_FRAME * 4);
        };
        assert_eq!(frames, (w * h) as usize / TRIM_TEXELS_PER_FRAME);
        assert_eq!(out, whole);
        // A picture of another size (the pack replaced meanwhile) ends it, empty.
        let mut c = Colouring::new((), w, h, None);
        assert_eq!(
            c.step(&picture[..16], TRIM_TEXELS_PER_FRAME),
            Some(Vec::new())
        );
    }
}

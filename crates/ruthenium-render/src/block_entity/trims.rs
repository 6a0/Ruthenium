//! The armour trims' pictures in their palettes, as far as they were asked for: kept up to
//! [`KEPT_TRIMS`], the one drawn least lately let go of past that (its texture handed back
//! to be destroyed), and made at most [`TRIM_TEXELS_PER_FRAME`] texels a frame, so that a
//! pack's large pictures or many trims at once can't stall a frame.

use std::collections::HashMap;
use std::hash::Hash;

/// Trims whose pictures are kept.
pub(crate) const KEPT_TRIMS: usize = 1024;
/// The largest trim picture coloured (texels): vanilla's are 64 by 32; this is eight
/// times that each way, for packs drawn finer. A larger one from a pack is not drawn.
pub(crate) const MAX_TRIM_TEXELS: usize = 512 * 256;
/// The largest palette a trim is coloured with (colours): vanilla's have 8.
pub(crate) const MAX_PALETTE_COLOURS: usize = 256;
/// Texels of trim pictures coloured in one frame (on the render thread): one of the
/// largest, or 64 of vanilla's. The rest wait for the next frames.
pub(crate) const TRIM_TEXELS_PER_FRAME: usize = MAX_TRIM_TEXELS;

/// Whether a trim picture of `texels` coloured with `palettes` (RGBA) is made: within
/// [`MAX_TRIM_TEXELS`] and [`MAX_PALETTE_COLOURS`].
pub(crate) fn trim_allowed(texels: usize, palettes: &[&[u8]]) -> bool {
    texels <= MAX_TRIM_TEXELS && palettes.iter().all(|p| p.len() <= MAX_PALETTE_COLOURS * 4)
}

/// See the module docs. `T` is a texture.
#[derive(Debug)]
pub(crate) struct TrimCache<K, T> {
    kept: HashMap<K, (Option<T>, u64)>,
    wanted: Vec<K>,
    uses: u64,
    gone: Vec<T>,
}

impl<K, T> Default for TrimCache<K, T> {
    fn default() -> Self {
        TrimCache {
            kept: HashMap::new(),
            wanted: Vec::new(),
            uses: 0,
            gone: Vec::new(),
        }
    }
}

impl<K: Clone + Eq + Hash, T: Copy> TrimCache<K, T> {
    /// A trim to draw: its texture once it is made (`None` until then, and for one that
    /// can't be). One not kept is asked for; past [`KEPT_TRIMS`] the one used least
    /// lately goes.
    pub fn get(&mut self, key: &K) -> Option<T> {
        self.uses += 1;
        if let Some((texture, used)) = self.kept.get_mut(key) {
            *used = self.uses;
            return *texture;
        }
        if self.kept.len() >= KEPT_TRIMS
            && let Some(oldest) = self
                .kept
                .iter()
                .min_by_key(|(_, (_, used))| *used)
                .map(|(k, _)| k.clone())
            && let Some((texture, _)) = self.kept.remove(&oldest)
        {
            self.gone.extend(texture);
            self.wanted.retain(|k| *k != oldest);
        }
        self.kept.insert(key.clone(), (None, self.uses));
        self.wanted.push(key.clone());
        None
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
        out.extend(self.kept.drain().filter_map(|(_, (t, _))| t));
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
            assert_eq!(c.get(&k), None);
        }
        for k in c.take_wanted(|_| 1) {
            assert_eq!(c.made(&k, Some(k as u32 + 1000)), None);
        }
        assert_eq!(c.get(&0), Some(1000));
        c.get(&KEPT_TRIMS);
        // 0 was used last of all but the new one: 1 goes.
        assert_eq!(c.take_gone(), [1001]);
        assert_eq!(c.kept.len(), KEPT_TRIMS);
        assert_eq!(c.get(&0), Some(1000));
        // One let go of while it was being made: its texture comes back at once.
        let mut c: TrimCache<usize, u32> = TrimCache::default();
        c.get(&5);
        let make = c.take_wanted(|_| 1);
        c.clear();
        assert_eq!(c.made(&make[0], Some(7)), Some(7));
    }

    /// Pictures and palettes past the caps are not coloured.
    #[test]
    fn oversized_trims_are_not_made() {
        let eight = [0u8; 32];
        assert!(trim_allowed(64 * 32, &[&eight, &eight]));
        assert!(trim_allowed(MAX_TRIM_TEXELS, &[]));
        assert!(!trim_allowed(MAX_TRIM_TEXELS + 1, &[]));
        let huge = vec![0u8; (MAX_PALETTE_COLOURS + 1) * 4];
        assert!(!trim_allowed(64 * 32, &[&eight, &huge]));
    }

    /// A frame makes at most its share of texels, and always one.
    #[test]
    fn a_frame_colours_a_bounded_share() {
        let mut c: TrimCache<usize, u32> = TrimCache::default();
        for k in 0..100 {
            c.get(&k);
        }
        let small = 64 * 32;
        let made = c.take_wanted(|_| small);
        assert_eq!(made.len(), TRIM_TEXELS_PER_FRAME / small);
        let made = c.take_wanted(|_| TRIM_TEXELS_PER_FRAME * 4);
        assert_eq!(made.len(), 1);
    }
}

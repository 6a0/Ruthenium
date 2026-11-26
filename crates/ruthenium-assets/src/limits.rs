//! How much of anything a pack may make the client read, decode, hold or build.
//!
//! A server's resource pack is downloaded from wherever the server says and loaded over the
//! game's own assets, so every byte of it is hostile: it may lie in its zip headers, claim
//! pictures of a million texels a side, nest JSON as deep as it likes, give a model a parent
//! that is its own child or an animation of a million frames. The readers check each
//! number against [`Limits`]; what is over a limit fails as a broken file fails in vanilla
//! (the file is skipped with a warning, the sprite or model is the missing one, the atlas
//! or the whole load fails), within a bounded time and memory.
//!
//! The vanilla pack and local packs the player chose are read under [`Limits::trusted`],
//! which only keeps the arithmetic from overflowing; a server's pack under
//! [`Limits::untrusted`]. A limit that bounds something built from every pack (an atlas,
//! the baked models, a font) is the strictest of the packs' ([`Resources::limits`]).
//!
//! [`Resources::limits`]: crate::Resources::limits

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};

/// See the module docs. Every number says why it is what it is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    // The zip file.
    /// The largest pack file: vanilla's own limit on a server pack download
    /// (`DownloadedPackSource`'s `MAX_PACK_SIZE_BYTES`, 250 MiB).
    pub pack_bytes: u64,
    /// Entries in a zip's central directory. The biggest server packs (made by item and
    /// furniture plugins) have some tens of thousands of files; the game's own client jar
    /// has 34 000. Each costs the index about 80 bytes.
    pub zip_entries: usize,
    /// An entry name's length. Real paths are under 200 bytes.
    pub zip_name_bytes: usize,
    /// One file, inflated. The largest a pack can use: a music track of a few MiB, a
    /// 4096 x 4096 picture, Unifont's 10 MiB `.hex` file.
    pub entry_bytes: u64,
    /// Inflated bytes a pack may cost one load (reset by [`Pack::reset_budget`]). Pictures
    /// and sounds hardly compress, so a real 250 MiB pack inflates to far less; a hostile
    /// one could declare 4000 entries of 64 MiB (deflate goes to 1032:1). This bounds the
    /// time spent inflating to a second or two.
    ///
    /// [`Pack::reset_budget`]: crate::Pack::reset_budget
    pub load_bytes: u64,
    /// Overlay directories in `pack.mcmeta`. Every lookup asks each in turn; real packs
    /// have one per range of versions they support.
    pub pack_overlays: usize,
    /// `filter.block` patterns in `pack.mcmeta`: regular expressions tried on every
    /// lookup below the pack. The game's packs have none, packs that use them one or two.
    pub pack_filters: usize,
    /// Filter patterns of every pack of a stack together (up to 16 server packs): the
    /// packs above the limit are loaded without their filters.
    pub stack_filters: usize,

    // Files of the pack.
    /// One JSON file (models, block states, item definitions, atlases, fonts, language
    /// files, `sounds.json`, `.mcmeta`). The game's largest is `en_us.json` at 534 KiB;
    /// a parsed value takes up to 30 times its text, so this bounds one to ~128 MiB.
    /// Nesting is bounded by `serde_json`'s recursion limit of 128 levels.
    pub json_bytes: u64,
    /// One `.mcmeta` file. Sprites' metadata is read on every thread at once, and its
    /// largest part, a list of 8192 animation frames, takes some 200 KiB.
    pub mcmeta_bytes: u64,
    /// A picture's side: Vulkan's usual `maxImageDimension2D`. Callers that upload a
    /// picture as it is also pass the device's own.
    pub image_side: u32,
    /// A picture's texels: 4096 x 4096, 64 MiB decoded. Larger pictures are never in packs
    /// for a reason; a PNG header may claim any size, and decoding allocates for it.
    pub image_texels: u64,
    /// Texels a pack's pictures may decode to in one load: 1 GiB of RGBA. With the
    /// decoder's own buffer of each picture, a load's pictures peak at about twice that.
    pub load_texels: u64,

    // What is built from the files.
    /// An atlas's side. 8192 x 8192 is 256 MiB of RGBA, plus a third for mip levels; the
    /// game's block atlas is 2048 x 2048, a 128x pack's 8192 x 8192. An atlas that cannot
    /// fit is an error (vanilla's reload fails too), not a larger allocation.
    pub atlas_side: u32,
    /// Sources in one atlas definition file; the game's have a handful.
    pub atlas_sources: usize,
    /// Sprites in one atlas. `paletted_permutations` multiplies textures by palettes; the
    /// game's largest atlas (the blocks') has some 2000 sprites.
    pub atlas_sprites: usize,
    /// Frames of one animated sprite, listed or implied by its picture's size. The game's
    /// longest list has 38 (still lava); a frame list is walked for every quad that shows
    /// the sprite.
    pub animation_frames: usize,
    /// Parents of a model; the game's chains are at most five long.
    pub model_parents: usize,
    /// Elements of a model: 24 576 quads. Exported furniture models have up to a couple
    /// of thousand.
    pub model_elements: usize,
    /// Texture slots of one model file. Each is followed through its references; the
    /// game's models have at most 8.
    pub model_textures: usize,
    /// Quads one bake (every block model, or every item model) may make: about 190 MiB.
    /// The game's blocks bake to 455 000, its items to 103 000.
    pub baked_quads: usize,
    /// Distinct baked parts (a model under a rotation) of all block states: the game
    /// bakes 7881.
    pub baked_parts: usize,
    /// Variants over all block-state files of a bake (each a model to pick); the game's
    /// have 10 290.
    pub baked_variants: usize,
    /// Variants of a block-state file (keys of `variants`, entries of `multipart`, models of
    /// a weighted list). The game's largest (the repeater's) has 64.
    pub blockstate_cases: usize,
    /// Terms of the multipart conditions of one block-state file. Each is tested for every
    /// state of its block (up to 1296); the game's largest file (the chiseled bookshelf's) has 148.
    pub condition_terms: usize,
    /// Multipart parts over every state of every block, after the conditions: 16 MiB of
    /// indices.
    pub multipart_parts: usize,
    /// How deep an item definition's nodes nest.
    pub item_model_depth: usize,
    /// Nodes of all item definitions together. Packs that key models on
    /// `custom_model_data` have thousands of cases in one definition.
    pub item_model_nodes: usize,
    /// Providers of one font, references followed. Plugin-made packs put a bitmap provider
    /// per custom picture into `default.json`, some thousands of them.
    pub font_providers: usize,
    /// How deep font references nest; the game's go two deep.
    pub font_references: usize,
    /// Characters one font may give glyphs for, over all its providers. Unifont has some
    /// 57 000.
    pub font_glyphs: usize,
    /// Entries of a language file; `en_us.json` has 8559.
    pub lang_entries: usize,
    /// Events in one `sounds.json`; the game's has 1991.
    pub sound_events: usize,
    /// Sounds of one event; the game's have at most 32.
    pub sounds_per_event: usize,
    /// A sound's sample rate. OpenAL takes any; 192 kHz is the highest anyone records at.
    /// The stream buffers are two seconds of it.
    pub sample_rate: u32,
    /// Samples of a sound decoded whole (one not marked `stream`): 16 MiB, 87 s of 48 kHz
    /// stereo. The game streams its music; its whole sounds are seconds long.
    pub sound_samples: usize,
}

impl Limits {
    /// For the game's own pack and packs the player chose: nothing but what keeps the
    /// arithmetic in range.
    pub const fn trusted() -> Limits {
        Limits {
            pack_bytes: u64::MAX,
            zip_entries: usize::MAX,
            zip_name_bytes: usize::MAX,
            entry_bytes: u64::MAX,
            load_bytes: u64::MAX,
            pack_overlays: usize::MAX,
            pack_filters: usize::MAX,
            stack_filters: usize::MAX,
            json_bytes: u64::MAX,
            mcmeta_bytes: u64::MAX,
            // stb_image's own limit (`STBI_MAX_DIMENSIONS`).
            image_side: 1 << 24,
            image_texels: u64::MAX,
            load_texels: u64::MAX,
            atlas_side: u32::MAX,
            atlas_sources: usize::MAX,
            atlas_sprites: usize::MAX,
            animation_frames: usize::MAX,
            model_parents: usize::MAX,
            model_elements: usize::MAX,
            model_textures: usize::MAX,
            baked_quads: usize::MAX,
            baked_parts: usize::MAX,
            baked_variants: usize::MAX,
            blockstate_cases: usize::MAX,
            condition_terms: usize::MAX,
            multipart_parts: usize::MAX,
            item_model_depth: usize::MAX,
            item_model_nodes: usize::MAX,
            font_providers: usize::MAX,
            font_references: usize::MAX,
            font_glyphs: usize::MAX,
            lang_entries: usize::MAX,
            sound_events: usize::MAX,
            sounds_per_event: usize::MAX,
            sample_rate: u32::MAX,
            sound_samples: usize::MAX,
        }
    }

    /// For a pack a server sent. See each field for its reason.
    pub const fn untrusted() -> Limits {
        Limits {
            pack_bytes: 262_144_000,
            zip_entries: 1 << 17,
            zip_name_bytes: 1024,
            entry_bytes: 64 << 20,
            load_bytes: 1 << 30,
            pack_overlays: 64,
            pack_filters: 8,
            stack_filters: 16,
            json_bytes: 4 << 20,
            mcmeta_bytes: 256 << 10,
            image_side: 16384,
            image_texels: 1 << 24,
            load_texels: 1 << 28,
            atlas_side: 8192,
            atlas_sources: 4096,
            atlas_sprites: 1 << 16,
            animation_frames: 1 << 13,
            model_parents: 64,
            model_elements: 4096,
            model_textures: 256,
            baked_quads: 1 << 21,
            baked_parts: 1 << 16,
            baked_variants: 1 << 18,
            blockstate_cases: 4096,
            condition_terms: 1 << 14,
            multipart_parts: 1 << 22,
            item_model_depth: 64,
            item_model_nodes: 1 << 20,
            font_providers: 1 << 14,
            font_references: 16,
            font_glyphs: 1 << 18,
            lang_entries: 1 << 16,
            sound_events: 1 << 16,
            sounds_per_event: 4096,
            sample_rate: 192_000,
            sound_samples: 1 << 23,
        }
    }

    /// The stricter of two, field by field.
    pub fn min(&self, o: &Limits) -> Limits {
        Limits {
            pack_bytes: self.pack_bytes.min(o.pack_bytes),
            zip_entries: self.zip_entries.min(o.zip_entries),
            zip_name_bytes: self.zip_name_bytes.min(o.zip_name_bytes),
            entry_bytes: self.entry_bytes.min(o.entry_bytes),
            load_bytes: self.load_bytes.min(o.load_bytes),
            pack_overlays: self.pack_overlays.min(o.pack_overlays),
            pack_filters: self.pack_filters.min(o.pack_filters),
            stack_filters: self.stack_filters.min(o.stack_filters),
            json_bytes: self.json_bytes.min(o.json_bytes),
            mcmeta_bytes: self.mcmeta_bytes.min(o.mcmeta_bytes),
            image_side: self.image_side.min(o.image_side),
            image_texels: self.image_texels.min(o.image_texels),
            load_texels: self.load_texels.min(o.load_texels),
            atlas_side: self.atlas_side.min(o.atlas_side),
            atlas_sources: self.atlas_sources.min(o.atlas_sources),
            atlas_sprites: self.atlas_sprites.min(o.atlas_sprites),
            animation_frames: self.animation_frames.min(o.animation_frames),
            model_parents: self.model_parents.min(o.model_parents),
            model_elements: self.model_elements.min(o.model_elements),
            model_textures: self.model_textures.min(o.model_textures),
            baked_quads: self.baked_quads.min(o.baked_quads),
            baked_parts: self.baked_parts.min(o.baked_parts),
            baked_variants: self.baked_variants.min(o.baked_variants),
            blockstate_cases: self.blockstate_cases.min(o.blockstate_cases),
            condition_terms: self.condition_terms.min(o.condition_terms),
            multipart_parts: self.multipart_parts.min(o.multipart_parts),
            item_model_depth: self.item_model_depth.min(o.item_model_depth),
            item_model_nodes: self.item_model_nodes.min(o.item_model_nodes),
            font_providers: self.font_providers.min(o.font_providers),
            font_references: self.font_references.min(o.font_references),
            font_glyphs: self.font_glyphs.min(o.font_glyphs),
            lang_entries: self.lang_entries.min(o.lang_entries),
            sound_events: self.sound_events.min(o.sound_events),
            sounds_per_event: self.sounds_per_event.min(o.sounds_per_event),
            sample_rate: self.sample_rate.min(o.sample_rate),
            sound_samples: self.sound_samples.min(o.sound_samples),
        }
    }
}

impl Default for Limits {
    fn default() -> Limits {
        Limits::trusted()
    }
}

/// What a pack has cost the current load: inflated bytes and decoded texels. Shared by
/// every reader of the pack, across threads.
#[derive(Debug, Default)]
pub struct Budget {
    bytes: AtomicU64,
    texels: AtomicU64,
    /// Set by whoever runs the load: once it is true, the load is given up.
    cancel: OnceLock<Arc<AtomicBool>>,
}

impl Budget {
    /// Makes `flag` the load's cancel flag: once it is set, every lookup in a stack with
    /// this pack finds nothing, every read fails, and the builders (atlases, models,
    /// items, fonts) return early with what they have. The first flag given stays: a pack
    /// is opened for one load.
    pub fn cancel_with(&self, flag: Arc<AtomicBool>) {
        let _ = self.cancel.set(flag);
    }

    /// Whether the load is given up.
    pub fn cancelled(&self) -> bool {
        self.cancel.get().is_some_and(|f| f.load(Ordering::Relaxed))
    }

    /// Takes `n` from what `limit` leaves, or fails and takes nothing.
    fn take(counter: &AtomicU64, n: u64, limit: u64) -> bool {
        counter
            .try_update(Ordering::Relaxed, Ordering::Relaxed, |used| {
                used.checked_add(n).filter(|&total| total <= limit)
            })
            .is_ok()
    }

    /// Takes `n` inflated bytes, or fails.
    pub fn take_bytes(&self, n: u64, limits: &Limits) -> bool {
        Budget::take(&self.bytes, n, limits.load_bytes)
    }

    /// Takes `n` decoded texels, or fails.
    pub fn take_texels(&self, n: u64, limits: &Limits) -> bool {
        Budget::take(&self.texels, n, limits.load_texels)
    }

    /// A new load begins.
    pub fn reset(&self) {
        self.bytes.store(0, Ordering::Relaxed);
        self.texels.store(0, Ordering::Relaxed);
    }

    /// Inflated bytes and decoded texels taken so far.
    pub fn used(&self) -> (u64, u64) {
        (
            self.bytes.load(Ordering::Relaxed),
            self.texels.load(Ordering::Relaxed),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_budget_is_never_overdrawn() {
        let limits = Limits {
            load_bytes: 10,
            load_texels: 5,
            ..Limits::untrusted()
        };
        let b = Budget::default();
        assert!(b.take_bytes(6, &limits));
        assert!(!b.take_bytes(5, &limits), "6 + 5 is over 10");
        assert!(b.take_bytes(4, &limits));
        assert!(!b.take_bytes(1, &limits));
        assert!(!b.take_bytes(u64::MAX, &limits), "no overflow");
        assert!(b.take_texels(5, &limits) && !b.take_texels(1, &limits));
        assert_eq!(b.used(), (10, 5));
        b.reset();
        assert!(b.take_bytes(10, &limits));
    }

    #[test]
    fn the_stricter_wins() {
        let m = Limits::trusted().min(&Limits::untrusted());
        assert_eq!(m, Limits::untrusted());
        assert_eq!(
            Limits::untrusted().min(&Limits::trusted()),
            Limits::untrusted()
        );
    }
}

//! Plays an atlas's animated sprites: which frame each shows after a number of ticks, and
//! the texels to write into the atlas for it.
//!
//! As 26.3's `SpriteContents` and `TextureAtlas` play them: a frame lasts its own number
//! of ticks; a sprite is written again when the frame it shows becomes another one, and
//! an interpolated sprite every tick, as a mix of its frame and the next. Every mip level
//! is written from the frame's own, with its edge texels repeated into the padding.

use crate::atlas::{Animation, Atlas};

/// A rectangle of one mip level of the atlas, in that level's texels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    pub level: u32,
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

/// Where an animation stands.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct Playhead {
    /// The place in the list of frames, and the ticks it has been shown for.
    frame: usize,
    ticks: u32,
}

impl Playhead {
    /// A tick goes by. Whether another frame is shown now.
    fn tick(&mut self, frames: &[(u32, u32)]) -> bool {
        self.ticks += 1;
        let (shown, time) = frames[self.frame];
        if self.ticks < time {
            return false;
        }
        self.frame = (self.frame + 1) % frames.len();
        self.ticks = 0;
        frames[self.frame].0 != shown
    }

    /// The frame shown, the one after it, and how far it is on its way there, in whole
    /// thousandths (as the game hands it to its shader).
    fn blend(&self, frames: &[(u32, u32)]) -> (u32, u32, f32) {
        let (shown, time) = frames[self.frame];
        let next = frames[(self.frame + 1) % frames.len()].0;
        let progress = self.ticks as f32 / time as f32;
        (shown, next, (progress * 1000.0) as i32 as f32 / 1000.0)
    }
}

/// An animated sprite in its atlas.
#[derive(Debug)]
struct Sprite {
    animation: Animation,
    /// The sprite's whole image, every frame of it, per mip level: its width in texels
    /// and its RGBA8 texels.
    levels: Vec<(u32, Vec<u8>)>,
    /// The padded slot's corner, the padding and a frame's size, at level 0.
    at: (u32, u32),
    padding: u32,
    size: (u32, u32),
    head: Playhead,
    /// The atlas does not show what it should, and the sprite is to be written.
    stale: bool,
    /// It should be written, and waits for room in a tick's budget ([`MAX_STALE_TEXELS`]).
    owed: bool,
    /// Disabled after a permanent GPU upload error (e.g., too large for the upload ring).
    disabled: bool,
}

/// The most texels the sprites to be written may take at once: 8 MiB of RGBA. The game's
/// 56 animated sprites take 187 000 if all are written at once, padding and levels too; a pack's 65 536 interpolated sprites would
/// be every texel of the atlas, every tick, composed on the frame's thread. Past it
/// sprites wait their turn (round the list, so that each gets one): they move on less
/// smoothly, and the work stays bounded. No sprite of a server's pack takes more than this
/// in one write (one that would is drawn still: `atlas::build`), so a tick's writes from
/// server packs never pass it; a larger one of the game's or the player's own packs is
/// written whole, alone in its tick, as vanilla writes it.
pub const MAX_STALE_TEXELS: usize = 1 << 21;

/// Texels one write of an animated sprite takes: its slot at every one of `levels` mip
/// levels, padding and all (what [`AtlasAnimations::compose`] makes of it).
pub fn write_texels(width: u32, height: u32, padding: u32, levels: u32) -> u64 {
    (0..levels.min(32))
        .map(|l| {
            let pad = 2 * u64::from(padding >> l);
            (u64::from(width >> l) + pad) * (u64::from(height >> l) + pad)
        })
        .sum()
}

/// What staging a piece of sprites came to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Upload {
    /// Staged: the atlas shows them from the next frame.
    Done,
    /// No room now: try again in a later frame.
    NoRoom,
    /// Never possible (larger than the upload ring, say): not to be tried again.
    Never,
}

/// What [`AtlasAnimations::write_stale`] did.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Writes {
    /// Sprites written, and their bytes.
    pub sprites: u64,
    pub bytes: u64,
    /// Times it stopped for lack of room.
    pub waits: u64,
    /// Sprites given up for good.
    pub disabled: u64,
    /// Whether there was anything to write.
    pub worked: bool,
}

/// The scratch lists of [`AtlasAnimations::write_stale`]: a piece's sprites, their
/// rectangles and their texels.
#[derive(Debug, Default)]
pub struct Piece {
    pub sprites: Vec<usize>,
    pub rects: Vec<Rect>,
    pub texels: Vec<u8>,
}

impl Sprite {
    /// A tick goes by: the sprite is owed a write if it shows another frame, or blends.
    fn tick(&mut self) {
        let changed = self.head.tick(&self.animation.frames);
        self.owed |= changed || self.animation.interpolate;
    }

    /// Texels a write of it takes: every level's slot, padding and all.
    fn texels(&self) -> usize {
        (0..self.levels.len() as u32)
            .map(|l| {
                let pad = 2 * (self.padding >> l) as usize;
                ((self.size.0 >> l) as usize + pad) * ((self.size.1 >> l) as usize + pad)
            })
            .sum()
    }

    /// Appends the slot's texels at one level.
    fn compose_level(&self, level: usize, out: &mut Vec<u8>) -> Rect {
        let l = level as u32;
        let (image_width, texels) = &self.levels[level];
        let (w, h, pad) = (self.size.0 >> l, self.size.1 >> l, self.padding >> l);
        let (shown, next, t) = self.head.blend(&self.animation.frames);
        // A row of a frame's texels.
        let row = |frame: u32, y: u32| {
            let (fx, fy) = self.animation.frame_pos(frame);
            let start = (((fy * h + y) * image_width + fx * w) * 4) as usize;
            &texels[start..start + (w * 4) as usize]
        };
        let mixed = self.animation.interpolate && t > 0.0;
        let pad_bytes = (pad * 4) as usize;
        let stride = (w * 4) as usize + 2 * pad_bytes;
        // Each row once, with its ends repeated sideways.
        let first_row = out.len() + stride * pad as usize;
        out.resize(out.len() + stride * (h + 2 * pad) as usize, 0);
        for y in 0..h {
            let start = first_row + stride * y as usize;
            let line = &mut out[start..start + stride];
            let (left, rest) = line.split_at_mut(pad_bytes);
            let (middle, right) = rest.split_at_mut((w * 4) as usize);
            middle.copy_from_slice(row(shown, y));
            if mixed {
                for (c, n) in middle.iter_mut().zip(row(next, y)) {
                    let (a, b) = (f32::from(*c), f32::from(*n));
                    *c = (a * (1.0 - t) + b * t + 0.5) as u8;
                }
            }
            for texel in left.as_chunks_mut::<4>().0 {
                texel.copy_from_slice(&middle[..4]);
            }
            for texel in right.as_chunks_mut::<4>().0 {
                texel.copy_from_slice(&middle[middle.len() - 4..]);
            }
        }
        // Then the first and the last of them over the padding above and below.
        let last_row = first_row + stride * (h - 1) as usize;
        for y in 0..pad as usize {
            let above = first_row - stride * (y + 1);
            out.copy_within(first_row..first_row + stride, above);
            let below = last_row + stride * (y + 1);
            out.copy_within(last_row..last_row + stride, below);
        }
        Rect {
            level: l,
            x: self.at.0 >> l,
            y: self.at.1 >> l,
            width: w + 2 * pad,
            height: h + 2 * pad,
        }
    }
}

/// An atlas's animated sprites as they play.
#[derive(Debug, Default)]
pub struct AtlasAnimations {
    sprites: Vec<Sprite>,
    /// Where the next tick starts handing out writes.
    cursor: usize,
    /// What the sprites to be written may take ([`MAX_STALE_TEXELS`]).
    budget: usize,
}

impl AtlasAnimations {
    /// Takes the animated sprites' frames out of an atlas, whose texels show the first
    /// frame of each.
    pub fn take(atlas: &mut Atlas) -> AtlasAnimations {
        let mut places = vec![None; atlas.animated.len()];
        for s in atlas.sprites.values() {
            if let Some(i) = s.animated {
                places[i] = Some((s.x, s.y, s.padding, s.width, s.height));
            }
        }
        let sprites = atlas
            .animated
            .drain(..)
            .zip(places)
            .filter_map(|(a, place)| {
                let (x, y, padding, width, height) = place?;
                Some(Sprite {
                    animation: a.animation,
                    levels: a.levels.iter().map(|i| (i.width, i.to_rgba8())).collect(),
                    at: (x, y),
                    padding,
                    size: (width, height),
                    head: Playhead::default(),
                    stale: false,
                    owed: false,
                    disabled: false,
                })
            })
            .collect();
        AtlasAnimations {
            sprites,
            cursor: 0,
            budget: MAX_STALE_TEXELS,
        }
    }

    pub fn len(&self) -> usize {
        self.sprites.len()
    }

    pub fn is_empty(&self) -> bool {
        self.sprites.is_empty()
    }

    /// A tick goes by. The sprites owed a write are to be written, as many as the budget
    /// holds with those still waiting to be, from where the last tick stopped.
    pub fn tick(&mut self) {
        for s in &mut self.sprites {
            if !s.disabled {
                s.tick();
            }
        }
        let mut taken: usize = self.stale_texels();
        let n = self.sprites.len();
        for k in 0..n {
            let i = (self.cursor + k) % n;
            let s = &mut self.sprites[i];
            if !s.owed {
                continue;
            }
            if s.stale {
                s.owed = false;
                continue;
            }
            let texels = s.texels();
            if taken > 0 && taken.saturating_add(texels) > self.budget {
                self.cursor = i;
                return;
            }
            s.stale = true;
            s.owed = false;
            taken = taken.saturating_add(texels);
        }
    }

    /// Texels a write of the sprite takes, all its levels.
    pub fn texels(&self, sprite: usize) -> usize {
        self.sprites.get(sprite).map_or(0, Sprite::texels)
    }

    /// Texels of every sprite to be written.
    pub fn stale_texels(&self) -> usize {
        self.sprites
            .iter()
            .filter(|s| s.stale)
            .map(Sprite::texels)
            .sum()
    }

    /// How many sprites the atlas does not show as it should.
    pub fn stale_count(&self) -> usize {
        self.sprites.iter().filter(|s| s.stale).count()
    }

    /// The first sprite from the `from`th on that the atlas does not show as it should.
    pub fn next_stale(&self, from: usize) -> Option<usize> {
        (from..self.sprites.len()).find(|&i| self.sprites[i].stale && !self.sprites[i].disabled)
    }

    /// Permanently disables a sprite whose upload failed for a non-transient reason.
    pub fn disable(&mut self, sprite: usize) {
        if let Some(s) = self.sprites.get_mut(sprite) {
            s.disabled = true;
            s.stale = false;
            s.owed = false;
        }
    }

    /// Appends what the atlas should show of a sprite: the texels of its rectangles, one
    /// after another, to `texels`, and the rectangles to `rects`.
    pub fn compose(&self, sprite: usize, rects: &mut Vec<Rect>, texels: &mut Vec<u8>) {
        let s = &self.sprites[sprite];
        for level in 0..s.levels.len() {
            rects.push(s.compose_level(level, texels));
        }
    }

    /// The atlas shows a sprite as it should now.
    pub fn written(&mut self, sprite: usize) {
        self.sprites[sprite].stale = false;
    }

    /// Writes up to `count` of the sprites the atlas doesn't show as it should, in pieces
    /// of a few at a time: a piece takes sprites until it holds `piece_bytes` or
    /// `piece_rects` rectangles, and is handed to `upload` (its rectangles, and their
    /// texels one after another). A piece with no room waits for a later frame (and the
    /// rest with it). A piece that can never be staged is tried again a sprite at a time,
    /// so that only a sprite that can't go even alone is given up (disabled, drawn as it
    /// last was), and the others in its piece play on.
    pub fn write_stale(
        &mut self,
        count: usize,
        piece_bytes: usize,
        piece_rects: usize,
        piece: &mut Piece,
        mut upload: impl FnMut(&[Rect], &[u8]) -> Upload,
    ) -> Writes {
        let mut out = Writes::default();
        let mut left = count;
        let mut from = 0;
        // After a piece that could never go: one sprite a piece, for the rest of the call.
        let mut alone = false;
        while left > 0
            && let Some(first) = self.next_stale(from)
        {
            out.worked = true;
            piece.sprites.clear();
            piece.rects.clear();
            piece.texels.clear();
            let mut next = Some(first);
            while let Some(sprite) = next {
                piece.sprites.push(sprite);
                self.compose(sprite, &mut piece.rects, &mut piece.texels);
                from = sprite + 1;
                left -= 1;
                next = (!alone
                    && left > 0
                    && piece.texels.len() < piece_bytes
                    && piece.rects.len() < piece_rects)
                    .then(|| self.next_stale(from))
                    .flatten();
            }
            match upload(&piece.rects, &piece.texels) {
                Upload::Done => {
                    for &sprite in &piece.sprites {
                        self.written(sprite);
                    }
                    out.sprites += piece.sprites.len() as u64;
                    out.bytes += piece.texels.len() as u64;
                }
                Upload::NoRoom => {
                    out.waits += 1;
                    break;
                }
                Upload::Never if piece.sprites.len() > 1 => {
                    // One of them is too large for a piece: each goes alone.
                    alone = true;
                    from = first;
                    left += piece.sprites.len();
                }
                Upload::Never => {
                    self.disable(first);
                    out.disabled += 1;
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Four 2 x 2 frames in two rows, every texel another grey, and their 1 x 1 means'
    /// stand-ins as a second level.
    fn sprite(frames: &[(u32, u32)], interpolate: bool) -> Sprite {
        let grey = |values: Vec<u8>| values.into_iter().flat_map(|v| [v, v, v, 255]).collect();
        Sprite {
            animation: Animation {
                frames: frames.to_vec(),
                per_row: 2,
                interpolate,
            },
            levels: vec![
                (4, grey((0..16).map(|i| i * 10).collect())),
                (2, grey(vec![200, 201, 202, 203])),
            ],
            at: (8, 4),
            padding: 2,
            size: (2, 2),
            head: Playhead::default(),
            stale: false,
            owed: false,
            disabled: false,
        }
    }

    fn greys(bytes: &[u8]) -> Vec<u8> {
        bytes.as_chunks::<4>().0.iter().map(|t| t[0]).collect()
    }

    #[test]
    fn a_frame_is_shown_for_its_time() {
        let mut s = sprite(&[(0, 2), (3, 1), (3, 2), (1, 1)], false);
        let mut seen = Vec::new();
        for _ in 0..7 {
            s.owed = false;
            s.tick();
            seen.push((s.head.frame, s.head.ticks, s.owed));
        }
        // The second and the third show the same frame: nothing to write between them.
        assert_eq!(
            seen,
            [
                (0, 1, false),
                (1, 0, true),
                (2, 0, false),
                (2, 1, false),
                (3, 0, true),
                (0, 0, true),
                (0, 1, false),
            ]
        );
        // Not written yet, it stays owed.
        s.owed = true;
        s.tick();
        assert!(s.owed);
        s.tick();
        assert!(s.owed);
    }

    #[test]
    fn a_level_is_written_with_its_edges_repeated() {
        let mut s = sprite(&[(0, 1), (3, 1)], false);
        s.tick();
        let mut out = Vec::new();
        let rect = s.compose_level(0, &mut out);
        assert_eq!(
            rect,
            Rect {
                level: 0,
                x: 8,
                y: 4,
                width: 6,
                height: 6
            }
        );
        // Frame 3 is the bottom right one: 100, 110 over 140, 150.
        let top = [100, 100, 100, 110, 110, 110];
        let bottom = [140, 140, 140, 150, 150, 150];
        assert_eq!(
            greys(&out),
            [top, top, top, bottom, bottom, bottom].concat()
        );
        out.clear();
        let rect = s.compose_level(1, &mut out);
        assert_eq!((rect.level, rect.x, rect.y), (1, 4, 2));
        assert_eq!((rect.width, rect.height), (3, 3));
        assert_eq!(greys(&out), [203; 9]);
    }

    #[test]
    fn interpolation_mixes_the_next_frame_in_by_thousandths() {
        let mut s = sprite(&[(0, 3), (1, 3)], true);
        let first = |s: &Sprite| {
            let mut out = Vec::new();
            s.compose_level(0, &mut out);
            greys(&out)[2 * 6 + 2..2 * 6 + 4].to_vec()
        };
        assert_eq!(first(&s), [0, 10]);
        // Every tick is written; a third of the way is 0.333.
        s.tick();
        assert!(s.owed);
        assert_eq!(s.head.blend(&s.animation.frames), (0, 1, 0.333));
        assert_eq!(first(&s), [7, 17]);
        s.tick();
        assert_eq!(s.head.blend(&s.animation.frames), (0, 1, 0.666));
        assert_eq!(first(&s), [13, 23]);
        // The last frame goes over into the first.
        s.tick();
        s.tick();
        assert_eq!(s.head.blend(&s.animation.frames), (1, 0, 0.333));
        assert_eq!(first(&s), [13, 23]);
    }

    #[test]
    fn only_what_changed_is_handed_out() {
        let mut a = AtlasAnimations {
            sprites: vec![
                sprite(&[(0, 5), (1, 5)], false),
                sprite(&[(0, 1), (1, 1)], false),
                sprite(&[(0, 5), (1, 5)], true),
            ],
            cursor: 0,
            budget: MAX_STALE_TEXELS,
        };
        assert_eq!(a.next_stale(0), None);
        a.tick();
        assert_eq!(a.next_stale(0), Some(1));
        assert_eq!(a.next_stale(2), Some(2));
        let (mut rects, mut texels) = (Vec::new(), Vec::new());
        a.compose(1, &mut rects, &mut texels);
        assert_eq!(rects.len(), 2);
        assert_eq!(texels.len(), (36 + 9) * 4);
        a.written(1);
        a.written(2);
        assert_eq!(a.next_stale(0), None);
    }

    /// A pack's thousand interpolated sprites, each written every tick: the sprites to be
    /// written never take more than the budget, and every one gets its turn.
    #[test]
    fn a_ticks_writes_are_bounded_and_each_sprite_gets_its_turn() {
        let one = sprite(&[(0, 1), (1, 1)], true).texels();
        assert_eq!(one, 36 + 9);
        let mut a = AtlasAnimations {
            sprites: (0..1000).map(|_| sprite(&[(0, 1), (1, 1)], true)).collect(),
            cursor: 0,
            budget: 100 * one,
        };
        let mut written = vec![0u32; 1000];
        for _ in 0..30 {
            a.tick();
            assert!(a.stale_texels() <= 100 * one, "{}", a.stale_texels());
            // The frame writes them all.
            let mut at = 0;
            while let Some(s) = a.next_stale(at) {
                at = s + 1;
                a.written(s);
                written[s] += 1;
            }
        }
        // A hundred a tick: each of the thousand three times in thirty ticks.
        assert!(written.iter().all(|&w| w == 3), "{written:?}");
        // While none can be written (no room to upload), no more are handed out.
        for _ in 0..5 {
            a.tick();
            assert!(a.stale_texels() <= 100 * one);
        }
        // A sprite larger than the whole budget is still written, alone.
        let mut a = AtlasAnimations {
            sprites: (0..3).map(|_| sprite(&[(0, 1), (1, 1)], true)).collect(),
            cursor: 0,
            budget: 10,
        };
        a.tick();
        assert_eq!(a.stale_count(), 1);
        assert_eq!(a.stale_texels(), one);
    }

    /// A piece that can never be staged (one sprite in it too large for the upload ring)
    /// gives up only that sprite: the others in its piece are written, alone, and play on.
    #[test]
    fn a_sprite_that_can_never_be_written_gives_up_only_itself() {
        let mut big = sprite(&[(0, 1), (1, 1)], false);
        // Ten times the texels of the others.
        big.levels = vec![(40, vec![0; 40 * 40 * 4])];
        big.size = (20, 20);
        big.padding = 0;
        let mut a = AtlasAnimations {
            sprites: vec![
                sprite(&[(0, 1), (1, 1)], false),
                big,
                sprite(&[(0, 1), (1, 1)], false),
            ],
            cursor: 0,
            budget: MAX_STALE_TEXELS,
        };
        let too_large = 400 * 4;
        let mut piece = Piece::default();
        let mut uploads = Vec::new();
        let mut upload = |rects: &[Rect], texels: &[u8]| {
            uploads.push(rects.len());
            if texels.len() >= too_large {
                Upload::Never
            } else {
                Upload::Done
            }
        };
        for _ in 0..3 {
            a.tick();
            let w = a.write_stale(usize::MAX, 1 << 20, 512, &mut piece, &mut upload);
            assert_eq!(a.stale_count(), 0);
            assert!(w.sprites >= 2, "{w:?}");
        }
        assert!(a.sprites[1].disabled);
        assert!(!a.sprites[0].disabled && !a.sprites[2].disabled);
        // The first tick: the piece of all three, then each alone; later ticks, the two
        // small ones together.
        assert_eq!(uploads, [5, 2, 1, 2, 4, 4]);
        // No room: it stops, and nothing is given up.
        let mut a = AtlasAnimations {
            sprites: vec![sprite(&[(0, 1), (1, 1)], false)],
            cursor: 0,
            budget: MAX_STALE_TEXELS,
        };
        a.tick();
        let w = a.write_stale(1, 1 << 20, 512, &mut piece, |_, _| Upload::NoRoom);
        assert_eq!((w.waits, w.disabled, a.stale_count()), (1, 0, 1));
    }

    /// A write's texels, as `Sprite::texels` counts them.
    #[test]
    fn a_writes_texels_are_its_slots_at_every_level() {
        let s = sprite(&[(0, 1), (1, 1)], false);
        assert_eq!(write_texels(2, 2, 2, 2) as usize, s.texels());
        assert_eq!(write_texels(16, 16, 1, 1), 18 * 18);
        assert_eq!(
            write_texels(1 << 20, 1 << 20, 0, 40),
            (0..32).map(|l| (1u64 << 20 >> l).pow(2)).sum::<u64>()
        );
    }

    /// Where the game's own animation states stand after each of 64 ticks, as hashes of
    /// 250 made-up animations each.
    const GOLDEN: [u64; 8] = [
        0xd14d459f136c4707,
        0x68ade7898bf0a175,
        0x7d95e88c67b38dc1,
        0x89a59feeea26b445,
        0x88a3c52567135257,
        0x10ebd3cdbabf4634,
        0xfeb0f424e218b6c5,
        0xd0c64226787805b6,
    ];

    #[test]
    fn animations_play_as_the_games() {
        let mut state = 0x5EED_u64;
        let mut next = |n: u64| {
            state = state
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            ((state >> 33) % n) as u32
        };
        for golden in GOLDEN {
            let mut hash = 0xcbf2_9ce4_8422_2325_u64;
            for _ in 0..250 {
                let frames = 2 + next(6);
                let interpolate = next(3) == 0;
                let frames: Vec<(u32, u32)> = (0..frames).map(|_| (next(4), 1 + next(7))).collect();
                let mut s = sprite(&frames, interpolate);
                for _ in 0..64 {
                    s.owed = false;
                    s.tick();
                    for word in [s.head.frame as u32, s.head.ticks, u32::from(s.owed)] {
                        for byte in word.to_le_bytes() {
                            hash = (hash ^ u64::from(byte)).wrapping_mul(0x100_0000_01b3);
                        }
                    }
                }
            }
            assert_eq!(hash, golden);
        }
    }
}

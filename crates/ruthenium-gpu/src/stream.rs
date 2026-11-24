//! The first upload of an image in tiles, for images larger than a burst of the staging
//! ring can hold (the block atlas with its mip levels is larger than the whole ring).
//! Pure bookkeeping: [`crate::UploadRing`] owns the queues and records the copies.
//!
//! A tile is a band of rows across a whole mip level. Its top and height honour the
//! format's block size and the queue family's `minImageTransferGranularity` (a band is as
//! wide as its level, which every granularity allows); its bytes are tightly packed rows,
//! staged at an offset with the ring's copy alignment.
//!
//! [`ImageStream`] is the state of one image generation while it streams:
//!
//! | Event | What changes |
//! |---|---|
//! | Tiles staged | They wait in the open batch. The image stays private. |
//! | The batch is submitted | The first submission takes the image from UNDEFINED to TRANSFER_DST_OPTIMAL; later ones keep layout and ownership. The one that carries the last tiles also carries the transition to SHADER_READ_ONLY_OPTIMAL (and the release to the graphics queue family), and its batch is what the image's ticket waits for. |
//! | The batch is deferred | Nothing: the tiles stay staged for the retry, and submitted coverage, layout and last submission keep their values. |
//! | Cancelled | Staged tiles are dropped and no more are scheduled. The image is never shown; it may be destroyed once the last submission that wrote it has finished. |

use ash::vk;

use crate::memory::texel_bytes;

/// How an image is cut into tiles.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct TilePlan {
    extent: vk::Extent2D,
    mip_levels: u32,
    /// Bytes of one block of texels, and its size in texels (1 × 1 for the uncompressed
    /// formats there is an upload path for).
    block_bytes: u32,
    block_width: u32,
    block_height: u32,
    /// A tile's top is a multiple of this many rows, and so is its height unless it
    /// reaches the bottom of its level. 0: only whole levels can be copied.
    row_step: u32,
}

/// Where the next tile of an image starts: a row of a mip level. Tiles go level by level
/// from level 0, top to bottom.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Cursor {
    pub mip: u32,
    pub row: u32,
}

/// A band of rows across one mip level.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Tile {
    pub mip: u32,
    pub y: u32,
    pub rows: u32,
    pub width: u32,
    pub bytes: u64,
}

fn lcm(a: u32, b: u32) -> u32 {
    let (mut x, mut y) = (a, b);
    while y != 0 {
        (x, y) = (y, x % y);
    }
    a / x * b
}

impl TilePlan {
    /// The plan for an image of `format` copied by a queue family with this
    /// `minImageTransferGranularity`. `None`: there is no upload path for the format.
    pub(crate) fn new(
        format: vk::Format,
        extent: vk::Extent2D,
        mip_levels: u32,
        granularity: vk::Extent3D,
    ) -> Option<TilePlan> {
        let block_bytes = texel_bytes(format)?;
        let (block_width, block_height) = (1, 1);
        // (0, 0, 0) allows whole levels only. Otherwise offsets and extents are multiples
        // of the granularity, or reach the edge of the level.
        let row_step = match granularity.height {
            0 => 0,
            g => lcm(g, block_height),
        };
        Some(TilePlan {
            extent,
            mip_levels,
            block_bytes,
            block_width,
            block_height,
            row_step,
        })
    }

    fn level(&self, mip: u32) -> (u32, u32) {
        (
            (self.extent.width >> mip).max(1),
            (self.extent.height >> mip).max(1),
        )
    }

    /// Bytes of `rows` rows of level `mip` (a multiple of the block height, or the rows
    /// down to the bottom of the level).
    fn bytes_of(&self, mip: u32, rows: u32) -> u64 {
        let (width, _) = self.level(mip);
        u64::from(width.div_ceil(self.block_width))
            * u64::from(rows.div_ceil(self.block_height))
            * u64::from(self.block_bytes)
    }

    /// Bytes of every level together, tightly packed.
    pub(crate) fn total_bytes(&self) -> u64 {
        (0..self.mip_levels)
            .map(|m| self.bytes_of(m, self.level(m).1))
            .sum()
    }

    /// Bytes of level `mip`, and of the levels before it together.
    pub(crate) fn level_bytes(&self, mip: u32) -> (u64, u64) {
        let before = (0..mip).map(|m| self.bytes_of(m, self.level(m).1)).sum();
        (self.bytes_of(mip, self.level(mip).1), before)
    }

    /// Bytes of the largest tile that cannot be made smaller: a tile must be able to hold
    /// this much for the image to stream at all.
    pub(crate) fn largest_step(&self) -> u64 {
        (0..self.mip_levels)
            .map(|m| {
                let (_, height) = self.level(m);
                let rows = match self.row_step {
                    0 => height,
                    step => step.min(height),
                };
                self.bytes_of(m, rows)
            })
            .max()
            .unwrap_or(0)
    }

    pub(crate) fn done(&self, at: Cursor) -> bool {
        at.mip >= self.mip_levels
    }

    /// Bytes of the tiles before `at`.
    #[cfg(test)]
    pub(crate) fn bytes_before(&self, at: Cursor) -> u64 {
        if self.done(at) {
            return self.total_bytes();
        }
        self.level_bytes(at.mip).1 + self.bytes_of(at.mip, at.row)
    }

    /// The largest tile starting at `at` of at most `max_bytes`. `None`: the image is
    /// done, or not even the smallest tile there fits.
    pub(crate) fn next(&self, at: Cursor, max_bytes: u64) -> Option<Tile> {
        if self.done(at) {
            return None;
        }
        let (width, height) = self.level(at.mip);
        let left = height - at.row;
        let row_bytes = self.bytes_of(at.mip, self.block_height);
        let block_rows = (max_bytes / row_bytes).min(u64::from(u32::MAX / self.block_height));
        let affordable = block_rows as u32 * self.block_height;
        let rows = if affordable >= left {
            left
        } else {
            match self.row_step {
                0 => 0,
                step => affordable / step * step,
            }
        };
        (rows > 0).then(|| Tile {
            mip: at.mip,
            y: at.row,
            rows,
            width,
            bytes: self.bytes_of(at.mip, rows),
        })
    }

    /// The smallest tile that can start at `at`: one step of rows, or what is left of the
    /// level. `None`: the image is done.
    pub(crate) fn smallest(&self, at: Cursor) -> Option<Tile> {
        if self.done(at) {
            return None;
        }
        let (width, height) = self.level(at.mip);
        let left = height - at.row;
        let rows = match self.row_step {
            0 => left,
            step => step.min(left),
        };
        Some(Tile {
            mip: at.mip,
            y: at.row,
            rows,
            width,
            bytes: self.bytes_of(at.mip, rows),
        })
    }

    /// Where `tile`'s bytes start in the tightly packed rows of its level.
    pub(crate) fn offset_in_level(&self, tile: &Tile) -> u64 {
        self.bytes_of(tile.mip, tile.y)
    }

    /// Where the tile after `tile` starts.
    pub(crate) fn after(&self, tile: &Tile) -> Cursor {
        let (_, height) = self.level(tile.mip);
        if tile.y + tile.rows >= height {
            Cursor {
                mip: tile.mip + 1,
                row: 0,
            }
        } else {
            Cursor {
                mip: tile.mip,
                row: tile.y + tile.rows,
            }
        }
    }
}

/// Submissions of one image that can be unfinished at once: one per command buffer of the
/// copy queue.
pub(crate) const MAX_FLYING: usize = 8;

/// What a submission has to do for an image besides its copies.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Transitions {
    /// Take the image from UNDEFINED to TRANSFER_DST_OPTIMAL before the copies.
    pub first: bool,
    /// Leave it in SHADER_READ_ONLY_OPTIMAL (released to the graphics queue family, if
    /// that is another one) after them.
    pub last: bool,
}

/// What cancelling a stream leaves behind.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Cancelled {
    /// Staged tiles that were dropped from the open batch, and their bytes.
    pub dropped_tiles: u32,
    pub dropped_bytes: u64,
    /// The last batch whose submission wrote the image (0: none did): the image may be
    /// destroyed once it has finished.
    pub last_batch: u64,
}

/// The upload state of one image generation. See the module docs.
#[derive(Clone, Copy, Debug)]
pub(crate) struct ImageStream {
    plan: TilePlan,
    total: u64,
    /// Where the next tile to stage starts.
    cursor: Cursor,
    staged_bytes: u64,
    /// Tiles in the open batch: staged, in no submission yet.
    pending_tiles: u32,
    pending_bytes: u64,
    submitted_bytes: u64,
    /// The first submission went: the image is in TRANSFER_DST_OPTIMAL and the copy
    /// queue's family owns it.
    committed: bool,
    /// The last batch whose submission used the image (0: none).
    last_batch: u64,
    /// The batch whose submission carried the last tiles and the final transition (0: not
    /// submitted yet).
    final_batch: u64,
    /// Unfinished submissions: the batch and the coverage submitted with it.
    flying: [(u64, u64); MAX_FLYING],
    flying_count: usize,
    completed_bytes: u64,
}

impl ImageStream {
    pub(crate) fn new(plan: TilePlan) -> ImageStream {
        ImageStream {
            plan,
            total: plan.total_bytes(),
            cursor: Cursor::default(),
            staged_bytes: 0,
            pending_tiles: 0,
            pending_bytes: 0,
            submitted_bytes: 0,
            committed: false,
            last_batch: 0,
            final_batch: 0,
            flying: [(0, 0); MAX_FLYING],
            flying_count: 0,
            completed_bytes: 0,
        }
    }

    pub(crate) fn plan(&self) -> &TilePlan {
        &self.plan
    }

    pub(crate) fn total(&self) -> u64 {
        self.total
    }

    /// The next tile to stage of at most `max_bytes`, if there is one that small.
    pub(crate) fn next_tile(&self, max_bytes: u64) -> Option<Tile> {
        self.plan.next(self.cursor, max_bytes)
    }

    /// The smallest tile that could be staged next.
    pub(crate) fn smallest_tile(&self) -> Option<Tile> {
        self.plan.smallest(self.cursor)
    }

    /// Every tile is staged (they may still wait for a submission).
    pub(crate) fn all_staged(&self) -> bool {
        self.plan.done(self.cursor)
    }

    pub(crate) fn has_pending(&self) -> bool {
        self.pending_tiles > 0
    }

    pub(crate) fn staged_bytes(&self) -> u64 {
        self.staged_bytes
    }

    pub(crate) fn submitted_bytes(&self) -> u64 {
        self.submitted_bytes
    }

    pub(crate) fn completed_bytes(&self) -> u64 {
        self.completed_bytes
    }

    pub(crate) fn committed(&self) -> bool {
        self.committed
    }

    pub(crate) fn last_batch(&self) -> u64 {
        self.last_batch
    }

    /// The batch the image's ticket waits for, once the last tiles are submitted.
    pub(crate) fn final_batch(&self) -> Option<u64> {
        (self.final_batch != 0).then_some(self.final_batch)
    }

    /// `tile` (the one [`Self::next_tile`] gave) was staged into the open batch.
    pub(crate) fn staged(&mut self, tile: &Tile) {
        debug_assert_eq!((tile.mip, tile.y), (self.cursor.mip, self.cursor.row));
        self.cursor = self.plan.after(tile);
        self.staged_bytes += tile.bytes;
        self.pending_tiles += 1;
        self.pending_bytes += tile.bytes;
    }

    /// What the submission of the open batch has to do for this image, if the batch has
    /// tiles of it. Asking changes nothing: a deferred submission leaves the stream as it
    /// was.
    pub(crate) fn transitions(&self) -> Option<Transitions> {
        self.has_pending().then_some(Transitions {
            first: !self.committed,
            // Every tile before these went in an earlier submission to the same queue,
            // so with the last one staged the image is whole after this submission.
            last: self.all_staged(),
        })
    }

    /// Whether another submission of this image can be tracked now.
    pub(crate) fn can_submit(&self) -> bool {
        self.flying_count < MAX_FLYING
    }

    /// The open batch was submitted as `batch`, with this image's staged tiles and the
    /// transitions [`Self::transitions`] gave.
    pub(crate) fn submitted(&mut self, batch: u64) {
        debug_assert!(self.has_pending() && batch > self.last_batch);
        self.committed = true;
        self.submitted_bytes += self.pending_bytes;
        self.pending_tiles = 0;
        self.pending_bytes = 0;
        self.last_batch = batch;
        if self.all_staged() {
            self.final_batch = batch;
        }
        self.flying[self.flying_count] = (batch, self.submitted_bytes);
        self.flying_count += 1;
    }

    /// Every batch up to `done` has finished on the GPU.
    pub(crate) fn finished(&mut self, done: u64) {
        let mut kept = 0;
        for i in 0..self.flying_count {
            let (batch, bytes) = self.flying[i];
            if batch <= done {
                self.completed_bytes = self.completed_bytes.max(bytes);
            } else {
                self.flying[kept] = (batch, bytes);
                kept += 1;
            }
        }
        self.flying_count = kept;
    }

    /// Stops the stream: its staged tiles leave the open batch and nothing more is
    /// scheduled.
    pub(crate) fn cancel(&mut self) -> Cancelled {
        let cancelled = Cancelled {
            dropped_tiles: self.pending_tiles,
            dropped_bytes: self.pending_bytes,
            last_batch: self.last_batch,
        };
        self.staged_bytes -= self.pending_bytes;
        self.pending_tiles = 0;
        self.pending_bytes = 0;
        cancelled
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plan(width: u32, height: u32, mips: u32, granularity: u32) -> TilePlan {
        TilePlan::new(
            vk::Format::R8G8B8A8_UNORM,
            vk::Extent2D { width, height },
            mips,
            vk::Extent3D {
                width: granularity,
                height: granularity,
                depth: granularity.min(1),
            },
        )
        .unwrap()
    }

    /// Stages the whole image with tiles of at most `max` bytes and returns them.
    fn tiles(p: &TilePlan, max: u64) -> Vec<Tile> {
        let mut out = Vec::new();
        let mut at = Cursor::default();
        while let Some(t) = p.next(at, max) {
            assert_eq!(p.bytes_before(at) + t.bytes, p.bytes_before(p.after(&t)));
            at = p.after(&t);
            out.push(t);
        }
        assert!(p.done(at), "stuck at {at:?} with {max} bytes a tile");
        out
    }

    #[test]
    fn an_atlas_larger_than_the_ring_is_cut_into_tiles_that_fit() {
        // 2048 x 2048 with every mip level: 16 MiB of level 0 and 5.3 MiB of mips.
        let p = plan(2048, 2048, 12, 1);
        assert_eq!(p.total_bytes(), 22_369_620);
        assert_eq!(p.largest_step(), 2048 * 4);
        let all = tiles(&p, 4 << 20);
        assert!(all.iter().all(|t| t.bytes <= 4 << 20));
        assert_eq!(all.iter().map(|t| t.bytes).sum::<u64>(), p.total_bytes());
        // Level 0 takes four tiles of 512 rows; every smaller level is one tile.
        assert_eq!(all.len(), 4 + 11);
        assert_eq!((all[3].mip, all[3].y, all[3].rows), (0, 1536, 512));
        assert_eq!((all[4].mip, all[4].y, all[4].rows), (1, 0, 1024));
        assert_eq!(all[14].bytes, 4);
    }

    #[test]
    fn tiles_honour_the_transfer_granularity() {
        // A copy engine that moves 16 x 16 blocks: tops and heights are multiples of 16
        // rows, except for a tile that ends at the bottom of its level.
        let p = plan(1000, 1000, 10, 16);
        assert_eq!(p.largest_step(), 16 * 1000 * 4);
        for max in [64_000, 100_000, 1 << 20] {
            for t in tiles(&p, max) {
                let height = (1000u32 >> t.mip).max(1);
                assert_eq!(t.y % 16, 0, "{t:?}");
                assert!(t.rows % 16 == 0 || t.y + t.rows == height, "{t:?}");
                assert_eq!(t.width, (1000u32 >> t.mip).max(1));
                assert!(t.bytes <= max);
            }
        }
        // Less than one step of rows is no tile at all.
        assert_eq!(p.next(Cursor::default(), 63_999), None);
        let smallest = p.smallest(Cursor::default()).unwrap();
        assert_eq!((smallest.rows, smallest.bytes), (16, 64_000));
        let last = Cursor { mip: 0, row: 992 };
        assert_eq!(p.smallest(last).unwrap().rows, 8);
        assert_eq!(p.offset_in_level(&p.smallest(last).unwrap()), 992 * 4000);
    }

    #[test]
    fn a_whole_level_queue_copies_whole_levels() {
        let p = plan(64, 64, 7, 0);
        assert_eq!(p.largest_step(), 64 * 64 * 4);
        assert_eq!(p.next(Cursor::default(), 64 * 64 * 4 - 1), None);
        let all = tiles(&p, 64 * 64 * 4);
        assert_eq!(all.len(), 7);
        assert!(all.iter().all(|t| t.y == 0));
    }

    #[test]
    fn levels_narrower_than_a_texel_row_still_have_one() {
        let p = plan(64, 4, 7, 1);
        let all = tiles(&p, 1 << 20);
        assert_eq!(all.len(), 7);
        assert_eq!((all[6].width, all[6].rows, all[6].bytes), (1, 1, 4));
        assert_eq!(p.level_bytes(1), (32 * 2 * 4, 64 * 4 * 4));
    }

    #[test]
    fn the_first_submission_commits_and_the_last_one_finishes() {
        let p = plan(256, 256, 1, 1);
        let mut s = ImageStream::new(p);
        assert_eq!(s.transitions(), None);
        let quarter = 256 * 64 * 4;
        let t = s.next_tile(quarter).unwrap();
        s.staged(&t);
        assert_eq!(
            s.transitions(),
            Some(Transitions {
                first: true,
                last: false
            })
        );
        s.submitted(5);
        assert!(s.committed());
        assert_eq!(
            (s.submitted_bytes(), s.last_batch(), s.final_batch()),
            (quarter, 5, None)
        );
        // A middle batch keeps layout and ownership.
        let t = s.next_tile(quarter).unwrap();
        s.staged(&t);
        assert_eq!(
            s.transitions(),
            Some(Transitions {
                first: false,
                last: false
            })
        );
        s.submitted(6);
        // The rest: the submission that carries it carries the final transition.
        let t = s.next_tile(u64::MAX).unwrap();
        s.staged(&t);
        assert!(s.all_staged());
        assert_eq!(
            s.transitions(),
            Some(Transitions {
                first: false,
                last: true
            })
        );
        assert_eq!(s.final_batch(), None, "not before it is submitted");
        s.submitted(9);
        assert_eq!(s.final_batch(), Some(9));
        assert_eq!(s.submitted_bytes(), s.total());
        // Coverage completes with the submissions, in any polling order.
        s.finished(5);
        assert_eq!(s.completed_bytes(), quarter);
        s.finished(9);
        assert_eq!(s.completed_bytes(), s.total());
    }

    #[test]
    fn a_deferred_batch_changes_nothing_and_is_retried_once() {
        let p = plan(256, 256, 1, 1);
        let mut s = ImageStream::new(p);
        let t = s.next_tile(256 * 64 * 4).unwrap();
        s.staged(&t);
        // Deferred: the submission did not happen, so nobody calls submitted(). The same
        // tiles are still what the next attempt carries, with the same transitions.
        let before = (
            s.submitted_bytes(),
            s.committed(),
            s.last_batch(),
            s.final_batch(),
        );
        let asked = s.transitions();
        assert_eq!(s.transitions(), asked);
        assert_eq!(
            (
                s.submitted_bytes(),
                s.committed(),
                s.last_batch(),
                s.final_batch()
            ),
            before
        );
        assert!(s.has_pending());
        // More tiles may join the batch while it waits; all go once.
        let t = s.next_tile(u64::MAX).unwrap();
        s.staged(&t);
        assert_eq!(
            s.transitions(),
            Some(Transitions {
                first: true,
                last: true
            })
        );
        s.submitted(1);
        assert_eq!(s.submitted_bytes(), s.total());
        assert!(!s.has_pending());
        assert_eq!(s.transitions(), None);
    }

    #[test]
    fn cancelling_drops_staged_tiles_and_names_the_last_submission() {
        let p = plan(256, 256, 1, 1);
        // Before the first submission: nothing on the GPU uses the image.
        let mut s = ImageStream::new(p);
        let t = s.next_tile(1024).unwrap();
        s.staged(&t);
        assert_eq!(
            s.cancel(),
            Cancelled {
                dropped_tiles: 1,
                dropped_bytes: 1024,
                last_batch: 0
            }
        );
        assert_eq!(s.transitions(), None);
        // During streaming: the submitted part is held until its batch has finished.
        let mut s = ImageStream::new(p);
        let t = s.next_tile(1024).unwrap();
        s.staged(&t);
        s.submitted(3);
        let t = s.next_tile(2048).unwrap();
        s.staged(&t);
        let c = s.cancel();
        assert_eq!(
            (c.dropped_tiles, c.dropped_bytes, c.last_batch),
            (1, 2048, 3)
        );
        assert_eq!(s.submitted_bytes(), 1024);
        // After the final submission: nothing to drop, held until that batch.
        let mut s = ImageStream::new(p);
        let t = s.next_tile(u64::MAX).unwrap();
        s.staged(&t);
        s.submitted(4);
        assert_eq!(
            s.cancel(),
            Cancelled {
                dropped_tiles: 0,
                dropped_bytes: 0,
                last_batch: 4
            }
        );
    }
}

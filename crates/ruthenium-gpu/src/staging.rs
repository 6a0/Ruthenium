//! Bookkeeping of the staging ring's bytes and of the batches that copy from them. Pure
//! arithmetic: the buffer itself, the queues and the commands are the upload ring's
//! ([`crate::UploadRing`]).
//!
//! - [`Ring`] hands out byte positions in order and takes them back in the same order, each
//!   run of bytes once its [`Users`] are done: the batch of copies that reads it, on
//!   either queue or on both.
//! - [`Batches`] numbers the batches of one kind. A batch has an id from the moment
//!   something is staged for it, and gets a value (a frame number, a transfer sequence
//!   value) only when it is actually submitted; until then nothing waits for it and its
//!   bytes stay reserved.

use std::collections::VecDeque;

use crate::upload::UploadError;
use crate::users::{Completed, Users};

/// Runs of bytes with different users the ring can track at once. Bytes staged one after
/// another for the same batch are one run, so this is a few per batch in flight.
const MAX_SLICES: usize = 1024;

/// A run of ring bytes with the same users.
#[derive(Clone, Copy, Debug)]
struct Slice {
    /// The ring position after its last byte; it starts where the slice before it ends.
    end: u64,
    users: Users,
    /// The ring's clock when its first byte was reserved.
    since: u64,
}

/// What was allocated, to take it back if the rest of a call fails.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Checkpoint {
    head: u64,
    slices: usize,
    last_end: u64,
    lap: u64,
    wraps: u64,
}

/// Byte positions in the staging buffer. Positions only grow (a `u64` does not wrap in
/// practice); position `p` is byte `p % capacity` of the buffer.
#[derive(Debug)]
pub(crate) struct Ring {
    capacity: u64,
    /// Where the next allocation starts.
    head: u64,
    /// Everything before this is free again.
    tail: u64,
    /// The bytes between tail and head, oldest first.
    slices: VecDeque<Slice>,
    /// Which pass over the buffer the last allocation was in.
    lap: u64,
    /// Times an allocation started the next pass over the buffer while bytes of the pass
    /// before were still in use.
    wraps: u64,
    /// The most bytes that were in use at once.
    peak: u64,
}

impl Ring {
    pub(crate) fn new(capacity: u64) -> Ring {
        Ring {
            capacity,
            head: 0,
            tail: 0,
            slices: VecDeque::with_capacity(MAX_SLICES),
            lap: 0,
            wraps: 0,
            peak: 0,
        }
    }

    pub(crate) fn capacity(&self) -> u64 {
        self.capacity
    }

    /// Bytes between the oldest byte still in use and the newest, padding included.
    pub(crate) fn occupied(&self) -> u64 {
        self.head - self.tail
    }

    pub(crate) fn peak(&self) -> u64 {
        self.peak
    }

    pub(crate) fn wraps(&self) -> u64 {
        self.wraps
    }

    /// How long the oldest bytes still in use have been reserved, on the clock `now` is
    /// read from.
    pub(crate) fn oldest_age(&self, now: u64) -> u64 {
        self.slices.front().map_or(0, |s| now - s.since)
    }

    /// The largest block that can be reserved now with this alignment: without leaving
    /// the end of the buffer unused, and if the rest of the buffer is skipped.
    pub(crate) fn room(&self, align: u64) -> (u64, u64) {
        let start = self.head.next_multiple_of(align);
        let offset = start % self.capacity;
        let free = (self.tail + self.capacity).saturating_sub(start);
        let here = free.min(self.capacity - offset);
        let skipped = self.capacity - offset;
        let wrapped = free.saturating_sub(skipped).min(self.capacity);
        if self.slices.len() == MAX_SLICES {
            return (0, 0);
        }
        (here, wrapped)
    }

    pub(crate) fn checkpoint(&self) -> Checkpoint {
        Checkpoint {
            head: self.head,
            slices: self.slices.len(),
            last_end: self.slices.back().map_or(self.tail, |s| s.end),
            lap: self.lap,
            wraps: self.wraps,
        }
    }

    /// Takes back everything reserved since `at`. Nothing may have been retired since.
    pub(crate) fn rollback(&mut self, at: Checkpoint) {
        self.slices.truncate(at.slices);
        if let Some(last) = self.slices.back_mut() {
            last.end = at.last_end;
        }
        self.head = at.head;
        self.lap = at.lap;
        self.wraps = at.wraps;
    }

    /// Reserves `size` contiguous bytes aligned to `align` (a power of two dividing the
    /// capacity) for `users`, and returns their offset in the buffer. A block never wraps
    /// around the end: if it does not fit before it, the rest of the buffer is skipped.
    pub(crate) fn alloc(
        &mut self,
        size: u64,
        align: u64,
        users: Users,
        now: u64,
    ) -> Result<u64, UploadError> {
        debug_assert!(align.is_power_of_two() && self.capacity.is_multiple_of(align));
        if size > self.capacity {
            return Err(UploadError::TooLarge);
        }
        let mut start = self.head.next_multiple_of(align);
        let offset = start % self.capacity;
        if offset + size > self.capacity {
            start += self.capacity - offset;
        }
        if start + size - self.tail > self.capacity {
            return Err(UploadError::Full);
        }
        let full = self.slices.len() == MAX_SLICES;
        match self.slices.back_mut() {
            Some(last) if last.users == users => last.end = start + size,
            _ if full => return Err(UploadError::Full),
            _ => self.slices.push_back(Slice {
                end: start + size,
                users,
                since: now,
            }),
        }
        // A new pass over the buffer with the last one still in use is a wrap; an idle
        // ring that starts again at the beginning is not.
        let lap = start / self.capacity;
        self.wraps += u64::from(lap > self.lap && self.head > self.tail);
        self.lap = lap;
        self.head = start + size;
        self.peak = self.peak.max(self.occupied());
        Ok(start % self.capacity)
    }

    /// Frees the oldest bytes whose users have all finished. Bytes behind a run that is
    /// still in use wait for it: space comes back in the order it was handed out.
    pub(crate) fn retire(&mut self, completed: &Completed) {
        while let Some(s) = self.slices.front() {
            if !s.users.done(completed) {
                break;
            }
            self.tail = s.end;
            self.slices.pop_front();
        }
        // Nothing in flight: start again at the beginning, so a large block fits whole.
        if self.slices.is_empty() {
            let rewound = self.head.next_multiple_of(self.capacity);
            self.head = rewound;
            self.tail = rewound;
        }
    }
}

/// The batches of one kind (copies recorded into frames, or copies submitted on their
/// own), in order. Ids start at 1; a ticket holds an id.
#[derive(Debug)]
pub(crate) struct Batches {
    /// The batch being filled: not submitted, so it has no value and nothing can wait on
    /// it.
    open: u64,
    /// Whether anything was staged for the open batch.
    dirty: bool,
    /// Submitted batches not known to be finished, oldest first: the last batch that
    /// went with a submission, and the value that submission signals.
    flying: VecDeque<(u64, u64)>,
    /// Every batch up to this has finished on the GPU.
    done: u64,
}

impl Batches {
    pub(crate) fn new() -> Batches {
        Batches {
            open: 1,
            dirty: false,
            flying: VecDeque::with_capacity(16),
            done: 0,
        }
    }

    /// The id of the batch being filled, which the next successful submission takes.
    pub(crate) fn open(&self) -> u64 {
        self.open
    }

    /// The last batch that was submitted (0: none yet).
    pub(crate) fn submitted(&self) -> u64 {
        self.open - 1
    }

    /// Every batch up to this has finished.
    pub(crate) fn done(&self) -> u64 {
        self.done
    }

    /// Something was staged for the open batch.
    pub(crate) fn touch(&mut self) {
        self.dirty = true;
    }

    pub(crate) fn dirty(&self) -> bool {
        self.dirty
    }

    /// The open batch went with a submission that signals `value` (above every value
    /// before it). Returns the batch's id; a new batch is open.
    pub(crate) fn submit(&mut self, value: u64) -> u64 {
        debug_assert!(self.flying.back().is_none_or(|&(_, v)| v < value));
        let batch = self.open;
        self.flying.push_back((batch, value));
        self.open += 1;
        self.dirty = false;
        batch
    }

    /// The open batch has nothing to submit after all (what was staged for it was
    /// cancelled): it is finished as soon as every submission before it is.
    pub(crate) fn close_empty(&mut self) {
        let batch = self.open;
        match self.flying.back_mut() {
            Some(last) => last.0 = batch,
            None => self.done = batch,
        }
        self.open += 1;
        self.dirty = false;
    }

    /// Every submission that signals `value` or less has finished.
    pub(crate) fn finish(&mut self, value: u64) {
        while let Some(&(batch, v)) = self.flying.front() {
            if v > value {
                break;
            }
            self.done = batch;
            self.flying.pop_front();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frames(batch: u64) -> Users {
        Users {
            frame_batch: batch,
            ..Users::NONE
        }
    }

    fn copies(batch: u64) -> Users {
        Users {
            copy_batch: batch,
            ..Users::NONE
        }
    }

    fn done(frame_batch: u64, copy_batch: u64) -> Completed {
        Completed {
            frame: 0,
            frame_batch,
            copy_batch,
        }
    }

    #[test]
    fn hands_out_aligned_blocks_until_full() {
        let mut r = Ring::new(1024);
        assert_eq!(r.alloc(100, 16, frames(1), 0), Ok(0));
        assert_eq!(r.alloc(100, 16, frames(1), 0), Ok(112));
        assert_eq!(r.alloc(800, 16, frames(1), 0), Ok(224));
        assert_eq!(r.alloc(1, 16, frames(1), 0), Err(UploadError::Full));
        assert_eq!(r.alloc(2000, 16, frames(1), 0), Err(UploadError::TooLarge));
        assert_eq!(r.occupied(), 1024);
    }

    #[test]
    fn space_returns_when_its_batch_finishes() {
        let mut r = Ring::new(1024);
        r.alloc(600, 16, frames(1), 0).unwrap();
        r.alloc(300, 16, frames(2), 0).unwrap();
        assert_eq!(r.alloc(600, 16, frames(3), 0), Err(UploadError::Full));
        r.retire(&done(0, 0));
        assert_eq!(r.alloc(600, 16, frames(3), 0), Err(UploadError::Full));
        r.retire(&done(1, 0));
        // 608..908 is still in use by batch 2; the block wraps to the start.
        assert_eq!(r.alloc(600, 16, frames(3), 0), Ok(0));
        assert_eq!(r.wraps(), 1);
        r.retire(&done(2, 0));
        // Free now: 608..908 only (908..1024 was skipped by batch 3, which uses 0..600).
        assert_eq!(r.alloc(304, 16, frames(4), 0), Err(UploadError::Full));
        assert_eq!(r.alloc(288, 16, frames(4), 0), Ok(608));
    }

    #[test]
    fn a_block_never_straddles_the_end() {
        let mut r = Ring::new(1024);
        r.alloc(900, 16, frames(1), 0).unwrap();
        r.alloc(16, 16, frames(2), 0).unwrap(); // 912..928
        r.retire(&done(1, 0));
        assert_eq!(r.room(16), (96, 900));
        // 200 bytes don't fit in 928..1024: skip to 0 (free since batch 1 finished).
        assert_eq!(r.alloc(200, 16, frames(3), 0), Ok(0));
        // ...but not past the bytes of batch 2.
        assert_eq!(r.alloc(800, 16, frames(3), 0), Err(UploadError::Full));
    }

    #[test]
    fn an_idle_ring_rewinds_so_a_whole_ring_block_fits() {
        let mut r = Ring::new(1024);
        r.alloc(100, 16, frames(1), 0).unwrap();
        r.retire(&done(1, 0));
        assert_eq!(r.occupied(), 0);
        assert_eq!(r.alloc(1024, 16, frames(2), 0), Ok(0));
        assert_eq!(r.wraps(), 0, "starting over when idle is no wrap");
        // Filled to its last byte, then on from the start once the first half is free.
        let mut r = Ring::new(1024);
        r.alloc(512, 16, frames(1), 0).unwrap();
        r.alloc(512, 16, frames(2), 0).unwrap();
        r.retire(&done(1, 0));
        assert_eq!(r.alloc(256, 16, frames(3), 0), Ok(0));
        assert_eq!(r.wraps(), 1);
    }

    #[test]
    fn bytes_used_by_both_queues_wait_for_both() {
        let mut r = Ring::new(1024);
        r.alloc(512, 16, frames(1).and(copies(1)), 0).unwrap();
        r.alloc(256, 16, frames(2), 0).unwrap();
        // The frames have finished, but the transfer copy of the first block has not:
        // nothing is free, not even the second block behind it.
        r.retire(&done(2, 0));
        assert_eq!(r.alloc(512, 16, frames(3), 0), Err(UploadError::Full));
        // Nor with only the transfer copy done.
        let mut other = Ring::new(1024);
        other.alloc(512, 16, frames(1).and(copies(1)), 0).unwrap();
        other.retire(&done(0, 1));
        assert_eq!(other.occupied(), 512);
        r.retire(&done(2, 1));
        assert_eq!(r.alloc(1024, 16, frames(3), 0), Ok(0));
    }

    #[test]
    fn an_unsubmitted_batch_keeps_its_bytes() {
        let mut ring = Ring::new(1024);
        let mut batches = Batches::new();
        // Staged for batch 1, which is deferred: no value, so nothing frees it, however
        // far the queue has got with other work.
        ring.alloc(256, 16, copies(batches.open()), 0).unwrap();
        batches.touch();
        batches.finish(100);
        ring.retire(&done(0, batches.done()));
        assert_eq!(ring.occupied(), 256);
        // Submitted with the next value and finished: free.
        assert_eq!(batches.submit(101), 1);
        batches.finish(100);
        ring.retire(&done(0, batches.done()));
        assert_eq!(ring.occupied(), 256);
        batches.finish(101);
        ring.retire(&done(0, batches.done()));
        assert_eq!(ring.occupied(), 0);
    }

    #[test]
    fn a_rollback_takes_back_only_what_came_after_it() {
        let mut r = Ring::new(1024);
        r.alloc(100, 16, frames(1), 0).unwrap();
        let at = r.checkpoint();
        r.alloc(100, 16, frames(1), 0).unwrap();
        r.alloc(300, 16, copies(1), 0).unwrap();
        assert_eq!(r.occupied(), 524);
        r.rollback(at);
        assert_eq!(r.occupied(), 100);
        assert_eq!(r.alloc(100, 16, frames(1), 0), Ok(112));
        r.retire(&done(1, 0));
        assert_eq!(r.occupied(), 0);
    }

    #[test]
    fn the_oldest_bytes_have_an_age() {
        let mut r = Ring::new(1024);
        assert_eq!(r.oldest_age(5), 0);
        r.alloc(100, 16, frames(1), 5).unwrap();
        r.alloc(100, 16, frames(2), 7).unwrap();
        assert_eq!(r.oldest_age(9), 4);
        r.retire(&done(1, 0));
        assert_eq!(r.oldest_age(9), 2);
    }

    #[test]
    fn batches_finish_in_order_with_their_submissions() {
        let mut b = Batches::new();
        assert_eq!((b.open(), b.submitted(), b.done()), (1, 0, 0));
        assert_eq!(b.submit(10), 1);
        assert_eq!(b.submit(11), 2);
        // Two submissions between the same two frames have values of their own.
        b.finish(10);
        assert_eq!(b.done(), 1);
        b.finish(12);
        assert_eq!(b.done(), 2);
        assert_eq!(b.open(), 3);
    }

    #[test]
    fn an_emptied_batch_finishes_with_the_submissions_before_it() {
        let mut b = Batches::new();
        // Nothing in flight: finished at once.
        b.touch();
        b.close_empty();
        assert_eq!((b.done(), b.open(), b.dirty()), (1, 2, false));
        // Behind a submission: finished when that is.
        b.submit(1);
        b.touch();
        b.close_empty();
        assert_eq!(b.done(), 1);
        b.finish(1);
        assert_eq!(b.done(), 3);
    }
}

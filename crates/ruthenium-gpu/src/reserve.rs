//! Slices of staging memory that workers write: who holds which bytes, and when they may
//! be handed out again. The memory itself (a mapped GPU buffer) and the copies recorded
//! from it are the upload ring's ([`crate::UploadRing`]).
//!
//! The ring's owner grants a slice of a known size: a [`StagingWriter`] for the worker
//! and a [`Reservation`] for itself. The worker fills the slice front to back and
//! publishes that it has finished (a release store the owner reads with acquire, so the
//! bytes are visible with it); only then does the owner record copies from it. A slice is
//! handed out again only when three things are true: the owner has let go of it, the
//! writer has let go of it, and every copy recorded from it has finished on the GPU, on
//! each queue that reads it.
//!
//! Slices come from a free list, not from a ring, so a slow worker holds its own slice
//! and nothing behind it. How many can be out at once is limited, and ordinary
//! reservations leave some slots and bytes for urgent ones (a block edit the player is
//! looking at), so a burst of background meshing never starves those.

use std::fmt;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};

use crate::pool::{Block, RangeAllocator};
use crate::upload::UploadError;
use crate::users::{Completed, Users};

/// Slices that can be out at once, and how many of them only urgent edits may take.
pub(crate) const MAX_RESERVATIONS: usize = 64;
pub(crate) const URGENT_RESERVATIONS: usize = 8;

/// States of a slice granted to a worker.
const WRITING: u32 = 0;
const COMPLETE: u32 = 1;
const ABANDONED: u32 = 2;

/// What the owner and the writer share about one slice.
#[derive(Debug)]
struct SliceCell {
    state: AtomicU32,
    written: AtomicU64,
}

/// What the owner and all writers share.
#[derive(Debug)]
struct Shared {
    cells: Box<[SliceCell]>,
    /// Writers that have not let go of their slice yet.
    writers: AtomicU32,
}

/// An exclusive slice of staging memory granted to a worker ([`crate::UploadRing::reserve`]).
/// The worker appends its bytes in order and calls [`Self::finish`]; dropping the writer
/// without finishing abandons the slice. The memory is write-combined on many GPUs: it is
/// only ever written here, front to back, never read.
pub struct StagingWriter {
    shared: Arc<Shared>,
    index: u32,
    start: *mut u8,
    capacity: usize,
    written: usize,
    finished: bool,
}

// SAFETY: the writer is the only user of its bytes of the memory until it lets go (the
// owner neither reads, hands out nor copies them before), and the owner keeps the memory
// alive while any writer exists.
unsafe impl Send for StagingWriter {}

impl fmt::Debug for StagingWriter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("StagingWriter")
            .field("capacity", &self.capacity)
            .field("written", &self.written)
            .finish_non_exhaustive()
    }
}

impl StagingWriter {
    /// The size the slice was granted with.
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    pub fn written(&self) -> usize {
        self.written
    }

    /// Appends `data`. `false`, with nothing written, if it does not fit in what is left
    /// of the slice.
    pub fn push(&mut self, data: &[u8]) -> bool {
        if data.len() > self.capacity - self.written {
            return false;
        }
        // SAFETY: [written, written + len) lies inside the slice, which only this writer
        // touches (see the Send impl); the source is ordinary memory that cannot overlap
        // the slice.
        unsafe {
            std::ptr::copy_nonoverlapping(data.as_ptr(), self.start.add(self.written), data.len())
        };
        self.written += data.len();
        true
    }

    /// The slice is written: the owner may record copies from its first
    /// [`Self::written`] bytes.
    pub fn finish(mut self) {
        self.finished = true;
    }
}

impl Drop for StagingWriter {
    fn drop(&mut self) {
        let cell = &self.shared.cells[self.index as usize];
        cell.written.store(self.written as u64, Ordering::Relaxed);
        let state = if self.finished { COMPLETE } else { ABANDONED };
        // Release: whoever reads the state with Acquire also sees the bytes and the count
        // written before it.
        cell.state.store(state, Ordering::Release);
        self.shared.writers.fetch_sub(1, Ordering::Release);
    }
}

/// A slice of worker staging memory, as the ring's owner holds it. A handle that has been
/// let go and reclaimed names nothing, also after its place is used again.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Reservation {
    index: u32,
    generation: u32,
}

/// What a worker has done with its slice.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReservationState {
    /// The worker still has it.
    Writing,
    /// Written: copies may be recorded from its first `written` bytes.
    Complete { written: u64 },
    /// The worker dropped it unfinished.
    Abandoned,
    /// The handle is stale: the slice was let go and reclaimed.
    Gone,
}

/// One place in the table of reservations.
#[derive(Clone, Copy, Debug)]
struct Slot {
    generation: u32,
    /// A slice is out under this slot.
    live: bool,
    block: Block,
    /// The owner has let go: the slice is reclaimed once the writer has too and every
    /// copy from it has finished.
    released: bool,
    /// Its completion (or abandonment) has been counted.
    counted: bool,
    /// The batches that copy from it.
    users: Users,
    /// The owner's clock when it was granted.
    since: u64,
}

/// What the table has granted and holds.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct ReservationCounters {
    pub granted: u64,
    /// Refused for want of a slot or of bytes.
    pub refused: u64,
    /// Dropped by their writer unfinished.
    pub abandoned: u64,
    /// Bytes written into finished slices.
    pub written_bytes: u64,
    /// Slices out now and at most, and the bytes they hold.
    pub outstanding: u32,
    pub peak_outstanding: u32,
    pub bytes: u64,
    pub peak_bytes: u64,
}

/// The slices of one block of staging memory. See the module docs.
#[derive(Debug)]
pub(crate) struct Reservations {
    base: *mut u8,
    ranges: RangeAllocator,
    shared: Arc<Shared>,
    slots: [Slot; MAX_RESERVATIONS],
    /// Bytes ordinary reservations leave free for urgent ones.
    urgent_room: u64,
    counters: ReservationCounters,
}

impl Reservations {
    /// The table for `capacity` bytes at `base`, handed out in multiples of `granule`. An
    /// eighth of them is kept for urgent reservations.
    ///
    /// # Safety
    /// `base` points at `capacity` writable bytes that nothing else writes, and that stay
    /// valid until no writer is left ([`Self::writers`] is zero).
    pub(crate) unsafe fn new(base: *mut u8, capacity: u64, granule: u64) -> Reservations {
        let cells = (0..MAX_RESERVATIONS)
            .map(|_| SliceCell {
                state: AtomicU32::new(ABANDONED),
                written: AtomicU64::new(0),
            })
            .collect();
        Reservations {
            base,
            ranges: RangeAllocator::new(capacity, granule),
            shared: Arc::new(Shared {
                cells,
                writers: AtomicU32::new(0),
            }),
            slots: [Slot {
                generation: 0,
                live: false,
                block: Block {
                    offset: 0,
                    size: 0,
                    node: 0,
                },
                released: false,
                counted: false,
                users: Users::NONE,
                since: 0,
            }; MAX_RESERVATIONS],
            urgent_room: capacity / 8,
            counters: ReservationCounters::default(),
        }
    }

    /// Grants a slice of `size` bytes at an offset aligned to `align` (a multiple of the
    /// granule). An ordinary reservation is refused while it would leave fewer slots or
    /// bytes than are kept for urgent ones.
    ///
    /// # Panics
    /// If `size` is zero.
    pub(crate) fn grant(
        &mut self,
        size: u64,
        align: u64,
        urgent: bool,
        now: u64,
    ) -> Result<(Reservation, StagingWriter), UploadError> {
        assert!(size > 0, "an empty slice");
        let (kept_slots, kept_bytes) = if urgent {
            (0, 0)
        } else {
            (URGENT_RESERVATIONS as u32, self.urgent_room)
        };
        if size.saturating_add(kept_bytes) > self.ranges.size() {
            return Err(UploadError::TooLarge);
        }
        let room = self.counters.outstanding + kept_slots < MAX_RESERVATIONS as u32
            && self.ranges.free_bytes() >= size + kept_bytes;
        let Some(block) = room.then(|| self.ranges.alloc(size, align)).flatten() else {
            self.counters.refused += 1;
            return Err(UploadError::Full);
        };
        let index = self
            .slots
            .iter()
            .position(|s| !s.live)
            .expect("fewer slices are out than there are slots");
        let slot = &mut self.slots[index];
        slot.generation = slot.generation.wrapping_add(1);
        slot.live = true;
        slot.block = block;
        slot.released = false;
        slot.counted = false;
        slot.users = Users::NONE;
        slot.since = now;
        let cell = &self.shared.cells[index];
        cell.written.store(0, Ordering::Relaxed);
        cell.state.store(WRITING, Ordering::Release);
        self.shared.writers.fetch_add(1, Ordering::Release);
        let c = &mut self.counters;
        c.granted += 1;
        c.outstanding += 1;
        c.peak_outstanding = c.peak_outstanding.max(c.outstanding);
        c.bytes = self.ranges.size() - self.ranges.free_bytes();
        c.peak_bytes = c.peak_bytes.max(c.bytes);
        let writer = StagingWriter {
            shared: self.shared.clone(),
            index: index as u32,
            // SAFETY: the block lies inside the memory (new()'s contract).
            start: unsafe { self.base.add(block.offset as usize) },
            capacity: size as usize,
            written: 0,
            finished: false,
        };
        Ok((
            Reservation {
                index: index as u32,
                generation: slot.generation,
            },
            writer,
        ))
    }

    /// The live slot a reservation names, if it still names one.
    fn slot(&self, r: Reservation) -> Option<usize> {
        let slot = self.slots.get(r.index as usize)?;
        (slot.live && slot.generation == r.generation).then_some(r.index as usize)
    }

    /// What the worker has done with the slice.
    pub(crate) fn state(&self, r: Reservation) -> ReservationState {
        let Some(index) = self.slot(r) else {
            return ReservationState::Gone;
        };
        let cell = &self.shared.cells[index];
        // Acquire: pairs with the writer's Release, so the bytes and the count it wrote
        // before are visible here.
        match cell.state.load(Ordering::Acquire) {
            WRITING => ReservationState::Writing,
            COMPLETE => ReservationState::Complete {
                written: cell.written.load(Ordering::Relaxed),
            },
            _ => ReservationState::Abandoned,
        }
    }

    /// A copy from the slice was recorded for `users`: it is held until they are done as
    /// well. Returns where the slice starts in the memory; `None` for a stale reservation.
    pub(crate) fn copied_by(&mut self, r: Reservation, users: Users) -> Option<u64> {
        let index = self.slot(r)?;
        let slot = &mut self.slots[index];
        slot.users = slot.users.and(users);
        Some(slot.block.offset)
    }

    /// The owner lets the slice go. Nothing happens for a stale reservation.
    pub(crate) fn release(&mut self, r: Reservation) {
        if let Some(index) = self.slot(r) {
            self.slots[index].released = true;
        }
    }

    /// Counts the slices whose writer has finished, and takes back those that nobody
    /// uses any more: let go by the owner and by the writer, with every copy done.
    pub(crate) fn reclaim(&mut self, completed: &Completed) {
        for (index, slot) in self.slots.iter_mut().enumerate() {
            if !slot.live {
                continue;
            }
            let cell = &self.shared.cells[index];
            let state = cell.state.load(Ordering::Acquire);
            if state == WRITING {
                // The writer still has the memory, whatever the owner decided.
                continue;
            }
            if !slot.counted {
                slot.counted = true;
                if state == COMPLETE {
                    self.counters.written_bytes += cell.written.load(Ordering::Relaxed);
                } else {
                    self.counters.abandoned += 1;
                }
            }
            if slot.released && slot.users.done(completed) {
                self.ranges.free(slot.block);
                slot.live = false;
                self.counters.outstanding -= 1;
            }
        }
        self.counters.bytes = self.ranges.size() - self.ranges.free_bytes();
    }

    /// Writers that have not let go of their slice yet.
    pub(crate) fn writers(&self) -> u32 {
        self.shared.writers.load(Ordering::Acquire)
    }

    pub(crate) fn counters(&self) -> ReservationCounters {
        self.counters
    }

    /// How long the oldest slice still out has been held, on the clock `now` is read from.
    pub(crate) fn oldest_age(&self, now: u64) -> u64 {
        self.slots
            .iter()
            .filter(|slot| slot.live)
            .map(|slot| now - slot.since)
            .max()
            .unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const KIB: u64 = 1024;

    /// A table over ordinary memory, which the returned vector keeps alive.
    fn table(capacity: u64) -> (Reservations, Vec<u8>) {
        let mut memory = vec![0u8; capacity as usize];
        // SAFETY: the vector outlives the table and every writer in each test.
        let table = unsafe { Reservations::new(memory.as_mut_ptr(), capacity, 16) };
        (table, memory)
    }

    fn copies(batch: u64) -> Users {
        Users {
            copy_batch: batch,
            ..Users::NONE
        }
    }

    const NOTHING: Completed = Completed {
        frame: 0,
        frame_batch: 0,
        copy_batch: 0,
    };

    #[test]
    fn a_writer_publishes_what_it_wrote_or_that_it_gave_up() {
        let (mut t, memory) = table(64 * KIB);
        let (first, mut writer) = t.grant(8, 16, false, 0).unwrap();
        let (second, abandoned) = t.grant(8, 16, false, 0).unwrap();
        assert_eq!(t.state(first), ReservationState::Writing);
        assert_eq!(t.writers(), 2);
        // Written on another thread, as a worker does; never past its slice.
        std::thread::spawn(move || {
            assert!(writer.push(&[1, 2, 3]));
            assert!(!writer.push(&[0; 6]), "more than is left");
            assert!(writer.push(&[4, 5]));
            assert_eq!((writer.written(), writer.capacity()), (5, 8));
            writer.finish();
        })
        .join()
        .unwrap();
        assert_eq!(t.state(first), ReservationState::Complete { written: 5 });
        // Dropped unfinished: abandoned, and let go all the same.
        drop(abandoned);
        assert_eq!(t.state(second), ReservationState::Abandoned);
        assert_eq!(t.writers(), 0);
        let at = t.copied_by(first, Users::NONE).unwrap() as usize;
        assert_eq!(memory[at..at + 5], [1, 2, 3, 4, 5]);
        t.reclaim(&NOTHING);
        let c = t.counters();
        assert_eq!((c.written_bytes, c.abandoned, c.outstanding), (5, 1, 2));
    }

    #[test]
    fn a_slice_goes_back_only_when_owner_writer_and_copies_are_done() {
        let (mut t, _memory) = table(64 * KIB);
        let (r, writer) = t.grant(4 * KIB, 16, false, 3).unwrap();
        assert_eq!(t.oldest_age(10), 7);
        // Let go by the owner while the worker still writes: cancelled, but not free.
        t.release(r);
        t.reclaim(&NOTHING);
        assert_eq!(t.state(r), ReservationState::Writing);
        assert_eq!(t.counters().bytes, 4 * KIB);
        drop(writer);
        t.reclaim(&NOTHING);
        assert_eq!(t.state(r), ReservationState::Gone);
        assert_eq!(t.counters().bytes, 0);

        // Written and copied on both queues: held until both batches have finished.
        let (r, writer) = t.grant(4 * KIB, 16, false, 0).unwrap();
        writer.finish();
        t.copied_by(r, copies(7)).unwrap();
        t.copied_by(
            r,
            Users {
                frame_batch: 2,
                ..Users::NONE
            },
        )
        .unwrap();
        // Not while the owner may still record copies from it, whatever has finished.
        let everything = Completed {
            frame: 9,
            frame_batch: 9,
            copy_batch: 9,
        };
        t.reclaim(&everything);
        assert_eq!(t.state(r), ReservationState::Complete { written: 0 });
        t.release(r);
        for (frame_batch, copy_batch) in [(0, 0), (2, 6), (1, 7)] {
            t.reclaim(&Completed {
                frame: 0,
                frame_batch,
                copy_batch,
            });
            assert_ne!(
                t.state(r),
                ReservationState::Gone,
                "{frame_batch} {copy_batch}"
            );
        }
        t.reclaim(&Completed {
            frame: 0,
            frame_batch: 2,
            copy_batch: 7,
        });
        assert_eq!(t.state(r), ReservationState::Gone);
        assert_eq!(t.counters().outstanding, 0);
    }

    #[test]
    fn a_stale_reservation_names_nothing_also_after_its_place_is_reused() {
        let (mut t, _memory) = table(64 * KIB);
        let (old, writer) = t.grant(KIB, 16, false, 0).unwrap();
        drop(writer);
        t.release(old);
        t.reclaim(&NOTHING);
        let (new, writer) = t.grant(KIB, 16, false, 0).unwrap();
        assert_eq!(old.index, new.index, "the place is used again");
        assert_eq!(t.state(old), ReservationState::Gone);
        assert_eq!(t.copied_by(old, Users::NONE), None);
        // Letting the stale one go does not let the new one go.
        t.release(old);
        writer.finish();
        t.reclaim(&NOTHING);
        assert_eq!(t.state(new), ReservationState::Complete { written: 0 });
    }

    #[test]
    fn ordinary_reservations_leave_bytes_for_urgent_ones() {
        // 64 KiB, of which 8 KiB are kept: seven ordinary slices of 8 KiB fit.
        let (mut t, _memory) = table(64 * KIB);
        let mut held = Vec::new();
        for _ in 0..7 {
            held.push(t.grant(8 * KIB, 16, false, 0).unwrap());
        }
        assert_eq!(
            t.grant(8 * KIB, 16, false, 0).map(|(r, _)| r),
            Err(UploadError::Full)
        );
        assert_eq!(t.counters().refused, 1);
        // The urgent one takes the room that was kept; then that is used up too.
        held.push(t.grant(8 * KIB, 16, true, 0).unwrap());
        assert_eq!(
            t.grant(16, 16, true, 0).map(|(r, _)| r),
            Err(UploadError::Full)
        );
        // More than could ever be granted is not a matter of waiting.
        assert_eq!(
            t.grant(60 * KIB, 16, false, 0).map(|(r, _)| r),
            Err(UploadError::TooLarge)
        );
        assert_eq!(
            t.grant(65 * KIB, 16, true, 0).map(|(r, _)| r),
            Err(UploadError::TooLarge)
        );
        assert_eq!(t.counters().peak_bytes, 64 * KIB);
        // A slow worker holds its own slice only: when the others are back, there is room.
        let (slow, slow_writer) = held.remove(3);
        for (r, writer) in held {
            drop(writer);
            t.release(r);
        }
        t.release(slow);
        t.reclaim(&NOTHING);
        assert_eq!(t.counters().bytes, 8 * KIB);
        assert!(t.grant(30 * KIB, 16, false, 0).is_ok());
        drop(slow_writer);
    }

    #[test]
    fn ordinary_reservations_leave_slots_for_urgent_ones() {
        let (mut t, _memory) = table(1024 * KIB);
        let mut held = Vec::new();
        for _ in 0..MAX_RESERVATIONS - URGENT_RESERVATIONS {
            held.push(t.grant(16, 16, false, 0).unwrap());
        }
        assert_eq!(
            t.grant(16, 16, false, 0).map(|(r, _)| r),
            Err(UploadError::Full)
        );
        for _ in 0..URGENT_RESERVATIONS {
            held.push(t.grant(16, 16, true, 0).unwrap());
        }
        assert_eq!(
            t.grant(16, 16, true, 0).map(|(r, _)| r),
            Err(UploadError::Full)
        );
        assert_eq!(t.counters().peak_outstanding, MAX_RESERVATIONS as u32);
        // Every handle is distinct.
        for (i, (a, _)) in held.iter().enumerate() {
            assert!(held[i + 1..].iter().all(|(b, _)| a != b));
        }
    }
}

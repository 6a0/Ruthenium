//! The handoff from the main thread to the game thread: the frames' input and the
//! session's controls, in one numbered order, in a queue that cannot grow without
//! bound.
//!
//! - Every item takes the next sequence number as it is handed over, frames and
//!   controls alike, and the game thread takes them in that order. A frame that carries
//!   a key going down or up, a click, a scroll or a turn stays a frame of its own: the
//!   game thread sees the same boundaries, in the same order, as the main thread made.
//!   Nothing is merged, not even two frames that hold the same keys and carry nothing
//!   else.
//! - The queue holds a bounded number of frames and of events (scrolls and turns)
//!   inside them. When it is full, the main thread waits before it hands over more: the
//!   input already queued is kept, the new frame's input is kept, and how long the wait
//!   lasted is counted ([`HandoffStats`]). Controls are few and always go in.
//! - A waiting game thread is woken once per burst: a wake already on its way stands
//!   for the items queued behind it.
//! - Closing is a flag, not an item, so it reaches the game thread even when the queue
//!   is full and everything in it is still to do.

use std::collections::VecDeque;
use std::sync::{Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

/// How much the queue holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// Frames waiting at once.
    pub frames: usize,
    /// Scrolls and turns waiting at once, in all the frames together. A single frame
    /// with more than this still goes in, alone.
    pub events: usize,
}

impl Default for Limits {
    /// Room for a game thread that falls 64 frames behind: starting values, to be set
    /// from measurements of chunk bursts.
    fn default() -> Limits {
        Limits {
            frames: 64,
            events: 4096,
        }
    }
}

/// What the handoff has seen since it was made.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct HandoffStats {
    /// Frames handed over.
    pub frames: u64,
    /// The most frames that ever waited.
    pub high_water: usize,
    /// The longest an item waited before the game thread took it.
    pub oldest_age: Duration,
    /// Times the main thread had to wait for room.
    pub stalls: u64,
    /// How long it waited in all, and at worst.
    pub stall_time: Duration,
    pub longest_stall: Duration,
    /// Wakes of the game thread that an earlier, still pending one stood for.
    pub wakes_coalesced: u64,
}

#[derive(Debug)]
pub(crate) enum Item<F, C> {
    Frame(F),
    Control(C),
}

/// An item as the game thread takes it.
#[derive(Debug)]
pub(crate) struct Taken<F, C> {
    pub sequence: u64,
    /// When it was handed over.
    pub at: Instant,
    pub item: Item<F, C>,
}

#[derive(Debug)]
pub(crate) enum Take<F, C> {
    Item(Taken<F, C>),
    /// The session asked to close (reported once; items keep coming after it).
    Closing,
    /// The session is gone: nothing more will come.
    Dropped,
}

#[derive(Debug)]
struct Queued<F, C> {
    sequence: u64,
    at: Instant,
    events: usize,
    item: Item<F, C>,
}

#[derive(Debug)]
struct State<F, C> {
    queue: VecDeque<Queued<F, C>>,
    sequence: u64,
    frames: usize,
    events: usize,
    wake_pending: bool,
    closing: bool,
    closing_reported: bool,
    dropped: bool,
    /// The game thread has stopped taking items.
    ended: bool,
    stats: HandoffStats,
}

#[derive(Debug)]
pub(crate) struct Handoff<F, C> {
    state: Mutex<State<F, C>>,
    /// The game thread waits here for items.
    wake: Condvar,
    /// The main thread waits here for room.
    room: Condvar,
    limits: Limits,
}

impl<F, C> Handoff<F, C> {
    pub fn new(limits: Limits) -> Handoff<F, C> {
        Handoff {
            state: Mutex::new(State {
                queue: VecDeque::with_capacity(limits.frames + 8),
                sequence: 0,
                frames: 0,
                events: 0,
                wake_pending: false,
                closing: false,
                closing_reported: false,
                dropped: false,
                ended: false,
                stats: HandoffStats::default(),
            }),
            wake: Condvar::new(),
            room: Condvar::new(),
            limits: Limits {
                frames: limits.frames.max(1),
                events: limits.events.max(1),
            },
        }
    }

    fn lock(&self) -> MutexGuard<'_, State<F, C>> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn push(&self, mut s: MutexGuard<'_, State<F, C>>, events: usize, item: Item<F, C>) -> u64 {
        s.sequence += 1;
        let sequence = s.sequence;
        if s.ended {
            // Nobody takes it; the number is still used up, as the caller counts by it.
            return sequence;
        }
        s.queue.push_back(Queued {
            sequence,
            at: Instant::now(),
            events,
            item,
        });
        if s.wake_pending {
            s.stats.wakes_coalesced += 1;
        } else {
            s.wake_pending = true;
            self.wake.notify_one();
        }
        sequence
    }

    /// Hands a frame over, waiting for room first if the queue is full, and returns
    /// its sequence number. `events` is how many scrolls and turns it carries.
    pub fn push_frame(&self, frame: F, events: usize) -> u64 {
        let mut s = self.lock();
        let mut waited: Option<Instant> = None;
        while !s.ended
            && (s.frames >= self.limits.frames
                || (s.frames > 0 && s.events + events > self.limits.events))
        {
            waited.get_or_insert_with(Instant::now);
            s = self
                .room
                .wait(s)
                .unwrap_or_else(std::sync::PoisonError::into_inner);
        }
        if let Some(since) = waited {
            let d = since.elapsed();
            s.stats.stalls += 1;
            s.stats.stall_time += d;
            s.stats.longest_stall = s.stats.longest_stall.max(d);
        }
        if !s.ended {
            s.frames += 1;
            s.events += events;
            s.stats.frames += 1;
            s.stats.high_water = s.stats.high_water.max(s.frames);
        }
        self.push(s, events, Item::Frame(frame))
    }

    /// Hands a control over, behind everything already queued. Never waits.
    pub fn push_control(&self, control: C) -> u64 {
        let s = self.lock();
        self.push(s, 0, Item::Control(control))
    }

    /// Asks the game thread to close, whatever is queued.
    pub fn close(&self) {
        let mut s = self.lock();
        s.closing = true;
        self.wake.notify_one();
    }

    /// The session is being dropped: the game thread stops at its next take.
    pub fn drop_session(&self) {
        let mut s = self.lock();
        s.dropped = true;
        self.wake.notify_one();
    }

    /// The game thread takes no more: a main thread waiting for room goes on.
    pub fn end(&self) {
        let mut s = self.lock();
        s.ended = true;
        s.queue.clear();
        s.frames = 0;
        s.events = 0;
        self.room.notify_all();
    }

    /// The next item in order, waiting for one. Closing and dropping come first.
    pub fn take(&self) -> Take<F, C> {
        let mut s = self.lock();
        loop {
            if s.dropped {
                return Take::Dropped;
            }
            if s.closing && !s.closing_reported {
                s.closing_reported = true;
                return Take::Closing;
            }
            if let Some(q) = s.queue.pop_front() {
                if matches!(q.item, Item::Frame(_)) {
                    s.frames -= 1;
                    s.events -= q.events;
                    self.room.notify_one();
                }
                let age = q.at.elapsed();
                s.stats.oldest_age = s.stats.oldest_age.max(age);
                return Take::Item(Taken {
                    sequence: q.sequence,
                    at: q.at,
                    item: q.item,
                });
            }
            s.wake_pending = false;
            s = self
                .wake
                .wait(s)
                .unwrap_or_else(std::sync::PoisonError::into_inner);
        }
    }

    pub fn stats(&self) -> HandoffStats {
        self.lock().stats
    }

    /// Frames waiting now.
    pub fn waiting(&self) -> usize {
        self.lock().frames
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    type H = Handoff<(u32, Vec<i32>), &'static str>;

    fn limits(frames: usize, events: usize) -> Limits {
        Limits { frames, events }
    }

    fn item(t: Take<(u32, Vec<i32>), &'static str>) -> Taken<(u32, Vec<i32>), &'static str> {
        match t {
            Take::Item(t) => t,
            other => panic!("expected an item, got {other:?}"),
        }
    }

    #[test]
    fn frames_and_controls_keep_one_order() {
        let h = H::new(Limits::default());
        assert_eq!(h.push_frame((1, vec![]), 0), 1);
        assert_eq!(h.push_control("respawn"), 2);
        assert_eq!(h.push_frame((2, vec![5]), 1), 3);
        assert_eq!(h.waiting(), 2);
        let a = item(h.take());
        assert!(matches!(a.item, Item::Frame((1, _))) && a.sequence == 1);
        let b = item(h.take());
        assert!(matches!(b.item, Item::Control("respawn")) && b.sequence == 2);
        let c = item(h.take());
        assert!(matches!(c.item, Item::Frame((2, _))) && c.sequence == 3);
        let s = h.stats();
        assert_eq!((s.frames, s.high_water, s.stalls), (2, 2, 0));
        // One wake stood for the two items queued behind the first.
        assert_eq!(s.wakes_coalesced, 2);
    }

    #[test]
    fn a_full_queue_holds_the_main_thread_back_and_loses_nothing() {
        // The game thread is slower than the main thread: 200 frames, each with its own
        // input, through a queue of 4.
        let h = Arc::new(H::new(limits(4, 64)));
        let producer = {
            let h = h.clone();
            std::thread::spawn(move || {
                for i in 0..200_u32 {
                    let turns = vec![i as i32; (i % 3) as usize];
                    let n = turns.len();
                    assert_eq!(h.push_frame((i, turns), n), u64::from(i) + 1);
                }
            })
        };
        let mut got = Vec::new();
        while got.len() < 200 {
            let t = item(h.take());
            let Item::Frame((i, turns)) = t.item else {
                panic!("a control nobody sent");
            };
            assert_eq!(t.sequence, u64::from(i) + 1);
            assert_eq!(turns, vec![i as i32; (i % 3) as usize]);
            got.push(i);
            if i % 16 == 0 {
                std::thread::sleep(Duration::from_millis(2));
            }
        }
        producer.join().unwrap();
        assert_eq!(got, (0..200).collect::<Vec<_>>());
        let s = h.stats();
        assert_eq!(s.frames, 200);
        assert!(s.high_water <= 4, "{s:?}");
        assert!(s.stalls > 0 && s.stall_time >= s.longest_stall, "{s:?}");
        assert!(s.oldest_age > Duration::ZERO);
        assert_eq!(h.waiting(), 0);
    }

    #[test]
    fn events_are_bounded_too_but_one_large_frame_still_goes_in() {
        let h = Arc::new(H::new(limits(8, 10)));
        // More events than the bound, into an empty queue: admitted alone.
        h.push_frame((0, vec![0; 30]), 30);
        let blocked = {
            let h = h.clone();
            std::thread::spawn(move || h.push_frame((1, vec![0; 1]), 1))
        };
        std::thread::sleep(Duration::from_millis(30));
        // The second frame waits although only one frame is queued.
        assert_eq!(h.waiting(), 1);
        item(h.take());
        assert_eq!(blocked.join().unwrap(), 2);
        assert_eq!(h.stats().stalls, 1);
    }

    #[test]
    fn closing_gets_through_a_full_queue() {
        let h = Arc::new(H::new(limits(2, 8)));
        h.push_frame((0, vec![]), 0);
        h.push_frame((1, vec![]), 0);
        h.close();
        // Closing first, once; then the frames, still in order.
        assert!(matches!(h.take(), Take::Closing));
        assert!(matches!(item(h.take()).item, Item::Frame((0, _))));
        assert!(matches!(item(h.take()).item, Item::Frame((1, _))));
        // A dropped session ends the taking at once.
        h.push_frame((2, vec![]), 0);
        h.drop_session();
        assert!(matches!(h.take(), Take::Dropped));
    }

    #[test]
    fn a_game_thread_that_ended_lets_the_main_thread_go() {
        let h = Arc::new(H::new(limits(1, 8)));
        h.push_frame((0, vec![]), 0);
        let blocked = {
            let h = h.clone();
            std::thread::spawn(move || h.push_frame((1, vec![]), 0))
        };
        std::thread::sleep(Duration::from_millis(30));
        h.end();
        // The waiting frame is let go, numbered but not kept; later ones too.
        assert_eq!(blocked.join().unwrap(), 2);
        assert_eq!(h.push_frame((2, vec![]), 0), 3);
        assert_eq!(h.waiting(), 0);
    }
}

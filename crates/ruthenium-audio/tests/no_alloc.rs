//! The mixer neither allocates nor frees while it renders: plays starting, voices ending
//! and being handed back, streams, parameter changes and stops, all counted by a global
//! allocator that watches the rendering thread.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use ruthenium_audio::decode::Pcm;
use ruthenium_audio::mixer::{Command, Feed, Mixer, StreamFeed, VoiceParams};
use ruthenium_audio::spatial::{Listener, ListenerCell, Placement};

struct Counting;

static CALLS: AtomicUsize = AtomicUsize::new(0);

thread_local! {
    static WATCHED: Cell<bool> = const { Cell::new(false) };
}

fn watched() -> bool {
    WATCHED.try_with(Cell::get).unwrap_or(false)
}

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if watched() {
            CALLS.fetch_add(1, Ordering::Relaxed);
        }
        // SAFETY: forwarded as given.
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        if watched() {
            CALLS.fetch_add(1, Ordering::Relaxed);
        }
        // SAFETY: forwarded as given.
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        if watched() {
            CALLS.fetch_add(1, Ordering::Relaxed);
        }
        // SAFETY: forwarded as given.
        unsafe { System.realloc(ptr, layout, size) }
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

fn render_watched(m: &mut Mixer, out: &mut [f32]) {
    WATCHED.with(|w| w.set(true));
    m.render(out);
    WATCHED.with(|w| w.set(false));
}

#[test]
fn rendering_never_allocates_or_frees() {
    let listener = Arc::new(ListenerCell::default());
    let (mut mixer, mut link) = Mixer::new(48_000, listener.clone());
    let mut out = vec![0.0f32; 2 * 512];
    let params = |x: f32| VoiceParams {
        gain: 0.7,
        pitch: 1.3,
        place: Placement {
            position: [x, 0.0, -3.0],
            relative: false,
            max_distance: Some(16.0),
        },
    };
    let short = Arc::new(Pcm {
        channels: 1,
        rate: 44_100,
        samples: vec![1000; 300].into_boxed_slice(),
    });
    let long = Arc::new(Pcm {
        channels: 2,
        rate: 48_000,
        samples: vec![-2000; 2 * 48_000].into_boxed_slice(),
    });
    let (mut tx, rx) = rtrb::RingBuffer::new(8192);
    for _ in 0..4000 {
        tx.push(500).unwrap();
    }
    let ended = Arc::new(AtomicBool::new(false));
    for slot in 0..200u16 {
        let feed = Feed::Static(if slot % 2 == 0 {
            short.clone()
        } else {
            long.clone()
        });
        link.commands
            .push(Command::Play {
                slot,
                feed,
                params: params(slot as f32),
                looping: slot % 5 == 0,
            })
            .unwrap();
    }
    link.commands
        .push(Command::Play {
            slot: 250,
            feed: Feed::Stream(Box::new(StreamFeed {
                channels: 1,
                rate: 22_050,
                samples: rx,
                ended: ended.clone(),
            })),
            params: params(0.0),
            looping: false,
        })
        .unwrap();
    CALLS.store(0, Ordering::Relaxed);
    for block in 0..60 {
        if block == 10 {
            for slot in (1..200u16).step_by(3) {
                link.commands
                    .push(Command::Set {
                        slot,
                        params: params(-(slot as f32)),
                    })
                    .unwrap();
            }
            listener.store(&Listener::camera([1.0, 2.0, 3.0], 45.0, 10.0));
        }
        if block == 20 {
            for slot in (0..200u16).step_by(5) {
                link.commands.push(Command::Stop { slot }).unwrap();
            }
            ended.store(true, Ordering::Release);
        }
        render_watched(&mut mixer, &mut out);
    }
    assert_eq!(
        CALLS.load(Ordering::Relaxed),
        0,
        "allocations or frees while rendering"
    );
    // Every voice ended or was stopped, and came back through the ring.
    let mut back = 0;
    while link.retired.pop().is_ok() {
        back += 1;
    }
    // The 100 short ones (20 of them looping, then stopped), 20 stopped long ones and
    // the stream; the other 80 long ones play on.
    assert_eq!((back, mixer.playing()), (121, 80));
}

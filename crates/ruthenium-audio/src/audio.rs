//! Sound for the whole client: the engine behind a lock the game, frame and loader
//! threads share (the mixer never takes it), the listener every frame writes, and the
//! output thread.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread::JoinHandle;
use std::time::Instant;

use crate::engine::{Engine, EntityState, InstanceId, PlayResult, SoundInstance};
use crate::files::SoundFiles;
use crate::output::Output;
use crate::source::{SoundSource, Volumes};
use crate::spatial::{Listener, ListenerCell};

/// The game's handle on the engine; does nothing when sound is off.
#[derive(Clone, Default)]
pub struct SoundHandle {
    engine: Option<Arc<Mutex<Engine>>>,
}

impl std::fmt::Debug for SoundHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SoundHandle")
            .field("on", &self.engine.is_some())
            .finish()
    }
}

impl SoundHandle {
    /// A handle that plays nothing.
    pub fn off() -> SoundHandle {
        SoundHandle::default()
    }

    pub fn is_on(&self) -> bool {
        self.engine.is_some()
    }

    /// The engine, locked: for several calls at once.
    pub fn lock(&self) -> Option<MutexGuard<'_, Engine>> {
        self.engine
            .as_ref()
            .map(|e| e.lock().unwrap_or_else(PoisonError::into_inner))
    }

    pub fn play(&self, sound: SoundInstance) -> PlayResult {
        let started = crate::loader::trace().then(Instant::now);
        let event = started.is_some().then(|| sound.event.to_string());
        let result = self
            .lock()
            .map_or(PlayResult::NotStarted, |mut e| e.play(sound));
        if let (Some(t), Some(event)) = (started, event) {
            eprintln!(
                "[sound] play {event} on {:?}: {:?} in {:.1} us",
                std::thread::current().name().unwrap_or("?"),
                result,
                t.elapsed().as_secs_f64() * 1e6
            );
        }
        result
    }

    pub fn play_delayed(&self, sound: SoundInstance, delay: i32) -> Option<InstanceId> {
        self.lock().map(|mut e| e.play_delayed(sound, delay))
    }

    pub fn stop(&self, id: InstanceId) {
        if let Some(mut e) = self.lock() {
            e.stop(id);
        }
    }

    pub fn stop_matching(&self, event: Option<&str>, source: Option<SoundSource>) {
        if let Some(mut e) = self.lock() {
            e.stop_matching(event, source);
        }
    }

    pub fn stop_all(&self) {
        if let Some(mut e) = self.lock() {
            e.stop_all();
        }
    }

    pub fn tick(&self, paused: bool, entity: &dyn Fn(i32) -> Option<EntityState>) {
        if let Some(mut e) = self.lock() {
            e.tick(paused, entity);
        }
    }

    pub fn set_volumes(&self, volumes: Volumes) {
        if let Some(mut e) = self.lock() {
            e.set_volumes(volumes);
        }
    }

    pub fn is_active(&self, id: InstanceId) -> bool {
        self.lock().is_some_and(|e| e.is_active(id))
    }

    /// Where the listener is, for sounds delayed by their distance.
    pub fn listener_position(&self) -> Option<[f32; 3]> {
        self.lock().map(|e| e.listener_position())
    }

    /// Takes the sounds of `files` in place of the ones loaded (a resource reload with
    /// other packs): their events are read here, on the caller's thread, a new loader
    /// fetches their samples, and every sound playing stops, as vanilla's sound engine
    /// does on a reload. Nothing changes without sound.
    pub fn replace_files(&self, files: SoundFiles) {
        let Some(engine) = &self.engine else { return };
        load_sounds(
            Arc::new(files),
            &Arc::downgrade(engine),
            Instant::now(),
            false,
        );
    }
}

/// Reads the events of `files` and hands them to the engine with a loader for them.
fn load_sounds(
    files: Arc<SoundFiles>,
    engine: &std::sync::Weak<Mutex<Engine>>,
    started: Instant,
    initial: bool,
) {
    let (events, warnings) = files.events();
    if files.cancelled() {
        // A reload given up: its events (none, or not all of them) never replace the
        // sounds in use.
        eprintln!("[sound] the load was cancelled: the sounds in use stay");
        return;
    }
    for w in warnings.iter().take(20) {
        eprintln!("[sound] {w}");
    }
    if warnings.len() > 20 {
        eprintln!("[sound] ... and {} more warnings", warnings.len() - 20);
    }
    eprintln!(
        "[sound] {} events, {} files in {:.0} ms",
        events.len(),
        files.files(),
        started.elapsed().as_secs_f64() * 1e3
    );
    let Some(strong) = engine.upgrade() else {
        return;
    };
    let mut guard = strong.lock().unwrap_or_else(PoisonError::into_inner);
    if initial && guard.generation() > 0 {
        // A session resource reload already installed newer sounds; don't overwrite them.
        return;
    }
    let next_gen = guard.generation().wrapping_add(1);
    let Ok(loader) = crate::loader::spawn(files, engine.clone(), next_gen) else {
        eprintln!("[sound] couldn't start the loader: no sounds");
        return;
    };
    let actual_gen = guard.set_sounds(Arc::new(events), Box::new(loader));
    debug_assert_eq!(actual_gen, next_gen);
}

/// Sound for the client: see the module docs. Dropping it closes the output.
pub struct Audio {
    handle: SoundHandle,
    listener: Arc<ListenerCell>,
    stop: Arc<AtomicBool>,
    output: Option<JoinHandle<()>>,
}

impl std::fmt::Debug for Audio {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Audio")
            .field("handle", &self.handle)
            .finish()
    }
}

impl Audio {
    /// No sound at all: no device, no decoding, no threads.
    pub fn off() -> Audio {
        Audio {
            handle: SoundHandle::off(),
            listener: Arc::default(),
            stop: Arc::default(),
            output: None,
        }
    }

    /// Starts the output, and loads the sounds of `files` (called on a thread of its
    /// own, as it lists and parses the packs).
    pub fn start(
        output: Output,
        volumes: Volumes,
        files: impl FnOnce() -> Result<SoundFiles, String> + Send + 'static,
    ) -> Audio {
        let listener = Arc::new(ListenerCell::default());
        let mut engine = Engine::new(listener.clone());
        engine.set_volumes(volumes);
        let engine = Arc::new(Mutex::new(engine));
        let weak = Arc::downgrade(&engine);
        let loading = std::thread::Builder::new()
            .name("sound-load".into())
            .spawn(move || {
                let started = Instant::now();
                let files = match files() {
                    Ok(f) => Arc::new(f),
                    Err(e) => {
                        eprintln!("[sound] no sounds: {e}");
                        return;
                    }
                };
                load_sounds(files, &weak, started, true);
            });
        if let Err(e) = loading {
            eprintln!("[sound] couldn't start loading sounds: {e}");
        }
        let stop = Arc::new(AtomicBool::new(false));
        let output = crate::output::spawn(output, engine.clone(), listener.clone(), stop.clone())
            .map_err(|e| eprintln!("[sound] couldn't start the output: {e}"))
            .ok();
        Audio {
            handle: SoundHandle {
                engine: Some(engine),
            },
            listener,
            stop,
            output,
        }
    }

    pub fn handle(&self) -> SoundHandle {
        self.handle.clone()
    }

    /// Where sounds are heard from: the camera, every frame (`updateSource`).
    pub fn set_listener(&self, listener: &Listener) {
        if self.handle.is_on() {
            self.listener.store(listener);
        }
    }
}

impl Drop for Audio {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(t) = self.output.take() {
            let _ = t.join();
        }
    }
}

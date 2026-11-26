//! The game's side of sound: vanilla's `SoundEngine` and `ChannelAccess` over the
//! mixer's voices.
//!
//! A play resolves its event to one sound with the instance's random, works out the
//! final volume (`clamp(volume) * clamp(category * master)`, skipping a silent one
//! unless it may start silent or is music), takes a voice from the static or the
//! streaming pool (none left: the sound doesn't play, as in vanilla) and starts it as
//! soon as its samples are there: at once from the cache, else when the decoding thread
//! has them. Nothing here reads files or decodes.
//!
//! Ticks follow `tickInGameSound`: the tick count goes on, sounds that follow an entity
//! (`EntityBoundSoundInstance`) move with it or stop when it is gone, a stopped voice's
//! instance is forgotten once it is 20 ticks old, and delayed sounds start when due.

use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use ruthenium_assets::Id;
use ruthenium_javamath::random::JavaRandom;

use crate::cache::{DEFAULT_BUDGET, PcmCache};
use crate::decode::Pcm;
use crate::events::{Choice, Chosen, INTENTIONALLY_EMPTY, SoundEvents, id_key};
use crate::mixer::{Command, Feed, MixerLink, STATIC_VOICES, StreamFeed, VOICES, VoiceParams};
use crate::source::{SoundSource, Volumes};
use crate::spatial::{ListenerCell, Placement};

/// `SoundEngine.MIN_SOURCE_LIFETIME`.
const MIN_SOURCE_LIFETIME: i32 = 20;

/// What a sound follows as it plays.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Follow {
    /// Stays where it started.
    None,
    /// An entity, by id (`EntityBoundSoundInstance`).
    Entity(i32),
    /// Whoever played it moves it with [`Engine::update`] each tick.
    Caller,
}

/// One sound to play (`SoundInstance`).
#[derive(Debug, Clone, PartialEq)]
pub struct SoundInstance {
    pub event: Cow<'static, str>,
    pub source: SoundSource,
    pub volume: f32,
    pub pitch: f32,
    /// `RandomSource.create(seed)`: picks the event's sound.
    pub seed: i64,
    pub position: [f64; 3],
    pub looping: bool,
    /// Ticks between plays of a sound that loops by playing again.
    pub delay: i32,
    /// `Attenuation.LINEAR` (else none).
    pub linear: bool,
    /// Placed relative to the listener (UI, music).
    pub relative: bool,
    pub follow: Follow,
    pub can_start_silent: bool,
}

impl SoundInstance {
    /// `SimpleSoundInstance(event, source, volume, pitch, random, x, y, z)`.
    pub fn at(
        event: impl Into<Cow<'static, str>>,
        source: SoundSource,
        volume: f32,
        pitch: f32,
        seed: i64,
        position: [f64; 3],
    ) -> SoundInstance {
        SoundInstance {
            event: event.into(),
            source,
            volume,
            pitch,
            seed,
            position,
            looping: false,
            delay: 0,
            linear: true,
            relative: false,
            follow: Follow::None,
            can_start_silent: false,
        }
    }

    /// `SimpleSoundInstance.forUI`: at the listener, no distance.
    pub fn ui(
        event: impl Into<Cow<'static, str>>,
        pitch: f32,
        volume: f32,
        seed: i64,
    ) -> SoundInstance {
        SoundInstance {
            linear: false,
            relative: true,
            ..SoundInstance::at(event, SoundSource::Ui, volume, pitch, seed, [0.0; 3])
        }
    }

    /// `EntityBoundSoundInstance`: starts where the entity is (as floats) and follows it.
    pub fn entity(
        event: impl Into<Cow<'static, str>>,
        source: SoundSource,
        volume: f32,
        pitch: f32,
        seed: i64,
        id: i32,
        position: [f64; 3],
    ) -> SoundInstance {
        SoundInstance {
            follow: Follow::Entity(id),
            ..SoundInstance::at(
                event,
                source,
                volume,
                pitch,
                seed,
                position.map(|v| f64::from(v as f32)),
            )
        }
    }
}

/// What a play came to (`SoundEngine.PlayResult`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlayResult {
    Started(InstanceId),
    StartedSilently(InstanceId),
    NotStarted,
}

impl PlayResult {
    pub fn id(self) -> Option<InstanceId> {
        match self {
            PlayResult::Started(id) | PlayResult::StartedSilently(id) => Some(id),
            PlayResult::NotStarted => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct InstanceId(u64);

/// What an entity is to a sound that follows it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EntityState {
    pub position: [f64; 3],
    pub silent: bool,
}

/// Asks for samples off the engine's thread; the answers come back as
/// [`Engine::decoded`] and [`Engine::stream_opened`].
pub trait Fetch: Send {
    fn decode(&mut self, file: &Id);
    fn stream(&mut self, slot: u16, file: &Id, looping: bool);
}

#[derive(Debug)]
struct Instance {
    sound: SoundInstance,
    random: JavaRandom,
    /// The sound the last play chose.
    chosen: Option<Chosen>,
    /// The voice, while it holds one (`instanceToChannel`).
    slot: Option<u16>,
    /// The voice has stopped (`ChannelHandle.isStopped`).
    released: bool,
    delete_time: i32,
    /// A followed entity is gone or the caller stopped it (`stop()` of a tickable).
    stopped: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Slot {
    Free,
    /// Waiting for its samples.
    Waiting(InstanceId),
    Playing(InstanceId),
}

/// See the module docs.
pub struct Engine {
    events: Arc<SoundEvents>,
    link: Option<MixerLink>,
    fetch: Option<Box<dyn Fetch>>,
    volumes: Volumes,
    tick_count: i32,
    next_id: u64,
    instances: HashMap<InstanceId, Instance>,
    slots: Box<[Slot]>,
    /// Voice parameters waiting for their samples, by slot.
    waiting_params: Box<[Option<(VoiceParams, bool)>]>,
    ticking: Vec<InstanceId>,
    queued: Vec<(InstanceId, i32)>,
    cache: PcmCache,
    /// Files being decoded, and the slots waiting for each.
    decoding: HashMap<String, Vec<u16>>,
    listener: Arc<ListenerCell>,
    warned: HashSet<String>,
    /// Plays dropped because no output was open.
    pub dropped: u64,
}

impl std::fmt::Debug for Engine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Engine")
            .field("events", &self.events.len())
            .field("loaded", &self.link.is_some())
            .field("instances", &self.instances.len())
            .finish_non_exhaustive()
    }
}

impl Engine {
    pub fn new(listener: Arc<ListenerCell>) -> Engine {
        Engine {
            events: Arc::default(),
            link: None,
            fetch: None,
            volumes: Volumes::default(),
            tick_count: 0,
            next_id: 0,
            instances: HashMap::new(),
            slots: vec![Slot::Free; VOICES].into_boxed_slice(),
            waiting_params: vec![None; VOICES].into_boxed_slice(),
            ticking: Vec::new(),
            queued: Vec::new(),
            cache: PcmCache::new(DEFAULT_BUDGET),
            decoding: HashMap::new(),
            listener,
            warned: HashSet::new(),
            dropped: 0,
        }
    }

    /// The events of the packs, and what fetches their samples.
    pub fn set_sounds(&mut self, events: Arc<SoundEvents>, fetch: Box<dyn Fetch>) {
        self.stop_all();
        self.cache.clear();
        self.decoding.clear();
        self.events = events;
        self.fetch = Some(fetch);
    }

    pub fn events(&self) -> &SoundEvents {
        &self.events
    }

    /// A mixer's rings: the output is open. What played before is stopped (vanilla
    /// reloads its sound engine for a new device).
    pub fn attach(&mut self, link: MixerLink) {
        self.detach();
        self.link = Some(link);
    }

    /// The output closed: every sound stops, and its voices go with its mixer.
    pub fn detach(&mut self) {
        self.stop_all();
        self.link = None;
        self.slots.fill(Slot::Free);
        self.waiting_params.fill(None);
    }

    /// Sounds can play (`loaded`).
    pub fn is_loaded(&self) -> bool {
        self.link.is_some() && self.fetch.is_some()
    }

    pub fn volumes(&self) -> Volumes {
        self.volumes
    }

    /// A slider moved: the playing sounds of that category (all of them for master) take
    /// the new volume at once (`refreshCategoryVolume`).
    pub fn set_volume(&mut self, source: SoundSource, value: f32) {
        self.volumes.set(source, value.clamp(0.0, 1.0));
        self.pump();
        let ids: Vec<InstanceId> = self.instances.keys().copied().collect();
        for id in ids {
            let i = &self.instances[&id];
            if source == SoundSource::Master || i.sound.source == source {
                self.refresh(id);
            }
        }
    }

    pub fn set_volumes(&mut self, volumes: Volumes) {
        for s in SoundSource::ALL {
            if volumes.get(s) != self.volumes.get(s) {
                self.set_volume(s, volumes.get(s));
            }
        }
    }

    /// Voices in use, static and streaming (`getChannelDebugString`).
    pub fn voices_in_use(&self) -> (usize, usize) {
        let used =
            |r: std::ops::Range<usize>| self.slots[r].iter().filter(|s| **s != Slot::Free).count();
        (used(0..STATIC_VOICES), used(STATIC_VOICES..VOICES))
    }

    pub fn tick_count(&self) -> i32 {
        self.tick_count
    }

    pub fn cache(&self) -> &PcmCache {
        &self.cache
    }

    /// `SoundEngine.play`.
    pub fn play(&mut self, sound: SoundInstance) -> PlayResult {
        self.pump();
        if !self.is_loaded() {
            self.dropped += 1;
            return PlayResult::NotStarted;
        }
        let id = self.new_instance(sound);
        let result = self.start(id);
        if result == PlayResult::NotStarted {
            self.instances.remove(&id);
        }
        result
    }

    /// `SoundEngine.playDelayed`: plays `delay` ticks from now.
    pub fn play_delayed(&mut self, sound: SoundInstance, delay: i32) -> InstanceId {
        let id = self.new_instance(sound);
        self.queued.push((id, self.tick_count + delay));
        id
    }

    fn new_instance(&mut self, sound: SoundInstance) -> InstanceId {
        self.next_id += 1;
        let id = InstanceId(self.next_id);
        self.instances.insert(
            id,
            Instance {
                random: JavaRandom::new(sound.seed),
                sound,
                chosen: None,
                slot: None,
                released: false,
                delete_time: 0,
                stopped: false,
            },
        );
        id
    }

    /// `clamp(volume) * clamp(final category volume)`.
    fn final_volume(&self, volume: f32, source: SoundSource) -> f32 {
        volume.clamp(0.0, 1.0) * self.volumes.final_volume(source).clamp(0.0, 1.0)
    }

    /// The instance's volume with its sound's (`getVolume`), and pitch (`getPitch`).
    fn volume_and_pitch(i: &Instance) -> (f32, f32) {
        let c = i.chosen.as_ref();
        (
            i.sound.volume * c.map_or(1.0, |c| c.volume),
            i.sound.pitch * c.map_or(1.0, |c| c.pitch),
        )
    }

    fn start(&mut self, id: InstanceId) -> PlayResult {
        if !self.is_loaded() {
            return PlayResult::NotStarted;
        }
        let events = self.events.clone();
        let Some(i) = self.instances.get_mut(&id) else {
            return PlayResult::NotStarted;
        };
        if i.sound.event == INTENTIONALLY_EMPTY {
            return PlayResult::NotStarted;
        }
        let chosen = match events.choose(&i.sound.event, &mut i.random) {
            Choice::Sound(c) => c,
            Choice::Unknown => {
                let event = i.sound.event.to_string();
                if self.warned.insert(event.clone()) {
                    eprintln!("[sound] Unable to play unknown soundEvent: {event}");
                }
                return PlayResult::NotStarted;
            }
            Choice::Empty => {
                let event = i.sound.event.to_string();
                if self.warned.insert(event.clone()) {
                    eprintln!("[sound] Unable to play empty soundEvent: {event}");
                }
                return PlayResult::NotStarted;
            }
        };
        i.chosen = Some(chosen);
        let (volume, pitch) = Engine::volume_and_pitch(i);
        let chosen = i.chosen.clone().expect("just set");
        let attenuation = chosen.attenuation(volume);
        let source = i.sound.source;
        let final_volume = self.final_volume(volume, source);
        let i = &self.instances[&id];
        let silently = final_volume == 0.0;
        if silently && !i.sound.can_start_silent && source != SoundSource::Music {
            return PlayResult::NotStarted;
        }
        let Some(slot) = self.free_slot(chosen.stream) else {
            return PlayResult::NotStarted;
        };
        let params = VoiceParams {
            gain: final_volume,
            pitch: pitch.clamp(0.5, 2.0),
            place: Placement {
                position: i.sound.position.map(|v| v as f32),
                relative: i.sound.relative,
                max_distance: i.sound.linear.then_some(attenuation),
            },
        };
        // `shouldLoopAutomatically`: looping with no delay; a stream loops itself.
        let looping = i.sound.looping && i.sound.delay <= 0;
        let tickable = i.sound.follow != Follow::None;
        let i = self.instances.get_mut(&id).expect("present");
        i.slot = Some(slot);
        i.released = false;
        i.delete_time = self.tick_count + MIN_SOURCE_LIFETIME;
        self.slots[usize::from(slot)] = Slot::Waiting(id);
        let file = chosen.file_id();
        if chosen.stream {
            self.waiting_params[usize::from(slot)] = Some((params, false));
            if let Some(f) = self.fetch.as_mut() {
                f.stream(slot, &file, looping);
            }
        } else {
            self.waiting_params[usize::from(slot)] = Some((params, looping));
            let key = id_key(&file);
            match self.cache.get(&key) {
                Some(pcm) => self.begin(slot, Feed::Static(pcm)),
                None => {
                    let first = !self.decoding.contains_key(&key);
                    self.decoding.entry(key).or_default().push(slot);
                    if first && let Some(f) = self.fetch.as_mut() {
                        f.decode(&file);
                    }
                }
            }
        }
        if tickable && !self.ticking.contains(&id) {
            self.ticking.push(id);
        }
        if silently {
            PlayResult::StartedSilently(id)
        } else {
            PlayResult::Started(id)
        }
    }

    /// A free voice of the pool (`CountingChannelPool.acquire`).
    fn free_slot(&self, stream: bool) -> Option<u16> {
        let range = if stream {
            STATIC_VOICES..VOICES
        } else {
            0..STATIC_VOICES
        };
        range
            .into_iter()
            .find(|&s| self.slots[s] == Slot::Free)
            .map(|s| s as u16)
    }

    /// Starts a waiting voice with its samples.
    fn begin(&mut self, slot: u16, feed: Feed) {
        let s = usize::from(slot);
        let (Slot::Waiting(id), Some((params, looping))) =
            (self.slots[s], self.waiting_params[s].take())
        else {
            return;
        };
        let Some(link) = self.link.as_mut() else {
            return;
        };
        match link.commands.push(Command::Play {
            slot,
            feed,
            params,
            looping,
        }) {
            Ok(()) => self.slots[s] = Slot::Playing(id),
            Err(_) => self.release(slot),
        }
    }

    /// A file's samples arrived (or couldn't be had).
    pub fn decoded(&mut self, file: &Id, pcm: Result<Arc<Pcm>, String>) {
        self.pump();
        let key = id_key(file);
        let slots = self.decoding.remove(&key).unwrap_or_default();
        match pcm {
            Ok(pcm) => {
                self.cache.insert(key, pcm.clone());
                for slot in slots {
                    self.begin(slot, Feed::Static(pcm.clone()));
                }
            }
            Err(e) => {
                if self.warned.insert(key.clone()) {
                    eprintln!("[sound] {key}: {e}");
                }
                for slot in slots {
                    if matches!(self.slots[usize::from(slot)], Slot::Waiting(_)) {
                        self.release(slot);
                    }
                }
            }
        }
    }

    /// A stream is ready to play in `slot` (or couldn't be opened).
    pub fn stream_opened(&mut self, slot: u16, feed: Result<StreamFeed, String>) {
        self.pump();
        match feed {
            Ok(feed) => self.begin(slot, Feed::Stream(Box::new(feed))),
            Err(e) => {
                eprintln!("[sound] stream: {e}");
                if matches!(self.slots.get(usize::from(slot)), Some(Slot::Waiting(_))) {
                    self.release(slot);
                }
            }
        }
    }

    /// The voice is free again (`ChannelHandle.release`).
    fn release(&mut self, slot: u16) {
        let s = usize::from(slot);
        if let Slot::Waiting(id) | Slot::Playing(id) = self.slots[s]
            && let Some(i) = self.instances.get_mut(&id)
            && i.slot == Some(slot)
        {
            i.released = true;
        }
        self.slots[s] = Slot::Free;
        self.waiting_params[s] = None;
    }

    /// Takes back the voices the mixer finished (`scheduleTick`'s release of stopped
    /// channels); their samples are dropped here, never in the mixer.
    pub fn pump(&mut self) {
        let mut done: Vec<u16> = Vec::new();
        if let Some(link) = self.link.as_mut() {
            while let Ok(r) = link.retired.pop() {
                done.push(r.slot);
                drop(r.feed);
            }
        }
        for slot in done {
            if matches!(self.slots.get(usize::from(slot)), Some(Slot::Playing(_))) {
                self.release(slot);
            }
        }
    }

    /// `SoundEngine.stop(instance)`.
    pub fn stop(&mut self, id: InstanceId) {
        self.pump();
        let Some(slot) = self.instances.get(&id).and_then(|i| i.slot) else {
            return;
        };
        match self.slots[usize::from(slot)] {
            Slot::Playing(p) if p == id => {
                if let Some(link) = self.link.as_mut() {
                    let _ = link.commands.push(Command::Stop { slot });
                }
            }
            // Its samples never came: nothing to stop in the mixer.
            Slot::Waiting(w) if w == id => self.release(slot),
            _ => {}
        }
    }

    /// `SoundEngine.stop(sound, source)`: by event and category; neither stops all.
    pub fn stop_matching(&mut self, event: Option<&str>, source: Option<SoundSource>) {
        if event.is_none() && source.is_none() {
            self.stop_all();
            return;
        }
        let ids: Vec<InstanceId> = self
            .instances
            .iter()
            .filter(|(_, i)| i.slot.is_some() && !i.released)
            .filter(|(_, i)| source.is_none_or(|s| i.sound.source == s))
            .filter(|(_, i)| event.is_none_or(|e| i.sound.event == e))
            .map(|(id, _)| *id)
            .collect();
        for id in ids {
            self.stop(id);
        }
    }

    /// `stopAll`: every voice stops and every instance is forgotten.
    pub fn stop_all(&mut self) {
        self.pump();
        for slot in 0..VOICES {
            match self.slots[slot] {
                Slot::Playing(_) => {
                    if let Some(link) = self.link.as_mut() {
                        let _ = link.commands.push(Command::Stop { slot: slot as u16 });
                    }
                    // Freed when the mixer hands it back; forgotten now.
                }
                Slot::Waiting(_) => self.release(slot as u16),
                Slot::Free => {}
            }
        }
        self.instances.clear();
        self.ticking.clear();
        self.queued.clear();
    }

    /// Whether a sound is still playing or about to (`isActive`).
    pub fn is_active(&self, id: InstanceId) -> bool {
        self.instances.get(&id).is_some_and(|i| i.slot.is_some())
            || self.queued.iter().any(|(q, _)| *q == id)
    }

    /// Moves a [`Follow::Caller`] sound: its volume, pitch and place for this tick.
    pub fn update(&mut self, id: InstanceId, volume: f32, pitch: f32, position: [f64; 3]) {
        if let Some(i) = self.instances.get_mut(&id) {
            i.sound.volume = volume;
            i.sound.pitch = pitch;
            i.sound.position = position;
        }
    }

    /// Stops a [`Follow::Caller`] sound at the next tick (`AbstractTickableSoundInstance.stop`).
    pub fn finish(&mut self, id: InstanceId) {
        if let Some(i) = self.instances.get_mut(&id) {
            i.stopped = true;
            i.sound.looping = false;
        }
    }

    /// Sends an instance's current volume, pitch and place to its voice.
    fn refresh(&mut self, id: InstanceId) {
        let Some(i) = self.instances.get(&id) else {
            return;
        };
        let Some(slot) = i.slot else { return };
        let (volume, pitch) = Engine::volume_and_pitch(i);
        let gain = self.final_volume(volume, i.sound.source);
        let s = usize::from(slot);
        let update = |p: VoiceParams| VoiceParams {
            gain,
            pitch: pitch.clamp(0.5, 2.0),
            place: Placement {
                position: i.sound.position.map(|v| v as f32),
                ..p.place
            },
        };
        match self.slots[s] {
            Slot::Waiting(w) if w == id => {
                if let Some((p, _)) = self.waiting_params[s].as_mut() {
                    *p = update(*p);
                }
            }
            Slot::Playing(p) if p == id => {
                let Some(link) = self.link.as_mut() else {
                    return;
                };
                // The placement's range and relativity don't change once playing.
                let i = &self.instances[&id];
                let params = VoiceParams {
                    gain,
                    pitch: pitch.clamp(0.5, 2.0),
                    place: Placement {
                        position: i.sound.position.map(|v| v as f32),
                        relative: i.sound.relative,
                        max_distance: if i.sound.linear {
                            i.chosen.as_ref().map(|c| c.attenuation(volume))
                        } else {
                            None
                        },
                    },
                };
                let _ = link.commands.push(Command::Set { slot, params });
            }
            _ => {}
        }
    }

    /// `SoundEngine.tick`: `entity` says where a followed entity is, `None` once it
    /// is gone.
    pub fn tick(&mut self, paused: bool, entity: &dyn Fn(i32) -> Option<EntityState>) {
        self.pump();
        if paused {
            // `tickMusicWhenPaused`: finished music is forgotten.
            self.instances
                .retain(|_, i| !(i.sound.source == SoundSource::Music && i.released));
            return;
        }
        self.tick_count += 1;
        for id in self.ticking.clone() {
            let Some(i) = self.instances.get_mut(&id) else {
                continue;
            };
            let mut stop = false;
            if let Follow::Entity(e) = i.sound.follow {
                match entity(e) {
                    Some(state) => {
                        // `canPlaySound`: a silent entity's sound stops.
                        stop |= state.silent;
                        i.sound.position = state.position.map(|v| f64::from(v as f32));
                        if crate::loader::trace() {
                            eprintln!(
                                "[sound] {} follows entity {e} to {:?}",
                                i.sound.event, i.sound.position
                            );
                        }
                    }
                    None => {
                        i.stopped = true;
                        i.sound.looping = false;
                    }
                }
            }
            if stop || i.stopped {
                self.stop(id);
            } else {
                self.refresh(id);
            }
        }
        // Forget what has stopped and is old enough; a sound that loops by playing again
        // is queued to.
        let tick = self.tick_count;
        let done: Vec<InstanceId> = self
            .instances
            .iter()
            .filter(|(_, i)| i.slot.is_some() && i.released && i.delete_time <= tick)
            .map(|(id, _)| *id)
            .collect();
        for id in done {
            let i = self.instances.get_mut(&id).expect("present");
            i.slot = None;
            if i.sound.looping && i.sound.delay > 0 {
                let delay = i.sound.delay;
                self.queued.push((id, tick + delay));
            } else {
                self.instances.remove(&id);
            }
            self.ticking.retain(|t| *t != id);
        }
        let (due, later): (Vec<_>, Vec<_>) = self.queued.drain(..).partition(|(_, at)| tick >= *at);
        self.queued = later;
        for (id, _) in due {
            if let Some(Follow::Entity(e)) = self.instances.get(&id).map(|i| i.sound.follow)
                && let Some(state) = entity(e)
                && let Some(i) = self.instances.get_mut(&id)
            {
                i.sound.position = state.position.map(|v| f64::from(v as f32));
            }
            if self.start(id) == PlayResult::NotStarted {
                self.instances.remove(&id);
                self.ticking.retain(|t| *t != id);
            }
        }
    }

    /// Where the listener is now, for sounds delayed by their distance.
    pub fn listener_position(&self) -> [f32; 3] {
        self.listener.load().map_or([0.0; 3], |l| l.position)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::SoundsJson;
    use crate::mixer::Mixer;
    use std::sync::Mutex;

    #[derive(Default)]
    struct Asked {
        decodes: Vec<String>,
        streams: Vec<(u16, String, bool)>,
    }

    struct FakeFetch(Arc<Mutex<Asked>>);

    impl Fetch for FakeFetch {
        fn decode(&mut self, file: &Id) {
            self.0.lock().unwrap().decodes.push(id_key(file));
        }
        fn stream(&mut self, slot: u16, file: &Id, looping: bool) {
            self.0
                .lock()
                .unwrap()
                .streams
                .push((slot, id_key(file), looping));
        }
    }

    const SOUNDS: &str = r#"{
        "hit": {"sounds": [{"name": "hit1", "volume": 0.5, "pitch": 1.5}]},
        "big": {"sounds": [{"name": "big", "attenuation_distance": 8}]},
        "song": {"sounds": [{"name": "song", "stream": true}]},
        "empty": {"sounds": []}
    }"#;

    struct Rig {
        engine: Engine,
        mixer: Mixer,
        asked: Arc<Mutex<Asked>>,
    }

    fn rig() -> Rig {
        let listener = Arc::new(ListenerCell::default());
        let mut engine = Engine::new(listener.clone());
        let (events, w) = SoundEvents::load(
            [SoundsJson {
                namespace: "minecraft",
                pack: "test",
                bytes: SOUNDS.as_bytes(),
                limits: ruthenium_assets::Limits::trusted(),
            }],
            |_| true,
        );
        assert!(w.is_empty());
        let asked = Arc::new(Mutex::new(Asked::default()));
        engine.set_sounds(Arc::new(events), Box::new(FakeFetch(asked.clone())));
        let (mixer, link) = Mixer::new(48_000, listener);
        engine.attach(link);
        Rig {
            engine,
            mixer,
            asked,
        }
    }

    fn pcm(frames: usize) -> Arc<Pcm> {
        Arc::new(Pcm {
            channels: 1,
            rate: 48_000,
            samples: vec![16384; frames].into_boxed_slice(),
        })
    }

    fn hit(at: [f64; 3]) -> SoundInstance {
        SoundInstance::at("minecraft:hit", SoundSource::Players, 1.0, 1.0, 7, at)
    }

    fn render(m: &mut Mixer, frames: usize) -> Vec<f32> {
        let mut out = vec![0.0; frames * 2];
        m.render(&mut out);
        out
    }

    const NONE: &dyn Fn(i32) -> Option<EntityState> = &|_| None;

    #[test]
    fn a_first_play_waits_for_its_samples_then_the_cache_serves() {
        let mut r = rig();
        let id = r.engine.play(hit([0.0, 0.0, -2.0])).id().unwrap();
        assert_eq!(
            r.asked.lock().unwrap().decodes,
            ["minecraft:sounds/hit1.ogg"]
        );
        assert_eq!(r.engine.voices_in_use(), (1, 0));
        assert!(render(&mut r.mixer, 64).iter().all(|s| *s == 0.0));
        let file = Id::minecraft("sounds/hit1.ogg").unwrap();
        r.engine.decoded(&file, Ok(pcm(48_000)));
        // Volume 0.5 (the sound's), pitch 1.5, straight ahead.
        let out = render(&mut r.mixer, 256);
        let front = 0.5 + 0.095_662_1;
        // Two blocks away of 16.
        assert!(
            (out[200] - 0.5 * 0.5 * 0.875 * front).abs() < 1e-4,
            "{}",
            out[200]
        );
        assert!(r.engine.is_active(id));
        // A second play is served from the cache: no new decode.
        r.engine.play(hit([0.0; 3]));
        assert_eq!(r.asked.lock().unwrap().decodes.len(), 1);
        assert_eq!(r.engine.voices_in_use(), (2, 0));
    }

    #[test]
    fn volumes_are_clamped_and_multiplied() {
        let mut r = rig();
        r.engine.set_volume(SoundSource::Master, 0.5);
        r.engine.set_volume(SoundSource::Players, 0.8);
        let mut loud = hit([0.0, 0.0, -1.0]);
        loud.volume = 3.0; // 3 * 0.5 = 1.5, clamped to 1, and a range of 1.5 * 16.
        r.engine
            .decoded(&Id::minecraft("sounds/hit1.ogg").unwrap(), Ok(pcm(48_000)));
        r.engine.play(loud);
        let out = render(&mut r.mixer, 256);
        let front = 0.5 + 0.095_662_1;
        // 1 (clamped) * 0.4 (category * master); 1 block away of 24.
        let want = 0.5 * 0.4 * (1.0 - 1.0 / 24.0) * front;
        assert!((out[200] - want).abs() < 1e-4, "{} {want}", out[200]);
        // The slider moves a playing sound at once.
        r.engine.set_volume(SoundSource::Players, 0.2);
        let out = render(&mut r.mixer, 256);
        assert!((out[200] - want / 4.0).abs() < 1e-4);
        // Master moves every category.
        r.engine.set_volume(SoundSource::Master, 0.25);
        let out = render(&mut r.mixer, 256);
        assert!((out[200] - want / 8.0).abs() < 1e-4);
    }

    #[test]
    fn silent_and_unknown_sounds_do_not_start() {
        let mut r = rig();
        r.engine.set_volume(SoundSource::Players, 0.0);
        assert_eq!(r.engine.play(hit([0.0; 3])), PlayResult::NotStarted);
        let mut silent = hit([0.0; 3]);
        silent.can_start_silent = true;
        assert!(matches!(
            r.engine.play(silent),
            PlayResult::StartedSilently(_)
        ));
        let mut music = hit([0.0; 3]);
        music.source = SoundSource::Music;
        r.engine.set_volume(SoundSource::Music, 0.0);
        assert!(matches!(
            r.engine.play(music),
            PlayResult::StartedSilently(_)
        ));
        for event in ["minecraft:nope", "minecraft:empty", INTENTIONALLY_EMPTY] {
            let mut s = hit([0.0; 3]);
            s.event = event.into();
            assert_eq!(r.engine.play(s), PlayResult::NotStarted, "{event}");
        }
    }

    #[test]
    fn the_pools_hold_vanillas_numbers() {
        let mut r = rig();
        for n in 0..STATIC_VOICES {
            assert!(r.engine.play(hit([0.0; 3])).id().is_some(), "{n}");
        }
        assert_eq!(r.engine.play(hit([0.0; 3])), PlayResult::NotStarted);
        let song = || SoundInstance::ui("minecraft:song", 1.0, 1.0, 0);
        for _ in 0..8 {
            assert!(r.engine.play(song()).id().is_some());
        }
        assert_eq!(r.engine.play(song()), PlayResult::NotStarted);
        assert_eq!(r.engine.voices_in_use(), (247, 8));
        assert_eq!(r.asked.lock().unwrap().streams.len(), 8);
    }

    #[test]
    fn stopping_frees_the_voice_and_the_instance_goes_after_20_ticks() {
        let mut r = rig();
        r.engine
            .decoded(&Id::minecraft("sounds/hit1.ogg").unwrap(), Ok(pcm(480_000)));
        let id = r.engine.play(hit([0.0; 3])).id().unwrap();
        render(&mut r.mixer, 64);
        r.engine.stop(id);
        render(&mut r.mixer, 256);
        r.engine.pump();
        assert_eq!(r.engine.voices_in_use(), (0, 0));
        for _ in 0..19 {
            r.engine.tick(false, NONE);
            assert!(r.engine.is_active(id));
        }
        r.engine.tick(false, NONE);
        assert!(!r.engine.is_active(id));
        // A sound waiting for its samples just lets its voice go.
        let waiting = r
            .engine
            .play(SoundInstance::ui("minecraft:big", 1.0, 1.0, 0));
        assert_eq!(r.engine.voices_in_use(), (1, 0));
        r.engine.stop(waiting.id().unwrap());
        assert_eq!(r.engine.voices_in_use(), (0, 0));
        r.engine
            .decoded(&Id::minecraft("sounds/big.ogg").unwrap(), Ok(pcm(100)));
        assert_eq!(r.mixer.playing(), 0);
        render(&mut r.mixer, 64);
        assert_eq!(r.mixer.playing(), 0);
    }

    #[test]
    fn stop_by_event_and_category() {
        let mut r = rig();
        r.engine
            .decoded(&Id::minecraft("sounds/hit1.ogg").unwrap(), Ok(pcm(480_000)));
        r.engine
            .decoded(&Id::minecraft("sounds/big.ogg").unwrap(), Ok(pcm(480_000)));
        r.engine.play(hit([0.0; 3]));
        let mut block =
            SoundInstance::at("minecraft:big", SoundSource::Blocks, 1.0, 1.0, 0, [0.0; 3]);
        r.engine.play(block.clone());
        block.source = SoundSource::Players;
        r.engine.play(block);
        render(&mut r.mixer, 64);
        r.engine
            .stop_matching(Some("minecraft:big"), Some(SoundSource::Players));
        render(&mut r.mixer, 128);
        r.engine.pump();
        assert_eq!(r.mixer.playing(), 2);
        r.engine.stop_matching(None, Some(SoundSource::Players));
        render(&mut r.mixer, 128);
        assert_eq!(r.mixer.playing(), 1);
        r.engine.stop_matching(None, None);
        render(&mut r.mixer, 128);
        assert_eq!(r.mixer.playing(), 0);
    }

    #[test]
    fn an_entity_sound_follows_it_and_stops_when_it_is_gone() {
        let mut r = rig();
        r.engine
            .decoded(&Id::minecraft("sounds/big.ogg").unwrap(), Ok(pcm(480_000)));
        let s = SoundInstance::entity(
            "minecraft:big",
            SoundSource::Hostile,
            1.0,
            1.0,
            0,
            5,
            [4.0, 0.0, 0.0],
        );
        let id = r.engine.play(s).id().unwrap();
        let out = render(&mut r.mixer, 256);
        // 4 blocks right of 8: half the gain, all on the right.
        assert!(out[400].abs() < 1e-6 && (out[401] - 0.25).abs() < 1e-4);
        r.engine.tick(false, &|e| {
            (e == 5).then_some(EntityState {
                position: [-6.0, 0.0, 0.0],
                silent: false,
            })
        });
        let out = render(&mut r.mixer, 256);
        assert!((out[400] - 0.5 * 0.25).abs() < 1e-4 && out[401].abs() < 1e-6);
        r.engine.tick(false, NONE);
        render(&mut r.mixer, 256);
        r.engine.pump();
        assert_eq!(r.mixer.playing(), 0);
        assert!(r.engine.is_active(id));
    }

    #[test]
    fn delayed_sounds_start_when_due() {
        let mut r = rig();
        r.engine
            .decoded(&Id::minecraft("sounds/big.ogg").unwrap(), Ok(pcm(480_000)));
        let id = r
            .engine
            .play_delayed(SoundInstance::ui("minecraft:big", 1.0, 1.0, 0), 3);
        r.engine.tick(false, NONE);
        r.engine.tick(false, NONE);
        assert_eq!(r.engine.voices_in_use(), (0, 0));
        assert!(r.engine.is_active(id));
        r.engine.tick(false, NONE);
        assert_eq!(r.engine.voices_in_use(), (1, 0));
    }

    #[test]
    fn a_stream_starts_when_opened() {
        let mut r = rig();
        let id = r
            .engine
            .play(SoundInstance::ui("minecraft:song", 1.0, 1.0, 0))
            .id()
            .unwrap();
        let (slot, file, looping) = r.asked.lock().unwrap().streams[0].clone();
        assert_eq!(
            (file.as_str(), looping, usize::from(slot) >= STATIC_VOICES),
            ("minecraft:sounds/song.ogg", false, true)
        );
        let (mut tx, rx) = rtrb::RingBuffer::new(1000);
        tx.push(1000).unwrap();
        r.engine.stream_opened(
            slot,
            Ok(StreamFeed {
                channels: 1,
                rate: 48_000,
                samples: rx,
                ended: Arc::default(),
            }),
        );
        render(&mut r.mixer, 64);
        assert_eq!(r.mixer.playing(), 1);
        assert!(r.engine.is_active(id));
    }

    #[test]
    fn a_new_output_stops_everything() {
        let mut r = rig();
        r.engine.play(hit([0.0; 3]));
        let (mixer, link) = Mixer::new(44_100, Arc::new(ListenerCell::default()));
        r.engine.attach(link);
        assert_eq!(r.engine.voices_in_use(), (0, 0));
        r.engine.detach();
        assert_eq!(r.engine.play(hit([0.0; 3])), PlayResult::NotStarted);
        drop(mixer);
    }
}

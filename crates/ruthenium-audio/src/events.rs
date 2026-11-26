//! `sounds.json`: the sound events, the sounds each may play, and the choice of one as
//! vanilla makes it (`SoundManager.Preparations`, `SoundEventRegistrationSerializer`,
//! `WeighedSoundEvents`).
//!
//! Every pack's `<namespace>:sounds.json` is read, lowest pack first. An event a pack
//! names again gets the new sounds added to its own, or replaced with `"replace":
//! true`. A sound naming a file that no pack has is left out (vanilla warns). A file that
//! doesn't parse, or has any entry vanilla's reader refuses, adds nothing; a key that
//! isn't a valid path stops the file there, keeping what came before it.
//!
//! Choosing: an event's weight is the sum of its sounds' weights, where a sound that is
//! itself an event (`"type": "event"`) weighs what that event weighs in total (not its
//! own `weight`, which vanilla only carries along). `nextInt` of the total picks one;
//! a referenced event picks again with the same random, and its sound's volume and pitch
//! are multiplied by the reference's.

use std::collections::HashMap;
use std::fmt;

use ruthenium_assets::{Id, Limits};
use ruthenium_javamath::random::JavaRandom;
use serde_json::Value;

/// Vanilla's `SoundEvent` ids for "nothing to play" (`SoundManager`).
pub const EMPTY: &str = "minecraft:empty";
pub const INTENTIONALLY_EMPTY: &str = "minecraft:intentionally_empty";

/// Event references deeper than this choose nothing (vanilla overflows its stack).
const MAX_DEPTH: u32 = 32;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SoundKind {
    /// `sounds/<path>.ogg` in the name's namespace.
    File,
    /// Another event.
    Event,
}

/// One sound of an event (`Sound`).
#[derive(Debug, Clone, PartialEq)]
pub struct SoundDef {
    pub name: Id,
    pub volume: f32,
    pub pitch: f32,
    pub weight: i32,
    pub kind: SoundKind,
    pub stream: bool,
    pub preload: bool,
    pub attenuation_distance: i32,
}

impl SoundDef {
    /// A plain file with every default (a bare string in `sounds`).
    fn file(name: Id) -> SoundDef {
        SoundDef {
            name,
            volume: 1.0,
            pitch: 1.0,
            weight: 1,
            kind: SoundKind::File,
            stream: false,
            preload: false,
            attenuation_distance: 16,
        }
    }

    /// The `.ogg` file of a file sound (`Sound.getPath`).
    pub fn file_id(&self) -> Id {
        self.name.wrap("sounds/", ".ogg")
    }
}

/// An event: its sounds in the order the packs gave them.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct EventDef {
    pub sounds: Vec<SoundDef>,
    pub subtitle: Option<String>,
}

/// The sound chosen for one play: a file, with the volume and pitch factors of the
/// sound and of any references on the way to it.
#[derive(Debug, Clone, PartialEq)]
pub struct Chosen {
    /// The sound's name (`Sound.getLocation`).
    pub name: Id,
    pub volume: f32,
    pub pitch: f32,
    pub stream: bool,
    pub preload: bool,
    pub attenuation_distance: i32,
}

impl Chosen {
    /// The `.ogg` file.
    pub fn file_id(&self) -> Id {
        self.name.wrap("sounds/", ".ogg")
    }

    /// `Sound.getAttenuationDistance(volume)`: louder than 1 carries further.
    pub fn attenuation(&self, volume: f32) -> f32 {
        volume.max(1.0) * self.attenuation_distance as f32
    }
}

/// What an event gives for a play.
#[derive(Debug, Clone, PartialEq)]
pub enum Choice {
    /// No event of that name (vanilla warns once and plays nothing).
    Unknown,
    /// The event has no sound with any weight (`EMPTY_SOUND`).
    Empty,
    Sound(Chosen),
}

/// The events of every pack, by `namespace:path`.
#[derive(Debug, Default, Clone)]
pub struct SoundEvents {
    events: HashMap<String, EventDef>,
    /// Each event's weight at every depth of references, worked out once: summed afresh
    /// for every play, an event that names another a thousand times, which names a third
    /// a thousand times, and so on 32 deep, would take forever.
    weights: HashMap<String, Box<[i32]>>,
}

/// One pack's `sounds.json` for one namespace.
#[derive(Debug, Clone, Copy)]
pub struct SoundsJson<'a> {
    pub namespace: &'a str,
    /// For warnings.
    pub pack: &'a str,
    pub bytes: &'a [u8],
    /// The limits of the pack it is in.
    pub limits: Limits,
}

/// A registration as vanilla's deserializer reads it.
struct Registration {
    sounds: Vec<SoundDef>,
    replace: bool,
    subtitle: Option<String>,
}

impl SoundEvents {
    /// Reads the files, lowest pack first; `has_file` says whether a pack has a sound
    /// file (`sounds/...ogg`). Returns the warnings vanilla would log.
    pub fn load<'a>(
        files: impl IntoIterator<Item = SoundsJson<'a>>,
        has_file: impl Fn(&Id) -> bool,
    ) -> (SoundEvents, Vec<String>) {
        let mut events = SoundEvents::default();
        let mut warnings = Vec::new();
        for f in files {
            let entries = match parse_file(f.bytes, &f.limits) {
                Ok(e) => e,
                Err(e) => {
                    warnings.push(format!("invalid sounds.json in {}: {e}", f.pack));
                    continue;
                }
            };
            for (key, reg) in entries {
                let Some(id) = Id::new(f.namespace, &key) else {
                    warnings.push(format!(
                        "invalid sounds.json in {}: bad event name {key:?}",
                        f.pack
                    ));
                    break;
                };
                events.register(id, reg, &has_file, &mut warnings);
            }
        }
        events.weigh();
        (events, warnings)
    }

    /// Works out every event's weight at every depth (`weights`), deepest first.
    fn weigh(&mut self) {
        let depths = MAX_DEPTH as usize + 1;
        let mut weights: HashMap<String, Box<[i32]>> = self
            .events
            .keys()
            .map(|k| (k.clone(), vec![0; depths].into_boxed_slice()))
            .collect();
        for depth in (0..depths).rev() {
            let mut layer = Vec::with_capacity(self.events.len());
            for (key, event) in &self.events {
                let w = event.sounds.iter().fold(0i32, |sum, s| {
                    sum.wrapping_add(match s.kind {
                        SoundKind::File => s.weight,
                        SoundKind::Event if depth >= MAX_DEPTH as usize => 0,
                        SoundKind::Event => {
                            weights.get(&id_key(&s.name)).map_or(0, |w| w[depth + 1])
                        }
                    })
                });
                layer.push((key, w));
            }
            for (key, w) in layer {
                if let Some(ws) = weights.get_mut(key) {
                    ws[depth] = w;
                }
            }
        }
        self.weights = weights;
    }

    /// `Preparations.handleRegistration`.
    fn register(
        &mut self,
        id: Id,
        reg: Registration,
        has_file: &impl Fn(&Id) -> bool,
        warnings: &mut Vec<String>,
    ) {
        let key = id_key(&id);
        let event = match self.events.get_mut(&key) {
            Some(e) if !reg.replace => e,
            _ => {
                self.events.insert(
                    key.clone(),
                    EventDef {
                        sounds: Vec::new(),
                        subtitle: reg.subtitle,
                    },
                );
                self.events.get_mut(&key).expect("just inserted")
            }
        };
        for sound in reg.sounds {
            if sound.kind == SoundKind::File && !has_file(&sound.file_id()) {
                warnings.push(format!(
                    "File {} does not exist, cannot add it to event {key}",
                    id_key(&sound.file_id())
                ));
                continue;
            }
            event.sounds.push(sound);
        }
    }

    pub fn get(&self, event: &str) -> Option<&EventDef> {
        self.events.get(event)
    }

    pub fn len(&self) -> usize {
        self.events.len()
    }

    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }

    /// Every event's name.
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.events.keys().map(String::as_str)
    }

    /// `WeighedSoundEvents.getWeight` of the event `key` at `depth`.
    fn weight(&self, key: &str, depth: u32) -> i32 {
        self.weights
            .get(key)
            .and_then(|w| w.get(depth as usize))
            .copied()
            .unwrap_or(0)
    }

    fn sound_weight(&self, sound: &SoundDef, depth: u32) -> i32 {
        match sound.kind {
            SoundKind::File => sound.weight,
            SoundKind::Event if depth >= MAX_DEPTH => 0,
            SoundKind::Event => self.weight(&id_key(&sound.name), depth + 1),
        }
    }

    /// `getOrResolve` + `WeighedSoundEvents.getSound`: the sound `event` plays with
    /// `random` (`RandomSource.create(seed)` for a server's sound).
    pub fn choose(&self, event: &str, random: &mut JavaRandom) -> Choice {
        match self.events.get(event) {
            None => Choice::Unknown,
            Some(e) => self
                .choose_in(event, e, random, 0)
                .map_or(Choice::Empty, Choice::Sound),
        }
    }

    fn choose_in(
        &self,
        key: &str,
        event: &EventDef,
        random: &mut JavaRandom,
        depth: u32,
    ) -> Option<Chosen> {
        let weight = self.weight(key, depth);
        if event.sounds.is_empty() || weight <= 0 {
            return None;
        }
        let mut index = random.next_int(weight);
        for sound in &event.sounds {
            // Java's int arithmetic: a pack's weights may add up past an int.
            index = index.wrapping_sub(self.sound_weight(sound, depth));
            if index < 0 {
                return match sound.kind {
                    SoundKind::File => Some(Chosen {
                        name: sound.name.clone(),
                        volume: sound.volume,
                        pitch: sound.pitch,
                        stream: sound.stream,
                        preload: sound.preload,
                        attenuation_distance: sound.attenuation_distance,
                    }),
                    SoundKind::Event => {
                        let inner_key = id_key(&sound.name);
                        let inner = self.events.get(&inner_key)?;
                        let wrapped = self.choose_in(&inner_key, inner, random, depth + 1)?;
                        Some(Chosen {
                            volume: wrapped.volume * sound.volume,
                            pitch: wrapped.pitch * sound.pitch,
                            stream: wrapped.stream || sound.stream,
                            ..wrapped
                        })
                    }
                };
            }
        }
        None
    }
}

/// `namespace:path`.
pub fn id_key(id: &Id) -> String {
    format!("{}:{}", id.namespace(), id.path())
}

/// A file's entries in file order, each read as vanilla's deserializer reads it; any
/// entry it refuses fails the whole file.
/// Over [`Limits::json_bytes`], [`Limits::sound_events`] or [`Limits::sounds_per_event`],
/// the whole file is refused too.
fn parse_file(bytes: &[u8], limits: &Limits) -> Result<Vec<(String, Registration)>, String> {
    if bytes.len() as u64 > limits.json_bytes {
        return Err(format!(
            "larger than a JSON file may be ({})",
            limits.json_bytes
        ));
    }
    let entries: Ordered = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
    if entries.0.len() > limits.sound_events {
        return Err(format!(
            "{} events is more than a sounds.json may have ({})",
            entries.0.len(),
            limits.sound_events
        ));
    }
    if let Some((k, _)) = entries.0.iter().find(|(_, v)| {
        v.get("sounds")
            .and_then(Value::as_array)
            .is_some_and(|s| s.len() > limits.sounds_per_event)
    }) {
        return Err(format!(
            "{k}: more sounds than an event may have ({})",
            limits.sounds_per_event
        ));
    }
    entries
        .0
        .into_iter()
        .map(|(k, v)| {
            Ok((
                k.clone(),
                registration(&v).map_err(|e| format!("{k}: {e}"))?,
            ))
        })
        .collect()
}

/// `SoundEventRegistrationSerializer.deserialize`.
fn registration(v: &Value) -> Result<Registration, String> {
    let o = v.as_object().ok_or("entry is not an object")?;
    let replace = match o.get("replace") {
        None => false,
        Some(v) => as_bool(v).ok_or("replace is not a boolean")?,
    };
    let subtitle = match o.get("subtitle") {
        None => None,
        Some(v) => Some(as_string(v).ok_or("subtitle is not a string")?),
    };
    let mut sounds = Vec::new();
    if let Some(list) = o.get("sounds") {
        for s in list.as_array().ok_or("sounds is not an array")? {
            sounds.push(match s {
                Value::String(name) => SoundDef::file(parse_id(name)?),
                other => sound(other)?,
            });
        }
    }
    Ok(Registration {
        sounds,
        replace,
        subtitle,
    })
}

/// `SoundEventRegistrationSerializer.getSound`.
fn sound(v: &Value) -> Result<SoundDef, String> {
    let o = v.as_object().ok_or("sound is not an object")?;
    let name = o
        .get("name")
        .and_then(as_string)
        .ok_or("sound has no name")?;
    let kind = match o.get("type") {
        None => SoundKind::File,
        Some(t) => match as_string(t).as_deref() {
            Some("file") => SoundKind::File,
            Some("event") => SoundKind::Event,
            _ => return Err("Invalid type".into()),
        },
    };
    let float = |key: &str| -> Result<f32, String> {
        o.get(key).map_or(Ok(1.0), |v| {
            as_f32(v).ok_or_else(|| format!("{key} is not a number"))
        })
    };
    let volume = float("volume")?;
    if volume.is_nan() || volume <= 0.0 {
        return Err("Invalid volume".into());
    }
    let pitch = float("pitch")?;
    if pitch.is_nan() || pitch <= 0.0 {
        return Err("Invalid pitch".into());
    }
    let int = |key: &str, default: i32| -> Result<i32, String> {
        o.get(key).map_or(Ok(default), |v| {
            as_i32(v).ok_or_else(|| format!("{key} is not a number"))
        })
    };
    let weight = int("weight", 1)?;
    if weight <= 0 {
        return Err("Invalid weight".into());
    }
    let flag = |key: &str| -> Result<bool, String> {
        o.get(key).map_or(Ok(false), |v| {
            as_bool(v).ok_or_else(|| format!("{key} is not a boolean"))
        })
    };
    Ok(SoundDef {
        name: parse_id(&name)?,
        volume,
        pitch,
        weight,
        kind,
        stream: flag("stream")?,
        preload: flag("preload")?,
        attenuation_distance: int("attenuation_distance", 16)?,
    })
}

/// `Identifier.parse`.
fn parse_id(s: &str) -> Result<Id, String> {
    Id::parse(s).ok_or_else(|| format!("bad name {s:?}"))
}

/// `GsonHelper.convertToString`: any primitive, numbers and booleans as written.
fn as_string(v: &Value) -> Option<String> {
    match v {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        Value::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}

/// `GsonHelper.convertToBoolean`: a boolean, or a string that is "true" in any case.
fn as_bool(v: &Value) -> Option<bool> {
    match v {
        Value::Bool(b) => Some(*b),
        Value::String(s) => Some(s.eq_ignore_ascii_case("true")),
        Value::Number(_) => Some(false),
        _ => None,
    }
}

/// `GsonHelper.convertToFloat`: numbers only (`Float.parseFloat` of the text).
fn as_f32(v: &Value) -> Option<f32> {
    match v {
        Value::Number(n) => n.to_string().parse().ok(),
        _ => None,
    }
}

/// `GsonHelper.convertToInt`: numbers only, a fraction cut off.
fn as_i32(v: &Value) -> Option<i32> {
    match v {
        Value::Number(n) => n
            .as_i64()
            .map(|i| i as i32)
            .or_else(|| n.as_f64().map(|f| f as i32)),
        _ => None,
    }
}

/// A JSON object's members in file order; a repeated key fails it, as Gson's map
/// reader does.
struct Ordered(Vec<(String, Value)>);

impl<'de> serde::Deserialize<'de> for Ordered {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Ordered, D::Error> {
        struct Visit;
        impl<'de> serde::de::Visitor<'de> for Visit {
            type Value = Ordered;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("an object of sound events")
            }

            fn visit_map<A: serde::de::MapAccess<'de>>(
                self,
                mut map: A,
            ) -> Result<Ordered, A::Error> {
                let mut out: Vec<(String, Value)> = Vec::new();
                while let Some((k, v)) = map.next_entry::<String, Value>()? {
                    if out.iter().any(|(o, _)| *o == k) {
                        return Err(serde::de::Error::custom(format!("duplicate key: {k}")));
                    }
                    out.push((k, v));
                }
                Ok(Ordered(out))
            }
        }
        d.deserialize_map(Visit)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn load(files: &[(&str, &str)], missing: &[&str]) -> (SoundEvents, Vec<String>) {
        SoundEvents::load(
            files.iter().map(|(ns, json)| SoundsJson {
                namespace: ns,
                pack: "test",
                bytes: json.as_bytes(),
                limits: Limits::trusted(),
            }),
            |id| !missing.contains(&id_key(id).as_str()),
        )
    }

    fn names(events: &SoundEvents, event: &str) -> Vec<String> {
        events
            .get(event)
            .unwrap()
            .sounds
            .iter()
            .map(|s| id_key(&s.name))
            .collect()
    }

    #[test]
    fn reads_vanillas_fields_and_defaults() {
        let (e, w) = load(
            &[(
                "minecraft",
                r#"{"a.b": {"subtitle": "subtitles.a", "sounds": ["x/one",
                    {"name": "mod:x/two", "volume": 0.5, "pitch": 1.2, "weight": 3,
                     "stream": true, "preload": "TRUE", "attenuation_distance": 8},
                    {"name": "a.c", "type": "event"}]}}"#,
            )],
            &[],
        );
        assert!(w.is_empty(), "{w:?}");
        let a = e.get("minecraft:a.b").unwrap();
        assert_eq!(a.subtitle.as_deref(), Some("subtitles.a"));
        assert_eq!(a.sounds[0], SoundDef::file(Id::minecraft("x/one").unwrap()));
        let two = &a.sounds[1];
        assert_eq!(
            (two.volume, two.pitch, two.weight, two.stream, two.preload),
            (0.5, 1.2, 3, true, true)
        );
        assert_eq!(two.attenuation_distance, 8);
        assert_eq!(id_key(&two.file_id()), "mod:sounds/x/two.ogg");
        assert_eq!(a.sounds[2].kind, SoundKind::Event);
    }

    #[test]
    fn later_packs_add_unless_they_replace() {
        let (e, _) = load(
            &[
                (
                    "minecraft",
                    r#"{"a": {"subtitle": "s1", "sounds": ["one"]}, "b": {"sounds": ["one"]}}"#,
                ),
                (
                    "minecraft",
                    r#"{"a": {"subtitle": "s2", "sounds": ["two"]}, "b": {"replace": true, "sounds": ["two"]}}"#,
                ),
            ],
            &[],
        );
        assert_eq!(names(&e, "minecraft:a"), ["minecraft:one", "minecraft:two"]);
        assert_eq!(
            e.get("minecraft:a").unwrap().subtitle.as_deref(),
            Some("s1")
        );
        assert_eq!(names(&e, "minecraft:b"), ["minecraft:two"]);
        assert_eq!(e.get("minecraft:b").unwrap().subtitle, None);
    }

    #[test]
    fn refused_files_add_nothing_and_missing_files_are_left_out() {
        let (e, w) = load(
            &[
                ("minecraft", r#"{"a": {"sounds": ["one", "gone"]}}"#),
                // A bad volume anywhere fails the whole file.
                (
                    "minecraft",
                    r#"{"a": {"sounds": ["two"]}, "b": {"sounds": [{"name": "x", "volume": 0}]}}"#,
                ),
                (
                    "minecraft",
                    r#"{"c": {"sounds": [{"name": "x", "type": "other"}]}}"#,
                ),
                (
                    "minecraft",
                    r#"{"c": {"sounds": ["x"]}, "c": {"sounds": ["y"]}}"#,
                ),
                ("minecraft", r#"[1, 2]"#),
                // A bad key stops the file, keeping what came before.
                (
                    "minecraft",
                    r#"{"d": {"sounds": ["x"]}, "Bad": {"sounds": ["x"]}, "e": {"sounds": ["x"]}}"#,
                ),
            ],
            &["minecraft:sounds/gone.ogg"],
        );
        assert_eq!(names(&e, "minecraft:a"), ["minecraft:one"]);
        assert!(e.get("minecraft:b").is_none() && e.get("minecraft:c").is_none());
        assert!(e.get("minecraft:d").is_some() && e.get("minecraft:e").is_none());
        assert_eq!(w.len(), 6, "{w:#?}");
    }

    /// The draws are checked against vanilla's own classes in `tests/vanilla_choice.rs`;
    /// this only pins down the rules on a small case.
    #[test]
    fn a_reference_weighs_its_whole_event() {
        let (e, _) = load(
            &[(
                "minecraft",
                r#"{"top": {"sounds": [{"name": "inner", "type": "event", "weight": 100, "volume": 0.5},
                                       {"name": "plain", "weight": 1}]},
                    "inner": {"sounds": [{"name": "i1", "weight": 2, "pitch": 1.5}, {"name": "i2"}]},
                    "loop": {"sounds": [{"name": "loop", "type": "event"}]},
                    "none": {"sounds": []}}"#,
            )],
            &[],
        );
        // Total weight 3 + 1 = 4: draws of 0..2 go to "inner", 3 to "plain".
        let mut counts = HashMap::new();
        for seed in 0..400 {
            let mut r = JavaRandom::new(seed);
            let Choice::Sound(c) = e.choose("minecraft:top", &mut r) else {
                panic!()
            };
            *counts.entry(id_key(&c.name)).or_insert(0) += 1;
            if c.name.path() == "i1" {
                assert_eq!((c.volume, c.pitch), (0.5, 1.5));
            }
        }
        assert!(counts["minecraft:plain"] < 150, "{counts:?}");
        assert_eq!(
            e.choose("minecraft:nope", &mut JavaRandom::new(0)),
            Choice::Unknown
        );
        assert_eq!(
            e.choose("minecraft:none", &mut JavaRandom::new(0)),
            Choice::Empty
        );
        assert_eq!(
            e.choose("minecraft:loop", &mut JavaRandom::new(0)),
            Choice::Empty
        );
    }
}

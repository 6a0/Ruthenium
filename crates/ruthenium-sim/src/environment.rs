//! What a level's look depends on besides its blocks, kept as the server defines it:
//! the dimension type's environment attributes (sky, fog and cloud colours, fog
//! distances, the light colours and factors), which way faces are shaded
//! (`cardinal_light`), the timelines that move attributes with a world clock (the day,
//! the moon), each biome's attribute overrides and special effects, and the world
//! clocks themselves as the server sets them and the ticks count them on.
//!
//! Everything comes from the registries of the session's configuration. An entry the
//! server sent in full is used as sent (a server may define its own sky); an entry it
//! sent by name only, because the client said it has that data pack, is looked up in
//! the player's own game ([`LocalRegistries`]). Definitions are kept as JSON-shaped
//! values, as their files have them: this module keeps what drawing will evaluate (the
//! interpolation between biomes and along timelines is not done here).

use std::collections::HashMap;

use ruthenium_javamath::mth;
use ruthenium_protocol::nbt::Tag;
use ruthenium_protocol::packets::configuration::{RegistryEntry, RegistryTags};
use ruthenium_protocol::packets::play::ClockState;
use serde_json::{Map, Value};

use crate::scene::ClockSample;

pub const DIMENSION_TYPE: &str = "minecraft:dimension_type";
pub const TIMELINE: &str = "minecraft:timeline";
pub const WORLD_CLOCK: &str = "minecraft:world_clock";
pub const BIOME: &str = "minecraft:worldgen/biome";

/// The player's own game's entries of the registries read here, by registry and entry
/// id (`data/minecraft/<registry>/<entry>.json`), for the entries a server names
/// without sending.
pub type LocalRegistries = HashMap<String, HashMap<String, Value>>;

/// `CardinalLighting.Type`: how much light each face of a block keeps.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CardinalLight {
    #[default]
    Default,
    Nether,
}

impl CardinalLight {
    /// The factors for a face looking down, up, north, south, west and east.
    pub fn shades(self) -> [f32; 6] {
        match self {
            CardinalLight::Default => [0.5, 1.0, 0.8, 0.8, 0.6, 0.6],
            CardinalLight::Nether => [0.9, 0.9, 0.8, 0.8, 0.6, 0.6],
        }
    }
}

/// A timeline that applies in the dimension.
#[derive(Debug, Clone, PartialEq)]
pub struct Timeline {
    pub id: String,
    /// The world clock it runs on, by its id in the `world_clock` registry.
    pub clock: Option<i32>,
    /// Its definition: `period_ticks`, `tracks` (per attribute: keyframes, ease,
    /// modifier), `time_markers`.
    pub definition: Value,
}

/// What a biome changes about the look of things.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct BiomeLook {
    pub id: String,
    /// Its climate, which picks its colours from the colour maps where its effects name
    /// none (`temperature`, `downfall`).
    pub temperature: f32,
    pub downfall: f32,
    /// It rains or snows there (`has_precipitation`; yes where the definition does not
    /// say).
    pub has_precipitation: bool,
    /// Its environment attribute overrides (`attributes`).
    pub attributes: Value,
    /// Its special effects (`effects`: water, grass and foliage colours, the grass
    /// colour modifier).
    pub effects: Value,
}

/// A dimension's environment as the session's registries define it.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Environment {
    /// The dimension type's id.
    pub dimension_type: String,
    pub has_skylight: bool,
    pub has_ceiling: bool,
    pub ambient_light: f32,
    pub cardinal_light: CardinalLight,
    /// The dimension type's `skybox` (`overworld` when it names none).
    pub skybox: String,
    /// The dimension type's environment attributes by id (`minecraft:visual/sky_color`...).
    pub attributes: Map<String, Value>,
    /// The timelines of the dimension type's `timelines`, in the order of its tag (as
    /// the server sent it) or of its list. Attributes take them in this order.
    pub timelines: Vec<Timeline>,
    /// The world clocks' names by registry id.
    pub clocks: Vec<String>,
    /// The dimension's own clock (`default_clock`), which the day runs on.
    pub default_clock: Option<i32>,
    /// The biomes by network id.
    pub biomes: Vec<BiomeLook>,
}

/// An NBT value as the JSON its file would hold. Bytes become numbers, so a boolean
/// the server sent reads 0 or 1 where a file says false or true: [`truthy`] reads both.
pub fn nbt_to_json(tag: &Tag) -> Value {
    match tag {
        Tag::Byte(v) => Value::from(*v),
        Tag::Short(v) => Value::from(*v),
        Tag::Int(v) => Value::from(*v),
        Tag::Long(v) => Value::from(*v),
        Tag::Float(v) => Value::from(*v),
        Tag::Double(v) => Value::from(*v),
        Tag::String(s) => Value::from(s.as_str()),
        Tag::ByteArray(a) => a.iter().map(|v| Value::from(*v)).collect(),
        Tag::IntArray(a) => a.iter().map(|v| Value::from(*v)).collect(),
        Tag::LongArray(a) => a.iter().map(|v| Value::from(*v)).collect(),
        Tag::List(l) => l.iter().map(nbt_to_json).collect(),
        Tag::Compound(c) => Value::Object(
            c.iter()
                .map(|(k, v)| (k.to_owned(), nbt_to_json(v)))
                .collect(),
        ),
    }
}

/// A boolean from a file (true/false) or from the network (a byte).
pub fn truthy(v: &Value) -> Option<bool> {
    v.as_bool().or_else(|| v.as_i64().map(|n| n != 0))
}

/// A registry entry's definition: what the server sent, else the player's game's.
fn definition(registry: &str, entry: &RegistryEntry, local: &LocalRegistries) -> Option<Value> {
    match &entry.data {
        Some(tag) => Some(nbt_to_json(tag)),
        None => local.get(registry)?.get(&entry.id).cloned(),
    }
}

impl Environment {
    /// The environment of the dimension type with network id `dimension_type`, from
    /// the configuration's registries and tags.
    pub fn resolve(
        registries: &[(String, Vec<RegistryEntry>)],
        tags: &[RegistryTags],
        local: &LocalRegistries,
        dimension_type: i32,
    ) -> Environment {
        let registry = |name: &str| {
            registries
                .iter()
                .find(|(r, _)| r == name)
                .map(|(_, e)| e.as_slice())
                .unwrap_or(&[])
        };
        let clocks: Vec<String> = registry(WORLD_CLOCK).iter().map(|e| e.id.clone()).collect();
        let clock_id = |name: &str| {
            let name = full_id(name);
            clocks.iter().position(|c| *c == name).map(|i| i as i32)
        };
        let biomes = registry(BIOME)
            .iter()
            .map(|e| {
                let d = definition(BIOME, e, local).unwrap_or(Value::Null);
                let number = |name: &str| d.get(name).and_then(Value::as_f64).unwrap_or(0.0) as f32;
                BiomeLook {
                    id: e.id.clone(),
                    temperature: number("temperature"),
                    downfall: number("downfall"),
                    has_precipitation: d.get("has_precipitation").and_then(truthy).unwrap_or(true),
                    attributes: d.get("attributes").cloned().unwrap_or(Value::Null),
                    effects: d.get("effects").cloned().unwrap_or(Value::Null),
                }
            })
            .collect();
        let mut env = Environment {
            has_skylight: true,
            skybox: "overworld".into(),
            clocks: clocks.clone(),
            biomes,
            ..Environment::default()
        };
        let Some(entry) = usize::try_from(dimension_type)
            .ok()
            .and_then(|i| registry(DIMENSION_TYPE).get(i))
        else {
            return env;
        };
        env.dimension_type = entry.id.clone();
        let Some(d) = definition(DIMENSION_TYPE, entry, local) else {
            return env;
        };
        let flag = |name: &str, default: bool| d.get(name).and_then(truthy).unwrap_or(default);
        env.has_skylight = flag("has_skylight", true);
        env.has_ceiling = flag("has_ceiling", false);
        env.ambient_light = d
            .get("ambient_light")
            .and_then(Value::as_f64)
            .unwrap_or(0.0) as f32;
        env.cardinal_light = match d.get("cardinal_light").and_then(Value::as_str) {
            Some("nether") => CardinalLight::Nether,
            _ => CardinalLight::Default,
        };
        if let Some(s) = d.get("skybox").and_then(Value::as_str) {
            env.skybox = s.to_owned();
        }
        if let Some(Value::Object(a)) = d.get("attributes") {
            env.attributes = a.clone();
        }
        env.default_clock = d
            .get("default_clock")
            .and_then(Value::as_str)
            .and_then(clock_id);
        // `timelines` is a tag (`#name`), one timeline or a list of them.
        let all = registry(TIMELINE);
        let members: Vec<usize> = match d.get("timelines") {
            Some(Value::String(s)) => match s.strip_prefix('#') {
                Some(tag) => {
                    let tag = full_id(tag);
                    tags.iter()
                        .find(|t| t.registry == TIMELINE)
                        .and_then(|t| t.tags.iter().find(|(name, _)| full_id(name) == tag))
                        .map(|(_, ids)| {
                            ids.iter()
                                .filter_map(|&i| usize::try_from(i).ok())
                                .collect()
                        })
                        .unwrap_or_default()
                }
                None => all
                    .iter()
                    .position(|e| e.id == full_id(s))
                    .into_iter()
                    .collect(),
            },
            Some(Value::Array(list)) => list
                .iter()
                .filter_map(|v| match v {
                    Value::String(s) => all.iter().position(|e| e.id == full_id(s)),
                    v => v.as_u64().map(|i| i as usize),
                })
                .collect(),
            _ => Vec::new(),
        };
        env.timelines = members
            .into_iter()
            .filter(|&i| i < all.len())
            .filter_map(|i| {
                let definition = definition(TIMELINE, &all[i], local)?;
                Some(Timeline {
                    id: all[i].id.clone(),
                    clock: definition
                        .get("clock")
                        .and_then(Value::as_str)
                        .and_then(clock_id),
                    definition,
                })
            })
            .collect();
        env
    }
}

/// An identifier with its namespace spelled out.
fn full_id(s: &str) -> String {
    if s.contains(':') {
        s.to_owned()
    } else {
        format!("minecraft:{s}")
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Clock {
    id: i32,
    total_ticks: i64,
    partial_tick: f32,
    rate: f32,
}

/// The world clocks as the client keeps them (`ClientClockManager`): set by the
/// server with the time, counted on by every game tick at each clock's own rate.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Clocks {
    clocks: Vec<Clock>,
    last_tick_game_time: i64,
}

impl Clocks {
    /// `tick`: every clock moves on by the game time that passed, times its rate.
    pub fn tick(&mut self, game_time: i64) {
        let delta = game_time - self.last_tick_game_time;
        self.last_tick_game_time = game_time;
        for c in &mut self.clocks {
            let partial = f64::from(c.partial_tick) + delta as f64 * f64::from(c.rate);
            let full = i64::from(mth::floor(partial));
            c.partial_tick = (partial - full as f64) as f32;
            c.total_ticks += full;
        }
    }

    /// `handleUpdates`: the clocks tick up to the server's game time, then the ones
    /// it names take its values. A clock never named runs from 0 at rate 1.
    pub fn handle_updates(&mut self, game_time: i64, updates: &[(i32, ClockState)]) {
        self.tick(game_time);
        for &(id, state) in updates {
            let new = Clock {
                id,
                total_ticks: state.total_ticks,
                partial_tick: state.partial_tick,
                rate: state.rate,
            };
            match self.clocks.iter_mut().find(|c| c.id == id) {
                Some(c) => *c = new,
                None => self.clocks.push(new),
            }
        }
    }

    /// A clock's ticks, its part of a tick, and its rate (`getInstance`).
    pub fn get(&self, id: i32) -> ClockSample {
        self.clocks
            .iter()
            .find(|c| c.id == id)
            .map(|c| ClockSample {
                clock: c.id,
                total_ticks: c.total_ticks,
                partial_tick: c.partial_tick,
                rate: c.rate,
            })
            .unwrap_or(ClockSample {
                clock: id,
                total_ticks: 0,
                partial_tick: 0.0,
                rate: 1.0,
            })
    }

    /// Every clock the server has set, into `out` (emptied first).
    pub fn samples(&self, out: &mut Vec<ClockSample>) {
        out.clear();
        out.extend(self.clocks.iter().map(|c| self.get(c.id)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ruthenium_protocol::nbt::Compound;
    use serde_json::json;

    fn entry(id: &str, data: Option<Tag>) -> RegistryEntry {
        RegistryEntry {
            id: id.into(),
            data,
        }
    }

    fn local() -> LocalRegistries {
        let mut l = LocalRegistries::new();
        l.entry(DIMENSION_TYPE.into()).or_default().insert(
            "minecraft:overworld".into(),
            json!({
                "ambient_light": 0.0, "has_skylight": true, "has_ceiling": false,
                "default_clock": "minecraft:overworld", "timelines": "#minecraft:in_overworld",
                "attributes": {"minecraft:visual/sky_color": "#78a7ff", "minecraft:visual/cloud_height": 192.33},
            }),
        );
        l.entry(DIMENSION_TYPE.into()).or_default().insert(
            "minecraft:the_nether".into(),
            json!({
                "ambient_light": 0.1, "has_skylight": false, "has_ceiling": true,
                "cardinal_light": "nether", "skybox": "none", "timelines": "#minecraft:in_nether",
                "attributes": {"minecraft:visual/fog_end_distance": 96.0},
            }),
        );
        for (id, period) in [("minecraft:day", 24000), ("minecraft:moon", 192000)] {
            l.entry(TIMELINE.into()).or_default().insert(
                id.into(),
                json!({"clock": "minecraft:overworld", "period_ticks": period, "tracks": {}}),
            );
        }
        l.entry(TIMELINE.into()).or_default().insert(
            "minecraft:villager_schedule".into(),
            json!({"clock": "minecraft:overworld", "tracks": {}}),
        );
        l.entry(BIOME.into()).or_default().insert(
            "minecraft:swamp".into(),
            json!({"attributes": {"minecraft:visual/fog_color": "#c0d8ff"},
                   "temperature": 0.8, "downfall": 0.9,
                   "effects": {"water_color": "#617b64"}, "features": [[1, 2, 3]]}),
        );
        l
    }

    fn registries(overworld: Option<Tag>) -> Vec<(String, Vec<RegistryEntry>)> {
        vec![
            (
                DIMENSION_TYPE.into(),
                vec![
                    entry("minecraft:overworld", overworld),
                    entry("minecraft:the_nether", None),
                ],
            ),
            (
                TIMELINE.into(),
                vec![
                    entry("minecraft:day", None),
                    entry("minecraft:early_game", None),
                    entry("minecraft:moon", None),
                    entry("minecraft:villager_schedule", None),
                ],
            ),
            (
                WORLD_CLOCK.into(),
                vec![
                    entry("minecraft:overworld", None),
                    entry("minecraft:the_end", None),
                ],
            ),
            (
                BIOME.into(),
                vec![
                    entry("minecraft:plains", None),
                    entry("minecraft:swamp", None),
                ],
            ),
        ]
    }

    fn tags() -> Vec<RegistryTags> {
        vec![RegistryTags {
            registry: TIMELINE.into(),
            tags: vec![
                ("minecraft:universal".into(), vec![3]),
                ("minecraft:in_overworld".into(), vec![3, 0, 2, 1]),
                ("minecraft:in_nether".into(), vec![3]),
            ],
        }]
    }

    #[test]
    fn known_pack_entries_come_from_the_players_game() {
        let env = Environment::resolve(&registries(None), &tags(), &local(), 0);
        assert_eq!(env.dimension_type, "minecraft:overworld");
        assert!(env.has_skylight && !env.has_ceiling);
        assert_eq!(env.cardinal_light, CardinalLight::Default);
        assert_eq!(env.skybox, "overworld");
        assert_eq!(env.attributes["minecraft:visual/sky_color"], "#78a7ff");
        assert_eq!(env.default_clock, Some(0));
        // The tag's timelines in the tag's order (the order their layers apply in);
        // early_game has no local definition here and is left out.
        let ids: Vec<&str> = env.timelines.iter().map(|t| t.id.as_str()).collect();
        assert_eq!(
            ids,
            [
                "minecraft:villager_schedule",
                "minecraft:day",
                "minecraft:moon"
            ]
        );
        assert_eq!(env.timelines[2].clock, Some(0));
        assert_eq!(env.timelines[2].definition["period_ticks"], 192000);
        // Biomes by network id; one without data keeps empty looks.
        assert_eq!(env.biomes.len(), 2);
        assert_eq!(env.biomes[0].attributes, Value::Null);
        assert_eq!(env.biomes[1].effects["water_color"], "#617b64");
        // Its climate, which picks its grass and foliage colours.
        assert_eq!(
            (env.biomes[1].temperature, env.biomes[1].downfall),
            (0.8, 0.9)
        );
        assert_eq!(
            (env.biomes[0].temperature, env.biomes[0].downfall),
            (0.0, 0.0)
        );
    }

    #[test]
    fn another_dimension_has_its_own() {
        let env = Environment::resolve(&registries(None), &tags(), &local(), 1);
        assert_eq!(env.dimension_type, "minecraft:the_nether");
        assert!(!env.has_skylight && env.has_ceiling);
        assert_eq!(env.cardinal_light, CardinalLight::Nether);
        assert_eq!(env.cardinal_light.shades()[0], 0.9);
        assert_eq!(env.skybox, "none");
        assert_eq!(env.ambient_light, 0.1);
        assert_eq!(env.default_clock, None);
        assert_eq!(env.timelines.len(), 1);
        assert!(!env.attributes.contains_key("minecraft:visual/sky_color"));
    }

    #[test]
    fn what_the_server_sends_overrides_the_players_game() {
        // A server with its own overworld: a red sky, no sky light (a byte, as the
        // network sends booleans), and only the day.
        let mut attributes = Compound::new();
        attributes.put("minecraft:visual/sky_color", Tag::String("#ff0000".into()));
        let mut d = Compound::new();
        d.put("has_skylight", Tag::Byte(0));
        d.put("has_ceiling", Tag::Byte(0));
        d.put("ambient_light", Tag::Float(0.5));
        d.put("attributes", Tag::Compound(attributes));
        d.put(
            "timelines",
            Tag::List(vec![Tag::String("minecraft:day".into())]),
        );
        let env = Environment::resolve(&registries(Some(Tag::Compound(d))), &tags(), &local(), 0);
        assert!(!env.has_skylight);
        assert_eq!(env.ambient_light, 0.5);
        assert_eq!(env.attributes["minecraft:visual/sky_color"], "#ff0000");
        assert!(!env.attributes.contains_key("minecraft:visual/cloud_height"));
        assert_eq!(env.timelines.len(), 1);
        assert_eq!(env.default_clock, None);
        // An unknown dimension type keeps the defaults.
        let none = Environment::resolve(&registries(None), &tags(), &local(), 9);
        assert!(none.dimension_type.is_empty() && none.has_skylight);
        assert_eq!(none.biomes.len(), 2);
    }

    #[test]
    fn clocks_run_at_their_rate_between_the_servers_updates() {
        let state = |total_ticks, partial_tick, rate| ClockState {
            total_ticks,
            partial_tick,
            rate,
        };
        let mut c = Clocks::default();
        c.handle_updates(
            1000,
            &[(0, state(6000, 0.0, 1.0)), (1, state(50, 0.5, 0.25))],
        );
        // Advancing time: 3 game ticks later.
        for t in 1001..=1003 {
            c.tick(t);
        }
        assert_eq!((c.get(0).total_ticks, c.get(0).partial_tick), (6003, 0.0));
        // The slow clock: 0.5 + 3 × 0.25 = 1.25.
        assert_eq!((c.get(1).total_ticks, c.get(1).partial_tick), (51, 0.25));
        // Frozen time (`/time pause`: rate 0): the ticks leave it where it is.
        c.handle_updates(1003, &[(0, state(6003, 0.0, 0.0))]);
        for t in 1004..=1100 {
            c.tick(t);
        }
        assert_eq!(c.get(0).total_ticks, 6003);
        assert_eq!(c.get(1).total_ticks, 51 + 24);
        // The server's time jumps (a skipped night): the clocks tick up to it first,
        // then take what it says.
        c.handle_updates(2100, &[(0, state(1000, 0.0, 1.0))]);
        assert_eq!(c.get(0).total_ticks, 1000);
        assert_eq!(c.get(1).total_ticks, 51 + 24 + 250);
        // A clock the server never set.
        assert_eq!((c.get(7).total_ticks, c.get(7).rate), (0, 1.0));
        let mut out = vec![c.get(7)];
        c.samples(&mut out);
        assert_eq!(out.len(), 2);
        // Going backwards (the server set an earlier game time) counts back too.
        c.tick(2090);
        assert_eq!(c.get(0).total_ticks, 990);
    }

    #[test]
    fn network_values_read_as_their_files_would() {
        let mut c = Compound::new();
        c.put("flag", Tag::Byte(1));
        c.put("list", Tag::List(vec![Tag::Int(3), Tag::Int(4)]));
        c.put("longs", Tag::LongArray(vec![1 << 40]));
        c.put("half", Tag::Float(0.5));
        let v = nbt_to_json(&Tag::Compound(c));
        assert_eq!(
            v,
            json!({"flag": 1, "list": [3, 4], "longs": [1_i64 << 40], "half": 0.5})
        );
        assert_eq!(truthy(&v["flag"]), Some(true));
        assert_eq!(truthy(&json!(false)), Some(false));
        assert_eq!(truthy(&json!("x")), None);
    }
}

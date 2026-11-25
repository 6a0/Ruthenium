//! Environment attributes: the values the look of a level is drawn from (the sky's
//! and the fog's colours, the fog's distances, where the sun, the moon and the stars
//! stand, the colours and factors of the light map), as they change with the time of
//! day, the weather and the biomes around the camera. 26.3's
//! `EnvironmentAttributeSystem` evaluates them; this is that evaluation for the
//! attributes drawing reads.
//!
//! An attribute's value is built in layers, each changing what the one before left:
//! the dimension type's entry, the biomes' entries (blended around the camera), one
//! layer per timeline that has a track for the attribute (its modifier, with the
//! argument its keyframes give at the timeline's clock), then rain and thunder. The
//! result is clamped into the attribute's range.
//!
//! [`AttributeSystem`] holds a dimension's layers, compiled from its [`Environment`].
//! [`BiomeWeights`] is the blend of biomes around a position, and [`Probe`] what the
//! camera keeps from one client tick to the next, so that values move smoothly between
//! ticks. Arithmetic is single precision in vanilla's order: results equal vanilla's
//! bit for bit (the tests compare with values from the game's own code).

use std::f32::consts::{FRAC_PI_2, PI};

use ruthenium_javamath::mth;
use serde_json::Value as Json;

use crate::environment::Environment;
use crate::scene::ClockSample;

/// How an attribute's values are held and blended (`AttributeTypes`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Float,
    /// A float in degrees: between ticks it turns the shorter way round.
    Angle,
    Rgb,
    Argb,
    /// Not blended: it steps from one value to the next.
    MoonPhase,
}

/// The attributes drawing reads (`EnvironmentAttributes.java:19-95`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Attribute {
    FogColor,
    FogStartDistance,
    FogEndDistance,
    SkyFogEndDistance,
    CloudFogEndDistance,
    WaterFogColor,
    WaterFogStartDistance,
    WaterFogEndDistance,
    SkyColor,
    SunriseSunsetColor,
    CloudColor,
    CloudHeight,
    SunAngle,
    MoonAngle,
    StarAngle,
    MoonPhase,
    StarBrightness,
    BlockLightTint,
    SkyLightColor,
    SkyLightFactor,
    NightVisionColor,
    AmbientLightColor,
    /// How bright the sky counts as where rules ask for a light level, 0 to 15
    /// (`Level.getSkyDarken` is what it lacks to 15). It is the dimension's: no place
    /// has another.
    SkyLightLevel,
}

/// A value of an attribute.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Value {
    Float(f32),
    /// Red, green and blue, 0 to 1 (a modifier may take them outside).
    Rgb([f32; 3]),
    /// Red, green, blue and alpha.
    Argb([f32; 4]),
    /// A phase's index, 0 (full moon) to 7 (`MoonPhase.java:7-14`).
    MoonPhase(u8),
}

impl Value {
    /// The number of a float or angle attribute; 0 for a value of another kind.
    pub fn float(self) -> f32 {
        match self {
            Value::Float(v) => v,
            _ => 0.0,
        }
    }

    /// The colour of an rgb attribute (an argb one's without its alpha); black for a
    /// value of another kind.
    pub fn rgb(self) -> [f32; 3] {
        match self {
            Value::Rgb(c) => c,
            Value::Argb([r, g, b, _]) => [r, g, b],
            _ => [0.0; 3],
        }
    }

    /// The colour of an argb attribute (an rgb one's with alpha 1); clear black for a
    /// value of another kind.
    pub fn argb(self) -> [f32; 4] {
        match self {
            Value::Argb(c) => c,
            Value::Rgb([r, g, b]) => [r, g, b, 1.0],
            _ => [0.0; 4],
        }
    }

    /// The index of a moon phase; 0 for a value of another kind.
    pub fn moon_phase(self) -> u8 {
        match self {
            Value::MoonPhase(index) => index,
            _ => 0,
        }
    }
}

/// A channel of a colour written with 8 bits (`ARGB.java:361`).
const fn channel(bits: u32) -> f32 {
    (bits & 0xFF) as f32 / 255.0
}

/// The colour of a 24-bit number (`ARGB.vector3fFromRGB24`).
const fn rgb24(colour: u32) -> [f32; 3] {
    [channel(colour >> 16), channel(colour >> 8), channel(colour)]
}

/// The colour of a 32-bit number, alpha in the top byte (`ARGB.vector4fFromARGB32`).
const fn argb32(colour: u32) -> [f32; 4] {
    let [r, g, b] = rgb24(colour);
    [r, g, b, channel(colour >> 24)]
}

struct Spec {
    /// The id without its `minecraft:`.
    name: &'static str,
    kind: Kind,
    default: Value,
    /// The least and the greatest value it may have.
    range: Option<(f32, f32)>,
}

const fn spec(name: &'static str, kind: Kind, default: Value) -> Spec {
    Spec {
        name,
        kind,
        default,
        range: None,
    }
}

const fn ranged(name: &'static str, default: f32, min: f32, max: f32) -> Spec {
    Spec {
        name,
        kind: Kind::Float,
        default: Value::Float(default),
        range: Some((min, max)),
    }
}

/// How many attributes there are.
const COUNT: usize = 23;

/// Each attribute's id, type, default and range, in [`Attribute`]'s order
/// (`EnvironmentAttributes.java:19-95`; the ranges are `AttributeRange.java:7-9`).
const SPECS: [Spec; COUNT] = [
    spec("visual/fog_color", Kind::Rgb, Value::Rgb(rgb24(0x000000))),
    spec("visual/fog_start_distance", Kind::Float, Value::Float(0.0)),
    ranged("visual/fog_end_distance", 1024.0, 0.0, f32::INFINITY),
    ranged("visual/sky_fog_end_distance", 512.0, 0.0, f32::INFINITY),
    ranged("visual/cloud_fog_end_distance", 2048.0, 0.0, f32::INFINITY),
    spec(
        "visual/water_fog_color",
        Kind::Rgb,
        Value::Rgb(rgb24(0x050533)),
    ),
    spec(
        "visual/water_fog_start_distance",
        Kind::Float,
        Value::Float(-8.0),
    ),
    ranged("visual/water_fog_end_distance", 96.0, 0.0, f32::INFINITY),
    spec("visual/sky_color", Kind::Rgb, Value::Rgb(rgb24(0x000000))),
    spec(
        "visual/sunrise_sunset_color",
        Kind::Argb,
        Value::Argb(argb32(0)),
    ),
    spec("visual/cloud_color", Kind::Argb, Value::Argb(argb32(0))),
    spec("visual/cloud_height", Kind::Float, Value::Float(192.33)),
    spec("visual/sun_angle", Kind::Angle, Value::Float(0.0)),
    spec("visual/moon_angle", Kind::Angle, Value::Float(0.0)),
    spec("visual/star_angle", Kind::Angle, Value::Float(0.0)),
    spec("visual/moon_phase", Kind::MoonPhase, Value::MoonPhase(0)),
    ranged("visual/star_brightness", 0.0, 0.0, 1.0),
    spec(
        "visual/block_light_tint",
        Kind::Rgb,
        Value::Rgb(rgb24(0xFFD88C)),
    ),
    spec(
        "visual/sky_light_color",
        Kind::Rgb,
        Value::Rgb(rgb24(0xFFFFFF)),
    ),
    ranged("visual/sky_light_factor", 1.0, 0.0, 1.0),
    spec(
        "visual/night_vision_color",
        Kind::Rgb,
        Value::Rgb(rgb24(0x999999)),
    ),
    spec(
        "visual/ambient_light_color",
        Kind::Rgb,
        Value::Rgb(rgb24(0x000000)),
    ),
    ranged("gameplay/sky_light_level", 15.0, 0.0, 15.0),
];

/// The moon's phases by index (`MoonPhase.java:7-14`).
const MOON_PHASES: [&str; 8] = [
    "full_moon",
    "waning_gibbous",
    "third_quarter",
    "waning_crescent",
    "new_moon",
    "waxing_crescent",
    "first_quarter",
    "waxing_gibbous",
];

impl Attribute {
    pub const ALL: [Attribute; COUNT] = [
        Attribute::FogColor,
        Attribute::FogStartDistance,
        Attribute::FogEndDistance,
        Attribute::SkyFogEndDistance,
        Attribute::CloudFogEndDistance,
        Attribute::WaterFogColor,
        Attribute::WaterFogStartDistance,
        Attribute::WaterFogEndDistance,
        Attribute::SkyColor,
        Attribute::SunriseSunsetColor,
        Attribute::CloudColor,
        Attribute::CloudHeight,
        Attribute::SunAngle,
        Attribute::MoonAngle,
        Attribute::StarAngle,
        Attribute::MoonPhase,
        Attribute::StarBrightness,
        Attribute::BlockLightTint,
        Attribute::SkyLightColor,
        Attribute::SkyLightFactor,
        Attribute::NightVisionColor,
        Attribute::AmbientLightColor,
        Attribute::SkyLightLevel,
    ];

    fn spec(self) -> &'static Spec {
        &SPECS[self as usize]
    }

    /// Its id without the namespace (`visual/sky_color`); the namespace is `minecraft`.
    pub fn name(self) -> &'static str {
        self.spec().name
    }

    /// The attribute of an id, with or without its `minecraft:`.
    pub fn from_id(id: &str) -> Option<Attribute> {
        let name = id.strip_prefix("minecraft:").unwrap_or(id);
        Attribute::ALL.into_iter().find(|a| a.name() == name)
    }

    pub fn kind(self) -> Kind {
        self.spec().kind
    }

    /// Its value where nothing sets it.
    pub fn default(self) -> Value {
        self.spec().default
    }

    /// The least and the greatest value it may have, for the ones that are bounded.
    pub fn range(self) -> Option<(f32, f32)> {
        self.spec().range
    }

    /// Whether biomes' values blend around the camera. Otherwise the biome the
    /// position is in gives its own.
    pub fn spatially_interpolated(self) -> bool {
        self != Attribute::MoonPhase
    }

    /// The value brought into the attribute's range (`AttributeRange.java:34`).
    fn clamp(self, value: Value) -> Value {
        match (value, self.range()) {
            (Value::Float(v), Some((min, max))) if !(v >= min && v <= max) => {
                Value::Float(if v < min { min } else { mth::java_min(v, max) })
            }
            _ => value,
        }
    }
}

/// What an entry does to the value before it (`AttributeModifier.java:97-112`; the
/// ones for booleans, lists and spawn settings apply to no attribute here).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Modifier {
    Override,
    Add,
    Subtract,
    Multiply,
    Minimum,
    Maximum,
    AlphaBlend,
    BlendToGray,
}

/// What a modifier is given.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Argument {
    /// A number or a colour (or a moon phase, to override with).
    Plain(Value),
    /// A float to blend towards, and how far.
    Blend { value: f32, alpha: f32 },
    /// How bright the grey is, and how far to blend towards it.
    Gray { brightness: f32, factor: f32 },
}

/// Blends two values of one kind: numbers and colours linearly, channel by channel
/// (alpha too); a moon phase steps to `to` once `t` reaches `step`.
fn mix(t: f32, from: Value, to: Value, step: f32) -> Value {
    match (from, to) {
        (Value::Float(a), Value::Float(b)) => Value::Float(mth::lerp(t, a, b)),
        (Value::Rgb(a), Value::Rgb(b)) => {
            Value::Rgb(std::array::from_fn(|i| mth::lerp(t, a[i], b[i])))
        }
        (Value::Argb(a), Value::Argb(b)) => {
            Value::Argb(std::array::from_fn(|i| mth::lerp(t, a[i], b[i])))
        }
        _ if t >= step => to,
        _ => from,
    }
}

/// Where a moon phase steps along each kind of blend (`AttributeType.java:49-52`):
/// between keyframes only at the end, between biomes halfway, and with the weather
/// or between ticks at once.
const KEYFRAME_STEP: f32 = 1.0;
const SPATIAL_STEP: f32 = 0.5;
const STATE_STEP: f32 = 0.0;
const TICK_STEP: f32 = 0.0;

impl Argument {
    /// The argument between two keyframes: each of its numbers on its own.
    fn between(t: f32, from: Argument, to: Argument) -> Argument {
        match (from, to) {
            (Argument::Plain(a), Argument::Plain(b)) => {
                Argument::Plain(mix(t, a, b, KEYFRAME_STEP))
            }
            (
                Argument::Blend { value, alpha },
                Argument::Blend {
                    value: to_value,
                    alpha: to_alpha,
                },
            ) => Argument::Blend {
                value: mth::lerp(t, value, to_value),
                alpha: mth::lerp(t, alpha, to_alpha),
            },
            (
                Argument::Gray { brightness, factor },
                Argument::Gray {
                    brightness: to_brightness,
                    factor: to_factor,
                },
            ) => Argument::Gray {
                brightness: mth::lerp(t, brightness, to_brightness),
                factor: mth::lerp(t, factor, to_factor),
            },
            _ => from,
        }
    }
}

/// A colour's three channels, and its alpha.
fn split(colour: [f32; 4]) -> ([f32; 3], f32) {
    let [r, g, b, a] = colour;
    ([r, g, b], a)
}

fn join(colour: [f32; 3], alpha: f32) -> [f32; 4] {
    let [r, g, b] = colour;
    [r, g, b, alpha]
}

/// Each channel's sum, at most 1.
fn add(colour: [f32; 3], by: [f32; 3]) -> [f32; 3] {
    std::array::from_fn(|i| mth::java_min(colour[i] + by[i], 1.0))
}

/// Each channel's difference, at least 0.
fn subtract(colour: [f32; 3], by: [f32; 3]) -> [f32; 3] {
    std::array::from_fn(|i| mth::java_max(colour[i] - by[i], 0.0))
}

/// The colour blended towards a grey: its own brightness (red 0.3, green 0.59, blue
/// 0.11: `ARGB.java:184`) times `brightness`, held to 0..1.
fn towards_gray(colour: [f32; 3], brightness: f32, factor: f32) -> [f32; 3] {
    let own = colour[0] * 0.3 + colour[1] * 0.59 + colour[2] * 0.11;
    let gray = mth::java_min(1.0, mth::java_max(own * brightness, 0.0));
    colour.map(|c| mth::lerp(factor, c, gray))
}

impl Modifier {
    /// By its name in files (`AttributeModifier.java:97-112`).
    fn named(name: &str) -> Option<Modifier> {
        Some(match name {
            "override" => Modifier::Override,
            "add" => Modifier::Add,
            "subtract" => Modifier::Subtract,
            "multiply" => Modifier::Multiply,
            "minimum" => Modifier::Minimum,
            "maximum" => Modifier::Maximum,
            "alpha_blend" => Modifier::AlphaBlend,
            "blend_to_gray" => Modifier::BlendToGray,
            _ => return None,
        })
    }

    /// The value it makes of `subject`. An argument of a kind it does not take (which
    /// reading rules out) leaves the subject as it is.
    fn apply(self, subject: Value, argument: Argument) -> Value {
        use Argument::{Blend, Gray, Plain};
        use Value::{Argb, Float, Rgb};
        match (self, subject, argument) {
            (Modifier::Override, _, Plain(value)) => value,

            (Modifier::Add, Float(a), Plain(Float(b))) => Float(a + b),
            (Modifier::Subtract, Float(a), Plain(Float(b))) => Float(a - b),
            (Modifier::Multiply, Float(a), Plain(Float(b))) => Float(a * b),
            (Modifier::Minimum, Float(a), Plain(Float(b))) => Float(mth::java_min(a, b)),
            (Modifier::Maximum, Float(a), Plain(Float(b))) => Float(mth::java_max(a, b)),
            (Modifier::AlphaBlend, Float(a), Blend { value, alpha }) => {
                Float(mth::lerp(alpha, a, value))
            }

            (Modifier::Add, Rgb(c), Plain(Rgb(by))) => Rgb(add(c, by)),
            (Modifier::Subtract, Rgb(c), Plain(Rgb(by))) => Rgb(subtract(c, by)),
            // Vanilla gives the other side back where one is exactly white, which is
            // what the products are too.
            (Modifier::Multiply, Rgb(c), Plain(Rgb(by))) => {
                Rgb(std::array::from_fn(|i| c[i] * by[i]))
            }
            (Modifier::AlphaBlend, Rgb(c), Plain(Argb(over))) => {
                let (over, alpha) = split(over);
                if alpha == 0.0 {
                    Rgb(c)
                } else {
                    Rgb(std::array::from_fn(|i| mth::lerp(alpha, c[i], over[i])))
                }
            }
            (Modifier::BlendToGray, Rgb(c), Gray { brightness, factor }) => {
                Rgb(towards_gray(c, brightness, factor))
            }

            (Modifier::Add, Argb(c), Plain(Rgb(by))) => {
                let (c, alpha) = split(c);
                Argb(join(add(c, by), alpha))
            }
            (Modifier::Subtract, Argb(c), Plain(Rgb(by))) => {
                let (c, alpha) = split(c);
                Argb(join(subtract(c, by), alpha))
            }
            (Modifier::Multiply, Argb(c), Plain(Argb(by))) => {
                Argb(std::array::from_fn(|i| c[i] * by[i]))
            }
            // The usual "over": an opaque colour replaces, a clear one changes nothing
            // (`ARGB.java:224-240`).
            (Modifier::AlphaBlend, Argb(under), Plain(Argb(over))) => {
                let over_alpha = over[3];
                if over_alpha == 1.0 {
                    Argb(over)
                } else if over_alpha == 0.0 {
                    Argb(under)
                } else {
                    let alpha = over_alpha + under[3] * (1.0 - over_alpha);
                    let blend =
                        |i: usize| (over[i] * over_alpha + under[i] * (alpha - over_alpha)) / alpha;
                    Argb([blend(0), blend(1), blend(2), alpha])
                }
            }
            (Modifier::BlendToGray, Argb(c), Gray { brightness, factor }) => {
                let (c, alpha) = split(c);
                // The alpha blends with itself, as vanilla's does.
                Argb(join(
                    towards_gray(c, brightness, factor),
                    mth::lerp(factor, alpha, alpha),
                ))
            }

            _ => subject,
        }
    }
}

/// A number of a file, or of the network (where it may be of any numeric type).
fn number(json: &Json) -> Option<f32> {
    json.as_f64().map(|v| v as f32)
}

/// A number in 0..1 (`Codec.floatRange(0, 1)`).
fn unit(json: &Json) -> Option<f32> {
    number(json).filter(|v| (0.0..=1.0).contains(v))
}

/// A whole number as Java's `intValue` reads one: the low 32 bits of a whole number,
/// a fraction cut off.
fn integer(json: &Json) -> Option<i32> {
    match (json.as_i64(), json.as_u64()) {
        (Some(n), _) => Some(n as i32),
        (None, Some(n)) => Some(n as i32),
        (None, None) => json.as_f64().map(|v| v as i32),
    }
}

/// `#` and exactly `digits` hexadecimal digits (`ExtraCodecs.java:259-278`).
fn hex(text: &str, digits: usize) -> Option<u32> {
    let body = text.strip_prefix('#')?;
    if body.len() != digits || !body.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    u32::from_str_radix(body, 16).ok()
}

/// A list of exactly `N` numbers.
fn floats<const N: usize>(list: &[Json]) -> Option<[f32; N]> {
    if list.len() != N {
        return None;
    }
    let mut out = [0.0; N];
    for (slot, v) in out.iter_mut().zip(list) {
        *slot = number(v)?;
    }
    Some(out)
}

/// A colour without alpha: `#rrggbb`, a number (its low 24 bits), or its three
/// channels as floats, which are taken as they are (`ExtraCodecs.java:142-148`).
fn rgb(json: &Json) -> Option<[f32; 3]> {
    match json {
        Json::String(text) => hex(text, 6).map(rgb24),
        Json::Array(list) => floats(list),
        _ => integer(json).map(|c| rgb24(c as u32)),
    }
}

/// A colour with alpha: `#aarrggbb`, a 32-bit number, or red, green, blue and alpha
/// as floats (`ExtraCodecs.java:143-151`).
fn argb(json: &Json) -> Option<[f32; 4]> {
    match json {
        Json::String(text) => hex(text, 8).map(argb32),
        Json::Array(list) => floats(list),
        _ => integer(json).map(|c| argb32(c as u32)),
    }
}

/// A value of the attribute's type that lies in its range, as a file writes one.
fn plain(attribute: Attribute, json: &Json) -> Option<Value> {
    let value = match attribute.kind() {
        Kind::Float | Kind::Angle => Value::Float(number(json)?),
        Kind::Rgb => Value::Rgb(rgb(json)?),
        Kind::Argb => Value::Argb(argb(json)?),
        Kind::MoonPhase => {
            let name = json.as_str()?;
            Value::MoonPhase(MOON_PHASES.iter().position(|p| *p == name)? as u8)
        }
    };
    match (value, attribute.range()) {
        (Value::Float(v), Some((min, max))) if !(v >= min && v <= max) => None,
        _ => Some(value),
    }
}

/// A modifier's argument for an attribute: what the modifier takes depends on the
/// attribute's type (`AttributeModifier.java:28-66`).
fn argument(attribute: Attribute, modifier: Modifier, json: &Json) -> Result<Argument, String> {
    use Kind::{Angle, Argb, Float, Rgb};
    use Modifier::{Add, AlphaBlend, BlendToGray, Maximum, Minimum, Multiply, Override, Subtract};
    let read = match (modifier, attribute.kind()) {
        (Override, _) => plain(attribute, json).map(Argument::Plain),
        (Add | Subtract | Multiply | Minimum | Maximum, Float | Angle) => {
            number(json).map(|v| Argument::Plain(Value::Float(v)))
        }
        // A bare number blends all the way (`FloatWithAlpha.java:8-24`).
        (AlphaBlend, Float | Angle) => match json {
            Json::Object(fields) => (|| {
                Some(Argument::Blend {
                    value: number(fields.get("value")?)?,
                    alpha: match fields.get("alpha") {
                        Some(alpha) => unit(alpha)?,
                        None => 1.0,
                    },
                })
            })(),
            _ => number(json).map(|value| Argument::Blend { value, alpha: 1.0 }),
        },
        (Add | Subtract, Rgb | Argb) | (Multiply, Rgb) => {
            rgb(json).map(|c| Argument::Plain(Value::Rgb(c)))
        }
        (Multiply, Argb) | (AlphaBlend, Rgb | Argb) => {
            argb(json).map(|c| Argument::Plain(Value::Argb(c)))
        }
        (BlendToGray, Rgb | Argb) => (|| {
            Some(Argument::Gray {
                brightness: unit(json.get("brightness")?)?,
                factor: unit(json.get("factor")?)?,
            })
        })(),
        _ => return Err("the modifier does not apply to its type".into()),
    };
    read.ok_or_else(|| format!("unreadable argument {json}"))
}

/// An entry of a dimension type's or a biome's attributes: a modifier and its argument.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Entry {
    modifier: Modifier,
    argument: Argument,
}

impl Entry {
    /// A bare value overrides; else `{"modifier": name, "argument": x}`.
    fn parse(attribute: Attribute, json: &Json) -> Result<Entry, String> {
        if let Some(value) = plain(attribute, json) {
            return Ok(Entry {
                modifier: Modifier::Override,
                argument: Argument::Plain(value),
            });
        }
        let Json::Object(fields) = json else {
            return Err(format!("unreadable value {json}"));
        };
        let modifier = modifier_of(fields.get("modifier"))?.ok_or("it names no modifier")?;
        let given = fields.get("argument").ok_or("it has no argument")?;
        Ok(Entry {
            modifier,
            argument: argument(attribute, modifier, given)?,
        })
    }

    fn apply(self, subject: Value) -> Value {
        self.modifier.apply(subject, self.argument)
    }
}

/// The modifier a field names; `None` for no field.
fn modifier_of(field: Option<&Json>) -> Result<Option<Modifier>, String> {
    let Some(field) = field else {
        return Ok(None);
    };
    field
        .as_str()
        .and_then(Modifier::named)
        .map(Some)
        .ok_or_else(|| format!("unknown modifier {field}"))
}

/// One of the curves a named easing is the start, the end or both ends of
/// (`EasingType.java:15-44`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Shape {
    Back,
    Bounce,
    Circ,
    Cubic,
    Elastic,
    Expo,
    Quad,
    Quart,
    Quint,
    Sine,
}

/// One coordinate of a cubic Bézier curve from 0 to 1 as a polynomial in the curve's
/// parameter (`EasingType.java:78`).
#[derive(Debug, Clone, Copy, PartialEq)]
struct Cubic {
    a: f32,
    b: f32,
    c: f32,
}

impl Cubic {
    /// Through the two control values between its ends.
    fn through(v1: f32, v2: f32) -> Cubic {
        Cubic {
            a: 3.0 * v1 - 3.0 * v2 + 1.0,
            b: -6.0 * v1 + 3.0 * v2,
            c: 3.0 * v1,
        }
    }

    fn at(self, t: f32) -> f32 {
        ((self.a * t + self.b) * t + self.c) * t
    }

    fn slope(self, t: f32) -> f32 {
        (3.0 * self.a * t + 2.0 * self.b) * t + self.c
    }
}

/// How far along a stretch between two keyframes counts, given how much of its time
/// has passed (`EasingType`).
#[derive(Debug, Clone, Copy, PartialEq)]
enum Ease {
    /// The earlier keyframe's value throughout.
    Constant,
    Linear,
    In(Shape),
    Out(Shape),
    InOut(Shape),
    /// A cubic Bézier curve from (0, 0) to (1, 1): time along x, the result along y.
    Bezier {
        x: Cubic,
        y: Cubic,
    },
}

/// The steps the bouncing curve is made of: 1 / 2.75 wide parabolas of 7.5625 x²
/// (`Ease.java:71-80`).
fn bounce(x: f32) -> f32 {
    fn arc(x: f32, centre: f32, top: f32) -> f32 {
        7.5625 * sq(x - centre) + top
    }
    if x < 0.363_636_37 {
        7.5625 * sq(x)
    } else if x < 0.727_272_75 {
        arc(x, 0.545_454_56, 0.75)
    } else if f64::from(x) < 0.909_090_909_090_909_1 {
        arc(x, 0.818_181_8, 0.9375)
    } else {
        arc(x, 0.954_545_44, 0.984_375)
    }
}

/// The elastic curves' turn per unit of their phase: a third and (for both ends) two
/// ninths of a turn, as the doubles of vanilla's floats (`Ease.java:28, 152`).
const ELASTIC_TURN: f64 = 2.094_395_160_675_049;
const ELASTIC_TURN_BOTH: f64 = 1.396_263_480_186_462_4;

/// The "back" curves' swing past their ends, and one more than it; for both ends the
/// swing is 1.525 times as wide, and the first half takes twice one more than that
/// (`Ease.java:5-6, 162-168`; vanilla writes each as its own float).
const BACK: f32 = 1.701_58;
const BACK_MORE: f32 = 2.701_58;
const BACK_BOTH: f32 = 2.594_909_4;
const BACK_BOTH_MORE: f32 = 3.594_909_4;
const BACK_BOTH_TWICE_MORE: f32 = 7.189_819;

fn sq(x: f32) -> f32 {
    x * x
}

fn cube(x: f32) -> f32 {
    x * x * x
}

/// 2 to a power, in doubles.
fn exp2(power: f64) -> f64 {
    mth::java_pow(2.0, power)
}

/// A named curve at its start: slow first (`Ease`'s `in` functions).
fn ease_in(shape: Shape, x: f32) -> f32 {
    let wide = f64::from(x);
    match shape {
        Shape::Back => sq(x) * (BACK_MORE * x - BACK),
        Shape::Bounce => 1.0 - bounce(1.0 - x),
        Shape::Circ => (-f64::from(1.0 - x * x).sqrt()) as f32 + 1.0,
        Shape::Cubic => cube(x),
        Shape::Elastic if x == 0.0 => 0.0,
        Shape::Elastic if x == 1.0 => 1.0,
        Shape::Elastic => {
            (-exp2(10.0 * wide - 10.0) * mth::java_sin((wide * 10.0 - 10.75) * ELASTIC_TURN)) as f32
        }
        Shape::Expo if x == 0.0 => 0.0,
        Shape::Expo => exp2(10.0 * wide - 10.0) as f32,
        Shape::Quad => x * x,
        Shape::Quart => sq(sq(x)),
        Shape::Quint => sq(sq(x)) * x,
        Shape::Sine => 1.0 - mth::cos(f64::from(x * FRAC_PI_2)),
    }
}

/// A named curve at its end: fast first (`Ease`'s `out` functions).
fn ease_out(shape: Shape, x: f32) -> f32 {
    let wide = f64::from(x);
    match shape {
        Shape::Back => 1.0 + BACK_MORE * cube(x - 1.0) + BACK * sq(x - 1.0),
        Shape::Bounce => bounce(x),
        Shape::Circ => f64::from(1.0 - sq(x - 1.0)).sqrt() as f32,
        Shape::Cubic => 1.0 - cube(1.0 - x),
        Shape::Elastic if x == 0.0 => 0.0,
        Shape::Elastic if x == 1.0 => 1.0,
        Shape::Elastic => {
            (exp2(-10.0 * wide) * mth::java_sin((wide * 10.0 - 0.75) * ELASTIC_TURN) + 1.0) as f32
        }
        Shape::Expo if x == 1.0 => 1.0,
        Shape::Expo => 1.0 - exp2(-10.0 * wide) as f32,
        Shape::Quad => 1.0 - sq(1.0 - x),
        Shape::Quart => 1.0 - sq(sq(1.0 - x)),
        Shape::Quint => 1.0 - mth::java_pow(1.0 - wide, 5.0) as f32,
        Shape::Sine => mth::sin(f64::from(x * FRAC_PI_2)),
    }
}

/// A named curve at both ends: its start up to the middle, its end after (`Ease`'s
/// `inOut` functions). The second halves of the power curves are computed in doubles.
fn ease_in_out(shape: Shape, x: f32) -> f32 {
    let wide = f64::from(x);
    let first = x < 0.5;
    // The second half, mirrored: 2 - 2x.
    let back = -2.0 * wide + 2.0;
    match shape {
        Shape::Back if first => 4.0 * x * x * (BACK_BOTH_TWICE_MORE * x - BACK_BOTH) / 2.0,
        Shape::Back => {
            let d = 2.0 * x - 2.0;
            (d * d * (BACK_BOTH_MORE * d + BACK_BOTH) + 2.0) / 2.0
        }
        Shape::Bounce if first => (1.0 - bounce(1.0 - 2.0 * x)) / 2.0,
        Shape::Bounce => (1.0 + bounce(2.0 * x - 1.0)) / 2.0,
        Shape::Circ if first => ((1.0 - (1.0 - (2.0 * wide) * (2.0 * wide)).sqrt()) / 2.0) as f32,
        Shape::Circ => (((1.0 - back * back).sqrt() + 1.0) / 2.0) as f32,
        Shape::Cubic if first => 4.0 * cube(x),
        Shape::Cubic => (1.0 - mth::java_pow(back, 3.0) / 2.0) as f32,
        Shape::Elastic if x == 0.0 => 0.0,
        Shape::Elastic if x == 1.0 => 1.0,
        Shape::Elastic => {
            let wave = mth::java_sin((20.0 * wide - 11.125) * ELASTIC_TURN_BOTH);
            if first {
                (-(exp2(20.0 * wide - 10.0) * wave) / 2.0) as f32
            } else {
                (exp2(-20.0 * wide + 10.0) * wave / 2.0 + 1.0) as f32
            }
        }
        Shape::Expo if x == 0.0 => 0.0,
        Shape::Expo if x == 1.0 => 1.0,
        Shape::Expo if first => (exp2(20.0 * wide - 10.0) / 2.0) as f32,
        Shape::Expo => ((2.0 - exp2(-20.0 * wide + 10.0)) / 2.0) as f32,
        Shape::Quad if first => 2.0 * sq(x),
        Shape::Quad => (1.0 - back * back / 2.0) as f32,
        Shape::Quart if first => 8.0 * sq(sq(x)),
        Shape::Quart => (1.0 - mth::java_pow(back, 4.0) / 2.0) as f32,
        Shape::Quint if first => 16.0 * x * x * x * x * x,
        Shape::Quint => (1.0 - mth::java_pow(back, 5.0) / 2.0) as f32,
        Shape::Sine => -(mth::cos(f64::from(PI * x)) - 1.0) / 2.0,
    }
}

/// How close to the time asked for the curve's x has to come (`EasingType.java:91`).
const BEZIER_CLOSE: f32 = 1.0e-5;

/// The parameter at which a Bézier curve's x is `time`: up to four Newton steps from
/// `time` itself (each at most a quarter; none where the curve is flat or runs
/// backwards), then halving of 0..1 starting from where those came to
/// (`EasingType.java:86-126`).
fn bezier_parameter(x: Cubic, time: f32) -> f32 {
    let mut t = time;
    for _ in 0..4 {
        let error = x.at(t) - time;
        if error.abs() < BEZIER_CLOSE {
            return t;
        }
        let slope = x.slope(t);
        if slope < BEZIER_CLOSE {
            break;
        }
        let step = error / slope;
        t -= if step < -0.25 {
            -0.25
        } else {
            mth::java_min(step, 0.25)
        };
    }
    let (mut low, mut high) = (0.0f32, 1.0f32);
    while low < high {
        let error = x.at(t) - time;
        if error.abs() < BEZIER_CLOSE {
            break;
        }
        if error < 0.0 {
            low = t;
        } else {
            high = t;
        }
        t = (high + low) / 2.0;
    }
    t
}

impl Ease {
    /// `"linear"`, `"constant"`, `"in_"`, `"out_"` or `"in_out_"` and a curve's name, or
    /// `{"cubic_bezier": [x1, y1, x2, y2]}` with both x in 0..1
    /// (`EasingType.java:11-44, 153-168`).
    fn parse(json: &Json) -> Option<Ease> {
        if let Some(controls) = json.get("cubic_bezier") {
            let [x1, y1, x2, y2] = floats(controls.as_array()?)?;
            let inside = |x: f32| (0.0..=1.0).contains(&x);
            return (inside(x1) && inside(x2)).then(|| Ease::Bezier {
                x: Cubic::through(x1, x2),
                y: Cubic::through(y1, y2),
            });
        }
        let name = json.as_str()?;
        let shape = |name: &str| {
            Some(match name {
                "back" => Shape::Back,
                "bounce" => Shape::Bounce,
                "circ" => Shape::Circ,
                "cubic" => Shape::Cubic,
                "elastic" => Shape::Elastic,
                "expo" => Shape::Expo,
                "quad" => Shape::Quad,
                "quart" => Shape::Quart,
                "quint" => Shape::Quint,
                "sine" => Shape::Sine,
                _ => return None,
            })
        };
        if name == "constant" {
            Some(Ease::Constant)
        } else if name == "linear" {
            Some(Ease::Linear)
        } else if let Some(rest) = name.strip_prefix("in_out_") {
            shape(rest).map(Ease::InOut)
        } else if let Some(rest) = name.strip_prefix("in_") {
            shape(rest).map(Ease::In)
        } else {
            shape(name.strip_prefix("out_")?).map(Ease::Out)
        }
    }

    fn apply(self, x: f32) -> f32 {
        match self {
            Ease::Constant => 0.0,
            Ease::Linear => x,
            Ease::In(shape) => ease_in(shape, x),
            Ease::Out(shape) => ease_out(shape, x),
            Ease::InOut(shape) => ease_in_out(shape, x),
            Ease::Bezier { x: along, y } => y.at(bezier_parameter(along, x)),
        }
    }
}

/// A timeline's track for one attribute: a modifier whose argument moves between
/// keyframes with the timeline's clock (`AttributeTrack`, `KeyframeTrackSampler`).
#[derive(Debug, Clone, PartialEq)]
struct Track {
    /// The clock's id in the `world_clock` registry.
    clock: Option<i32>,
    /// After how many ticks it repeats, if it does.
    period: Option<i32>,
    modifier: Modifier,
    ease: Ease,
    /// The keyframes in the order of their ticks; never empty.
    keys: Vec<(i32, Argument)>,
}

impl Track {
    fn parse(
        attribute: Attribute,
        clock: Option<i32>,
        period: Option<i32>,
        json: &Json,
    ) -> Result<Track, String> {
        let modifier = modifier_of(json.get("modifier"))?.unwrap_or(Modifier::Override);
        let ease = match json.get("ease") {
            Some(ease) => Ease::parse(ease).ok_or_else(|| format!("unknown easing {ease}"))?,
            None => Ease::Linear,
        };
        let list = json
            .get("keyframes")
            .and_then(Json::as_array)
            .ok_or("it has no keyframes")?;
        let mut keys = Vec::with_capacity(list.len());
        for key in list {
            let ticks = key
                .get("ticks")
                .and_then(Json::as_i64)
                .and_then(|t| i32::try_from(t).ok())
                .filter(|t| *t >= 0 && period.is_none_or(|p| *t <= p))
                .ok_or_else(|| format!("a keyframe's ticks are out of range in {key}"))?;
            let value = key.get("value").ok_or("a keyframe has no value")?;
            keys.push((ticks, argument(attribute, modifier, value)?));
        }
        if keys.is_empty() {
            return Err("it has no keyframes".into());
        }
        if !keys.is_sorted_by_key(|k| k.0) {
            return Err("its keyframes are not in the order of their ticks".into());
        }
        Ok(Track {
            clock,
            period,
            modifier,
            ease,
            keys,
        })
    }

    /// The argument when the clock has counted `total_ticks`. A repeating track is read
    /// at the ticks into its period, and its last keyframe leads on to its first of
    /// the next round; a track that does not repeat keeps its first value before its
    /// first keyframe and its last after its last.
    fn argument_at(&self, total_ticks: i64) -> Argument {
        let keys = &self.keys;
        let (first, last) = (keys[0], keys[keys.len() - 1]);
        if keys.len() == 1 {
            return first.1;
        }
        let at = match self.period {
            Some(period) => total_ticks.rem_euclid(i64::from(period)),
            None => total_ticks,
        };
        // The stretch in use is the first that ends after `at`: the one into the first
        // keyframe that is later than it. Of two keyframes on one tick the second
        // counts from that tick on.
        let later = keys.partition_point(|k| i64::from(k.0) <= at);
        let (from, to) = match (self.period, later) {
            // From the round before (`KeyframeTrackSampler.java:30`; Java's int
            // arithmetic).
            (Some(period), 0) => ((last.0.wrapping_sub(period), last.1), first),
            (None, 0) => (first, keys[1]),
            (Some(period), n) if n == keys.len() => (last, (first.0.wrapping_add(period), first.1)),
            (None, n) if n == keys.len() => (keys[n - 2], last),
            (_, n) => (keys[n - 1], keys[n]),
        };
        if at <= i64::from(from.0) {
            return from.1;
        }
        if at >= i64::from(to.0) {
            return to.1;
        }
        let passed = (at - i64::from(from.0)) as f32 / to.0.wrapping_sub(from.0) as f32;
        Argument::between(self.ease.apply(passed), from.1, to.1)
    }
}

/// What rain and what thunder do to an attribute (`WeatherAttributes.java:13-40`).
/// Colours vanilla builds from floats go through 8 bits first (0.5 is 127, 0.6 is 153,
/// 0.25 is 63, 0.3 is 76), and so do the alphas of the sky light's colour (0.3125 is
/// 79, 0.52734375 is 134); the night's sky light is `#7a7aff` (`Timelines.java:30`).
fn weather(attribute: Attribute) -> Option<(Entry, Entry)> {
    let entry = |modifier, argument| Entry { modifier, argument };
    let gray =
        |brightness, factor| entry(Modifier::BlendToGray, Argument::Gray { brightness, factor });
    let blend = |value, alpha| entry(Modifier::AlphaBlend, Argument::Blend { value, alpha });
    let rgb = |modifier, colour| entry(modifier, Argument::Plain(Value::Rgb(rgb24(colour))));
    let argb = |modifier, colour| entry(modifier, Argument::Plain(Value::Argb(argb32(colour))));
    Some(match attribute {
        Attribute::SkyColor => (gray(0.6, 0.75), gray(0.24, 0.94)),
        Attribute::FogColor => (
            rgb(Modifier::Multiply, 0x7F7F99),
            rgb(Modifier::Multiply, 0x3F3F4C),
        ),
        Attribute::CloudColor => (gray(0.24, 0.5), gray(0.095, 0.94)),
        Attribute::SkyLightColor => (
            argb(Modifier::AlphaBlend, 0x4F7A_7AFF),
            argb(Modifier::AlphaBlend, 0x867A_7AFF),
        ),
        Attribute::SkyLightFactor => (blend(0.24, 0.3125), blend(0.24, 0.527_343_75)),
        Attribute::SkyLightLevel => (blend(4.0, 0.3125), blend(4.0, 0.527_343_75)),
        Attribute::StarBrightness => {
            let none = entry(Modifier::Override, Argument::Plain(Value::Float(0.0)));
            (none, none)
        }
        Attribute::SunriseSunsetColor => (
            argb(Modifier::Multiply, 0xFF7F_7F99),
            argb(Modifier::Multiply, 0xFF3F_3F4C),
        ),
        _ => return None,
    })
}

/// A layer of an attribute after the dimension type's own entry.
#[derive(Debug, Clone, PartialEq)]
enum Layer {
    /// Each biome's entry, by the biome's network id.
    Biomes(Vec<Option<Entry>>),
    Track(Track),
    Weather {
        rain: Entry,
        thunder: Entry,
    },
}

#[derive(Debug, Clone, PartialEq)]
struct Layers {
    /// The default with the dimension type's entry applied.
    base: Value,
    layers: Vec<Layer>,
}

/// The biomes around a position, each with how much it counts there: the camera's
/// `SpatialAttributeInterpolator`, which vanilla rebuilds every client tick.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct BiomeWeights {
    /// The quarter cells sampled and their weights, in the order sampled.
    samples: Vec<(u16, f64)>,
}

/// The weights along one axis of the cells around a position: a 1 4 6 4 1 kernel slid
/// by the position's fraction of a cell (`GaussianSampler.java:9`).
const KERNEL: [f64; 7] = [0.0, 1.0, 4.0, 6.0, 4.0, 1.0, 0.0];

impl BiomeWeights {
    /// The 6 x 6 x 6 quarter cells around `position` (in blocks), each weighed by the
    /// kernel along every axis (`GaussianSampler.sample`, with
    /// `EnvironmentAttributeProbe.java:31-35`'s scale). `biome_at_quart` gives the
    /// biome's network id of a quarter cell (x, y, z in units of 4 blocks, as the
    /// level stores biomes, without the jitter block colours use).
    pub fn sample(
        position: [f64; 3],
        mut biome_at_quart: impl FnMut(i32, i32, i32) -> u16,
    ) -> BiomeWeights {
        let centre = position.map(|p| p * 0.25 - 0.5);
        let cell = centre.map(mth::floor);
        // Per axis, the six cells' weights.
        let weights: [[f64; 6]; 3] = std::array::from_fn(|axis| {
            let part = centre[axis] - f64::from(cell[axis]);
            std::array::from_fn(|k| mth::lerp_f64(part, KERNEL[k + 1], KERNEL[k]))
        });
        let at = |axis: usize, k: usize| cell[axis].wrapping_add(k as i32 - 2);
        let mut samples = Vec::with_capacity(216);
        // The order matters: weights sum up per biome as they come.
        for z in 0..6 {
            for x in 0..6 {
                for y in 0..6 {
                    let weight = weights[0][x] * weights[1][y] * weights[2][z];
                    samples.push((biome_at_quart(at(0, x), at(1, y), at(2, z)), weight));
                }
            }
        }
        BiomeWeights { samples }
    }

    /// Each biome's weight, in the order the biomes first came up.
    pub fn weights(&self) -> Vec<(u16, f64)> {
        self.sources(Some)
            .into_iter()
            .filter_map(|(biome, weight)| Some((biome?, weight)))
            .collect()
    }

    /// The weights summed per source, in the order the sources first came up.
    /// `source` names a biome's source; the biomes it gives `None` for are one source
    /// together.
    fn sources(&self, source: impl Fn(u16) -> Option<u16>) -> Vec<(Option<u16>, f64)> {
        let mut sources: Vec<(Option<u16>, f64)> = Vec::new();
        for &(biome, weight) in &self.samples {
            let source = source(biome);
            match sources.iter_mut().find(|s| s.0 == source) {
                Some(found) => found.1 += weight,
                None => sources.push((source, weight)),
            }
        }
        sources
    }
}

/// What an attribute's value is asked at.
#[derive(Debug, Clone, Copy)]
pub struct Sample<'a> {
    /// The world clocks. One that is not listed counts as at 0 ticks.
    pub clocks: &'a [ClockSample],
    /// The level's rain level, 0 to 1.
    pub rain: f32,
    /// The level's thunder level as vanilla reads it (`Level.getThunderLevel`): what
    /// the server set times the rain level.
    pub thunder: f32,
    /// The biomes around the position, for the attributes that blend between biomes.
    pub biomes: Option<&'a BiomeWeights>,
    /// The biome of the position's own quarter cell (its block coordinates `>> 2`),
    /// which gives the other attributes their values, and all of them where there
    /// are no weights.
    pub biome: Option<u16>,
}

/// A dimension's attributes, ready to be evaluated (`EnvironmentAttributeSystem`).
#[derive(Debug, Clone, PartialEq)]
pub struct AttributeSystem {
    attributes: Vec<Layers>,
    /// Per biome by network id: whether its definition has attributes of its own.
    /// The biomes that have none count as one source when biomes blend (they share
    /// vanilla's one empty map).
    own: Vec<bool>,
}

impl AttributeSystem {
    /// The layers of every attribute for a dimension. `weather` says whether its level
    /// can have weather (`Level.canHaveWeather`: [`crate::weather::Weather::possible`]).
    /// An entry that cannot be read is left out, with a line on stderr.
    pub fn new(environment: &Environment, weather: bool) -> AttributeSystem {
        let skipped = |what: &str, why: String| eprintln!("[attributes] {what} skipped: {why}");
        let mut attributes: Vec<Layers> = Attribute::ALL
            .iter()
            .map(|a| Layers {
                base: a.default(),
                layers: Vec::new(),
            })
            .collect();

        for (id, json) in &environment.attributes {
            let Some(attribute) = Attribute::from_id(id) else {
                continue;
            };
            match Entry::parse(attribute, json) {
                Ok(entry) => {
                    let layers = &mut attributes[attribute as usize];
                    layers.base = entry.apply(layers.base);
                }
                Err(why) => skipped(
                    &format!("{id} of dimension type {}", environment.dimension_type),
                    why,
                ),
            }
        }

        let mut biomes: Vec<Option<Vec<Option<Entry>>>> = vec![None; COUNT];
        for (index, biome) in environment.biomes.iter().enumerate() {
            let Some(map) = biome.attributes.as_object() else {
                continue;
            };
            for (id, json) in map {
                let Some(attribute) = Attribute::from_id(id) else {
                    continue;
                };
                match Entry::parse(attribute, json) {
                    Ok(entry) => {
                        biomes[attribute as usize]
                            .get_or_insert_with(|| vec![None; environment.biomes.len()])[index] =
                            Some(entry);
                    }
                    Err(why) => skipped(&format!("{id} of biome {}", biome.id), why),
                }
            }
        }
        for (layers, entries) in attributes.iter_mut().zip(biomes) {
            layers.layers.extend(entries.map(Layer::Biomes));
        }

        for timeline in &environment.timelines {
            let period = match timeline.definition.get("period_ticks") {
                None => None,
                Some(json) => match json.as_i64().and_then(|p| i32::try_from(p).ok()) {
                    Some(period) if period > 0 => Some(period),
                    _ => {
                        skipped(
                            &format!("timeline {}", timeline.id),
                            format!("its period is {json}"),
                        );
                        continue;
                    }
                },
            };
            let Some(tracks) = timeline.definition.get("tracks").and_then(Json::as_object) else {
                continue;
            };
            for (id, json) in tracks {
                let Some(attribute) = Attribute::from_id(id) else {
                    continue;
                };
                match Track::parse(attribute, timeline.clock, period, json) {
                    Ok(track) => attributes[attribute as usize]
                        .layers
                        .push(Layer::Track(track)),
                    Err(why) => skipped(&format!("{id} of timeline {}", timeline.id), why),
                }
            }
        }

        if weather {
            for attribute in Attribute::ALL {
                if let Some((rain, thunder)) = self::weather(attribute) {
                    attributes[attribute as usize]
                        .layers
                        .push(Layer::Weather { rain, thunder });
                }
            }
        }

        AttributeSystem {
            attributes,
            own: environment
                .biomes
                .iter()
                .map(|b| !b.attributes.is_null())
                .collect(),
        }
    }

    /// An attribute's value (`EnvironmentAttributeSystem.getValue`). Where nothing sets
    /// it, that is its default.
    pub fn value(&self, attribute: Attribute, sample: &Sample) -> Value {
        let layers = &self.attributes[attribute as usize];
        let mut value = layers.base;
        for layer in &layers.layers {
            value = match layer {
                Layer::Biomes(entries) => self.among_biomes(attribute, entries, value, sample),
                Layer::Track(track) => {
                    let ticks = sample
                        .clocks
                        .iter()
                        .find(|c| Some(c.clock) == track.clock)
                        .map_or(0, |c| c.total_ticks);
                    track.modifier.apply(value, track.argument_at(ticks))
                }
                Layer::Weather { rain, thunder } => {
                    // Thunder comes with rain: what is left of the rain counts as rain
                    // (`WeatherAttributes.java:55-66`).
                    let raining = sample.rain - sample.thunder;
                    if raining > 0.0 {
                        value = mix(raining, value, rain.apply(value), STATE_STEP);
                    }
                    if sample.thunder > 0.0 {
                        value = mix(sample.thunder, value, thunder.apply(value), STATE_STEP);
                    }
                    value
                }
            };
        }
        attribute.clamp(value)
    }

    /// The biomes' layer: each biome's entry applied to `base`, blended by the weights
    /// around the position; or the position's own biome's, for an attribute that does
    /// not blend or where there are no weights.
    fn among_biomes(
        &self,
        attribute: Attribute,
        entries: &[Option<Entry>],
        base: Value,
        sample: &Sample,
    ) -> Value {
        let of = |biome: Option<u16>| match biome
            .and_then(|b| entries.get(usize::from(b)).copied().flatten())
        {
            Some(entry) => entry.apply(base),
            None => base,
        };
        let Some(weights) = sample.biomes.filter(|_| attribute.spatially_interpolated()) else {
            return of(sample.biome);
        };
        let sources = weights.sources(|biome| {
            self.own
                .get(usize::from(biome))
                .is_none_or(|own| *own)
                .then_some(biome)
        });
        // A running mean: each source takes its share of the weight so far
        // (`SpatialAttributeInterpolator.java:29-45`).
        let mut total = 0.0;
        let mut blend: Option<Value> = None;
        for (source, weight) in sources {
            total += weight;
            let value = of(source);
            blend = Some(match blend {
                None => value,
                Some(so_far) => mix((weight / total) as f32, so_far, value, SPATIAL_STEP),
            });
        }
        blend.unwrap_or(base)
    }
}

/// What the camera keeps of the attributes from one client tick to the next
/// (`EnvironmentAttributeProbe`), so that drawing can move from the last tick's values
/// to this one's with the part of the tick that has passed.
///
/// Vanilla computes an attribute's new value when it is first read after a tick, and
/// forgets an attribute that was not read during a whole tick (its next read starts
/// from the value of then). This computes every attribute at the tick.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Probe {
    /// Per attribute, the value at the tick before the last and at the last.
    ticks: Option<[(Value, Value); COUNT]>,
}

impl Probe {
    /// A client tick: the new values become the last ones, and `sample` gives the new.
    /// The first tick makes both the same.
    pub fn tick(&mut self, system: &AttributeSystem, sample: &Sample) {
        let before = self.ticks;
        self.ticks = Some(std::array::from_fn(|i| {
            let new = system.value(Attribute::ALL[i], sample);
            (before.map_or(new, |ticks| ticks[i].1), new)
        }));
    }

    /// An attribute's value `partial_tick` of the way from the last tick to this one;
    /// its default before the first tick. An angle turns the shorter way, and jumps
    /// where that is a quarter turn or more (`LerpFunction.java:22-27`,
    /// `AttributeTypes.java:27`); a moon phase is the new one.
    pub fn value(&self, attribute: Attribute, partial_tick: f32) -> Value {
        let Some(ticks) = &self.ticks else {
            return attribute.default();
        };
        match (attribute.kind(), ticks[attribute as usize]) {
            (Kind::Angle, (Value::Float(last), Value::Float(new))) => {
                let turn = mth::wrap_degrees(new - last);
                Value::Float(if turn.abs() >= 90.0 {
                    new
                } else {
                    last + partial_tick * turn
                })
            }
            (_, (last, new)) => mix(partial_tick, last, new, TICK_STEP),
        }
    }

    /// Forgets the level: values are the defaults until the next tick.
    pub fn reset(&mut self) {
        self.ticks = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::environment::{BiomeLook, Timeline};
    use serde_json::json;

    include!("attributes_golden.rs");

    fn clock(clock: i32, total_ticks: i64) -> ClockSample {
        ClockSample {
            clock,
            total_ticks,
            partial_tick: 0.0,
            rate: 1.0,
        }
    }

    /// A sample at the clocks' ticks, in clear weather, with no biome.
    fn at(clocks: &[ClockSample]) -> Sample<'_> {
        Sample {
            clocks,
            rain: 0.0,
            thunder: 0.0,
            biomes: None,
            biome: None,
        }
    }

    fn dimension(attributes: Json) -> Environment {
        Environment {
            attributes: attributes.as_object().expect("an object").clone(),
            ..Environment::default()
        }
    }

    fn biome(attributes: Json) -> BiomeLook {
        BiomeLook {
            attributes,
            ..BiomeLook::default()
        }
    }

    fn timeline(clock: i32, definition: Json) -> Timeline {
        Timeline {
            id: "test:timeline".into(),
            clock: Some(clock),
            definition,
        }
    }

    /// An attribute's value for a dimension type's attributes alone.
    fn of(attributes: Json, attribute: Attribute) -> Value {
        AttributeSystem::new(&dimension(attributes), false).value(attribute, &at(&[]))
    }

    fn eighths(c: [u8; 3]) -> Value {
        Value::Rgb(c.map(|c| f32::from(c) / 255.0))
    }

    /// The blend the expectations below are written with.
    fn lerp(t: f32, from: f32, to: f32) -> f32 {
        from + t * (to - from)
    }

    #[test]
    fn nothing_set_gives_the_defaults() {
        let system = AttributeSystem::new(&Environment::default(), false);
        let value = |a| system.value(a, &at(&[]));
        assert_eq!(value(Attribute::FogEndDistance), Value::Float(1024.0));
        assert_eq!(value(Attribute::SkyFogEndDistance).float(), 512.0);
        assert_eq!(value(Attribute::CloudFogEndDistance).float(), 2048.0);
        assert_eq!(value(Attribute::WaterFogStartDistance).float(), -8.0);
        assert_eq!(value(Attribute::WaterFogEndDistance).float(), 96.0);
        assert_eq!(value(Attribute::CloudHeight).float(), 192.33);
        assert_eq!(value(Attribute::SkyLightFactor).float(), 1.0);
        assert_eq!(value(Attribute::WaterFogColor), eighths([5, 5, 51]));
        assert_eq!(value(Attribute::BlockLightTint), eighths([255, 216, 140]));
        assert_eq!(value(Attribute::SkyLightColor).rgb(), [1.0; 3]);
        assert_eq!(value(Attribute::NightVisionColor), eighths([153; 3]));
        assert_eq!(value(Attribute::CloudColor).argb(), [0.0; 4]);
        assert_eq!(value(Attribute::MoonPhase).moon_phase(), 0);
        for a in [
            Attribute::FogColor,
            Attribute::SkyColor,
            Attribute::AmbientLightColor,
        ] {
            assert_eq!(value(a).rgb(), [0.0; 3]);
        }
        for a in [
            Attribute::FogStartDistance,
            Attribute::SunAngle,
            Attribute::MoonAngle,
            Attribute::StarAngle,
            Attribute::StarBrightness,
        ] {
            assert_eq!(value(a).float(), 0.0);
        }
    }

    #[test]
    fn values_read_as_their_kinds() {
        assert_eq!(Value::Float(2.5).float(), 2.5);
        assert_eq!(Value::Rgb([0.25, 0.5, 0.75]).rgb(), [0.25, 0.5, 0.75]);
        assert_eq!(
            Value::Argb([0.25, 0.5, 0.75, 0.125]).argb(),
            [0.25, 0.5, 0.75, 0.125]
        );
        assert_eq!(Value::MoonPhase(6).moon_phase(), 6);
        // A colour reads either way: without its alpha, or as opaque.
        assert_eq!(
            Value::Argb([0.25, 0.5, 0.75, 0.125]).rgb(),
            [0.25, 0.5, 0.75]
        );
        assert_eq!(Value::Rgb([0.25, 0.5, 0.75]).argb(), [0.25, 0.5, 0.75, 1.0]);
        // Anything else reads as nothing.
        assert_eq!(Value::MoonPhase(6).float(), 0.0);
        assert_eq!(Value::Float(2.5).rgb(), [0.0; 3]);
        assert_eq!(Value::Float(2.5).argb(), [0.0; 4]);
        assert_eq!(Value::Float(2.5).moon_phase(), 0);
    }

    #[test]
    fn attributes_go_by_their_ids() {
        let ids = [
            "visual/fog_color",
            "visual/fog_start_distance",
            "visual/fog_end_distance",
            "visual/sky_fog_end_distance",
            "visual/cloud_fog_end_distance",
            "visual/water_fog_color",
            "visual/water_fog_start_distance",
            "visual/water_fog_end_distance",
            "visual/sky_color",
            "visual/sunrise_sunset_color",
            "visual/cloud_color",
            "visual/cloud_height",
            "visual/sun_angle",
            "visual/moon_angle",
            "visual/star_angle",
            "visual/moon_phase",
            "visual/star_brightness",
            "visual/block_light_tint",
            "visual/sky_light_color",
            "visual/sky_light_factor",
            "visual/night_vision_color",
            "visual/ambient_light_color",
            "gameplay/sky_light_level",
        ];
        for (id, attribute) in ids.into_iter().zip(Attribute::ALL) {
            assert_eq!(Attribute::from_id(id), Some(attribute));
            assert_eq!(
                Attribute::from_id(&format!("minecraft:{id}")),
                Some(attribute)
            );
            assert_eq!(Attribute::ALL[attribute as usize], attribute);
        }
        assert_eq!(
            Attribute::from_id("minecraft:gameplay/water_evaporates"),
            None
        );
        assert_eq!(Attribute::from_id("other:visual/sky_color"), None);
        // Only the moon's phase keeps to the position's own biome.
        assert!(
            Attribute::ALL
                .iter()
                .all(|a| a.spatially_interpolated() == (a.kind() != Kind::MoonPhase))
        );
    }

    #[test]
    fn colours_read_as_files_and_servers_write_them() {
        let sky = |v: Json| {
            of(
                json!({"minecraft:visual/sky_color": v}),
                Attribute::SkyColor,
            )
        };
        assert_eq!(sky(json!("#78a7ff")), eighths([0x78, 0xa7, 0xff]));
        assert_eq!(sky(json!("#78A7FF")), eighths([0x78, 0xa7, 0xff]));
        assert_eq!(sky(json!(0x78a7ff)), eighths([0x78, 0xa7, 0xff]));
        // A number's alpha byte does not count (-16448205 is 0xFF050533).
        assert_eq!(sky(json!(-16448205)), eighths([5, 5, 51]));
        // Floats are taken as they are, in or out of 0..1.
        assert_eq!(sky(json!([0.25, 1.5, -1.0])), Value::Rgb([0.25, 1.5, -1.0]));
        let cloud = |v: Json| {
            of(
                json!({"minecraft:visual/cloud_color": v}),
                Attribute::CloudColor,
            )
        };
        let expected = Value::Argb([1.0, 1.0, 1.0, 204.0 / 255.0]);
        assert_eq!(cloud(json!("#ccffffff")), expected);
        assert_eq!(cloud(json!(0xccff_ffff_u32)), expected);
        assert_eq!(cloud(json!(0xccff_ffff_u32 as i32)), expected);
        assert_eq!(
            cloud(json!([0.5, 0.25, 0.125, 0.75])),
            Value::Argb([0.5, 0.25, 0.125, 0.75])
        );
        // What cannot be read leaves the default: a colour of the wrong length, of
        // the other kind, or not a colour.
        for bad in [
            json!("#78a7f"),
            json!("78a7ff"),
            json!("#78a7fg"),
            json!("#+8a7ff"),
            json!("#ff78a7ff"),
            json!([0.5, 0.5]),
            json!(true),
        ] {
            assert_eq!(sky(bad.clone()).rgb(), [0.0; 3], "{bad}");
        }
        assert_eq!(cloud(json!("#ffffff")).argb(), [0.0; 4]);
        let phase = |v: Json| of(json!({"visual/moon_phase": v}), Attribute::MoonPhase);
        assert_eq!(phase(json!("full_moon")).moon_phase(), 0);
        assert_eq!(phase(json!("waning_gibbous")).moon_phase(), 1);
        assert_eq!(phase(json!("third_quarter")).moon_phase(), 2);
        assert_eq!(phase(json!("waning_crescent")).moon_phase(), 3);
        assert_eq!(phase(json!("new_moon")).moon_phase(), 4);
        assert_eq!(phase(json!("waxing_crescent")).moon_phase(), 5);
        assert_eq!(phase(json!("first_quarter")).moon_phase(), 6);
        assert_eq!(phase(json!("waxing_gibbous")).moon_phase(), 7);
        assert_eq!(phase(json!("blue_moon")).moon_phase(), 0);
    }

    #[test]
    fn a_dimensions_entry_changes_the_default() {
        let fog_end = |v: Json| {
            of(
                json!({"minecraft:visual/fog_end_distance": v}),
                Attribute::FogEndDistance,
            )
            .float()
        };
        let with = |modifier: &str, argument: Json| {
            fog_end(json!({"modifier": modifier, "argument": argument}))
        };
        assert_eq!(fog_end(json!(96.0)), 96.0);
        // A whole number, as a server may send it.
        assert_eq!(fog_end(json!(96)), 96.0);
        assert_eq!(with("override", json!(3.5)), 3.5);
        assert_eq!(with("add", json!(0.5)), 1024.5);
        assert_eq!(with("subtract", json!(24)), 1000.0);
        assert_eq!(with("multiply", json!(0.25)), 256.0);
        assert_eq!(with("minimum", json!(100)), 100.0);
        assert_eq!(with("minimum", json!(2000)), 1024.0);
        assert_eq!(with("maximum", json!(2000)), 2000.0);
        assert_eq!(with("maximum", json!(100)), 1024.0);
        // Blending towards a value: all the way for a bare number.
        assert_eq!(with("alpha_blend", json!(24.0)), 24.0);
        assert_eq!(with("alpha_blend", json!({"value": 24.0})), 24.0);
        assert_eq!(
            with("alpha_blend", json!({"value": 24.0, "alpha": 0.25})),
            774.0
        );
        // The result is held to the attribute's range, whatever the modifier gave.
        assert_eq!(with("subtract", json!(5000)), 0.0);
        assert_eq!(with("multiply", json!(-1)), 0.0);
        let stars = |v: Json| {
            of(
                json!({"minecraft:visual/star_brightness": v}),
                Attribute::StarBrightness,
            )
            .float()
        };
        assert_eq!(stars(json!({"modifier": "add", "argument": 7})), 1.0);
        assert_eq!(stars(json!({"modifier": "add", "argument": -7})), 0.0);
        assert_eq!(stars(json!(0.5)), 0.5);
        let ranged = |id: &str, attribute: Attribute, by: f32| {
            of(json!({id: {"modifier": "add", "argument": by}}), attribute).float()
        };
        for (id, attribute) in [
            ("visual/sky_fog_end_distance", Attribute::SkyFogEndDistance),
            (
                "visual/cloud_fog_end_distance",
                Attribute::CloudFogEndDistance,
            ),
            (
                "visual/water_fog_end_distance",
                Attribute::WaterFogEndDistance,
            ),
        ] {
            assert_eq!(ranged(id, attribute, -1.0e6), 0.0);
            assert!(ranged(id, attribute, 1.0e6) > 1.0e5);
        }
        assert_eq!(
            ranged("visual/sky_light_factor", Attribute::SkyLightFactor, 5.0),
            1.0
        );
        assert_eq!(
            ranged("visual/sky_light_factor", Attribute::SkyLightFactor, -5.0),
            0.0
        );
        // An attribute without a range is not held to anything.
        assert_eq!(
            of(
                json!({"minecraft:visual/fog_start_distance": -500.0}),
                Attribute::FogStartDistance
            )
            .float(),
            -500.0
        );
        // What vanilla would not read is left out: a value outside the range, a
        // modifier that is not one, one that does not apply to floats, a blend
        // further than all the way, no argument.
        assert_eq!(fog_end(json!(-1.0)), 1024.0);
        assert_eq!(stars(json!(1.5)), 0.0);
        assert_eq!(with("override", json!(-1.0)), 1024.0);
        assert_eq!(with("divide", json!(2.0)), 1024.0);
        assert_eq!(with("blend_to_gray", json!(2.0)), 1024.0);
        assert_eq!(with("and", json!(true)), 1024.0);
        assert_eq!(
            with("alpha_blend", json!({"value": 24.0, "alpha": 1.5})),
            1024.0
        );
        assert_eq!(fog_end(json!({"modifier": "add"})), 1024.0);
        assert_eq!(fog_end(json!({"argument": 3.0})), 1024.0);
        assert_eq!(fog_end(json!("far")), 1024.0);
    }

    /// A modifier applied to a colour, with the argument as a file writes it.
    fn modified(attribute: Attribute, subject: Value, modifier: &str, given: Json) -> Value {
        let modifier = Modifier::named(modifier).expect("a modifier");
        modifier.apply(
            subject,
            argument(attribute, modifier, &given).expect("an argument"),
        )
    }

    #[test]
    fn colour_modifiers() {
        let rgb = |subject: [f32; 3], modifier: &str, given: Json| {
            modified(Attribute::SkyColor, Value::Rgb(subject), modifier, given).rgb()
        };
        let c = [0.5, 0.25, 0.75];
        assert_eq!(rgb(c, "add", json!([0.25, 0.25, 0.5])), [0.75, 0.5, 1.0]);
        assert_eq!(
            rgb(c, "subtract", json!([0.25, 0.5, 0.25])),
            [0.25, 0.0, 0.5]
        );
        assert_eq!(rgb(c, "multiply", json!([0.5, 2.0, 0.0])), [0.25, 0.5, 0.0]);
        assert_eq!(rgb(c, "multiply", json!("#ffffff")), c);
        // Towards the argument's colour by its alpha; not at all for alpha 0.
        assert_eq!(
            rgb(c, "alpha_blend", json!([1.0, 0.75, 0.25, 0.5])),
            [0.75, 0.5, 0.5]
        );
        assert_eq!(rgb(c, "alpha_blend", json!("#00ffffff")), c);
        assert_eq!(rgb(c, "alpha_blend", json!("#ffffffff")), [1.0; 3]);
        // Its own brightness: red 0.3, green 0.59, blue 0.11.
        let gray =
            |brightness: f32, factor: f32| json!({"brightness": brightness, "factor": factor});
        let towards = |c: [f32; 3], gray: f32, factor: f32| c.map(|c| lerp(factor, c, gray));
        for (c, own) in [
            ([1.0, 0.0, 0.0], 0.3),
            ([0.0, 1.0, 0.0], 0.59),
            ([0.0, 0.0, 1.0], 0.11),
        ] {
            assert_eq!(
                rgb(c, "blend_to_gray", gray(1.0, 1.0)),
                towards(c, own, 1.0)
            );
            assert_eq!(
                rgb(c, "blend_to_gray", gray(0.5, 1.0)),
                towards(c, own * 0.5, 1.0)
            );
            assert_eq!(
                rgb(c, "blend_to_gray", gray(1.0, 0.25)),
                towards(c, own, 0.25)
            );
            assert_eq!(rgb(c, "blend_to_gray", gray(1.0, 0.0)), c);
        }
        assert_eq!(
            rgb([0.5, 0.25, 1.0], "blend_to_gray", gray(1.0, 1.0)),
            [0.5 * 0.3 + 0.25 * 0.59 + 0.11; 3]
        );
        // The grey is held to 0..1 before the blend.
        assert_eq!(
            rgb([4.0, 0.0, 0.0], "blend_to_gray", gray(1.0, 1.0)),
            [1.0; 3]
        );
        assert_eq!(
            rgb([-4.0, 0.0, 0.0], "blend_to_gray", gray(1.0, 1.0)),
            [0.0; 3]
        );
        assert!(argument(Attribute::SkyColor, Modifier::BlendToGray, &gray(1.5, 0.5)).is_err());
        assert!(
            argument(
                Attribute::SkyColor,
                Modifier::BlendToGray,
                &json!({"factor": 0.5})
            )
            .is_err()
        );
        assert!(argument(Attribute::SkyColor, Modifier::Minimum, &json!("#ffffff")).is_err());

        let argb = |subject: [f32; 4], modifier: &str, given: Json| {
            modified(Attribute::CloudColor, Value::Argb(subject), modifier, given).argb()
        };
        let c = [0.5, 0.25, 0.75, 0.5];
        // Adding and subtracting leave the alpha alone.
        assert_eq!(
            argb(c, "add", json!([0.25, 0.25, 0.5])),
            [0.75, 0.5, 1.0, 0.5]
        );
        assert_eq!(
            argb(c, "subtract", json!([0.25, 0.5, 0.25])),
            [0.25, 0.0, 0.5, 0.5]
        );
        assert_eq!(
            argb(c, "multiply", json!([0.5, 2.0, 0.0, 0.5])),
            [0.25, 0.5, 0.0, 0.25]
        );
        assert_eq!(
            argb([0.0, 1.0, 0.0, 0.25], "blend_to_gray", gray(1.0, 0.5)),
            [
                lerp(0.5, 0.0, 0.59),
                lerp(0.5, 1.0, 0.59),
                lerp(0.5, 0.0, 0.59),
                0.25
            ]
        );
        // Over: alpha 0.5 over alpha 0.5 is alpha 0.75, the colours weighed 2 to 1.
        assert_eq!(
            argb(c, "alpha_blend", json!([1.0, 1.0, 0.0, 0.5])),
            [
                (1.0 * 0.5 + 0.5 * 0.25) / 0.75,
                (1.0 * 0.5 + 0.25 * 0.25) / 0.75,
                (0.0 * 0.5 + 0.75 * 0.25) / 0.75,
                0.75
            ]
        );
        assert_eq!(argb(c, "alpha_blend", json!("#00123456")), c);
        assert_eq!(
            argb(c, "alpha_blend", json!([0.125, 0.25, 0.5, 1.0])),
            [0.125, 0.25, 0.5, 1.0]
        );
        assert!(argument(Attribute::CloudColor, Modifier::Multiply, &json!("#ffffff")).is_err());
        assert!(argument(Attribute::CloudColor, Modifier::Add, &json!("#ffffffff")).is_err());
        // A moon phase can only be set.
        assert!(argument(Attribute::MoonPhase, Modifier::Add, &json!("new_moon")).is_err());
    }

    fn track(period: Option<i32>, definition: Json) -> Track {
        Track::parse(Attribute::SunAngle, Some(0), period, &definition).expect("a track")
    }

    fn angle_at(track: &Track, ticks: i64) -> f32 {
        match track.argument_at(ticks) {
            Argument::Plain(Value::Float(v)) => v,
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_repeating_track_runs_from_its_last_keyframe_round_to_its_first() {
        // The day's sun, with a straight line for its curve: two keyframes on tick
        // 6000, 360 before and 0 from then on.
        let sun = track(
            Some(24000),
            json!({"keyframes": [{"ticks": 6000, "value": 360.0}, {"ticks": 6000, "value": 0.0}]}),
        );
        assert_eq!(angle_at(&sun, 6000), 0.0);
        assert_eq!(angle_at(&sun, 5999), 23999.0 / 24000.0 * 360.0);
        assert_eq!(angle_at(&sun, 6001), 1.0 / 24000.0 * 360.0);
        assert_eq!(angle_at(&sun, 18000), 180.0);
        assert_eq!(angle_at(&sun, 0), 270.0);
        assert_eq!(angle_at(&sun, 23999), 17999.0 / 24000.0 * 360.0);
        // The ticks into the period count, also before tick 0.
        assert_eq!(angle_at(&sun, 24000 * 5 + 18000), 180.0);
        assert_eq!(angle_at(&sun, -6000), 180.0);
        assert_eq!(angle_at(&sun, -1), 17999.0 / 24000.0 * 360.0);

        let three = track(
            Some(100),
            json!({"keyframes": [
                {"ticks": 10, "value": 1.0}, {"ticks": 20, "value": 2.0}, {"ticks": 60, "value": 4.0}]}),
        );
        for (ticks, value) in [
            (10, 1.0),
            (15, 1.5),
            (20, 2.0),
            (40, 3.0),
            (60, 4.0),
            // From the last (4 at 60) to the first of the next round (1 at 110).
            (85, 2.5),
            (0, lerp(40.0 / 50.0, 4.0, 1.0)),
            (5, lerp(45.0 / 50.0, 4.0, 1.0)),
            (205, lerp(45.0 / 50.0, 4.0, 1.0)),
        ] {
            assert_eq!(angle_at(&three, ticks), value, "at {ticks}");
        }
    }

    #[test]
    fn a_track_without_a_period_holds_its_ends() {
        let t = track(
            None,
            json!({"keyframes": [
                {"ticks": 10, "value": 1.0}, {"ticks": 20, "value": 2.0}, {"ticks": 20, "value": 5.0},
                {"ticks": 60, "value": 7.0}]}),
        );
        for (ticks, value) in [
            (i64::MIN, 1.0),
            (-5, 1.0),
            (0, 1.0),
            (10, 1.0),
            (15, 1.5),
            (19, lerp(9.0 / 10.0, 1.0, 2.0)),
            // Of two keyframes on one tick the second counts from that tick on.
            (20, 5.0),
            (40, 6.0),
            (60, 7.0),
            (61, 7.0),
            (i64::MAX, 7.0),
        ] {
            assert_eq!(angle_at(&t, ticks), value, "at {ticks}");
        }
        // On a keyframe's own tick the value is the keyframe's itself, a negative zero
        // too (a blend from it by nothing would make it positive).
        let zero = track(
            None,
            json!({"keyframes": [{"ticks": 5, "value": -0.0}, {"ticks": 10, "value": 5.0}]}),
        );
        assert!(angle_at(&zero, 5).is_sign_negative() && angle_at(&zero, 6) == 1.0);
        let one = track(None, json!({"keyframes": [{"ticks": 10, "value": 3.0}]}));
        assert_eq!(
            (angle_at(&one, -1), angle_at(&one, 10), angle_at(&one, 99)),
            (3.0, 3.0, 3.0)
        );
        let one = track(
            Some(50),
            json!({"keyframes": [{"ticks": 10, "value": 3.0}]}),
        );
        assert_eq!((angle_at(&one, 0), angle_at(&one, 49)), (3.0, 3.0));
    }

    #[test]
    fn tracks_vanilla_would_not_read_are_refused() {
        let parse = |period, definition: Json| {
            Track::parse(Attribute::SunAngle, Some(0), period, &definition)
        };
        let key = |ticks: i64| json!({"ticks": ticks, "value": 1.0});
        assert!(parse(None, json!({"keyframes": [key(5), key(5), key(9)]})).is_ok());
        assert!(parse(None, json!({"keyframes": []})).is_err());
        assert!(parse(None, json!({})).is_err());
        assert!(parse(None, json!({"keyframes": [key(9), key(5)]})).is_err());
        assert!(parse(None, json!({"keyframes": [key(-1)]})).is_err());
        assert!(parse(Some(8), json!({"keyframes": [key(8)]})).is_ok());
        assert!(parse(Some(8), json!({"keyframes": [key(9)]})).is_err());
        assert!(parse(None, json!({"keyframes": [{"ticks": 1}]})).is_err());
        assert!(parse(None, json!({"keyframes": [{"ticks": 1, "value": "x"}]})).is_err());
        assert!(parse(None, json!({"keyframes": [key(1)], "ease": "in_wobble"})).is_err());
        assert!(parse(None, json!({"keyframes": [key(1)], "modifier": "xor"})).is_err());
        assert!(
            parse(
                None,
                json!({"keyframes": [key(1)], "modifier": "blend_to_gray"})
            )
            .is_err()
        );
    }

    #[test]
    fn what_does_not_blend_steps_at_the_next_keyframe() {
        // The moon: a phase every 24000 ticks, eight of them.
        let keys: Vec<Json> = MOON_PHASES
            .iter()
            .enumerate()
            .map(|(i, name)| json!({"ticks": i * 24000, "value": name}))
            .collect();
        let moon = Track::parse(
            Attribute::MoonPhase,
            Some(0),
            Some(192_000),
            &json!({"keyframes": keys}),
        )
        .expect("a track");
        let phase = |ticks: i64| match moon.argument_at(ticks) {
            Argument::Plain(Value::MoonPhase(index)) => index,
            other => panic!("{other:?}"),
        };
        assert_eq!(
            [0, 1, 23_999, 24_000, 47_999, 48_000, 191_999, 192_000].map(phase),
            [0, 0, 0, 1, 1, 2, 7, 0]
        );
        // A constant easing holds a number the same way.
        let held = track(
            None,
            json!({"ease": "constant", "keyframes": [
                {"ticks": 0, "value": 1.0}, {"ticks": 10, "value": 2.0}]}),
        );
        assert_eq!(
            [0, 5, 9, 10].map(|t| angle_at(&held, t)),
            [1.0, 1.0, 1.0, 2.0]
        );
    }

    #[test]
    fn easings() {
        let ease = |name: &str| Ease::parse(&json!(name)).unwrap_or_else(|| panic!("{name}"));
        let shapes = [
            "back", "bounce", "circ", "cubic", "elastic", "expo", "quad", "quart", "quint", "sine",
        ];
        // Every curve starts at 0 and ends at 1, whichever end it is slow at.
        for shape in shapes {
            for side in ["in", "out", "in_out"] {
                let e = ease(&format!("{side}_{shape}"));
                let (start, end) = (e.apply(0.0), e.apply(1.0));
                // (The sine curves read a table, which is a little off at a quarter turn.)
                assert!(
                    start.abs() < 2e-4 && (end - 1.0).abs() < 2e-4,
                    "{side}_{shape}: {start} {end}"
                );
            }
        }
        assert_eq!(ease("linear").apply(0.3), 0.3);
        assert_eq!(ease("constant").apply(0.999), 0.0);
        assert_eq!(ease("in_quad").apply(0.5), 0.25);
        assert_eq!(ease("out_quad").apply(0.5), 0.75);
        assert_eq!(ease("in_out_quad").apply(0.25), 0.125);
        assert_eq!(ease("in_out_quad").apply(0.75), 0.875);
        assert_eq!(ease("in_cubic").apply(0.5), 0.125);
        assert_eq!(ease("out_cubic").apply(0.5), 0.875);
        assert_eq!(ease("in_out_cubic").apply(0.25), 0.0625);
        assert_eq!(ease("in_out_cubic").apply(0.75), 0.9375);
        assert_eq!(ease("in_quart").apply(0.5), 0.0625);
        assert_eq!(ease("out_quart").apply(0.5), 0.9375);
        assert_eq!(ease("in_out_quart").apply(0.25), 0.03125);
        assert_eq!(ease("in_out_quart").apply(0.75), 0.96875);
        assert_eq!(ease("in_quint").apply(0.5), 0.03125);
        assert_eq!(ease("out_quint").apply(0.5), 0.96875);
        assert_eq!(ease("in_out_quint").apply(0.25), 0.015625);
        assert_eq!(ease("in_out_quint").apply(0.75), 0.984375);
        assert_eq!(ease("in_expo").apply(0.5), 0.03125);
        assert_eq!(ease("out_expo").apply(0.5), 0.96875);
        assert_eq!(ease("in_out_expo").apply(0.25), 0.015625);
        assert_eq!(ease("in_out_expo").apply(0.75), 0.984375);
        assert_eq!(ease("in_out_circ").apply(0.5), 0.5);
        assert_eq!(ease("in_out_bounce").apply(0.5), 0.5);
        assert!((ease("in_out_back").apply(0.5) - 0.5).abs() < 1e-6);
        assert!((ease("in_out_elastic").apply(0.5) - 0.5).abs() < 1e-6);
        // A circle's arc: at 0.6 it has risen 0.2, and from the other end 0.8.
        assert!((ease("in_circ").apply(0.6) - 0.2).abs() < 1e-6);
        assert!((ease("out_circ").apply(0.4) - 0.8).abs() < 1e-6);
        // The sine curves read the game's sine table.
        assert!((ease("out_sine").apply(0.5) - 0.5f32.sqrt()).abs() < 1e-4);
        assert!((ease("in_sine").apply(0.5) - (1.0 - 0.5f32.sqrt())).abs() < 1e-4);
        assert!((ease("in_out_sine").apply(0.5) - 0.5).abs() < 1e-4);
        // The bounce's steps: full height at the end of each.
        let bounce = ease("out_bounce");
        assert_eq!(bounce.apply(0.25), 7.5625 * 0.0625);
        assert_eq!(bounce.apply(0.545_454_56), 0.75);
        assert_eq!(bounce.apply(0.818_181_8), 0.9375);
        assert_eq!(bounce.apply(0.954_545_44), 0.984_375);
        assert_eq!(ease("in_bounce").apply(0.75), 1.0 - 7.5625 * 0.0625);
        // The third step ends at 10/11 as a double, which the nearest float is still
        // below: it is on the third step, a little under the 1 the fourth has there.
        assert_eq!(bounce.apply(0.909_090_9), 0.999_999_94);
        // The back curves swing past their ends, the elastic ones around them.
        assert!(ease("in_back").apply(0.25) < 0.0 && ease("out_back").apply(0.75) > 1.0);
        assert!(ease("in_out_back").apply(0.125) < 0.0 && ease("in_out_back").apply(0.875) > 1.0);
        assert!(ease("out_elastic").apply(0.125) > 1.0 && ease("in_elastic").apply(0.875) < 0.0);

        for unknown in [
            "",
            "in",
            "in_",
            "ease_in_quad",
            "out_in_quad",
            "Linear",
            "in_out",
        ] {
            assert_eq!(Ease::parse(&json!(unknown)), None, "{unknown}");
        }
        assert_eq!(Ease::parse(&json!(3)), None);
    }

    #[test]
    fn cubic_bezier_easing() {
        let bezier = |controls: [f32; 4]| Ease::parse(&json!({"cubic_bezier": controls}));
        // Control points on the diagonal make a straight line.
        let line = bezier([0.25, 0.25, 0.75, 0.75]).expect("an easing");
        for x in [0.0, 0.125, 0.5, 0.75, 1.0] {
            assert!((line.apply(x) - x).abs() < 2e-5, "{x}: {}", line.apply(x));
        }
        // The sky's curve (`Timelines.java:39`) is symmetric about its middle.
        let sky = bezier([0.362, 0.241, 0.638, 0.759]).expect("an easing");
        assert!((sky.apply(0.5) - 0.5).abs() < 2e-5);
        assert!((sky.apply(0.25) + sky.apply(0.75) - 1.0).abs() < 4e-5);
        assert!(sky.apply(0.25) < 0.25);
        // The y controls may lie anywhere, the x controls only in 0..1.
        assert!(bezier([0.0, -2.0, 1.0, 3.0]).is_some());
        assert_eq!(bezier([-0.1, 0.0, 1.0, 1.0]), None);
        assert_eq!(bezier([0.0, 0.0, 1.1, 1.0]), None);
        assert_eq!(Ease::parse(&json!({"cubic_bezier": [0.0, 0.0, 1.0]})), None);
        // A curve that is flat at its start (x is the cube of the parameter): Newton's
        // steps cannot leave it, and the search by halving finds the place.
        let x = Cubic::through(0.0, 0.0);
        assert_eq!((x.at(0.5), x.slope(0.5), x.slope(0.0)), (0.125, 0.75, 0.0));
        let t = bezier_parameter(x, 0.001);
        assert!((t - 0.1).abs() < 1e-3 && (x.at(t) - 0.001).abs() < BEZIER_CLOSE);
        let flat = bezier([0.0, 0.0, 0.0, 1.0]).expect("an easing");
        assert!((flat.apply(0.001) - Cubic::through(0.0, 1.0).at(0.1)).abs() < 1e-3);
    }

    #[test]
    fn timelines_apply_in_their_order_at_their_clocks() {
        let add = json!({"clock": "c", "tracks": {"minecraft:visual/cloud_height": {
            "modifier": "add", "keyframes": [{"ticks": 0, "value": 0.0}, {"ticks": 100, "value": 100.0}]}}});
        let double = json!({"clock": "c", "period_ticks": 10, "tracks": {"visual/cloud_height": {
            "modifier": "multiply", "ease": "constant",
            "keyframes": [{"ticks": 0, "value": 2.0}, {"ticks": 5, "value": 3.0}]}}});
        let mut env = dimension(json!({"minecraft:visual/cloud_height": 10.0}));
        env.timelines = vec![timeline(0, add.clone()), timeline(1, double.clone())];
        let system = AttributeSystem::new(&env, false);
        let height = |system: &AttributeSystem, clocks: &[ClockSample]| {
            system.value(Attribute::CloudHeight, &at(clocks)).float()
        };
        // Each timeline at its own clock: (10 + 50) × 3.
        assert_eq!(height(&system, &[clock(0, 50), clock(1, 7)]), 180.0);
        assert_eq!(height(&system, &[clock(1, 12), clock(0, 50)]), 120.0);
        // A clock the server has not set stands at 0.
        assert_eq!(height(&system, &[clock(1, 7)]), 30.0);
        assert_eq!(height(&system, &[]), 20.0);
        // The other way round: 10 × 3 + 50.
        env.timelines.reverse();
        let system = AttributeSystem::new(&env, false);
        assert_eq!(height(&system, &[clock(0, 50), clock(1, 7)]), 80.0);
        // A timeline whose period cannot be read is left out, and so is a track that
        // cannot; the others stay.
        env.timelines = vec![
            timeline(1, json!({"period_ticks": 0, "tracks": double["tracks"]})),
            timeline(
                0,
                json!({"tracks": {
                "minecraft:visual/cloud_height": {"keyframes": []},
                "minecraft:gameplay/monsters_burn": {"keyframes": []},
                "minecraft:visual/fog_start_distance": {"keyframes": [{"ticks": 3, "value": 4.0}]}}}),
            ),
        ];
        let system = AttributeSystem::new(&env, false);
        assert_eq!(height(&system, &[clock(0, 50), clock(1, 7)]), 10.0);
        assert_eq!(
            system.value(Attribute::FogStartDistance, &at(&[])).float(),
            4.0
        );
    }

    #[test]
    fn the_kernel_weighs_six_cells_a_side() {
        // The middle of quarter cell (0, 0, 0)... is half a cell from its corner: the
        // kernel sits on the corner between cells -1 and 0.
        let mut calls = Vec::new();
        let weights = BiomeWeights::sample([0.0, 4.0, -4.0], |x, y, z| {
            calls.push([x, y, z]);
            0
        });
        assert_eq!(calls.len(), 216);
        // Cells -3..=2 around x 0, -2..=3 around y 4, -4..=1 around z -4; y runs
        // fastest, then x, then z.
        assert_eq!(calls[0], [-3, -2, -4]);
        assert_eq!(calls[1], [-3, -1, -4]);
        assert_eq!(calls[6], [-2, -2, -4]);
        assert_eq!(calls[36], [-3, -2, -3]);
        assert_eq!(calls[215], [2, 3, 1]);
        // Half way between two cells: 0.5 2.5 5 5 2.5 0.5 along each axis.
        assert_eq!(weights.weights(), vec![(0, 16.0 * 16.0 * 16.0)]);
        let along_x = BiomeWeights::sample([0.0, 4.0, -4.0], |x, _, _| (x + 3) as u16);
        assert_eq!(
            along_x.weights(),
            [0.5, 2.5, 5.0, 5.0, 2.5, 0.5]
                .iter()
                .enumerate()
                .map(|(i, w)| (i as u16, w * 256.0))
                .collect::<Vec<_>>()
        );
        // In the middle of a cell (block 2 of 0..4): 1 4 6 4 1 around it, and the
        // sixth cell counts nothing but is still asked for.
        let centred = BiomeWeights::sample([2.0, 2.0, 2.0], |_, y, _| (y + 2) as u16);
        assert_eq!(
            centred.weights(),
            [1.0, 4.0, 6.0, 4.0, 1.0, 0.0]
                .iter()
                .enumerate()
                .map(|(i, w)| (i as u16, w * 256.0))
                .collect::<Vec<_>>()
        );
        // A quarter of a cell further: the kernel slides with the position.
        let slid = BiomeWeights::sample([3.0, 2.0, 2.0], |x, _, _| (x + 2) as u16);
        let expected = [0.75, 3.25, 5.5, 4.5, 1.75, 0.25];
        for (i, (biome, weight)) in slid.weights().into_iter().enumerate() {
            assert_eq!((biome, weight), (i as u16, expected[i] * 256.0));
        }
        // Biomes are listed in the order they first come up.
        let order = BiomeWeights::sample([2.0, 2.0, 2.0], |x, y, z| match (x, y, z) {
            (-2, -2, -2) => 7,
            (-2, -1, -2) => 3,
            _ => 5,
        });
        let biomes: Vec<u16> = order.weights().iter().map(|w| w.0).collect();
        assert_eq!(biomes, [7, 3, 5]);
    }

    #[test]
    fn biomes_blend_around_the_position() {
        let mut env = dimension(json!({
            "minecraft:visual/fog_end_distance": 100.0,
            "minecraft:visual/moon_phase": "new_moon",
        }));
        env.biomes = vec![
            biome(json!({"minecraft:visual/fog_end_distance": 200.0})),
            biome(json!({
                "minecraft:visual/fog_end_distance": {"modifier": "multiply", "argument": 5.0},
                "minecraft:visual/moon_phase": "first_quarter",
                "minecraft:gameplay/snow_golem_melts": true,
            })),
            biome(Json::Null),
        ];
        let system = AttributeSystem::new(&env, false);
        let value = |attribute: Attribute, biomes: Option<&BiomeWeights>, biome: Option<u16>| {
            system.value(
                attribute,
                &Sample {
                    biomes,
                    biome,
                    ..at(&[])
                },
            )
        };
        let fog = |biomes: Option<&BiomeWeights>, biome: Option<u16>| {
            value(Attribute::FogEndDistance, biomes, biome).float()
        };
        // The position's own biome where there are no weights: its entry applied to
        // the dimension's value, which stays for a biome without one.
        assert_eq!(fog(None, Some(0)), 200.0);
        assert_eq!(fog(None, Some(1)), 500.0);
        assert_eq!(fog(None, Some(2)), 100.0);
        assert_eq!(fog(None, Some(9)), 100.0);
        assert_eq!(fog(None, None), 100.0);
        // One biome all around is that biome's value.
        let all = |b: u16| BiomeWeights::sample([0.0; 3], |_, _, _| b);
        assert_eq!(fog(Some(&all(1)), Some(0)), 500.0);
        // Two biomes meeting at the position, half each.
        let halves = BiomeWeights::sample([0.0; 3], |x, _, _| u16::from(x >= 0));
        assert_eq!(fog(Some(&halves), Some(0)), 350.0);
        // Three in thirds would be a mean too; here biome 2 has a quarter (the cells
        // on one side of z among those on one side of x).
        let three = BiomeWeights::sample([0.0; 3], |x, _, z| match (x >= 0, z >= 0) {
            (false, _) => 0,
            (true, false) => 1,
            (true, true) => 2,
        });
        assert_eq!(
            fog(Some(&three), Some(0)),
            0.5 * 200.0 + 0.25 * 500.0 + 0.25 * 100.0
        );
        // The moon's phase does not blend: the position's own biome sets it.
        let phase = |biomes: Option<&BiomeWeights>, biome: Option<u16>| {
            value(Attribute::MoonPhase, biomes, biome).moon_phase()
        };
        assert_eq!(phase(Some(&halves), Some(1)), 6);
        assert_eq!(phase(Some(&halves), Some(0)), 4);
        assert_eq!(phase(Some(&all(1)), None), 4);
        // An attribute no biome has an entry for has no layer of biomes.
        assert_eq!(
            value(Attribute::CloudHeight, Some(&halves), Some(1)).float(),
            192.33
        );
    }

    #[test]
    fn biomes_without_attributes_count_as_one() {
        // Biomes 0 and 2 have no attributes, 1 has. As sources of a blend, 0 and 2 are
        // one: their weights sum up as their cells come, where the first came up.
        let around = |position: [f64; 3], biomes: u16| {
            BiomeWeights::sample(position, |x, _, z| (x + z).rem_euclid(3) as u16 % biomes)
        };
        let position = [1.3, 0.7, 2.9];
        let weights = around(position, 3);
        let sources = weights.sources(|biome| (biome == 1).then_some(biome));
        // The same arrangement with biome 0 in place of biome 2.
        let merged = around(position, 2).weights();
        assert_eq!(merged.len(), 2);
        assert_eq!(
            sources,
            merged
                .iter()
                .map(|&(biome, weight)| ((biome == 1).then_some(biome), weight))
                .collect::<Vec<_>>()
        );
        assert_eq!(weights.weights().len(), 3);

        let tints = |others: Json| {
            let mut env = dimension(json!({"minecraft:visual/sky_color": [0.1, 0.2, 0.3]}));
            env.biomes = vec![
                biome(others.clone()),
                biome(json!({"minecraft:visual/sky_color": [0.7, 0.9, 0.35]})),
                biome(others),
            ];
            AttributeSystem::new(&env, false)
        };
        let sky = |system: &AttributeSystem, weights: &BiomeWeights| {
            system
                .value(
                    Attribute::SkyColor,
                    &Sample {
                        biomes: Some(weights),
                        ..at(&[])
                    },
                )
                .rgb()
        };
        // The mean is taken source by source, each new one by its share of the weight
        // so far: one step for 0 and 2 together, two for each on its own (which a
        // biome with attributes is, even with none that count here). The means are
        // the same but for their rounding.
        let (shared, own) = (tints(Json::Null), tints(json!({})));
        let mut differ = 0;
        for position in [
            [1.3, 0.7, 2.9],
            [5.1, 0.0, -7.7],
            [-3.3, 9.0, 0.2],
            [8.6, 1.0, 4.4],
        ] {
            let (three, two) = (around(position, 3), around(position, 2));
            assert_eq!(sky(&shared, &three), sky(&own, &two));
            assert_eq!(sky(&shared, &three), sky(&shared, &two));
            let apart = sky(&own, &three);
            for (a, b) in sky(&shared, &three).iter().zip(apart) {
                assert!((a - b).abs() < 1e-6);
                differ += u32::from(*a != b);
            }
        }
        assert!(differ > 0);
    }

    #[test]
    fn rain_then_thunder() {
        let env = dimension(json!({
            "minecraft:visual/fog_color": "#ffffff",
            "minecraft:visual/sky_color": "#ff0000",
            "minecraft:visual/star_brightness": 0.5,
            "minecraft:visual/sky_light_factor": 1.0,
            "minecraft:visual/sky_light_color": "#000000",
            "minecraft:visual/sunrise_sunset_color": "#80ffffff",
            "minecraft:visual/cloud_color": "#8000ff00",
            "minecraft:visual/cloud_height": 100.0,
        }));
        let system = AttributeSystem::new(&env, true);
        let value = |system: &AttributeSystem, attribute, rain: f32, thunder: f32| {
            system.value(
                attribute,
                &Sample {
                    rain,
                    thunder,
                    ..at(&[])
                },
            )
        };
        let fog = |rain, thunder| value(&system, Attribute::FogColor, rain, thunder).rgb();
        // What rain and thunder multiply the fog by, in 255ths.
        let rainy = [127.0, 127.0, 153.0].map(|c: f32| c / 255.0);
        let stormy = [63.0, 63.0, 76.0].map(|c: f32| c / 255.0);
        assert_eq!(fog(0.0, 0.0), [1.0; 3]);
        assert_eq!(fog(1.0, 0.0), rainy.map(|c| lerp(1.0, 1.0, c)));
        // Thunder takes its share of the rain: all of it here.
        assert_eq!(fog(1.0, 1.0), stormy.map(|c| lerp(1.0, 1.0, c)));
        // Half of each: half way to the rain's colour, then half way to what thunder
        // makes of that.
        let half = rainy.map(|c| lerp(0.5, 1.0, c));
        let stormed: [f32; 3] = std::array::from_fn(|i| lerp(0.5, half[i], half[i] * stormy[i]));
        assert_eq!(fog(1.0, 0.5), stormed);
        assert_ne!(fog(1.0, 0.5), fog(0.5, 0.5));
        // More thunder than rain leaves no rain of its own.
        assert_eq!(fog(0.25, 0.5), stormy.map(|c| lerp(0.5, 1.0, c)));

        let float = |attribute, rain, thunder| value(&system, attribute, rain, thunder).float();
        assert_eq!(float(Attribute::StarBrightness, 1.0, 0.0), 0.0);
        assert_eq!(float(Attribute::StarBrightness, 0.5, 0.0), 0.25);
        assert_eq!(float(Attribute::StarBrightness, 1.0, 1.0), 0.0);
        assert_eq!(
            float(Attribute::SkyLightFactor, 1.0, 0.0),
            lerp(1.0, 1.0, lerp(0.3125, 1.0, 0.24))
        );
        assert_eq!(
            float(Attribute::SkyLightFactor, 1.0, 1.0),
            lerp(1.0, 1.0, lerp(0.527_343_75, 1.0, 0.24))
        );
        // The sky light's colour goes towards the night's by 79 and 134 of 255.
        assert_eq!(
            value(&system, Attribute::SkyLightColor, 1.0, 0.0).rgb(),
            [122.0, 122.0, 255.0].map(|c| 79.0 / 255.0 * (c / 255.0))
        );
        assert_eq!(
            value(&system, Attribute::SkyLightColor, 1.0, 1.0).rgb(),
            [122.0, 122.0, 255.0].map(|c| 134.0 / 255.0 * (c / 255.0))
        );
        // Red is 0.3 bright: the rain's grey is 0.6 of that, three quarters of the way;
        // the thunder's 0.24 of it, nearly all the way.
        let red = [1.0, 0.0, 0.0];
        assert_eq!(
            value(&system, Attribute::SkyColor, 1.0, 0.0).rgb(),
            red.map(|c| lerp(1.0, c, lerp(0.75, c, 0.3 * 0.6)))
        );
        assert_eq!(
            value(&system, Attribute::SkyColor, 1.0, 1.0).rgb(),
            red.map(|c| lerp(1.0, c, lerp(0.94, c, 0.3 * 0.24)))
        );
        // Rain first, then thunder on what rain left: each greys the colour it finds.
        let own = |c: [f32; 3]| c[0] * 0.3 + c[1] * 0.59 + c[2] * 0.11;
        let rained = red.map(|c| lerp(0.5, c, lerp(0.75, c, own(red) * 0.6)));
        let stormed = rained.map(|c| lerp(0.5, c, lerp(0.94, c, own(rained) * 0.24)));
        assert_eq!(value(&system, Attribute::SkyColor, 1.0, 0.5).rgb(), stormed);
        let half = 128.0 / 255.0;
        let sunrise =
            |rain, thunder| value(&system, Attribute::SunriseSunsetColor, rain, thunder).argb();
        assert_eq!(sunrise(1.0, 0.0)[..3], rainy.map(|c| lerp(1.0, 1.0, c)));
        assert_eq!(sunrise(1.0, 0.0)[3], half);
        assert_eq!(sunrise(1.0, 1.0)[..3], stormy.map(|c| lerp(1.0, 1.0, c)));
        // Green is 0.59 bright; the clouds keep their alpha.
        let green = [0.0, 1.0, 0.0];
        let clouds = |rain, thunder| value(&system, Attribute::CloudColor, rain, thunder).argb();
        assert_eq!(
            clouds(1.0, 0.0)[..3],
            green.map(|c| lerp(1.0, c, lerp(0.5, c, 0.59 * 0.24)))
        );
        assert_eq!(
            clouds(1.0, 1.0)[..3],
            green.map(|c| lerp(1.0, c, lerp(0.94, c, 0.59 * 0.095)))
        );
        assert_eq!((clouds(1.0, 0.0)[3], clouds(1.0, 1.0)[3]), (half, half));
        // What the weather does not touch, and a level that has none.
        assert_eq!(float(Attribute::CloudHeight, 1.0, 1.0), 100.0);
        let dry = AttributeSystem::new(&env, false);
        assert_eq!(value(&dry, Attribute::FogColor, 1.0, 1.0).rgb(), [1.0; 3]);
        assert_eq!(
            value(&dry, Attribute::StarBrightness, 1.0, 0.0).float(),
            0.5
        );
        // The sky's light level goes from 15 towards 4, less far in rain than in thunder.
        assert_eq!(
            value(&dry, Attribute::SkyLightLevel, 1.0, 1.0).float(),
            15.0
        );
        let wet = AttributeSystem::new(&Environment::default(), true);
        for (thunder, alpha) in [(0.0, 0.3125), (1.0, 0.527_343_75)] {
            assert_eq!(
                value(&wet, Attribute::SkyLightLevel, 1.0, thunder).float(),
                lerp(alpha, 15.0, 4.0)
            );
        }
        // Weather alone sets an attribute: its default goes through it.
        let bare = AttributeSystem::new(&Environment::default(), true);
        assert_eq!(
            value(&bare, Attribute::SkyLightColor, 1.0, 0.0).rgb(),
            [122.0, 122.0, 255.0].map(|c: f32| lerp(1.0, 1.0, lerp(79.0 / 255.0, 1.0, c / 255.0)))
        );
        assert_eq!(
            value(&bare, Attribute::CloudHeight, 1.0, 0.0).float(),
            192.33
        );
    }

    #[test]
    fn the_layers_come_in_order() {
        // The dimension's value, the biome's entry on it, the timeline's on that, the
        // weather last; then the range.
        let mut env = dimension(json!({"minecraft:visual/star_brightness": 0.25}));
        env.biomes = vec![biome(
            json!({"minecraft:visual/star_brightness": {"modifier": "add", "argument": 0.25}}),
        )];
        env.timelines = vec![timeline(
            0,
            json!({"tracks": {"minecraft:visual/star_brightness": {
                "modifier": "multiply", "keyframes": [{"ticks": 0, "value": 4.0}]}}}),
        )];
        let system = AttributeSystem::new(&env, true);
        let stars = |rain: f32| {
            system
                .value(
                    Attribute::StarBrightness,
                    &Sample {
                        rain,
                        biome: Some(0),
                        ..at(&[])
                    },
                )
                .float()
        };
        // (0.25 + 0.25) × 4 = 2, held to 1; in half rain half way from 2 to 0.
        assert_eq!(stars(0.0), 1.0);
        assert_eq!(stars(0.5), 1.0);
        assert_eq!(stars(0.75), 0.5);
    }

    #[test]
    fn the_probe_moves_between_ticks() {
        let mut env = dimension(json!({}));
        env.timelines = vec![timeline(
            0,
            json!({"tracks": {
                "minecraft:visual/cloud_height": {"keyframes": [
                    {"ticks": 0, "value": 0.0}, {"ticks": 128, "value": 128.0}]},
                "minecraft:visual/sun_angle": {"keyframes": [
                    {"ticks": 0, "value": 350.0}, {"ticks": 10, "value": 0.0},
                    {"ticks": 20, "value": 90.0}, {"ticks": 30, "value": 179.0}]},
                "minecraft:visual/moon_phase": {"keyframes": [
                    {"ticks": 0, "value": "full_moon"}, {"ticks": 10, "value": "new_moon"}]},
                "minecraft:visual/sky_color": {"keyframes": [
                    {"ticks": 0, "value": [0.0, 0.5, 1.0]}, {"ticks": 10, "value": [1.0, 0.5, 0.0]}]},
                "minecraft:visual/cloud_color": {"keyframes": [
                    {"ticks": 0, "value": [0.0, 0.0, 0.0, 0.0]}, {"ticks": 10, "value": [1.0, 0.5, 0.0, 1.0]}]},
            }}),
        )];
        let system = AttributeSystem::new(&env, false);
        let mut probe = Probe::default();
        let tick = |probe: &mut Probe, ticks: i64| probe.tick(&system, &at(&[clock(0, ticks)]));
        // Before any tick: the defaults.
        assert_eq!(probe.value(Attribute::CloudHeight, 0.5).float(), 192.33);
        assert_eq!(probe.value(Attribute::FogEndDistance, 0.5).float(), 1024.0);
        // The first tick has nothing to come from.
        tick(&mut probe, 0);
        assert_eq!(probe.value(Attribute::CloudHeight, 0.0).float(), 0.0);
        assert_eq!(probe.value(Attribute::CloudHeight, 0.75).float(), 0.0);
        assert_eq!(probe.value(Attribute::SunAngle, 0.5).float(), 350.0);
        tick(&mut probe, 10);
        assert_eq!(probe.value(Attribute::CloudHeight, 0.0).float(), 0.0);
        assert_eq!(probe.value(Attribute::CloudHeight, 0.25).float(), 2.5);
        assert_eq!(probe.value(Attribute::CloudHeight, 1.0).float(), 10.0);
        assert_eq!(
            probe.value(Attribute::SkyColor, 0.25).rgb(),
            [0.25, 0.5, 0.75]
        );
        assert_eq!(
            probe.value(Attribute::CloudColor, 0.5).argb(),
            [0.5, 0.25, 0.0, 0.5]
        );
        // An angle turns the shorter way: from 350 on through 360 to 0.
        assert_eq!(probe.value(Attribute::SunAngle, 0.5).float(), 355.0);
        assert_eq!(probe.value(Attribute::SunAngle, 1.0).float(), 360.0);
        // A moon phase is the new one at once.
        assert_eq!(probe.value(Attribute::MoonPhase, 0.0).moon_phase(), 4);
        // A quarter turn or more in one tick is a jump.
        tick(&mut probe, 20);
        assert_eq!(probe.value(Attribute::SunAngle, 0.0).float(), 90.0);
        assert_eq!(probe.value(Attribute::SunAngle, 0.5).float(), 90.0);
        tick(&mut probe, 30);
        assert_eq!(probe.value(Attribute::SunAngle, 0.5).float(), 134.5);
        assert_eq!(probe.value(Attribute::CloudHeight, 0.5).float(), 25.0);
        // Ticks at the same time: the value stands.
        tick(&mut probe, 30);
        assert_eq!(probe.value(Attribute::CloudHeight, 0.5).float(), 30.0);
        probe.reset();
        assert_eq!(probe.value(Attribute::CloudHeight, 0.5).float(), 192.33);
        tick(&mut probe, 50);
        assert_eq!(probe.value(Attribute::CloudHeight, 0.5).float(), 50.0);
    }

    #[test]
    fn what_does_not_blend_steps_by_the_kind_of_blend() {
        let (old, new) = (Value::MoonPhase(1), Value::MoonPhase(2));
        // Between keyframes at the end only, between biomes half way, with the weather
        // and between ticks at once.
        assert_eq!(mix(0.999, old, new, KEYFRAME_STEP), old);
        assert_eq!(mix(1.0, old, new, KEYFRAME_STEP), new);
        assert_eq!(mix(0.499, old, new, SPATIAL_STEP), old);
        assert_eq!(mix(0.5, old, new, SPATIAL_STEP), new);
        assert_eq!(mix(0.0, old, new, STATE_STEP), new);
        assert_eq!(mix(-0.001, old, new, STATE_STEP), old);
        assert_eq!(mix(0.0, old, new, TICK_STEP), new);
        assert_eq!(mix(f32::NAN, old, new, TICK_STEP), old);
    }

    #[test]
    fn the_part_of_a_stretch_that_has_passed_is_a_quotient_of_floats() {
        // Both the ticks passed and the stretch's length become floats first: 2^24 + 1
        // of 2^24 + 3 ticks is 2^24 / (2^24 + 4).
        let long = track(
            None,
            json!({"keyframes": [{"ticks": 5, "value": 0.0}, {"ticks": 16_777_224, "value": 1.0}]}),
        );
        assert_eq!(angle_at(&long, 16_777_222), 16_777_216.0 / 16_777_220.0);
        assert_ne!(
            angle_at(&long, 16_777_222),
            (16_777_217.0_f64 / 16_777_219.0) as f32
        );
    }

    #[test]
    fn a_cells_weight_is_its_three_axes_weights_multiplied_in_order() {
        let kernel = [0.0, 1.0, 4.0, 6.0, 4.0, 1.0, 0.0];
        let position = [1.3, 70.1, -5.7];
        // Every cell its own biome, numbered as they are asked for.
        let mut asked = 0;
        let weights = BiomeWeights::sample(position, |_, _, _| {
            asked += 1;
            asked - 1
        })
        .weights();
        assert_eq!(weights.len(), 216);
        let along = |axis: usize, k: usize| {
            let centre = position[axis] * 0.25 - 0.5;
            let part = centre - centre.floor();
            kernel[k + 1] + part * (kernel[k] - kernel[k + 1])
        };
        for (i, (biome, weight)) in weights.into_iter().enumerate() {
            let (z, x, y) = (i / 36, i / 6 % 6, i % 6);
            assert_eq!(usize::from(biome), i);
            assert_eq!(weight, along(0, x) * along(1, y) * along(2, z), "cell {i}");
        }
    }

    #[test]
    fn zeros_keep_their_sign_as_in_vanilla() {
        // A clear colour over another leaves it as it is, a negative zero too (a blend
        // by nothing would make it positive).
        let under = Value::Rgb([-0.0, 0.5, 1.0]);
        let over = Argument::Plain(Value::Argb([1.0, 1.0, 1.0, 0.0]));
        let Value::Rgb(kept) = Modifier::AlphaBlend.apply(under, over) else {
            panic!("not a colour");
        };
        assert!(kept[0] == 0.0 && kept[0].is_sign_negative());
        // An opaque colour over another is that colour itself, where the sum of the
        // two parts would make a negative zero positive.
        let opaque = Value::Argb([-0.0, 0.5, 1.0, 1.0]);
        let covered = Modifier::AlphaBlend.apply(Value::Argb([0.5; 4]), Argument::Plain(opaque));
        assert!(covered == opaque && covered.argb()[0].is_sign_negative());
        // Blending towards grey passes the alpha through a blend with itself, which
        // makes a negative zero positive.
        let subject = Value::Argb([0.5, 0.5, 0.5, -0.0]);
        let gray = Argument::Gray {
            brightness: 1.0,
            factor: 0.5,
        };
        let alpha = Modifier::BlendToGray.apply(subject, gray).argb()[3];
        assert!(alpha == 0.0 && alpha.is_sign_positive());
        // The grey is not below a positive zero.
        let dark = Modifier::BlendToGray.apply(
            Value::Rgb([-1.0, 0.0, 0.0]),
            Argument::Gray {
                brightness: 0.0,
                factor: 1.0,
            },
        );
        assert!(dark.rgb().iter().all(|c| *c == 0.0 && c.is_sign_positive()));
    }

    // The generator the golden values' program draws its cases from, and the tables it
    // goes by, written out again here so that the tables above are checked too.

    /// The attributes the golden values are of: the visual ones, which come first.
    const VISUAL: usize = 22;

    /// 0 a float, 1 an angle, 2 a colour, 3 a colour with alpha, 4 a moon phase.
    const KINDS: [u8; VISUAL] = [
        2, 0, 0, 0, 0, 2, 0, 0, 2, 3, 3, 0, 1, 1, 1, 4, 0, 2, 2, 0, 2, 2,
    ];
    /// What a plain value may be: 0 any, 1 not negative, 2 from 0 to 1.
    const RANGES: [u8; VISUAL] = [
        0, 0, 1, 1, 1, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 2, 0, 0, 2, 0, 0,
    ];
    const FLOAT_MODIFIERS: [&str; 7] = [
        "override",
        "add",
        "subtract",
        "multiply",
        "minimum",
        "maximum",
        "alpha_blend",
    ];
    const COLOUR_MODIFIERS: [&str; 6] = [
        "override",
        "add",
        "subtract",
        "multiply",
        "alpha_blend",
        "blend_to_gray",
    ];
    const PHASES: [&str; 8] = [
        "full_moon",
        "waning_gibbous",
        "third_quarter",
        "waning_crescent",
        "new_moon",
        "waxing_crescent",
        "first_quarter",
        "waxing_gibbous",
    ];
    const SIDES: [&str; 3] = ["in", "in_out", "out"];
    const SHAPES: [&str; 10] = [
        "back", "bounce", "circ", "cubic", "elastic", "expo", "quad", "quart", "quint", "sine",
    ];

    fn float(v: f32) -> Json {
        Json::from(f64::from(v))
    }

    struct Rng(u64);

    impl Rng {
        fn next(&mut self) -> i32 {
            self.0 = self
                .0
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (self.0 >> 33) as i32
        }

        fn below(&mut self, n: i32) -> i32 {
            self.next() % n
        }

        fn unit(&mut self) -> f32 {
            self.below(257) as f32 / 256.0
        }

        fn num(&mut self) -> f32 {
            (self.below(4001) - 2000) as f32 / 8.0
        }

        fn channel(&mut self) -> f32 {
            (self.below(385) - 64) as f32 / 256.0
        }

        fn rgb24(&mut self) -> u32 {
            match self.below(8) {
                0 => 0xFF_FFFF,
                1 => 0,
                _ => self.below(1 << 24) as u32,
            }
        }

        fn argb32(&mut self) -> u32 {
            let rgb = self.rgb24();
            let alpha = match self.below(4) {
                0 => 255,
                1 => 0,
                _ => self.below(256) as u32,
            };
            alpha << 24 | rgb
        }

        /// A colour in one of the three ways a file may write it.
        fn colour3(&mut self) -> Json {
            match self.below(3) {
                0 => Json::from(format!("#{:06x}", self.rgb24())),
                1 => Json::from(self.rgb24()),
                _ => (0..3).map(|_| float(self.channel())).collect(),
            }
        }

        fn colour4(&mut self) -> Json {
            match self.below(3) {
                0 => Json::from(format!("#{:08x}", self.argb32())),
                1 => Json::from(self.argb32() as i32),
                _ => (0..4).map(|_| float(self.channel())).collect(),
            }
        }

        fn plain(&mut self, a: usize) -> Json {
            match (KINDS[a], RANGES[a]) {
                (0, 1) => float(self.below(4097) as f32 / 4.0),
                (0, 2) => float(self.unit()),
                (0, _) => float(self.num()),
                (1, _) => float((self.below(5761) - 2880) as f32 / 4.0),
                (2, _) => self.colour3(),
                (3, _) => self.colour4(),
                _ => Json::from(PHASES[self.below(8) as usize]),
            }
        }

        fn modifier(&mut self, a: usize) -> usize {
            self.below(match KINDS[a] {
                0 | 1 => 7,
                2 | 3 => 6,
                _ => 1,
            }) as usize
        }

        fn modifier_name(a: usize, m: usize) -> &'static str {
            if KINDS[a] < 2 {
                FLOAT_MODIFIERS[m]
            } else {
                COLOUR_MODIFIERS[m]
            }
        }

        fn argument(&mut self, a: usize, m: usize) -> Json {
            match (KINDS[a], m) {
                (_, 0) => self.plain(a),
                (0 | 1, 3) => float((self.below(1025) - 256) as f32 / 256.0),
                (0 | 1, 6) => {
                    let bare = self.below(3) == 0;
                    let value = self.num();
                    if bare {
                        float(value)
                    } else {
                        json!({"value": float(value), "alpha": float(self.unit())})
                    }
                }
                (0 | 1, _) => float(self.num()),
                (_, 5) => {
                    let brightness = self.unit();
                    let factor = self.unit();
                    json!({"brightness": float(brightness), "factor": float(factor)})
                }
                (_, 4) | (3, 3) => self.colour4(),
                _ => self.colour3(),
            }
        }

        fn entry(&mut self, a: usize) -> Json {
            if self.below(2) == 0 {
                self.plain(a)
            } else {
                let m = self.modifier(a);
                json!({"modifier": Rng::modifier_name(a, m), "argument": self.argument(a, m)})
            }
        }

        fn track(&mut self, a: usize, period: i32) -> Json {
            let m = self.modifier(a);
            let e = self.below(36);
            let ease = match e {
                0 => json!("constant"),
                1 => json!("linear"),
                2..32 => {
                    let named = (e - 2) as usize;
                    json!(format!("{}_{}", SIDES[named / 10], SHAPES[named % 10]))
                }
                _ => {
                    let x1 = self.unit();
                    let y1 = (self.below(769) - 256) as f32 / 256.0;
                    let x2 = self.unit();
                    let y2 = (self.below(769) - 256) as f32 / 256.0;
                    json!({"cubic_bezier": [float(x1), float(y1), float(x2), float(y2)]})
                }
            };
            let n = 1 + self.below(6) as usize;
            let max = if period > 0 { period } else { 100_000 };
            let mut ticks: Vec<i32> = (0..n).map(|_| self.below(max + 1)).collect();
            if n >= 2 && self.below(3) == 0 {
                ticks[1] = ticks[0];
            }
            if period > 0 {
                let r = self.below(4);
                if r == 0 || r == 2 {
                    ticks[0] = 0;
                }
                if r == 1 || r == 2 {
                    ticks[n - 1] = period;
                }
            }
            ticks.sort_unstable();
            // At most two keyframes on a tick.
            let mut kept: Vec<i32> = Vec::new();
            for t in ticks {
                if !kept.ends_with(&[t, t]) {
                    kept.push(t);
                }
            }
            let keyframes: Vec<Json> = kept
                .iter()
                .map(|t| json!({"ticks": t, "value": self.argument(a, m)}))
                .collect();
            let mut track = json!({"keyframes": keyframes});
            // The defaults are left out some of the time.
            if m != 0 || kept.len() % 2 == 1 {
                track["modifier"] = json!(Rng::modifier_name(a, m));
            }
            if e != 1 || kept.len().is_multiple_of(2) {
                track["ease"] = ease;
            }
            track
        }

        fn clock_ticks(&mut self) -> i64 {
            match self.below(8) {
                0 => 0,
                1 => -i64::from(self.below(100_000)) - 1,
                r @ (2 | 3) => {
                    let high = i64::from(self.next());
                    let low = i64::from(self.next());
                    let huge = high << 31 | low;
                    if r == 2 { huge } else { -huge - 1 }
                }
                _ => i64::from(self.below(200_000)),
            }
        }
    }

    /// A value as the golden file holds it: a float's bits (any NaN as Java's one), a
    /// moon phase's index.
    fn words(value: Value, out: &mut Vec<u32>) {
        let bits = |v: f32| if v.is_nan() { 0x7fc0_0000 } else { v.to_bits() };
        match value {
            Value::Float(v) => out.push(bits(v)),
            Value::Rgb(c) => out.extend(c.map(bits)),
            Value::Argb(c) => out.extend(c.map(bits)),
            Value::MoonPhase(index) => out.push(u32::from(index)),
        }
    }

    fn id(a: usize) -> String {
        format!("minecraft:{}", Attribute::ALL[a].name())
    }

    /// One case: a made-up dimension with its biomes and timelines, its values at
    /// three places and times with the biomes blended and with the position's biome
    /// alone, and the probe's between the first two.
    fn case(rng: &mut Rng) -> Vec<u32> {
        let weather = rng.below(4) != 0;
        let mut env = Environment::default();
        for a in 0..VISUAL {
            if rng.below(2) == 0 {
                env.attributes.insert(id(a), rng.entry(a));
            }
        }
        let biome_count = 1 + rng.below(5);
        for _ in 0..biome_count {
            let attributes = match rng.below(4) {
                0 => Json::Null,
                1 => json!({}),
                _ => {
                    let mut map = serde_json::Map::new();
                    for a in 0..VISUAL {
                        if rng.below(4) == 0 {
                            map.insert(id(a), rng.entry(a));
                        }
                    }
                    if map.is_empty() {
                        let a = rng.below(VISUAL as i32) as usize;
                        map.insert(id(a), rng.entry(a));
                    }
                    Json::Object(map)
                }
            };
            env.biomes.push(biome(attributes));
        }
        for _ in 0..rng.below(4) {
            let clock = rng.below(3);
            let mut period = 0;
            if rng.below(3) != 0 {
                period = match rng.below(5) {
                    0 => 1,
                    1 => 2,
                    2 => 24000,
                    3 => 3 + rng.below(40),
                    _ => 100 + rng.below(5000),
                };
            }
            let mut tracks = serde_json::Map::new();
            for a in 0..VISUAL {
                if rng.below(5) == 0 {
                    tracks.insert(id(a), rng.track(a, period));
                }
            }
            let mut definition = json!({"clock": "test:clock", "tracks": tracks});
            if period > 0 {
                definition["period_ticks"] = json!(period);
            }
            env.timelines.push(timeline(clock, definition));
        }
        let step = [rng.below(3), rng.below(3), rng.below(3)];
        let table: Vec<u16> = (0..1 + rng.below(7))
            .map(|_| rng.below(biome_count) as u16)
            .collect();
        let biome_at = |x: i32, y: i32, z: i32| {
            let sum = x
                .wrapping_mul(step[0])
                .wrapping_add(y.wrapping_mul(step[1]))
                .wrapping_add(z.wrapping_mul(step[2]));
            table[sum.rem_euclid(table.len() as i32) as usize]
        };

        let system = AttributeSystem::new(&env, weather);
        let mut out = Vec::new();
        let mut probe = Probe::default();
        for e in 0..3 {
            // Clock 2 is never set.
            let clocks = [clock(0, rng.clock_ticks()), clock(1, rng.clock_ticks())];
            let rain = if rng.below(3) == 0 { 0.0 } else { rng.unit() };
            let thunder = match rng.below(4) {
                0 => 0.0,
                1 => rng.unit(),
                _ => rng.unit() * rain,
            };
            let position: [f64; 3] = std::array::from_fn(|_| {
                if rng.below(4) == 0 {
                    f64::from((rng.below(2001) - 1000) * 4 + 2)
                } else {
                    f64::from(rng.below(1 << 20) - (1 << 19)) * 0.001
                }
            });
            let weights = BiomeWeights::sample(position, biome_at);
            let [x, y, z] = position.map(|p| mth::floor(p) >> 2);
            let alone = Sample {
                clocks: &clocks,
                rain,
                thunder,
                biomes: None,
                biome: Some(biome_at(x, y, z)),
            };
            let blended = Sample {
                biomes: Some(&weights),
                ..alone
            };
            for a in &Attribute::ALL[..VISUAL] {
                words(system.value(*a, &blended), &mut out);
            }
            for a in &Attribute::ALL[..VISUAL] {
                words(system.value(*a, &alone), &mut out);
            }
            if e < 2 {
                probe.tick(&system, &blended);
            }
        }
        for a in &Attribute::ALL[..VISUAL] {
            let partial = rng.below(257) as f32 / 256.0;
            words(probe.value(*a, partial), &mut out);
        }
        out
    }

    #[test]
    fn values_equal_the_games_own() {
        let mut rng = Rng(0x456e_7669_726f_6e21);
        let mut hash = 0xcbf2_9ce4_8422_2325_u64;
        for c in 0..CASES {
            let out = case(&mut rng);
            assert_eq!(out.len(), 7 * WORDS);
            if let Some(first) = FIRST.get(c) {
                assert_eq!(&out[..WORDS], first, "case {c}");
            }
            for word in out {
                hash = (hash ^ u64::from(word)).wrapping_mul(0x0100_0000_01b3);
            }
            if c % BATCH == BATCH - 1 {
                assert_eq!(
                    hash,
                    HASHES[c / BATCH],
                    "a case from {} to {c}",
                    c + 1 - BATCH
                );
                hash = 0xcbf2_9ce4_8422_2325;
            }
        }
        assert_eq!(HASHES.len() * BATCH, CASES);
    }

    #[test]
    fn easings_equal_the_games_own() {
        // The times: 64ths, and a few close to the ends.
        let mut times: Vec<f32> = (0..=64).map(|k| k as f32 / 64.0).collect();
        times.extend([
            1.0e-7, 1.0e-6, 1.0e-5, 1.0e-4, 0.001, 0.999, 0.9999, 0.99999, 0.999_999,
        ]);
        // Curves that are flat at their start, at their end, at both and in the
        // middle, where the search has to halve; the sky's; and two more.
        let curves: [[f32; 4]; 7] = [
            [0.0, 0.0, 0.0, 1.0],
            [1.0, 0.0, 1.0, 1.0],
            [0.0, 0.3, 1.0, 0.7],
            [1.0, 0.2, 0.0, 0.8],
            [0.362, 0.241, 0.638, 0.759],
            [0.1, 0.9, 0.7, 0.3],
            [0.05, -1.5, 0.95, 2.5],
        ];
        assert_eq!(BEZIER.len(), curves.len());
        for (controls, expected) in curves.iter().zip(BEZIER) {
            let ease =
                Ease::parse(&json!({"cubic_bezier": controls.map(float)})).expect("an easing");
            let values: Vec<u32> = times.iter().map(|t| ease.apply(*t).to_bits()).collect();
            assert_eq!(values, expected, "{controls:?}");
        }
        // The same curves at many more times.
        let mut hash = 0xcbf2_9ce4_8422_2325_u64;
        for controls in curves {
            let ease =
                Ease::parse(&json!({"cubic_bezier": controls.map(float)})).expect("an easing");
            for k in 0..=4099 {
                let value = ease.apply(k as f32 / 4099.0);
                hash = (hash ^ u64::from(value.to_bits())).wrapping_mul(0x0100_0000_01b3);
            }
        }
        assert_eq!(hash, BEZIER_HASH);
        let mut hash = 0xcbf2_9ce4_8422_2325_u64;
        for side in SIDES {
            for shape in SHAPES {
                let ease = Ease::parse(&json!(format!("{side}_{shape}"))).expect("an easing");
                for time in &times {
                    hash = (hash ^ u64::from(ease.apply(*time).to_bits()))
                        .wrapping_mul(0x0100_0000_01b3);
                }
            }
        }
        assert_eq!(hash, NAMED_HASH);
    }

    /// The real overworld against the values the game's own code gives for it: a local
    /// check, since neither the game's data nor those values may be committed. The
    /// values come from the golden program's `real` mode; `RUTHENIUM_ATTR_REAL` names
    /// their file.
    #[test]
    #[ignore = "needs the game's data and RUTHENIUM_ATTR_REAL"]
    fn the_real_overworld_equals_the_games_own() {
        let Some(values) = std::env::var_os("RUTHENIUM_ATTR_REAL") else {
            eprintln!("RUTHENIUM_ATTR_REAL is not set: not checked");
            return;
        };
        let data = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../reference/26.3/resources/data/minecraft");
        let read = |path: &str| -> Json {
            let text =
                std::fs::read_to_string(data.join(path)).unwrap_or_else(|e| panic!("{path}: {e}"));
            serde_json::from_str(&text).unwrap_or_else(|e| panic!("{path}: {e}"))
        };
        let overworld = read("dimension_type/overworld.json");
        let mut env = dimension(overworld["attributes"].clone());
        env.dimension_type = "minecraft:overworld".into();
        // The timelines of the dimension type's tag, in the tag's order, a tag in it
        // put in its place.
        let tag = |name: &str| -> Vec<String> {
            read(&format!("tags/timeline/{name}.json"))["values"]
                .as_array()
                .expect("values")
                .iter()
                .map(|v| v.as_str().expect("a name").to_owned())
                .collect()
        };
        let name = overworld["timelines"].as_str().expect("a tag");
        let name = name.strip_prefix("#minecraft:").expect("a tag");
        let mut members = Vec::new();
        for member in tag(name) {
            match member.strip_prefix("#minecraft:") {
                Some(inner) => members.extend(tag(inner)),
                None => members.push(member),
            }
        }
        assert_eq!(members.len(), 4);
        let clocks = ["minecraft:overworld", "minecraft:the_end"];
        for member in members {
            let file = member.strip_prefix("minecraft:").expect("a name");
            let definition = read(&format!("timeline/{file}.json"));
            let clock = clocks
                .iter()
                .position(|c| Some(*c) == definition["clock"].as_str());
            env.timelines.push(Timeline {
                id: member.clone(),
                clock: clock.map(|c| c as i32),
                definition,
            });
        }
        let biomes = [
            "plains",
            "swamp",
            "mangrove_swamp",
            "pale_garden",
            "desert",
            "cherry_grove",
            "snowy_taiga",
            "dappled_forest",
        ];
        for name in biomes {
            let definition = read(&format!("worldgen/biome/{name}.json"));
            env.biomes.push(BiomeLook {
                id: format!("minecraft:{name}"),
                attributes: definition.get("attributes").cloned().unwrap_or(Json::Null),
                ..BiomeLook::default()
            });
        }
        let weather =
            overworld["has_skylight"] == json!(true) && overworld["has_ceiling"] == json!(false);
        assert!(weather);
        let system = AttributeSystem::new(&env, weather);

        let expected = std::fs::read_to_string(&values).expect("the values");
        let mut expected = expected.lines();
        let ticks = [
            0, 500, 1000, 6000, 11500, 12000, 12500, 13000, 13500, 14000, 18000, 22000, 22500,
            23000, 23500, 23999, 30000,
        ];
        let weathers = [(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.75, 0.25)];
        let mut lines = 0;
        for scenario in 0..5 {
            // One biome everywhere (plains, mangrove swamp, pale garden), all eight
            // in turns, and desert and swamp in stripes.
            let (step, table, position): ([i32; 3], &[u16], [f64; 3]) = match scenario {
                0 => ([0; 3], &[0], [10.3, 70.1, -5.7]),
                1 => ([0; 3], &[2], [10.3, 70.1, -5.7]),
                2 => ([0; 3], &[3], [10.3, 70.1, -5.7]),
                3 => ([1, 2, 3], &[0, 1, 2, 3, 4, 5, 6, 7], [10.3, 70.1, -5.7]),
                _ => ([1, 0, 0], &[4, 4, 1, 1], [1.3, 64.0, 0.5]),
            };
            let biome_at = |x: i32, y: i32, z: i32| {
                table[(x * step[0] + y * step[1] + z * step[2]).rem_euclid(table.len() as i32)
                    as usize]
            };
            let weights = BiomeWeights::sample(position, biome_at);
            let [x, y, z] = position.map(|p| mth::floor(p) >> 2);
            for tick in ticks {
                for (weather, (rain, thunder)) in weathers.into_iter().enumerate() {
                    let clocks = [clock(0, tick)];
                    for mode in 0..2 {
                        let sample = Sample {
                            clocks: &clocks,
                            rain,
                            thunder,
                            biomes: (mode == 0).then_some(&weights),
                            biome: Some(biome_at(x, y, z)),
                        };
                        let mut out = Vec::new();
                        for a in &Attribute::ALL[..VISUAL] {
                            words(system.value(*a, &sample), &mut out);
                        }
                        let mut line = format!("{scenario} {tick} {weather} {mode}");
                        for word in out {
                            line.push_str(&format!(" {word:08x}"));
                        }
                        assert_eq!(Some(line.as_str()), expected.next());
                        lines += 1;
                    }
                }
            }
        }
        assert_eq!(expected.next(), None);
        eprintln!(
            "the real overworld: {lines} evaluations of {VISUAL} attributes equal the game's"
        );
    }
}

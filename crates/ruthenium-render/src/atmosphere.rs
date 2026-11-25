//! What the air does to a frame: the fog's colour and reach, the sky's colours and
//! where its sun stands, and what the light map is made from. All of it comes from the
//! dimension's environment attributes as the time of day, the weather and the biomes
//! around the camera move them, read once a tick and blended between ticks as 26.3's
//! camera reads them, and worked out as its `FogRenderer`, `SkyRenderer` and
//! `LightmapRenderStateExtractor` do.
//!
//! Inside water, lava or powder snow the fog is the medium's (found by
//! `terrain::medium`), and with the player's eyes in water the frame is tinted by a
//! picture of it ([`WaterOverlay`]).
//!
//! Not here yet: the effects on the player (blindness, darkness, night vision, a
//! conduit's) and a boss's fog change nothing.

use std::sync::Arc;
use std::time::Instant;

use ruthenium_data::v26_3::registries::MobEffect;
use ruthenium_javamath::mth;
use ruthenium_sim::attributes::{Attribute, AttributeSystem, BiomeWeights, Probe, Sample, Value};
use ruthenium_sim::biome::zoomed_cell;
use ruthenium_sim::environment::Environment;
use ruthenium_sim::player::Pose;
use ruthenium_sim::scene::{Frame, Scene};

use crate::clouds::CloudLook;
use crate::sky::{OverworldSky, Sky};
use crate::terrain::{CameraPose, LightmapInputs};

/// Degrees to radians, as the game's single-precision constant.
const RADIANS: f32 = 0.017_453_292;

/// The fog of one frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Fog {
    /// Its colour; the alpha scales how much of it there is.
    pub colour: [f32; 4],
    /// Where the fog of the place starts and where it is complete, in blocks from the
    /// camera in a straight line.
    pub environmental: [f32; 2],
    /// The same for the fog at the edge of the view, in blocks sideways or in height,
    /// whichever is more.
    pub render_distance: [f32; 2],
    /// Where the sky's fog is complete.
    pub sky_end: f32,
}

impl Fog {
    /// No fog at any distance.
    pub const NONE: Fog = Fog {
        colour: [0.0; 4],
        environmental: [f32::MAX; 2],
        render_distance: [f32::MAX; 2],
        sky_end: f32::MAX,
    };
}

/// What the camera is in (`FogType`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Medium {
    #[default]
    Air,
    Water,
    Lava,
    PowderSnow,
}

/// What the field of view shrinks to in water and lava, with FOV Effects all the way up.
const FLUID_FOV: f32 = 0.857_142_87;

impl Medium {
    /// What the field of view is multiplied by; `effects` is the FOV Effects option, 0
    /// to 1.
    pub fn fov_scale(self, effects: f32) -> f32 {
        match self {
            Medium::Water | Medium::Lava => 1.0 + effects * (FLUID_FOV - 1.0),
            Medium::Air | Medium::PowderSnow => 1.0,
        }
    }

    /// Nothing of the sky is drawn from inside it.
    fn hides_sky(self) -> bool {
        matches!(self, Medium::Lava | Medium::PowderSnow)
    }
}

/// `GameType.SPECTATOR`'s id.
const SPECTATOR: i32 = 3;

/// The picture of water in front of a player whose eyes are in it
/// (`ScreenEffectRenderer.submitWater`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WaterOverlay {
    /// What the picture is multiplied by, 0xAARRGGBB.
    pub colour: u32,
    /// How far it has slid, in its own widths and heights, as the player turns.
    pub offset: [f32; 2],
}

impl WaterOverlay {
    /// With light `light` (0 to 15) at the eyes in a dimension of that ambient light.
    pub fn new(light: i32, ambient: f32, y_rot: f32, x_rot: f32) -> WaterOverlay {
        let v = light as f32 / 15.0;
        let curved = v / (4.0 - 3.0 * v);
        let brightness = curved + ambient * (1.0 - curved);
        let channel = (brightness * 255.0).floor() as u32 & 0xFF;
        // A tenth of it is there.
        let alpha = (0.1_f32 * 255.0).floor() as u32;
        WaterOverlay {
            colour: alpha << 24 | channel << 16 | channel << 8 | channel,
            offset: [-y_rot / 64.0, x_rot / 64.0],
        }
    }
}

/// What a frame's fog is worked out from.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FogInputs {
    pub medium: Medium,
    /// The render distance in chunks.
    pub view_distance: i32,
    /// The attributes at the camera: `fog_color`, `sky_color`, `sunrise_sunset_color`,
    /// `sun_angle` in degrees, the fog's and the sky fog's distances, and water's.
    pub fog_colour: [f32; 3],
    pub sky_colour: [f32; 3],
    pub sunrise: [f32; 4],
    pub sun_angle: f32,
    pub fog_start: f32,
    pub fog_end: f32,
    pub sky_fog_end: f32,
    pub water_fog_colour: [f32; 3],
    pub water_fog_start: f32,
    pub water_fog_end: f32,
    pub rain: f32,
    pub thunder: f32,
    /// How far the rain has drawn the fog in, 0 to 1 ([`rain_fog_target`]).
    pub rain_fog: f32,
    /// The direction the camera looks along.
    pub forward: [f32; 3],
    pub camera_y: f64,
    /// The level's floor, and whether it is a flat world's.
    pub min_y: i32,
    pub flat: bool,
    pub spectator: bool,
    pub fire_resistance: bool,
    /// How well the player sees under water, 0 to 1.
    pub water_vision: f32,
}

fn lerp3(t: f32, a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    std::array::from_fn(|i| (b[i] - a[i]) * t + a[i])
}

fn rgb24(colour: u32) -> [f32; 3] {
    [
        (colour >> 16 & 0xFF) as f32 / 255.0,
        (colour >> 8 & 0xFF) as f32 / 255.0,
        (colour & 0xFF) as f32 / 255.0,
    ]
}

/// The colour of the air's fog (`AtmosphericFogEnvironment.getBaseColor`): the fog's
/// own, towards the glow of a sunrise when the camera looks at it, then towards the
/// sky's colour the nearer the sky's fog ends.
fn air_colour(i: &FogInputs) -> [f32; 3] {
    let mut colour = i.fog_colour;
    if i.view_distance >= 4 {
        // The sun's side of the sky: west once it is past its height.
        let side = if mth::sin(f64::from(i.sun_angle * RADIANS)) > 0.0 {
            -1.0
        } else {
            1.0
        };
        let towards = i.forward[0] * side;
        let glow = i.sunrise[3];
        if towards > 0.0 && glow > 0.0 {
            colour = lerp3(
                towards * glow,
                colour,
                [i.sunrise[0], i.sunrise[1], i.sunrise[2]],
            );
        }
    }
    // The sky as the weather dims it, a second time over what its attribute says.
    let mut sky = i.sky_colour;
    if i.rain > 0.0 {
        let (dim, dim_blue) = (1.0 - i.rain * 0.5, 1.0 - i.rain * 0.4);
        sky = [
            (sky[0] * dim).clamp(0.0, 1.0),
            (sky[1] * dim).clamp(0.0, 1.0),
            (sky[2] * dim_blue).clamp(0.0, 1.0),
        ];
    }
    if i.thunder > 0.0 {
        sky = sky.map(|c| (c * (1.0 - i.thunder * 0.5)).clamp(0.0, 1.0));
    }
    let chunks = (i.sky_fog_end / 16.0).min(i.view_distance as f32);
    let near = chunks / 32.0;
    let near = if near < 0.0 {
        0.25
    } else if near > 1.0 {
        1.0
    } else {
        0.25 + near * (1.0 - 0.25)
    };
    let mix = 1.0 - f64::from(near).powf(0.25) as f32;
    lerp3(mix, colour, sky)
}

/// A frame's fog (`FogRenderer.setupFog` and `computeFogColor`).
pub fn fog(i: &FogInputs) -> Fog {
    let reach = (i.view_distance * 16) as f32;
    let (colour, start, end, sky_end) = match i.medium {
        Medium::Air => {
            let start = i.fog_start + -160.0 * i.rain_fog;
            let least = i.fog_end.min(96.0);
            let end = least.max(i.fog_end + -256.0 * i.rain_fog);
            (air_colour(i), start, end, reach.min(i.sky_fog_end))
        }
        Medium::Water => {
            let end = i.water_fog_end * i.water_vision.max(0.25);
            (i.water_fog_colour, i.water_fog_start, end, end)
        }
        Medium::Lava => {
            let (start, end) = if i.spectator {
                (-8.0, reach * 0.5)
            } else if i.fire_resistance {
                (0.0, 5.0)
            } else {
                (0.25, 1.0)
            };
            (rgb24(0x99_1900), start, end, end)
        }
        Medium::PowderSnow => {
            let (start, end) = if i.spectator {
                (-8.0, reach * 0.5)
            } else {
                (0.0, 2.0)
            };
            (rgb24(0x9F_BBCC), start, end, end)
        }
    };
    // Towards the level's floor the void darkens everything but lava and snow.
    let onset = if i.flat { 1.0_f32 } else { 32.0 };
    let darkness = ((onset + i.min_y as f32 - i.camera_y as f32) / onset).clamp(0.0, 1.0);
    let mut colour = colour;
    if darkness > 0.0 && !matches!(i.medium, Medium::Lava | Medium::PowderSnow) {
        let brightness = (1.0 - darkness) * (1.0 - darkness);
        colour = colour.map(|c| c * brightness);
    }
    // Under water the colour is lifted to full strength by how well the player sees.
    let lift = if i.medium == Medium::Water {
        i.water_vision
    } else {
        0.0
    };
    if colour.iter().all(|c| *c != 0.0) {
        let scale = 1.0 / colour[0].max(colour[1]).max(colour[2]);
        colour = colour.map(|c| c + lift * (c * scale - c));
    }
    let span = (reach / 10.0).clamp(4.0, 64.0);
    Fog {
        colour: [colour[0], colour[1], colour[2], 1.0],
        environmental: [start, end],
        render_distance: [reach - span, reach],
        sky_end,
    }
}

/// What the rain draws the fog in towards: all the way under an open sky where it
/// rains, half where the biome has no rain, not at all with less than sky light 8
/// (`AtmosphericFogEnvironment.updateRainFogState`).
pub fn rain_fog_target(rain: f32, sky_light: u8, precipitation: bool) -> f32 {
    let open = ((f32::from(sky_light) - 8.0) / 7.0).clamp(0.0, 1.0);
    rain * open * if precipitation { 1.0 } else { 0.5 }
}

/// What a caller chooses about the air.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AtmosphereOptions {
    /// The terrain is fogged (on unless said otherwise). The sky's own fog stays.
    pub fog: bool,
    /// Block light flickers (on unless said otherwise). Off, two frames of one place
    /// and time are the same picture.
    pub flicker: bool,
    /// The Brightness option, 0 to 1.
    pub brightness: f32,
    /// The Cloud Distance option, in chunks: where the clouds' fog ends at the most.
    pub cloud_range: u32,
}

impl Default for AtmosphereOptions {
    fn default() -> AtmosphereOptions {
        AtmosphereOptions {
            fog: true,
            flicker: true,
            brightness: 0.5,
            cloud_range: 128,
        }
    }
}

/// What a frame looks like for the air it is seen through.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Look {
    pub fog: Fog,
    pub sky: Sky,
    pub lightmap: LightmapInputs,
    pub water: Option<WaterOverlay>,
    /// What the sky's light lacks to its full strength, as of the tick (`getSkyDarken`).
    pub sky_darken: i32,
    /// The clouds' colour and height, and where they have faded out.
    pub clouds: CloudLook,
    pub cloud_fog_end: f32,
}

/// Where the clouds have faded out (`FogData.cloudEnd`): in the air at the Cloud
/// Distance or where the `cloud_fog_end_distance` attribute says, if that is nearer;
/// inside anything else where its fog is complete.
pub fn cloud_fog_end(medium: Medium, range: u32, attribute: f32, environmental_end: f32) -> f32 {
    match medium {
        Medium::Air => ((range * 16) as f32).min(attribute),
        Medium::Water | Medium::Lava | Medium::PowderSnow => environmental_end,
    }
}

fn float(v: Value) -> f32 {
    v.float()
}

fn rgb(v: Value) -> [f32; 3] {
    v.rgb()
}

fn argb(v: Value) -> [f32; 4] {
    v.argb()
}

/// The air of the level being drawn, from one frame to the next.
#[derive(Debug)]
pub(crate) struct Atmosphere {
    /// The dimension's environment and its attributes, compiled.
    environment: Option<Arc<Environment>>,
    system: Option<AttributeSystem>,
    /// The biome of a chunk that is not loaded.
    plains: u16,
    probe: Probe,
    /// The level and the tick the probe was last moved on at.
    epoch: u64,
    tick: Option<u64>,
    rain_fog: f32,
    last_frame: Option<Instant>,
    /// How much brighter or darker block light is than usual, and the generator it
    /// wanders by.
    flicker: f32,
    random: u64,
    /// What the light map was last made from: it changes once a tick.
    lightmap: LightmapInputs,
}

impl Atmosphere {
    pub fn new() -> Atmosphere {
        Atmosphere {
            environment: None,
            system: None,
            plains: 0,
            probe: Probe::default(),
            epoch: 0,
            tick: None,
            rain_fog: 0.0,
            last_frame: None,
            flicker: 0.0,
            random: 0x9E37_79B9_7F4A_7C15,
            lightmap: LightmapInputs::default(),
        }
    }

    /// A number from 0 up to 1.
    fn chance(&mut self) -> f32 {
        // xorshift64*: the game's own generator for this is seeded from the clock.
        self.random ^= self.random >> 12;
        self.random ^= self.random << 25;
        self.random ^= self.random >> 27;
        (self.random.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 40) as f32 / (1u64 << 24) as f32
    }

    /// The look of the frame `frame` seen from `camera`, which is in `medium`, `partial`
    /// of the way from its tick to the next, at `now`.
    #[allow(clippy::too_many_arguments)]
    pub fn update(
        &mut self,
        frame: &Frame,
        partial: f32,
        camera: &CameraPose,
        medium: Medium,
        scene: &Scene,
        view_distance: i32,
        now: Instant,
        options: &AtmosphereOptions,
    ) -> Look {
        let environment = &frame.environment;
        if !self
            .environment
            .as_ref()
            .is_some_and(|e| Arc::ptr_eq(e, environment))
            || self.epoch != frame.epoch
        {
            // `Level.canHaveWeather`.
            let weather = environment.has_skylight
                && !environment.has_ceiling
                && &*frame.dimension != "minecraft:the_end";
            self.system = Some(AttributeSystem::new(environment, weather));
            self.plains = environment
                .biomes
                .iter()
                .position(|b| b.id == "minecraft:plains")
                .unwrap_or(0) as u16;
            self.environment = Some(environment.clone());
            self.epoch = frame.epoch;
            self.probe.reset();
            self.tick = None;
            self.rain_fog = 0.0;
            self.flicker = 0.0;
        }
        let block = camera.position.map(|c| c.floor() as i32);
        let dims = scene.with_terrain(|t| t.dims());
        // The thunder's level counts as far as it rains (`Level.getThunderLevel`).
        let thunder = frame.thunder * frame.rain;
        let ticked = self.tick != Some(frame.ticks);
        if ticked && let Some(system) = &self.system {
            // Where no chunk is loaded the client has plains.
            let plains = self.plains;
            let (weights, biome) = scene.with_terrain(|t| {
                let weights = BiomeWeights::sample(camera.position, |x, y, z| {
                    t.biome_cell([x, y, z]).unwrap_or(plains)
                });
                (
                    weights,
                    t.biome_cell(block.map(|c| c >> 2)).unwrap_or(plains),
                )
            });
            self.probe.tick(
                system,
                &Sample {
                    clocks: &frame.clocks,
                    rain: frame.rain,
                    thunder,
                    biomes: Some(&weights),
                    biome: Some(biome),
                },
            );
            // As many steps of the flicker as ticks went by (a few at most count).
            let steps = self
                .tick
                .map_or(1, |last| frame.ticks.saturating_sub(last).clamp(1, 20));
            for _ in 0..steps {
                let wander = (self.chance() - self.chance()) * self.chance() * self.chance();
                self.flicker = (self.flicker + wander * 0.1) * 0.9;
            }
            self.tick = Some(frame.ticks);
        }
        let read = |a: Attribute| self.probe.value(a, partial);
        if ticked {
            self.lightmap = LightmapInputs {
                sky_factor: float(read(Attribute::SkyLightFactor)),
                block_factor: 1.4 + if options.flicker { self.flicker } else { 0.0 },
                block_light_tint: rgb(read(Attribute::BlockLightTint)),
                sky_light_color: rgb(read(Attribute::SkyLightColor)),
                ambient_color: rgb(read(Attribute::AmbientLightColor)),
                night_vision_color: rgb(read(Attribute::NightVisionColor)),
                night_vision_factor: 0.0,
                darkness_scale: 0.0,
                boss_overlay_darkening: 0.0,
                brightness: options.brightness,
            };
        }
        // The rain draws the fog in over a few ticks, by the frame's share of a tick.
        let elapsed = self.last_frame.map_or(0.0, |last| {
            now.saturating_duration_since(last).as_secs_f32()
        });
        self.last_frame = Some(now);
        // Only the air's fog takes part: inside anything else it stays as it was.
        if medium == Medium::Air && (frame.rain > 0.0 || self.rain_fog != 0.0) {
            let (sky_light, precipitation) = scene.with_terrain(|t| {
                let light = t.probe(block).map_or(15, |p| p.sky_light);
                let cell = zoomed_cell(dims.biome_seed, block);
                let biome = t.biome_cell(cell).unwrap_or(self.plains);
                let rains = environment
                    .biomes
                    .get(usize::from(biome))
                    .is_none_or(|b| b.has_precipitation);
                (light, rains)
            });
            let target = rain_fog_target(frame.rain, sky_light, precipitation);
            let ticks = elapsed * 1000.0 / frame.ms_per_tick.max(1.0);
            self.rain_fog += (target - self.rain_fog) * ticks * 0.2;
        }
        let sun_angle = float(read(Attribute::SunAngle));
        let sky_colour = rgb(read(Attribute::SkyColor));
        let sunrise = argb(read(Attribute::SunriseSunsetColor));
        let mut fog = fog(&FogInputs {
            medium,
            view_distance,
            fog_colour: rgb(read(Attribute::FogColor)),
            sky_colour,
            sunrise,
            sun_angle,
            fog_start: float(read(Attribute::FogStartDistance)),
            fog_end: float(read(Attribute::FogEndDistance)),
            sky_fog_end: float(read(Attribute::SkyFogEndDistance)),
            water_fog_colour: rgb(read(Attribute::WaterFogColor)),
            water_fog_start: float(read(Attribute::WaterFogStartDistance)),
            water_fog_end: float(read(Attribute::WaterFogEndDistance)),
            rain: frame.rain,
            thunder,
            rain_fog: self.rain_fog,
            forward: camera.axes()[0],
            camera_y: camera.position[1],
            min_y: dims.min_section << 4,
            flat: dims.flat,
            spectator: frame.player.game_type == SPECTATOR,
            fire_resistance: frame
                .player
                .effects
                .iter()
                .any(|e| e.0 == MobEffect::FireResistance.id() as i32),
            water_vision: frame.player.water_vision,
        });
        let cloud_fog_end = cloud_fog_end(
            medium,
            options.cloud_range,
            float(read(Attribute::CloudFogEndDistance)),
            fog.environmental[1],
        );
        if !options.fog {
            fog.environmental = Fog::NONE.environmental;
            fog.render_distance = Fog::NONE.render_distance;
        }
        let sky = match environment.skybox.trim_start_matches("minecraft:") {
            _ if medium.hides_sky() => Sky::None,
            "none" => Sky::None,
            "end" => Sky::End,
            _ => {
                // The horizon: the sea's level, or a flat world's floor.
                let horizon = if dims.flat {
                    f64::from(dims.min_section << 4)
                } else {
                    63.0
                };
                let moon_phase = read(Attribute::MoonPhase).moon_phase();
                Sky::Overworld(OverworldSky {
                    colour: sky_colour,
                    sunrise,
                    sun_angle: sun_angle * RADIANS,
                    moon_angle: float(read(Attribute::MoonAngle)) * RADIANS,
                    star_angle: float(read(Attribute::StarAngle)) * RADIANS,
                    moon_phase,
                    rain_brightness: 1.0 - frame.rain,
                    star_brightness: float(read(Attribute::StarBrightness)),
                    dark_disc: camera.position[1] - horizon < 0.0,
                })
            }
        };
        let player = &frame.player;
        let water = (player.eye_in_water
            && player.game_type != SPECTATOR
            && player.pose != Pose::Sleeping)
            .then(|| {
                let mut eye = player.position;
                eye[1] += f64::from(player.eye_height);
                let light = scene.with_terrain(|t| t.probe(eye.map(|c| c.floor() as i32)));
                // What the sky lacks to its full light, as of the tick.
                let lacks = (15.0 - float(self.probe.value(Attribute::SkyLightLevel, 1.0))) as i32;
                let light = light.map_or(15 - lacks, |p| {
                    (i32::from(p.sky_light) - lacks).max(i32::from(p.block_light))
                });
                WaterOverlay::new(
                    light.max(0),
                    environment.ambient_light,
                    camera.y_rot,
                    camera.x_rot,
                )
            });
        Look {
            fog,
            sky,
            lightmap: self.lightmap,
            water,
            sky_darken: (15.0 - float(self.probe.value(Attribute::SkyLightLevel, 1.0))) as i32,
            clouds: CloudLook {
                colour: argb(read(Attribute::CloudColor)),
                height: float(read(Attribute::CloudHeight)),
            },
            cloud_fog_end,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A clear noon in the overworld at render distance 12, looking south.
    fn noon() -> FogInputs {
        FogInputs {
            medium: Medium::Air,
            view_distance: 12,
            fog_colour: rgb24(0xC0_D8FF),
            sky_colour: rgb24(0x78_A7FF),
            sunrise: [0.0; 4],
            sun_angle: 0.0,
            fog_start: 0.0,
            fog_end: 1024.0,
            sky_fog_end: 512.0,
            water_fog_colour: rgb24(0x05_0533),
            water_fog_start: -8.0,
            water_fog_end: 96.0,
            rain: 0.0,
            thunder: 0.0,
            rain_fog: 0.0,
            forward: [0.0, 0.0, 1.0],
            camera_y: 80.0,
            min_y: -64,
            flat: false,
            spectator: false,
            fire_resistance: false,
            water_vision: 1.0,
        }
    }

    #[test]
    fn the_clouds_fade_out_at_their_distance_or_in_the_medium_s_fog() {
        assert_eq!(cloud_fog_end(Medium::Air, 128, 2048.0, 192.0), 2048.0);
        assert_eq!(cloud_fog_end(Medium::Air, 64, 2048.0, 192.0), 1024.0);
        assert_eq!(cloud_fog_end(Medium::Air, 128, 300.0, 192.0), 300.0);
        for m in [Medium::Water, Medium::Lava, Medium::PowderSnow] {
            assert_eq!(cloud_fog_end(m, 128, 2048.0, 24.0), 24.0);
        }
    }

    #[test]
    fn the_edge_of_the_view_is_fogged_over_its_last_tenth() {
        let f = fog(&noon());
        // 192 blocks: the last 19.2 of them.
        assert_eq!(f.render_distance, [192.0 - 19.2, 192.0]);
        assert_eq!(f.environmental, [0.0, 1024.0]);
        assert_eq!(f.sky_end, 192.0);
        // Never less than 4 blocks of it, never more than 64.
        let near = fog(&FogInputs {
            view_distance: 2,
            ..noon()
        });
        assert_eq!(near.render_distance, [28.0, 32.0]);
        let far = fog(&FogInputs {
            view_distance: 64,
            ..noon()
        });
        assert_eq!(far.render_distance, [1024.0 - 64.0, 1024.0]);
        // The sky's fog ends where the attribute says, if that is nearer.
        assert_eq!(far.sky_end, 512.0);
    }

    #[test]
    fn the_fog_takes_some_of_the_skys_colour() {
        let i = noon();
        let f = fog(&i);
        // The sky's fog ends 12 chunks out: 0.25 + 12 / 32 * 0.75 of the way, and the
        // fourth root of that is what stays of the fog's own colour.
        let stays = f64::from(0.25_f32 + 12.0 / 32.0 * 0.75).powf(0.25) as f32;
        for c in 0..3 {
            let want = (i.sky_colour[c] - i.fog_colour[c]) * (1.0 - stays) + i.fog_colour[c];
            assert_eq!(f.colour[c], want);
        }
        assert_eq!(f.colour[3], 1.0);
        // With the sky's fog at 32 chunks or beyond, none of the sky is in it.
        let far = fog(&FogInputs {
            view_distance: 32,
            ..i
        });
        assert_eq!(far.colour[..3], i.fog_colour);
        // Rain dims the sky that is mixed in, blue less than the rest.
        let wet = fog(&FogInputs { rain: 1.0, ..i });
        let dimmed = (i.sky_colour[2] * 0.6 - i.fog_colour[2]) * (1.0 - stays) + i.fog_colour[2];
        assert_eq!(wet.colour[2], dimmed);
        // Thunder halves what the rain left of it.
        let storm = fog(&FogInputs {
            rain: 1.0,
            thunder: 1.0,
            ..i
        });
        let halved =
            (i.sky_colour[2] * 0.6 * 0.5 - i.fog_colour[2]) * (1.0 - stays) + i.fog_colour[2];
        assert_eq!(storm.colour[2], halved);
        assert!(storm.colour[0] < wet.colour[0]);
    }

    #[test]
    fn a_sunset_colours_the_fog_only_towards_the_sun() {
        let glow = [1.0, 0.5, 0.2, 0.8];
        // An evening sun stands in the west (-x). At 32 chunks none of the sky is
        // mixed in, so what is left is the fog's colour and the glow.
        let evening = FogInputs {
            sunrise: glow,
            sun_angle: 80.0,
            view_distance: 32,
            ..noon()
        };
        let base = evening.fog_colour;
        let looking = |forward| fog(&FogInputs { forward, ..evening }).colour;
        let west = looking([-1.0, 0.0, 0.0]);
        assert_eq!(looking([1.0, 0.0, 0.0])[..3], base);
        assert_eq!(looking([0.0, 0.0, 1.0])[..3], base);
        for c in 0..3 {
            assert_eq!(west[c], (glow[c] - base[c]) * 0.8 + base[c]);
        }
        // Half turned towards it, half the glow.
        assert_eq!(
            looking([-0.5, 0.0, 0.75])[0],
            (glow[0] - base[0]) * (0.5 * 0.8) + base[0]
        );
        // A morning sun stands in the east.
        let morning = FogInputs {
            sun_angle: 280.0,
            forward: [1.0, 0.0, 0.0],
            ..evening
        };
        assert_eq!(fog(&morning).colour, west);
        // Under four chunks of view there is no glow in the fog.
        let short = FogInputs {
            view_distance: 3,
            forward: [-1.0, 0.0, 0.0],
            ..evening
        };
        assert_eq!(
            fog(&short).colour,
            fog(&FogInputs {
                sunrise: [0.0; 4],
                ..short
            })
            .colour
        );
    }

    #[test]
    fn rain_draws_the_fog_in() {
        let i = FogInputs {
            rain_fog: 0.5,
            ..noon()
        };
        // Half of 160 blocks nearer at its start and half of 256 at its end.
        assert_eq!(fog(&i).environmental, [-80.0, 1024.0 - 128.0]);
        // Its end never comes nearer than 96 blocks, or than it was.
        let nether = FogInputs {
            fog_start: 10.0,
            fog_end: 96.0,
            rain_fog: 1.0,
            ..noon()
        };
        assert_eq!(fog(&nether).environmental, [-150.0, 96.0]);
        let short = FogInputs {
            fog_end: 40.0,
            rain_fog: 1.0,
            ..noon()
        };
        assert_eq!(fog(&short).environmental[1], 40.0);
        // What the rain is headed for: nothing under a roof, half in a desert.
        assert_eq!(rain_fog_target(1.0, 15, true), 1.0);
        assert_eq!(rain_fog_target(1.0, 15, false), 0.5);
        assert_eq!(rain_fog_target(0.5, 15, true), 0.5);
        assert_eq!(rain_fog_target(1.0, 8, true), 0.0);
        assert_eq!(rain_fog_target(1.0, 0, true), 0.0);
        assert_eq!(rain_fog_target(1.0, 12, true), 4.0 / 7.0);
    }

    #[test]
    fn the_void_darkens_the_fog() {
        let clear = fog(&noon()).colour;
        // 32 blocks above the floor and higher: as it is.
        assert_eq!(
            fog(&FogInputs {
                camera_y: -32.0,
                ..noon()
            })
            .colour,
            clear
        );
        // Halfway down: a quarter of the brightness. At the floor: black.
        let half = fog(&FogInputs {
            camera_y: -48.0,
            ..noon()
        })
        .colour;
        for c in 0..3 {
            assert_eq!(half[c], clear[c] * 0.25);
        }
        let floor = fog(&FogInputs {
            camera_y: -64.0,
            ..noon()
        })
        .colour;
        assert_eq!(floor, [0.0, 0.0, 0.0, 1.0]);
        // A flat world darkens only within a block of its floor.
        let flat = FogInputs {
            flat: true,
            ..noon()
        };
        assert_eq!(
            fog(&FogInputs {
                camera_y: -63.0,
                ..flat
            })
            .colour,
            clear
        );
        let low = fog(&FogInputs {
            camera_y: -63.5,
            ..flat
        })
        .colour;
        assert_eq!(low[0], clear[0] * 0.25);
        // Lava glows whatever the depth.
        let lava = FogInputs {
            medium: Medium::Lava,
            camera_y: -64.0,
            ..noon()
        };
        assert_eq!(fog(&lava).colour, [0.6, 25.0 / 255.0, 0.0, 1.0]);
    }

    #[test]
    fn fluids_narrow_the_view() {
        assert_eq!(Medium::Water.fov_scale(1.0), 0.857_142_87);
        assert_eq!(Medium::Lava.fov_scale(1.0), 0.857_142_87);
        assert_eq!(Medium::Water.fov_scale(0.0), 1.0);
        assert_eq!(Medium::Air.fov_scale(1.0), 1.0);
        assert_eq!(Medium::PowderSnow.fov_scale(1.0), 1.0);
        // From water the sky is seen, from lava and powder snow it is not.
        assert!(!Medium::Air.hides_sky() && !Medium::Water.hides_sky());
        assert!(Medium::Lava.hides_sky() && Medium::PowderSnow.hides_sky());
    }

    /// A frame's look in `medium` above a level lit by full sky light, as `change` has
    /// the frame.
    fn look(medium: Medium, rain_fog: f32, change: impl Fn(&mut Frame)) -> (Look, f32) {
        let mut world = crate::terrain::testing::World::new(&[(0, 0)]);
        world.publish();
        let mut frame = Frame::new(
            Instant::now(),
            Arc::new(Environment {
                skybox: "minecraft:overworld".into(),
                has_skylight: true,
                ..Environment::default()
            }),
            Arc::from("minecraft:overworld"),
        );
        frame.player.position = [8.5, 4.0, 8.5];
        frame.player.eye_height = 1.62;
        change(&mut frame);
        let camera = CameraPose {
            position: [8.5, 5.62, 8.5],
            y_rot: 64.0,
            x_rot: 0.0,
            fov: 70.0,
        };
        let mut air = Atmosphere::new();
        let now = Instant::now();
        let options = AtmosphereOptions::default();
        let step = |air: &mut Atmosphere, at: Instant| {
            air.update(&frame, 1.0, &camera, medium, &world.scene, 12, at, &options)
        };
        step(&mut air, now);
        air.rain_fog = rain_fog;
        let look = step(&mut air, now + std::time::Duration::from_millis(50));
        (look, air.rain_fog)
    }

    #[test]
    fn a_frame_takes_the_mediums_look() {
        let sky = |medium| look(medium, 0.0, |_| {}).0.sky;
        assert!(matches!(sky(Medium::Air), Sky::Overworld(_)));
        assert!(matches!(sky(Medium::Water), Sky::Overworld(_)));
        assert_eq!(sky(Medium::Lava), Sky::None);
        assert_eq!(sky(Medium::PowderSnow), Sky::None);
        // Rain draws the air's fog in; inside water it waits.
        let rain = |f: &mut Frame| f.rain = 1.0;
        assert!(look(Medium::Air, 0.5, rain).1 > 0.5);
        assert_eq!(look(Medium::Water, 0.5, rain).1, 0.5);
        // Lava's fog is the player's: further with fire resistance, or as a spectator.
        let lava =
            |change: &dyn Fn(&mut Frame)| look(Medium::Lava, 0.0, change).0.fog.environmental;
        assert_eq!(lava(&|_| {}), [0.25, 1.0]);
        let id = MobEffect::FireResistance.id() as i32;
        assert_eq!(lava(&|f| f.player.effects.push((id, 0, 100))), [0.0, 5.0]);
        assert_eq!(
            lava(&|f| f.player.effects.push((id + 1, 0, 100))),
            [0.25, 1.0]
        );
        assert_eq!(lava(&|f| f.player.game_type = SPECTATOR), [-8.0, 96.0]);
        // Water's by how well the player sees in it.
        let reach = |vision| {
            let set = |f: &mut Frame| f.player.water_vision = vision;
            look(Medium::Water, 0.0, set).0.fog.environmental[1]
        };
        assert_eq!((reach(1.0), reach(0.0)), (96.0, 24.0));
    }

    #[test]
    fn water_is_before_the_eyes_of_a_player_in_it() {
        let water = |change: &dyn Fn(&mut Frame)| {
            let set = |f: &mut Frame| {
                f.player.eye_in_water = true;
                change(f);
            };
            look(Medium::Water, 0.0, set).0.water
        };
        let full = water(&|_| {}).expect("the picture of water");
        assert_eq!((full.colour, full.offset), (0x19FF_FFFF, [-1.0, 0.0]));
        assert_eq!(water(&|f| f.player.eye_in_water = false), None);
        assert_eq!(water(&|f| f.player.game_type = SPECTATOR), None);
        assert_eq!(water(&|f| f.player.pose = Pose::Sleeping), None);
        // A storm takes 5 levels of the sky's light (15 towards 4 by 0.527): 10 are left.
        let storm = water(&|f| {
            f.rain = 1.0;
            f.thunder = 1.0;
        });
        let dim = WaterOverlay::new(10, 0.0, 64.0, 0.0);
        assert_eq!(storm, Some(dim));
        assert!(dim.colour < full.colour);
    }

    #[test]
    fn the_water_before_the_eyes_is_as_bright_as_its_light() {
        // Full light is white, none is black, and a tenth of it is there.
        assert_eq!(WaterOverlay::new(15, 0.0, 0.0, 0.0).colour, 0x19FF_FFFF);
        assert_eq!(WaterOverlay::new(0, 0.0, 0.0, 0.0).colour, 0x1900_0000);
        // Light 12 is 0.8, which curves to 0.5.
        assert_eq!(WaterOverlay::new(12, 0.0, 0.0, 0.0).colour, 0x197F_7F7F);
        // The Nether's ambient light lifts it a tenth of the way to white.
        assert_eq!(WaterOverlay::new(0, 0.1, 0.0, 0.0).colour, 0x1919_1919);
        // It slides against the turn, once its width in 64 degrees.
        let turned = WaterOverlay::new(15, 0.0, 64.0, -32.0);
        assert_eq!(turned.offset, [-1.0, -0.5]);
    }

    #[test]
    fn what_the_camera_is_in_sets_its_own_fog() {
        let water = fog(&FogInputs {
            medium: Medium::Water,
            ..noon()
        });
        assert_eq!(water.environmental, [-8.0, 96.0]);
        assert_eq!(water.sky_end, 96.0);
        // Seen at full strength: the colour's largest channel is 1.
        assert_eq!(water.colour[2], 1.0);
        assert_eq!(water.colour[0], (5.0 / 255.0) / (51.0 / 255.0));
        // A player who has just gone under sees a quarter as far, in the colour as
        // it is.
        let fresh = fog(&FogInputs {
            medium: Medium::Water,
            water_vision: 0.0,
            ..noon()
        });
        assert_eq!(fresh.environmental[1], 24.0);
        assert_eq!(fresh.colour[..3], rgb24(0x05_0533));
        let lava = FogInputs {
            medium: Medium::Lava,
            ..noon()
        };
        assert_eq!(fog(&lava).environmental, [0.25, 1.0]);
        assert_eq!(
            fog(&FogInputs {
                fire_resistance: true,
                ..lava
            })
            .environmental,
            [0.0, 5.0]
        );
        assert_eq!(
            fog(&FogInputs {
                spectator: true,
                ..lava
            })
            .environmental,
            [-8.0, 96.0]
        );
        let snow = FogInputs {
            medium: Medium::PowderSnow,
            ..noon()
        };
        assert_eq!(fog(&snow).environmental, [0.0, 2.0]);
        assert_eq!(fog(&snow).colour[..3], rgb24(0x9F_BBCC));
        // The edge of the view is fogged the same in all of them.
        assert_eq!(fog(&snow).render_distance, fog(&noon()).render_distance);
    }
}

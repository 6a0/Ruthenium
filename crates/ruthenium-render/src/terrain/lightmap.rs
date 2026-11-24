//! The light map: the 16 x 16 texture that turns a vertex's block light and sky light
//! into the colour it is lit with, computed as 26.3's `lightmap.fsh` computes it.
//!
//! Until the environment is evaluated per frame (the world's clocks and timelines, the
//! camera's biome, effects on the player), the inputs are the dimension's own values:
//! no night, no flicker, no night vision, the default brightness.

use ruthenium_sim::environment::Environment;

/// Texels along each side: one per light level.
pub const SIZE: u32 = 16;

/// What the light map is computed from.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LightmapInputs {
    pub sky_factor: f32,
    /// 1.4 plus the flicker.
    pub block_factor: f32,
    pub block_light_tint: [f32; 3],
    pub sky_light_color: [f32; 3],
    pub ambient_color: [f32; 3],
    pub night_vision_color: [f32; 3],
    pub night_vision_factor: f32,
    pub darkness_scale: f32,
    pub boss_overlay_darkening: f32,
    /// The Brightness option, 0 to 1.
    pub brightness: f32,
}

fn rgb(colour: u32) -> [f32; 3] {
    [
        (colour >> 16 & 0xFF) as f32 / 255.0,
        (colour >> 8 & 0xFF) as f32 / 255.0,
        (colour & 0xFF) as f32 / 255.0,
    ]
}

impl Default for LightmapInputs {
    /// The attributes' defaults (`EnvironmentAttributes`), a steady block light and the
    /// default Brightness.
    fn default() -> LightmapInputs {
        LightmapInputs {
            sky_factor: 1.0,
            block_factor: 1.4,
            block_light_tint: rgb(0xFF_D88C),
            sky_light_color: [1.0; 3],
            ambient_color: [0.0; 3],
            night_vision_color: rgb(0x99_9999),
            night_vision_factor: 0.0,
            darkness_scale: 0.0,
            boss_overlay_darkening: 0.0,
            brightness: 0.5,
        }
    }
}

/// A colour attribute as dimension types write them: `"#rrggbb"`, or the number.
pub(crate) fn colour_attribute(env: &Environment, id: &str) -> Option<[f32; 3]> {
    let v = env.attributes.get(id)?;
    let colour = match v.as_str() {
        Some(s) => u32::from_str_radix(s.strip_prefix('#')?, 16).ok()?,
        None => v.as_i64()? as u32,
    };
    Some(rgb(colour))
}

impl LightmapInputs {
    /// The inputs a dimension gives by itself: its type's light attributes over the
    /// defaults.
    pub fn of(env: &Environment) -> LightmapInputs {
        let d = LightmapInputs::default();
        let colour = |id: &str, default: [f32; 3]| colour_attribute(env, id).unwrap_or(default);
        LightmapInputs {
            sky_factor: env
                .attributes
                .get("minecraft:visual/sky_light_factor")
                .and_then(serde_json::Value::as_f64)
                .map(|n| n as f32)
                .unwrap_or(d.sky_factor),
            block_light_tint: colour("minecraft:visual/block_light_tint", d.block_light_tint),
            sky_light_color: colour("minecraft:visual/sky_light_color", d.sky_light_color),
            ambient_color: colour("minecraft:visual/ambient_light_color", d.ambient_color),
            night_vision_color: colour("minecraft:visual/night_vision_color", d.night_vision_color),
            ..d
        }
    }

    /// The light map's texels as RGBA8, rows from sky light 0 to 15, block light 0 to 15
    /// along each.
    pub fn texels(&self) -> Vec<u8> {
        let brightness = |level: f32| level / (4.0 - 3.0 * level);
        let mix = |a: f32, b: f32, t: f32| a + (b - a) * t;
        let mut out = Vec::with_capacity((SIZE * SIZE * 4) as usize);
        for sky in 0..SIZE {
            for block in 0..SIZE {
                let (block_level, sky_level) = (block as f32 / 15.0, sky as f32 / 15.0);
                let block_brightness = brightness(block_level) * self.block_factor;
                let sky_brightness = brightness(sky_level) * self.sky_factor;
                // Block light goes from its tint towards white at both ends of its
                // range.
                let towards_white = 0.9 * (2.0 * block_level - 1.0) * (2.0 * block_level - 1.0);
                let mut c: [f32; 3] = std::array::from_fn(|i| {
                    let ambient = self.ambient_color[i]
                        .max(self.night_vision_color[i] * self.night_vision_factor);
                    let block_colour = mix(self.block_light_tint[i], 1.0, towards_white);
                    let lit = ambient
                        + self.sky_light_color[i] * sky_brightness
                        + block_colour * block_brightness;
                    let dimmed = mix(lit, lit * [0.7, 0.6, 0.6][i], self.boss_overlay_darkening);
                    (dimmed - self.darkness_scale).clamp(0.0, 1.0)
                });
                // The Brightness option lifts dark colours more than bright ones.
                let max = c[0].max(c[1]).max(c[2]);
                let inverted = 1.0 - max;
                let scaled = 1.0 - inverted * inverted * inverted * inverted;
                for v in &mut c {
                    // (A black texel stays black: there is nothing to scale.)
                    let lifted = if max > 0.0 { *v * (scaled / max) } else { 0.0 };
                    *v = mix(*v, lifted, self.brightness);
                }
                out.extend(c.map(|v| (v * 255.0).round() as u8));
                out.push(255);
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn texel(t: &[u8], block: usize, sky: usize) -> [u8; 4] {
        let i = (sky * 16 + block) * 4;
        [t[i], t[i + 1], t[i + 2], t[i + 3]]
    }

    #[test]
    fn full_light_is_white_and_no_light_is_dark() {
        let t = LightmapInputs::default().texels();
        assert_eq!(t.len(), 16 * 16 * 4);
        assert_eq!(texel(&t, 0, 15), [255, 255, 255, 255]);
        assert_eq!(texel(&t, 15, 0), [255, 255, 255, 255]);
        assert_eq!(texel(&t, 0, 0), [0, 0, 0, 255]);
        // More light is never darker, along both axes.
        for a in 0..16 {
            for b in 1..16 {
                assert!(texel(&t, b, a)[1] >= texel(&t, b - 1, a)[1]);
                assert!(texel(&t, a, b)[1] >= texel(&t, a, b - 1)[1]);
            }
        }
    }

    #[test]
    fn a_texel_is_the_shaders_arithmetic() {
        // Block light 7 in the dark, by hand: level 7/15; brightness l / (4 - 3l) times
        // 1.4; the tint moved towards white by 0.9 (2l - 1)^2; then the Brightness
        // option's curve, half applied.
        let l = 7.0_f32 / 15.0;
        let b = l / (4.0 - 3.0 * l) * 1.4;
        let w = 0.9 * (2.0 * l - 1.0) * (2.0 * l - 1.0);
        let tint = [1.0_f32, 216.0 / 255.0, 140.0 / 255.0];
        let c = tint.map(|t| ((t + (1.0 - t) * w) * b).clamp(0.0, 1.0));
        let max = c[0];
        let scaled = 1.0 - (1.0 - max).powi(4);
        let want = c.map(|v| ((v + (v * (scaled / max) - v) * 0.5) * 255.0).round() as u8);
        let t = LightmapInputs::default().texels();
        assert_eq!(texel(&t, 7, 0)[..3], want);
        // Torch light is warm: more red than blue.
        assert!(want[0] > want[2]);
    }

    #[test]
    fn a_dimension_sets_its_own_light() {
        let mut env = Environment::default();
        env.attributes.insert(
            "minecraft:visual/ambient_light_color".into(),
            "#302821".into(),
        );
        env.attributes
            .insert("minecraft:visual/sky_light_factor".into(), 0.0.into());
        let inputs = LightmapInputs::of(&env);
        assert_eq!(inputs.sky_factor, 0.0);
        assert_eq!(inputs.ambient_color, rgb(0x30_2821));
        assert_eq!(
            inputs.block_light_tint,
            LightmapInputs::default().block_light_tint
        );
        let t = inputs.texels();
        // Sky light does nothing there, and nothing is black.
        assert_eq!(texel(&t, 3, 0), texel(&t, 3, 15));
        assert!(texel(&t, 0, 0)[0] > 0x30);
    }
}

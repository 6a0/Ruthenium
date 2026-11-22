//! `.mcmeta` sections used by sprites: `animation`, `texture` and `gui`.
//!
//! Field names, defaults and limits follow the 26.3 codecs
//! (`client/resources/metadata/animation/AnimationMetadataSection.java`,
//! `AnimationFrame.java`, `metadata/texture/TextureMetadataSection.java`,
//! `metadata/gui/GuiSpriteScaling.java`). A section that fails to parse makes
//! the sprite unusable, as in vanilla.

use serde_json::{Map, Value};

/// Parses JSON the way packs need: a UTF-8 byte-order mark is skipped.
pub fn parse_json(bytes: &[u8]) -> Result<Value, String> {
    let bytes = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes);
    serde_json::from_slice(bytes).map_err(|e| e.to_string())
}

/// A parsed `.mcmeta` file (empty if there is none).
#[derive(Debug, Clone, Default)]
pub struct Metadata(Map<String, Value>);

impl Metadata {
    pub fn parse(bytes: &[u8]) -> Result<Metadata, String> {
        match parse_json(bytes)? {
            Value::Object(m) => Ok(Metadata(m)),
            _ => Err("metadata is not a JSON object".into()),
        }
    }

    pub fn section(&self, name: &str) -> Option<&Value> {
        self.0.get(name)
    }

    pub fn animation(&self) -> Result<Option<Animation>, String> {
        self.section("animation").map(Animation::parse).transpose()
    }

    pub fn texture(&self) -> Result<Option<TextureMeta>, String> {
        self.section("texture").map(TextureMeta::parse).transpose()
    }

    pub fn gui(&self) -> Result<Option<GuiScaling>, String> {
        self.section("gui")
            .map(|g| {
                let g = obj(g, "gui")?;
                g.get("scaling")
                    .map_or(Ok(GuiScaling::Stretch), GuiScaling::parse)
            })
            .transpose()
    }
}

fn obj<'a>(v: &'a Value, what: &str) -> Result<&'a Map<String, Value>, String> {
    v.as_object()
        .ok_or_else(|| format!("{what}: not an object"))
}

/// A JSON number as a Java `int` (`Number.intValue`: truncated).
fn int(v: &Value, what: &str) -> Result<i64, String> {
    let n = v.as_f64().ok_or_else(|| format!("{what}: not a number"))?;
    Ok((n as i64).clamp(i32::MIN.into(), i32::MAX.into()))
}

fn positive(v: &Value, what: &str) -> Result<u32, String> {
    let n = int(v, what)?;
    u32::try_from(n)
        .ok()
        .filter(|&n| n > 0)
        .ok_or_else(|| format!("{what}: value must be positive: {n}"))
}

fn non_negative(v: &Value, what: &str) -> Result<u32, String> {
    let n = int(v, what)?;
    u32::try_from(n).map_err(|_| format!("{what}: value must be non-negative: {n}"))
}

fn boolean(m: &Map<String, Value>, key: &str, default: bool) -> Result<bool, String> {
    match m.get(key) {
        None => Ok(default),
        Some(Value::Bool(b)) => Ok(*b),
        Some(_) => Err(format!("{key}: not a boolean")),
    }
}

fn opt_positive(m: &Map<String, Value>, key: &str) -> Result<Option<u32>, String> {
    m.get(key).map(|v| positive(v, key)).transpose()
}

/// One entry of `animation.frames`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameRef {
    pub index: u32,
    pub time: Option<u32>,
}

/// The `animation` section.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Animation {
    pub frames: Option<Vec<FrameRef>>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    /// Ticks per frame; default 1.
    pub frametime: u32,
    pub interpolate: bool,
}

impl Animation {
    pub fn parse(v: &Value) -> Result<Animation, String> {
        let m = obj(v, "animation")?;
        let frames = match m.get("frames") {
            None => None,
            Some(Value::Array(list)) => Some(
                list.iter()
                    .map(|f| match f {
                        Value::Object(f) => Ok(FrameRef {
                            index: non_negative(f.get("index").ok_or("frame: no index")?, "index")?,
                            time: opt_positive(f, "time")?,
                        }),
                        f => Ok(FrameRef {
                            index: non_negative(f, "frame")?,
                            time: None,
                        }),
                    })
                    .collect::<Result<_, String>>()?,
            ),
            Some(_) => return Err("frames: not a list".into()),
        };
        Ok(Animation {
            frames,
            width: opt_positive(m, "width")?,
            height: opt_positive(m, "height")?,
            frametime: opt_positive(m, "frametime")?.unwrap_or(1),
            interpolate: boolean(m, "interpolate", false)?,
        })
    }

    /// The frame size for an image of `width` x `height`: the given sizes,
    /// the image size for a missing one, or a square of the smaller side
    /// when neither is given.
    pub fn frame_size(&self, width: u32, height: u32) -> (u32, u32) {
        match (self.width, self.height) {
            (Some(w), Some(h)) => (w, h),
            (Some(w), None) => (w, height),
            (None, Some(h)) => (width, h),
            (None, None) => (width.min(height), width.min(height)),
        }
    }
}

/// How mip levels are built (`client/renderer/texture/MipmapStrategy.java`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MipmapStrategy {
    /// Cutout if the image has a fully transparent pixel, mean otherwise.
    #[default]
    Auto,
    Mean,
    Cutout,
    StrictCutout,
    DarkCutout,
}

/// The `texture` section.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct TextureMeta {
    pub blur: bool,
    pub clamp: bool,
    pub mipmap_strategy: MipmapStrategy,
    pub alpha_cutoff_bias: f32,
}

impl TextureMeta {
    pub fn parse(v: &Value) -> Result<TextureMeta, String> {
        let m = obj(v, "texture")?;
        let mipmap_strategy = match m.get("mipmap_strategy") {
            None => MipmapStrategy::Auto,
            Some(Value::String(s)) => match s.as_str() {
                "auto" => MipmapStrategy::Auto,
                "mean" => MipmapStrategy::Mean,
                "cutout" => MipmapStrategy::Cutout,
                "strict_cutout" => MipmapStrategy::StrictCutout,
                "dark_cutout" => MipmapStrategy::DarkCutout,
                other => return Err(format!("unknown mipmap_strategy {other:?}")),
            },
            Some(_) => return Err("mipmap_strategy: not a string".into()),
        };
        let alpha_cutoff_bias = match m.get("alpha_cutoff_bias") {
            None => 0.0,
            Some(v) => v.as_f64().ok_or("alpha_cutoff_bias: not a number")? as f32,
        };
        Ok(TextureMeta {
            blur: boolean(m, "blur", false)?,
            clamp: boolean(m, "clamp", false)?,
            mipmap_strategy,
            alpha_cutoff_bias,
        })
    }
}

/// Nine-slice borders in texels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Border {
    pub left: u32,
    pub top: u32,
    pub right: u32,
    pub bottom: u32,
}

/// `gui.scaling`: how a GUI sprite fills a larger rectangle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum GuiScaling {
    #[default]
    Stretch,
    Tile {
        width: u32,
        height: u32,
    },
    NineSlice {
        width: u32,
        height: u32,
        border: Border,
        stretch_inner: bool,
    },
}

impl GuiScaling {
    pub fn parse(v: &Value) -> Result<GuiScaling, String> {
        let m = obj(v, "scaling")?;
        let size = |m: &Map<String, Value>| -> Result<(u32, u32), String> {
            Ok((
                positive(m.get("width").ok_or("scaling: no width")?, "width")?,
                positive(m.get("height").ok_or("scaling: no height")?, "height")?,
            ))
        };
        match m.get("type").and_then(Value::as_str) {
            Some("stretch") => Ok(GuiScaling::Stretch),
            Some("tile") => {
                let (width, height) = size(m)?;
                Ok(GuiScaling::Tile { width, height })
            }
            Some("nine_slice") => {
                let (width, height) = size(m)?;
                let border = match m.get("border").ok_or("nine_slice: no border")? {
                    Value::Object(b) => {
                        let side = |k: &str| {
                            non_negative(b.get(k).ok_or_else(|| format!("border: no {k}"))?, k)
                        };
                        Border {
                            left: side("left")?,
                            top: side("top")?,
                            right: side("right")?,
                            bottom: side("bottom")?,
                        }
                    }
                    v => {
                        let s = positive(v, "border")?;
                        Border {
                            left: s,
                            top: s,
                            right: s,
                            bottom: s,
                        }
                    }
                };
                if border.left + border.right >= width {
                    return Err("nine_slice: no horizontal center slice".into());
                }
                if border.top + border.bottom >= height {
                    return Err("nine_slice: no vertical center slice".into());
                }
                Ok(GuiScaling::NineSlice {
                    width,
                    height,
                    border,
                    stretch_inner: boolean(m, "stretch_inner", false)?,
                })
            }
            Some(other) => Err(format!("unknown scaling type {other:?}")),
            None => Err("scaling: no type".into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn meta(s: &str) -> Metadata {
        Metadata::parse(s.as_bytes()).unwrap()
    }

    #[test]
    fn animation_defaults_and_frames() {
        let a = meta(r#"{"animation": {}}"#).animation().unwrap().unwrap();
        assert_eq!((a.frametime, a.interpolate, &a.frames), (1, false, &None));
        assert_eq!(a.frame_size(16, 64), (16, 16));
        let a = meta(r#"{"animation": {"frametime": 2, "width": 8, "frames": [3, {"index": 1, "time": 5}]}}"#)
            .animation()
            .unwrap()
            .unwrap();
        assert_eq!(a.frame_size(16, 64), (8, 64));
        assert_eq!(
            a.frames.unwrap(),
            [
                FrameRef {
                    index: 3,
                    time: None
                },
                FrameRef {
                    index: 1,
                    time: Some(5)
                }
            ]
        );
        assert!(
            meta(r#"{"animation": {"frametime": 0}}"#)
                .animation()
                .is_err()
        );
        assert!(
            meta(r#"{"animation": {"frames": [-1]}}"#)
                .animation()
                .is_err()
        );
    }

    #[test]
    fn texture_and_gui() {
        let t = meta(r#"{"texture": {"blur": true, "mipmap_strategy": "dark_cutout", "alpha_cutoff_bias": 0.25}}"#)
            .texture()
            .unwrap()
            .unwrap();
        assert!(t.blur && !t.clamp);
        assert_eq!(t.mipmap_strategy, MipmapStrategy::DarkCutout);
        assert_eq!(t.alpha_cutoff_bias, 0.25);
        let g = meta(r#"{"gui": {"scaling": {"type": "nine_slice", "width": 200, "height": 20, "border": 3}}}"#)
            .gui()
            .unwrap()
            .unwrap();
        assert!(matches!(
            g,
            GuiScaling::NineSlice {
                width: 200,
                border: Border { left: 3, .. },
                stretch_inner: false,
                ..
            }
        ));
        assert_eq!(
            meta(r#"{"gui": {}}"#).gui().unwrap(),
            Some(GuiScaling::Stretch)
        );
        assert!(meta(r#"{"gui": {"scaling": {"type": "nine_slice", "width": 4, "height": 4, "border": 2}}}"#).gui().is_err());
    }
}

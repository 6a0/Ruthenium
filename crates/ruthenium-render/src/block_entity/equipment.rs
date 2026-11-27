//! What worn things look like: the layers of each equipment asset (`equipment/*.json`), and
//! the pictures of armour trims, coloured by their materials' palettes.

use std::collections::HashMap;

use ruthenium_sim::item::Trim;
use serde_json::Value;

/// One layer of a worn thing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Layer {
    /// Its picture's name without its kind's folder (`diamond`).
    pub texture: String,
    /// Whether it takes the stack's dye, and what it is coloured (0xRRGGBB) without one:
    /// nothing, which leaves it out.
    pub dyeable: Option<Option<u32>>,
}

impl Layer {
    /// What its picture is multiplied by for a stack dyed so (0xRRGGBB), if it is drawn
    /// (`EquipmentLayerRenderer.getColorForLayer`).
    pub fn colour(&self, dyed: Option<u32>) -> Option<u32> {
        match self.dyeable {
            None => Some(0xFF_FFFF),
            Some(undyed) => dyed.or(undyed),
        }
    }
}

/// Another picture or palette for some trims on one kind of armour.
#[derive(Debug, Clone, PartialEq, Eq)]
struct TrimOverride {
    /// The keys it is for; any, where none is given.
    material: Option<String>,
    pattern: Option<String>,
    texture: Option<String>,
    palette: Option<String>,
}

/// An equipment asset.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Equipment {
    /// What is worn on the head, the chest and the feet, and on the legs.
    pub humanoid: Vec<Layer>,
    pub leggings: Vec<Layer>,
    /// A glider's wings.
    pub wings: Vec<Layer>,
    overrides: Vec<TrimOverride>,
}

fn full(id: &str) -> String {
    if id.contains(':') {
        id.to_owned()
    } else {
        format!("minecraft:{id}")
    }
}

/// An id's path, if it is the game's own.
fn own_path(id: &str) -> Option<&str> {
    match id.split_once(':') {
        Some(("minecraft", path)) => Some(path),
        Some(_) => None,
        None => Some(id),
    }
}

/// Caps on untrusted equipment JSON definitions so one piece of armour cannot push
/// hundreds of thousands of render instances per frame.
const MAX_EQUIPMENT_LAYERS: usize = 16;
const MAX_TRIM_OVERRIDES: usize = 256;

impl Equipment {
    /// From an asset's file. Nothing for one without layers.
    pub fn parse(json: &Value) -> Option<Equipment> {
        let layers = json.get("layers")?.as_object()?;
        let of = |kind: &str| -> Vec<Layer> {
            layers
                .get(kind)
                .and_then(Value::as_array)
                .map_or(&[][..], Vec::as_slice)
                .iter()
                .take(MAX_EQUIPMENT_LAYERS)
                .filter_map(|l| {
                    let texture = own_path(l.get("texture")?.as_str()?)?.to_owned();
                    let dyeable = l.get("dyeable").map(|d| {
                        d.get("color_when_undyed")
                            .and_then(Value::as_i64)
                            .map(|c| c as u32 & 0xFF_FFFF)
                    });
                    Some(Layer { texture, dyeable })
                })
                .collect()
        };
        let overrides = json
            .get("trim_overrides")
            .and_then(Value::as_array)
            .map_or(&[][..], Vec::as_slice)
            .iter()
            .take(MAX_TRIM_OVERRIDES)
            .filter_map(|o| {
                let when = o.get("when")?;
                let id = |v: &Value, key: &str| v.get(key).and_then(Value::as_str).map(full);
                Some(TrimOverride {
                    material: id(when, "material"),
                    pattern: id(when, "pattern"),
                    texture: id(o, "texture"),
                    palette: id(o, "palette"),
                })
            })
            .collect();
        Some(Equipment {
            humanoid: of("humanoid"),
            leggings: of("humanoid_leggings"),
            wings: of("wings"),
            overrides,
        })
    }

    /// The picture of a trim on this armour and the palette it is coloured by, if it is:
    /// the pattern's and the material's, unless the armour says otherwise for them
    /// (`TrimTextureKey.getOrPrepareTexture`).
    pub fn trim_picture(&self, trim: &Trim) -> (String, Option<String>) {
        let mut texture = trim.asset.to_string();
        let mut palette = Some(trim.palette.to_string());
        let is = |wanted: &Option<String>, key: &str| wanted.as_ref().is_none_or(|w| w == key);
        if let Some(o) = self
            .overrides
            .iter()
            .find(|o| is(&o.material, &trim.material) && is(&o.pattern, &trim.pattern))
        {
            if let Some(t) = &o.texture {
                texture.clone_from(t);
            }
            palette.clone_from(&o.palette);
        }
        (texture, palette)
    }
}

/// Every equipment asset by its id (`minecraft:diamond`).
#[derive(Debug, Clone, Default)]
pub struct EquipmentSet {
    by_id: HashMap<String, Equipment>,
}

/// The folders of the pictures of what is worn on the body, on the legs and as wings,
/// below `textures/entity`.
pub const HUMANOID_DIR: &str = "equipment/humanoid";
pub const LEGGINGS_DIR: &str = "equipment/humanoid_leggings";
pub const WINGS_DIR: &str = "equipment/wings";

impl EquipmentSet {
    /// From the assets' files: each one's id and what it holds.
    pub fn new<'a>(files: impl IntoIterator<Item = (&'a str, &'a Value)>) -> EquipmentSet {
        EquipmentSet {
            by_id: files
                .into_iter()
                .filter_map(|(id, json)| Some((full(id), Equipment::parse(json)?)))
                .collect(),
        }
    }

    pub fn get(&self, id: &str) -> Option<&Equipment> {
        self.by_id.get(id)
    }

    pub fn len(&self) -> usize {
        self.by_id.len()
    }

    pub fn is_empty(&self) -> bool {
        self.by_id.is_empty()
    }
}

/// A picture with its colours replaced by a palette's (`PaletteMapping`): each colour of
/// `base` becomes the colour at the same place in `target`, in every texel that is not
/// clear; a texel keeps its own opacity, lessened by its new colour's. All three are RGBA.
/// Palettes of different lengths change nothing.
#[cfg(test)]
pub fn map_palette(picture: &[u8], base: &[u8], target: &[u8]) -> Vec<u8> {
    let mut out = picture.to_vec();
    if let Some(mapping) = palette_mapping(base, target) {
        apply_palette(&mut out, &mapping);
    }
    out
}

/// What [`map_palette`] turns each colour of `base` into: nothing for palettes of
/// different lengths.
pub type PaletteMapping = HashMap<[u8; 3], [u8; 4]>;

/// The mapping of `base`'s colours to `target`'s, for [`apply_palette`]; none when
/// they differ in length (nothing changes).
pub fn palette_mapping(base: &[u8], target: &[u8]) -> Option<PaletteMapping> {
    if base.len() != target.len() {
        return None;
    }
    let mut mapping = PaletteMapping::new();
    for (from, to) in base
        .as_chunks::<4>()
        .0
        .iter()
        .zip(target.as_chunks::<4>().0)
    {
        if from[3] != 0 {
            mapping.insert([from[0], from[1], from[2]], *to);
        }
    }
    Some(mapping)
}

/// [`map_palette`] on `texels` (RGBA) in place: any run of a picture's texels.
pub fn apply_palette(texels: &mut [u8], mapping: &PaletteMapping) {
    for texel in texels.as_chunks_mut::<4>().0 {
        if texel[3] == 0 {
            continue;
        }
        if let Some(to) = mapping.get(&[texel[0], texel[1], texel[2]]) {
            let alpha = u32::from(texel[3]) * u32::from(to[3]) / 255;
            *texel = [to[0], to[1], to[2], alpha as u8];
        }
    }
}

/// The path of a trim's picture below `textures` without its suffix, for a pattern's
/// asset on what is worn on the legs or elsewhere; nothing for one that is not the game's.
pub fn trim_path(asset: &str, leggings: bool) -> Option<String> {
    let kind = if leggings {
        "humanoid_leggings"
    } else {
        "humanoid"
    };
    Some(format!("trims/entity/{kind}/{}", own_path(asset)?))
}

/// The path of a palette below `textures/palettes` without its suffix.
pub fn palette_path(id: &str) -> Option<&str> {
    own_path(id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn trim(material: &str, pattern: &str) -> Trim {
        Trim {
            material: format!("minecraft:{material}").into(),
            palette: format!("minecraft:trim/{material}").into(),
            pattern: format!("minecraft:{pattern}").into(),
            asset: format!("minecraft:{pattern}").into(),
            decal: false,
        }
    }

    #[test]
    fn layers_are_read_with_their_dyes() {
        let leather = Equipment::parse(&json!({
            "layers": {
                "humanoid": [
                    {"texture": "minecraft:leather", "dyeable": {"color_when_undyed": -6265536}},
                    {"texture": "minecraft:leather_overlay"}
                ],
                "humanoid_leggings": [{"texture": "leather", "dyeable": {}}],
                "horse_body": [{"texture": "minecraft:leather"}]
            }
        }))
        .unwrap();
        assert_eq!(leather.humanoid.len(), 2);
        assert_eq!(leather.humanoid[0].texture, "leather");
        // Undyed leather is brown; dyed, it is the dye's. Its overlay never is.
        assert_eq!(leather.humanoid[0].colour(None), Some(0xA0_6540));
        assert_eq!(leather.humanoid[0].colour(Some(0x12_3456)), Some(0x12_3456));
        assert_eq!(leather.humanoid[1].colour(Some(0x12_3456)), Some(0xFF_FFFF));
        // A layer that is only there when dyed.
        assert_eq!(leather.leggings[0].colour(None), None);
        assert_eq!(leather.leggings[0].colour(Some(1)), Some(1));
        assert!(leather.wings.is_empty());
        assert_eq!(Equipment::parse(&json!({"layer": {}})), None);
        // Another pack's pictures are not looked for.
        let other = Equipment::parse(&json!({"layers": {"humanoid": [{"texture": "x:y"}]}}));
        assert!(other.unwrap().humanoid.is_empty());
    }

    #[test]
    fn armour_of_a_trims_own_material_takes_a_darker_palette() {
        let gold = Equipment::parse(&json!({
            "layers": {"humanoid": [{"texture": "minecraft:gold"}]},
            "trim_overrides": [
                {"palette": "minecraft:trim/gold_darker", "when": {"material": "minecraft:gold"}},
                {"texture": "minecraft:plain", "when": {"pattern": "minecraft:bolt"}}
            ]
        }))
        .unwrap();
        assert_eq!(
            gold.trim_picture(&trim("gold", "bolt")),
            (
                "minecraft:bolt".to_owned(),
                Some("minecraft:trim/gold_darker".to_owned())
            )
        );
        assert_eq!(
            gold.trim_picture(&trim("iron", "coast")),
            (
                "minecraft:coast".to_owned(),
                Some("minecraft:trim/iron".to_owned())
            )
        );
        // One that gives a picture alone is drawn as it is, with no palette.
        assert_eq!(
            gold.trim_picture(&trim("iron", "bolt")),
            ("minecraft:plain".to_owned(), None)
        );
        assert_eq!(
            trim_path("minecraft:bolt", false).unwrap(),
            "trims/entity/humanoid/bolt"
        );
        assert_eq!(
            trim_path("bolt", true).unwrap(),
            "trims/entity/humanoid_leggings/bolt"
        );
        assert_eq!(trim_path("other:bolt", true), None);
        assert_eq!(palette_path("minecraft:trim/gold"), Some("trim/gold"));
    }

    #[test]
    fn a_palette_recolours_what_is_not_clear() {
        let base = [
            10, 10, 10, 255, 20, 20, 20, 255, 30, 30, 30, 0, 40, 40, 40, 255,
        ];
        let target = [1, 2, 3, 255, 4, 5, 6, 128, 7, 8, 9, 255, 0, 0, 0, 0];
        let picture = [
            10, 10, 10, 255, // becomes the first colour
            20, 20, 20, 200, // the second, at its opacity times the texel's
            30, 30, 30, 255, // a clear key maps nothing
            40, 40, 40, 255, // to a clear colour: clear
            10, 10, 10, 0, // clear stays as it is
            99, 99, 99, 77, // not in the palette
        ];
        assert_eq!(
            map_palette(&picture, &base, &target),
            [
                1, 2, 3, 255, 4, 5, 6, 100, 30, 30, 30, 255, 0, 0, 0, 0, 10, 10, 10, 0, 99, 99, 99,
                77
            ]
        );
        // Palettes that do not match in length map nothing.
        assert_eq!(map_palette(&picture, &base, &target[..8]), picture);
    }
}

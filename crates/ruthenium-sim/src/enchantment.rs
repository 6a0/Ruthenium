//! Enchantments as the client reads them from the `minecraft:enchantment` registry: only
//! the value effects used for riptide movement and crossbow charging.

use std::collections::HashMap;
use std::sync::Arc;

use ruthenium_protocol::nbt::Tag;
use ruthenium_protocol::packets::configuration::RegistryEntry;
use serde_json::Value;

use crate::transform::nbt_to_json;

/// `LevelBasedValue`: a number worked out from an enchantment's level.
#[derive(Debug, Clone, PartialEq)]
pub enum LevelValue {
    Constant(f32),
    Linear {
        base: f32,
        per_level_above_first: f32,
    },
    Clamped {
        value: Box<LevelValue>,
        min: f32,
        max: f32,
    },
    Fraction {
        numerator: Box<LevelValue>,
        denominator: Box<LevelValue>,
    },
    LevelsSquared {
        added: f32,
    },
    Exponent {
        base: Box<LevelValue>,
        power: Box<LevelValue>,
    },
    Lookup {
        values: Vec<f32>,
        fallback: Box<LevelValue>,
    },
}

impl LevelValue {
    /// `calculate`.
    pub fn calculate(&self, level: i32) -> f32 {
        match self {
            LevelValue::Constant(v) => *v,
            LevelValue::Linear {
                base,
                per_level_above_first,
            } => base + per_level_above_first * (level - 1) as f32,
            LevelValue::Clamped { value, min, max } => {
                // Mth.clamp(float): a NaN passes through.
                let v = value.calculate(level);
                if v < *min { *min } else { v.min(*max) }
            }
            LevelValue::Fraction {
                numerator,
                denominator,
            } => {
                let d = denominator.calculate(level);
                if d == 0.0 {
                    0.0
                } else {
                    numerator.calculate(level) / d
                }
            }
            LevelValue::LevelsSquared { added } => (level * level) as f32 + added,
            LevelValue::Exponent { base, power } => {
                f64::from(base.calculate(level)).powf(f64::from(power.calculate(level))) as f32
            }
            LevelValue::Lookup { values, fallback } => {
                match usize::try_from(level - 1)
                    .ok()
                    .filter(|_| level <= values.len() as i32)
                {
                    Some(i) => values[i],
                    None => fallback.calculate(level),
                }
            }
        }
    }

    /// The codec's forms: a bare number, or an object with a `type`.
    pub fn from_json(v: &Value) -> Option<LevelValue> {
        if let Some(n) = v.as_f64() {
            return Some(LevelValue::Constant(n as f32));
        }
        let f = |k: &str| v.get(k)?.as_f64().map(|n| n as f32);
        let boxed = |k: &str| LevelValue::from_json(v.get(k)?).map(Box::new);
        Some(match type_name(v)? {
            "constant" => LevelValue::Constant(f("value")?),
            "linear" => LevelValue::Linear {
                base: f("base")?,
                per_level_above_first: f("per_level_above_first")?,
            },
            "clamped" => LevelValue::Clamped {
                value: boxed("value")?,
                min: f("min")?,
                max: f("max")?,
            },
            "fraction" => LevelValue::Fraction {
                numerator: boxed("numerator")?,
                denominator: boxed("denominator")?,
            },
            "levels_squared" => LevelValue::LevelsSquared { added: f("added")? },
            "exponent" => LevelValue::Exponent {
                base: boxed("base")?,
                power: boxed("power")?,
            },
            "lookup" => LevelValue::Lookup {
                values: v
                    .get("values")?
                    .as_array()?
                    .iter()
                    .map(|n| n.as_f64().map(|n| n as f32))
                    .collect::<Option<_>>()?,
                fallback: boxed("fallback")?,
            },
            _ => return None,
        })
    }
}

/// `EnchantmentValueEffect`: how an enchantment changes a value.
#[derive(Debug, Clone, PartialEq)]
pub enum ValueEffect {
    Add(LevelValue),
    Multiply(LevelValue),
    Set(LevelValue),
    AllOf(Vec<ValueEffect>),
    Exponential {
        base: LevelValue,
        exponent: LevelValue,
    },
    /// Random: the client's value is left as it is.
    RemoveBinomial,
}

impl ValueEffect {
    /// `process`.
    pub fn process(&self, level: i32, value: f32) -> f32 {
        match self {
            ValueEffect::Add(v) => value + v.calculate(level),
            ValueEffect::Multiply(v) => value * v.calculate(level),
            ValueEffect::Set(v) => v.calculate(level),
            ValueEffect::AllOf(effects) => effects.iter().fold(value, |v, e| e.process(level, v)),
            ValueEffect::Exponential { base, exponent } => {
                (f64::from(value)
                    * f64::from(base.calculate(level)).powf(f64::from(exponent.calculate(level))))
                    as f32
            }
            ValueEffect::RemoveBinomial => value,
        }
    }

    pub fn from_json(v: &Value) -> Option<ValueEffect> {
        let level = |k: &str| LevelValue::from_json(v.get(k)?);
        Some(match type_name(v)? {
            "add" => ValueEffect::Add(level("value")?),
            "multiply" => ValueEffect::Multiply(level("factor")?),
            "set" => ValueEffect::Set(level("value")?),
            "all_of" => ValueEffect::AllOf(
                v.get("effects")?
                    .as_array()?
                    .iter()
                    .map(ValueEffect::from_json)
                    .collect::<Option<_>>()?,
            ),
            "exponential" => ValueEffect::Exponential {
                base: level("base")?,
                exponent: level("exponent")?,
            },
            "remove_binomial" => ValueEffect::RemoveBinomial,
            _ => return None,
        })
    }
}

/// A `type` field without its `minecraft:` namespace.
fn type_name(v: &Value) -> Option<&str> {
    let t = v.get("type")?.as_str()?;
    Some(t.strip_prefix("minecraft:").unwrap_or(t))
}

/// What the client uses of one enchantment.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Enchantment {
    /// `minecraft:trident_spin_attack_strength`.
    pub spin_attack_strength: Option<ValueEffect>,
    pub crossbow_charge_time: Option<ValueEffect>,
    /// `minecraft:prevent_armor_change` (curse of binding).
    pub prevent_armor_change: bool,
    /// Its name (`description`, a text component) and `max_level`, for an enchanting
    /// table's offers (`Enchantment.getFullname`).
    pub description: Option<Arc<Tag>>,
    pub max_level: i32,
}

impl Enchantment {
    /// An enchantment as its codec writes it (only its `effects` are read).
    pub fn from_json(v: &Value) -> Enchantment {
        let effects = v.get("effects");
        let effect = |name: &str| effects?.get(name).and_then(ValueEffect::from_json);
        Enchantment {
            spin_attack_strength: effect("minecraft:trident_spin_attack_strength"),
            crossbow_charge_time: effect("minecraft:crossbow_charge_time"),
            prevent_armor_change: effects
                .and_then(|e| e.get("minecraft:prevent_armor_change"))
                .is_some(),
            description: v.get("description").map(|d| Arc::new(json_text(d))),
            max_level: v.get("max_level").and_then(Value::as_i64).unwrap_or(1) as i32,
        }
    }
}

/// A text component's JSON as the NBT form the HUD draws (strings, numbers, lists and
/// objects as they are).
fn json_text(v: &Value) -> Tag {
    match v {
        Value::String(s) => Tag::String(s.clone()),
        Value::Bool(b) => Tag::Byte(i8::from(*b)),
        Value::Number(n) => n
            .as_i64()
            .and_then(|n| i32::try_from(n).ok())
            .map_or_else(|| Tag::Double(n.as_f64().unwrap_or(0.0)), Tag::Int),
        Value::Array(a) => Tag::List(a.iter().map(json_text).collect()),
        Value::Object(o) => {
            let mut c = ruthenium_protocol::nbt::Compound::new();
            for (k, v) in o {
                c.put(k, json_text(v));
            }
            Tag::Compound(c)
        }
        Value::Null => Tag::String(String::new()),
    }
}

/// The enchantment registry: by network id once the server sent it, by name for
/// vanilla's own (the jar's data pack).
#[derive(Debug, Clone, Default)]
pub struct Enchantments {
    by_id: Vec<Option<Arc<Enchantment>>>,
    by_name: HashMap<String, Arc<Enchantment>>,
}

impl Enchantments {
    /// Vanilla's enchantments from the jar's data pack: each file's path and contents.
    pub fn from_json_files<N: AsRef<str>, B: AsRef<[u8]>>(
        files: impl IntoIterator<Item = (N, B)>,
    ) -> Enchantments {
        let mut by_name = HashMap::new();
        for (name, bytes) in files {
            let file = name.as_ref().rsplit('/').next().unwrap_or_default();
            let Some(stem) = file.strip_suffix(".json") else {
                continue;
            };
            if let Ok(v) = serde_json::from_slice::<Value>(bytes.as_ref()) {
                by_name.insert(
                    format!("minecraft:{stem}"),
                    Arc::new(Enchantment::from_json(&v)),
                );
            }
        }
        Enchantments {
            by_id: Vec::new(),
            by_name,
        }
    }

    /// The registry as the server sent it: an entry's own data, else vanilla's entry
    /// of that name (a known pack's, sent without data).
    pub fn from_registry(entries: &[RegistryEntry], vanilla: &Enchantments) -> Enchantments {
        let mut by_id = Vec::with_capacity(entries.len());
        let mut by_name = HashMap::new();
        for e in entries {
            let ench = match &e.data {
                Some(data) => {
                    let mut ench = Enchantment::from_json(&nbt_to_json(data));
                    // (Its name as sent, not as it went through JSON.)
                    if let Some(d) = data.as_compound().and_then(|c| c.get("description")) {
                        ench.description = Some(Arc::new(d.clone()));
                    }
                    Some(Arc::new(ench))
                }
                None => vanilla.by_name.get(&e.id).cloned(),
            };
            if let Some(ench) = &ench {
                by_name.insert(e.id.clone(), ench.clone());
            }
            by_id.push(ench);
        }
        Enchantments { by_id, by_name }
    }

    /// Vanilla's registry order (the data pack's files sorted by name), as a server
    /// that sends it as a known pack lists it.
    pub fn vanilla_order(&self) -> Enchantments {
        let mut names: Vec<&String> = self.by_name.keys().collect();
        names.sort();
        let entries: Vec<RegistryEntry> = names
            .into_iter()
            .map(|n| RegistryEntry {
                id: n.clone(),
                data: None,
            })
            .collect();
        Enchantments::from_registry(&entries, self)
    }

    pub fn get(&self, id: i32) -> Option<&Enchantment> {
        usize::try_from(id)
            .ok()
            .and_then(|i| self.by_id.get(i)?.as_deref())
    }

    /// The network id of an enchantment, by name.
    pub fn id_of(&self, name: &str) -> Option<i32> {
        let e = self.by_name.get(name)?;
        self.by_id
            .iter()
            .position(|x| x.as_ref().is_some_and(|x| Arc::ptr_eq(x, e)))
            .and_then(|i| i32::try_from(i).ok())
    }

    pub fn len(&self) -> usize {
        self.by_name.len()
    }

    pub fn is_empty(&self) -> bool {
        self.by_name.is_empty()
    }

    /// `EnchantmentHelper.getTridentSpinAttackStrength`: each enchantment's effect on
    /// the value, from 0, in the stack's order.
    pub fn spin_attack_strength(&self, enchantments: &[(i32, i32)]) -> f32 {
        self.modify(enchantments, 0.0, |e| e.spin_attack_strength.as_ref())
    }

    /// `EnchantmentHelper.has(stack, PREVENT_ARMOR_CHANGE)`: one of the stack's
    /// enchantments has the effect.
    pub fn prevents_armor_change(&self, enchantments: &[(i32, i32)]) -> bool {
        enchantments
            .iter()
            .any(|&(id, _)| self.get(id).is_some_and(|e| e.prevent_armor_change))
    }

    /// Loading takes 1.25 seconds before the stack's charge-time effects are applied.
    pub fn crossbow_charge_ticks(&self, enchantments: &[(i32, i32)]) -> i32 {
        let seconds = self.modify(enchantments, 1.25, |e| e.crossbow_charge_time.as_ref());
        (seconds.max(0.0) * 20.0).floor() as i32
    }

    fn modify(
        &self,
        enchantments: &[(i32, i32)],
        mut value: f32,
        effect_of: impl Fn(&Enchantment) -> Option<&ValueEffect>,
    ) -> f32 {
        for &(id, level) in enchantments {
            if let Some(effect) = self.get(id).and_then(&effect_of) {
                value = effect.process(level, value);
            }
        }
        value
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn charge_time_uses_the_stack_level_and_server_effect() {
        let definition = r#"{"effects":{"minecraft:crossbow_charge_time":{"type":"minecraft:add","value":{"type":"minecraft:linear","base":-0.25,"per_level_above_first":-0.25}}}}"#;
        let e = Enchantments::from_json_files([(
            "data/minecraft/enchantment/test.json",
            definition.as_bytes(),
        )])
        .vanilla_order();
        assert_eq!(e.crossbow_charge_ticks(&[]), 25);
        for (level, ticks) in [(1, 20), (2, 15), (3, 10), (6, 0)] {
            assert_eq!(e.crossbow_charge_ticks(&[(0, level)]), ticks);
        }
    }

    #[test]
    fn riptide_strength_by_level() {
        let riptide = serde_json::json!({"effects": {"minecraft:trident_spin_attack_strength": {
            "type": "minecraft:add",
            "value": {"type": "minecraft:linear", "base": 1.5, "per_level_above_first": 0.75}
        }}});
        let sharp = serde_json::json!({"effects": {}});
        let files = [
            (
                "data/minecraft/enchantment/riptide.json",
                riptide.to_string(),
            ),
            (
                "data/minecraft/enchantment/sharpness.json",
                sharp.to_string(),
            ),
        ];
        let vanilla = Enchantments::from_json_files(files.iter().map(|(n, b)| (n, b.as_bytes())));
        let e = vanilla.vanilla_order();
        assert_eq!(e.id_of("minecraft:riptide"), Some(0));
        assert_eq!(e.spin_attack_strength(&[(1, 5)]), 0.0);
        assert_eq!(e.spin_attack_strength(&[(0, 1)]), 1.5);
        assert_eq!(e.spin_attack_strength(&[(1, 2), (0, 3)]), 3.0);
        // A server's own entry replaces vanilla's.
        let own = RegistryEntry {
            id: "minecraft:riptide".into(),
            data: None,
        };
        let e = Enchantments::from_registry(&[own], &Enchantments::default());
        assert_eq!(e.spin_attack_strength(&[(0, 3)]), 0.0);
    }

    #[test]
    fn level_values() {
        let v = |j: Value| LevelValue::from_json(&j).unwrap();
        assert_eq!(v(serde_json::json!(2.5)).calculate(3), 2.5);
        assert_eq!(
            v(serde_json::json!({"type": "levels_squared", "added": 1.0})).calculate(3),
            10.0
        );
        let lookup = serde_json::json!({"type": "lookup", "values": [1.0, 2.0], "fallback": 9.0});
        assert_eq!(v(lookup.clone()).calculate(2), 2.0);
        assert_eq!(v(lookup).calculate(3), 9.0);
        let clamp = serde_json::json!({"type": "clamped", "value": {"type": "linear", "base": 1.0, "per_level_above_first": 1.0}, "min": 0.0, "max": 2.0});
        assert_eq!(v(clamp).calculate(5), 2.0);
        let frac = serde_json::json!({"type": "fraction", "numerator": 1.0, "denominator": 0.0});
        assert_eq!(v(frac).calculate(1), 0.0);
    }
}

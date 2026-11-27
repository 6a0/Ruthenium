//! Language files (`lang/<code>.json`): a flat object of strings, as
//! `locale/Language.loadFromJson` reads one.

use std::collections::HashMap;
use std::fmt;

use serde::de::{Deserialize, DeserializeSeed, Deserializer, IgnoredAny, MapAccess, Visitor};

use crate::resources::Resource;

/// One language file's entries, no more than its pack's limits allow
/// ([`crate::Limits::lang_entries`], [`crate::Limits::json_bytes`]). Vanilla fails the
/// whole file (it is skipped with a warning) when it is not an object or a value is not a
/// string; so does this, and when it has more entries than the limit.
///
/// Deserializes directly into a `HashMap<String, String>` without an intermediate
/// `serde_json::Value` tree, stopping as soon as the entry cap or a non-string value is
/// hit.
pub fn read(file: &Resource<'_>) -> Result<HashMap<String, String>, String> {
    let max = file.limits().lang_entries;
    let text = file.text()?;
    let text = text.strip_prefix('\u{FEFF}').unwrap_or(&text);
    let mut de = serde_json::Deserializer::from_str(text);
    let map = LangSeed { max }
        .deserialize(&mut de)
        .map_err(|e| e.to_string())?;
    de.end().map_err(|e| e.to_string())?;
    Ok(map)
}

/// Streams a language file's string entries directly into `out`, skipping non-string
/// values without building an intermediate `serde_json::Value` tree.
///
/// If `max_bytes` is `Some(limit)` and the file's text is larger than `limit`, returns
/// `Err` before modifying `out`. New keys stop being added once `*added >= max_added`
/// (overrides of existing keys still apply). Returns the byte length of the file text.
pub fn merge_strings(
    file: &Resource<'_>,
    max_bytes: Option<usize>,
    max_added: usize,
    added: &mut usize,
    out: &mut HashMap<String, String>,
) -> Result<usize, String> {
    let text = file.text()?;
    let len = text.len();
    if max_bytes.is_some_and(|m| len > m) {
        return Err(format!("{len} bytes is past the language limit"));
    }
    let text = text.strip_prefix('\u{FEFF}').unwrap_or(&text);
    let mut de = serde_json::Deserializer::from_str(text);
    MergeSeed {
        max_added,
        added,
        out,
    }
    .deserialize(&mut de)
    .map_err(|e| e.to_string())?;
    de.end().map_err(|e| e.to_string())?;
    Ok(len)
}

enum MaybeString {
    Str(String),
    Other,
}

impl<'de> Deserialize<'de> for MaybeString {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = MaybeString;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a JSON value")
            }

            fn visit_str<E: serde::de::Error>(self, v: &str) -> Result<Self::Value, E> {
                Ok(MaybeString::Str(v.to_owned()))
            }

            fn visit_string<E: serde::de::Error>(self, v: String) -> Result<Self::Value, E> {
                Ok(MaybeString::Str(v))
            }

            fn visit_bool<E>(self, _: bool) -> Result<Self::Value, E> {
                Ok(MaybeString::Other)
            }

            fn visit_i64<E>(self, _: i64) -> Result<Self::Value, E> {
                Ok(MaybeString::Other)
            }

            fn visit_u64<E>(self, _: u64) -> Result<Self::Value, E> {
                Ok(MaybeString::Other)
            }

            fn visit_f64<E>(self, _: f64) -> Result<Self::Value, E> {
                Ok(MaybeString::Other)
            }

            fn visit_unit<E>(self) -> Result<Self::Value, E> {
                Ok(MaybeString::Other)
            }

            fn visit_seq<A: serde::de::SeqAccess<'de>>(
                self,
                mut seq: A,
            ) -> Result<Self::Value, A::Error> {
                while seq.next_element::<IgnoredAny>()?.is_some() {}
                Ok(MaybeString::Other)
            }

            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
                while map.next_entry::<IgnoredAny, IgnoredAny>()?.is_some() {}
                Ok(MaybeString::Other)
            }
        }
        deserializer.deserialize_any(V)
    }
}

struct MergeSeed<'a> {
    max_added: usize,
    added: &'a mut usize,
    out: &'a mut HashMap<String, String>,
}

impl<'de> DeserializeSeed<'de> for MergeSeed<'_> {
    type Value = ();

    fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<(), D::Error> {
        deserializer.deserialize_map(MergeVisitor {
            max_added: self.max_added,
            added: self.added,
            out: self.out,
        })
    }
}

struct MergeVisitor<'a> {
    max_added: usize,
    added: &'a mut usize,
    out: &'a mut HashMap<String, String>,
}

impl<'de> Visitor<'de> for MergeVisitor<'_> {
    type Value = ();

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a JSON object")
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<(), A::Error> {
        while let Some((k, v)) = map.next_entry::<String, MaybeString>()? {
            let MaybeString::Str(s) = v else {
                continue;
            };
            if let Some(slot) = self.out.get_mut(&k) {
                *slot = s;
            } else if *self.added < self.max_added {
                self.out.insert(k, s);
                *self.added += 1;
            }
        }
        Ok(())
    }
}

struct LangSeed {
    max: usize,
}

impl<'de> DeserializeSeed<'de> for LangSeed {
    type Value = HashMap<String, String>;

    fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<Self::Value, D::Error> {
        deserializer.deserialize_map(LangVisitor { max: self.max })
    }
}

struct LangVisitor {
    max: usize,
}

impl<'de> Visitor<'de> for LangVisitor {
    type Value = HashMap<String, String>;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a JSON object of strings")
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
        let mut out = HashMap::new();
        while let Some(k) = map.next_key::<String>()? {
            if out.len() >= self.max && !out.contains_key(&k) {
                return Err(serde::de::Error::custom(format!(
                    "more than {} entries in a language file",
                    self.max
                )));
            }
            let v: String = map.next_value()?;
            out.insert(k, v);
        }
        Ok(out)
    }
}

//! Values the network carries as NBT (types with no network codec of their own, the
//! partial predicates of adventure-mode predicates), hashed as their codecs write them.
//!
//! A `CompoundTag` codec (custom data, entity data) writes the tag as it is:
//! `NbtOps.convertTo` gives each tag type its own `HashOps` value. The other values are
//! decoded by their codec first, so e.g. a number is written with the type the codec
//! reads it as, whatever numeric tag carried it.
//!
//! Some values nest other values in their codec's NBT form: the `lock` component's item
//! predicate (its `components` and `predicates`), the partial predicates of an
//! adventure-mode predicate, a hover event's item. Those are written as their NBT is,
//! which is what the codec writes except where it reads a number as a boolean (NBT has
//! none: `true` is `1b`) or writes a list of ints that NBT keeps as an int array. Such a
//! value then hashes differently from vanilla's (a resync of that slot, no worse).

use super::ops::{self, Hash, Rec};
use super::values::{Res, compact};
use super::{Cx, HashError};
use crate::nbt::Tag;
use crate::wire::full_identifier;

fn decode_err<T>(what: &'static str) -> Result<T, HashError> {
    Err(HashError::Unsupported(what))
}

/// `NbtOps.convertTo(HashOps, tag)`.
pub(super) fn generic(t: &Tag) -> Hash {
    match t {
        Tag::Byte(v) => ops::byte(*v),
        Tag::Short(v) => ops::short(*v),
        Tag::Int(v) => ops::int(*v),
        Tag::Long(v) => ops::long(*v),
        Tag::Float(v) => ops::float(*v),
        Tag::Double(v) => ops::double(*v),
        Tag::ByteArray(v) => ops::byte_list(&v.iter().map(|b| *b as u8).collect::<Vec<_>>()),
        Tag::String(s) => ops::string(s),
        Tag::List(items) => ops::list(items.iter().map(generic)),
        Tag::Compound(c) => {
            let mut rec = Rec::new();
            for (k, v) in c.iter() {
                rec.put(k, generic(v));
            }
            rec.build()
        }
        Tag::IntArray(v) => ops::int_list(v.iter().copied()),
        Tag::LongArray(v) => ops::long_list(v.iter().copied()),
    }
}

/// `Tag.toString()` (`StringTagVisitor`): the SNBT a lenient NBT codec writes.
pub(super) fn snbt(t: &Tag) -> String {
    let mut out = String::new();
    write_snbt(t, &mut out);
    out
}

fn write_snbt(t: &Tag, o: &mut String) {
    use std::fmt::Write;
    match t {
        Tag::Byte(v) => {
            let _ = write!(o, "{v}b");
        }
        Tag::Short(v) => {
            let _ = write!(o, "{v}s");
        }
        Tag::Int(v) => {
            let _ = write!(o, "{v}");
        }
        Tag::Long(v) => {
            let _ = write!(o, "{v}L");
        }
        Tag::Float(v) => {
            o.push_str(&java_float(*v as f64, v.is_nan(), &format!("{v:e}")));
            o.push('f');
        }
        Tag::Double(v) => {
            o.push_str(&java_float(*v, v.is_nan(), &format!("{v:e}")));
            o.push('d');
        }
        Tag::ByteArray(v) => {
            o.push_str("[B;");
            for (i, b) in v.iter().enumerate() {
                if i != 0 {
                    o.push(',');
                }
                let _ = write!(o, "{b}B");
            }
            o.push(']');
        }
        Tag::IntArray(v) => {
            o.push_str("[I;");
            for (i, n) in v.iter().enumerate() {
                if i != 0 {
                    o.push(',');
                }
                let _ = write!(o, "{n}");
            }
            o.push(']');
        }
        Tag::LongArray(v) => {
            o.push_str("[L;");
            for (i, n) in v.iter().enumerate() {
                if i != 0 {
                    o.push(',');
                }
                let _ = write!(o, "{n}L");
            }
            o.push(']');
        }
        Tag::String(s) => quote(s, o),
        Tag::List(items) => {
            o.push('[');
            for (i, e) in items.iter().enumerate() {
                if i != 0 {
                    o.push(',');
                }
                write_snbt(e, o);
            }
            o.push(']');
        }
        Tag::Compound(c) => {
            o.push('{');
            let mut entries: Vec<(&str, &Tag)> = c.iter().collect();
            // `String.compareTo`: by UTF-16 units.
            entries.sort_by(|a, b| a.0.encode_utf16().cmp(b.0.encode_utf16()));
            for (i, (k, v)) in entries.into_iter().enumerate() {
                if i != 0 {
                    o.push(',');
                }
                let plain = !k.eq_ignore_ascii_case("true")
                    && !k.eq_ignore_ascii_case("false")
                    && unquoted_key(k);
                if plain {
                    o.push_str(k);
                } else {
                    quote(k, o);
                }
                o.push(':');
                write_snbt(v, o);
            }
            o.push('}');
        }
    }
}

/// `[A-Za-z._]+[A-Za-z0-9._+-]*`.
fn unquoted_key(k: &str) -> bool {
    let mut chars = k.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() || c == '.' || c == '_' => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '+' | '-'))
}

/// `StringTag.quoteAndEscape`: double quotes unless the first quote inside is one.
fn quote(s: &str, o: &mut String) {
    let mut body = String::new();
    let mut q: Option<char> = None;
    for c in s.chars() {
        match c {
            '\\' => body.push_str("\\\\"),
            '"' | '\'' => {
                let quote = *q.get_or_insert(if c == '"' { '\'' } else { '"' });
                if quote == c {
                    body.push('\\');
                }
                body.push(c);
            }
            '\u{8}' => body.push_str("\\b"),
            '\t' => body.push_str("\\t"),
            '\n' => body.push_str("\\n"),
            '\u{c}' => body.push_str("\\f"),
            '\r' => body.push_str("\\r"),
            c if (c as u32) < 0x20 => body.push_str(&format!("\\x{:02X}", c as u32)),
            c => body.push(c),
        }
    }
    let q = q.unwrap_or('"');
    o.push(q);
    o.push_str(&body);
    o.push(q);
}

/// `Float.toString` / `Double.toString` from Rust's shortest digits (`sci`, as
/// `{:e}` prints them): plain from 10^-3 up to 10^7, else `d.dddE±n`.
fn java_float(v: f64, nan: bool, sci: &str) -> String {
    if nan {
        return "NaN".into();
    }
    if v.is_infinite() {
        return if v > 0.0 {
            "Infinity".into()
        } else {
            "-Infinity".into()
        };
    }
    if v == 0.0 {
        return if v.is_sign_negative() {
            "-0.0".into()
        } else {
            "0.0".into()
        };
    }
    let (mantissa, exp) = sci.split_once('e').unwrap_or((sci, "0"));
    let exp: i32 = exp.parse().unwrap_or(0);
    let (neg, mantissa) = match mantissa.strip_prefix('-') {
        Some(m) => (true, m),
        None => (false, mantissa),
    };
    let digits: String = mantissa.chars().filter(|c| c.is_ascii_digit()).collect();
    let mut out = String::new();
    if neg {
        out.push('-');
    }
    if (-3..7).contains(&exp) {
        if exp < 0 {
            out.push_str("0.");
            for _ in 0..(-exp - 1) {
                out.push('0');
            }
            out.push_str(&digits);
        } else {
            let int_len = exp as usize + 1;
            if digits.len() <= int_len {
                out.push_str(&digits);
                for _ in digits.len()..int_len {
                    out.push('0');
                }
                out.push_str(".0");
            } else {
                out.push_str(&digits[..int_len]);
                out.push('.');
                out.push_str(&digits[int_len..]);
            }
        }
    } else {
        out.push_str(&digits[..1]);
        out.push('.');
        out.push_str(if digits.len() > 1 { &digits[1..] } else { "0" });
        out.push_str(&format!("E{exp}"));
    }
    out
}

/// A numeric tag as a `Number` (`NbtOps.getNumberValue`).
fn number(t: &Tag) -> Option<f64> {
    Some(match t {
        Tag::Byte(v) => *v as f64,
        Tag::Short(v) => *v as f64,
        Tag::Int(v) => *v as f64,
        Tag::Long(v) => *v as f64,
        Tag::Float(v) => *v as f64,
        Tag::Double(v) => *v,
        _ => return None,
    })
}

fn long(t: &Tag) -> Option<i64> {
    Some(match t {
        Tag::Byte(v) => *v as i64,
        Tag::Short(v) => *v as i64,
        Tag::Int(v) => *v as i64,
        Tag::Long(v) => *v,
        Tag::Float(v) => *v as i64,
        Tag::Double(v) => *v as i64,
        _ => return None,
    })
}

fn float(t: &Tag) -> Option<f32> {
    Some(match t {
        Tag::Float(v) => *v,
        other => number(other)? as f32,
    })
}

fn identifier(t: &Tag) -> Result<String, HashError> {
    match t {
        Tag::String(s) if crate::wire::valid_identifier(s) => Ok(full_identifier(s)),
        _ => decode_err("an identifier that isn't one"),
    }
}

/// `MapDecorations.CODEC`: name to `{type, x, z, rotation}`.
pub(super) fn map_decorations(t: &Tag) -> Res {
    let Tag::Compound(c) = t else {
        return decode_err("map decorations");
    };
    let mut rec = Rec::new();
    for (name, v) in c.iter() {
        let Tag::Compound(e) = v else {
            return decode_err("map decoration");
        };
        let mut entry = Rec::new();
        entry.put(
            "type",
            ops::string(&identifier(e.get("type").unwrap_or(&Tag::Int(0)))?),
        );
        let num = |k: &str| {
            e.get(k)
                .and_then(number)
                .ok_or(HashError::Unsupported("map decoration"))
        };
        entry.put("x", ops::double(num("x")?));
        entry.put("z", ops::double(num("z")?));
        let rot = e
            .get("rotation")
            .and_then(float)
            .ok_or(HashError::Unsupported("map decoration"))?;
        entry.put("rotation", ops::float(rot));
        rec.put(name, entry.build());
    }
    Ok(rec.build())
}

/// `DebugStickState.CODEC`: block to the name of its selected property.
pub(super) fn debug_stick_state(t: &Tag) -> Res {
    let Tag::Compound(c) = t else {
        return decode_err("debug stick state");
    };
    let mut rec = Rec::new();
    for (block, v) in c.iter() {
        let Tag::String(prop) = v else {
            return decode_err("debug stick state");
        };
        rec.entry(
            ops::string(&identifier(&Tag::String(block.to_owned()))?),
            ops::string(prop),
        );
    }
    Ok(rec.build())
}

/// The `recipes` component: a list of recipe keys.
pub(super) fn recipes(t: &Tag) -> Res {
    let Tag::List(items) = t else {
        return decode_err("recipes");
    };
    let mut out = Vec::with_capacity(items.len());
    for i in items {
        out.push(ops::string(&identifier(i)?));
    }
    Ok(ops::list(out))
}

/// `SeededContainerLoot.CODEC`: the loot table, and the seed unless it is 0.
pub(super) fn container_loot(t: &Tag) -> Res {
    let Tag::Compound(c) = t else {
        return decode_err("container loot");
    };
    let mut rec = Rec::new();
    rec.put(
        "loot_table",
        ops::string(&identifier(c.get("loot_table").unwrap_or(&Tag::Int(0)))?),
    );
    if let Some(seed) = c.get("seed") {
        let seed = long(seed).ok_or(HashError::Unsupported("container loot seed"))?;
        if seed != 0 {
            rec.put("seed", ops::long(seed));
        }
    }
    Ok(rec.build())
}

/// A holder set written in NBT (`#tag`, a key, or a list of keys), as its codec
/// rewrites it.
fn holder_set(t: &Tag) -> Res {
    match t {
        Tag::String(s) => match s.strip_prefix('#') {
            Some(tag) => Ok(ops::string(&format!(
                "#{}",
                identifier(&Tag::String(tag.to_owned()))?
            ))),
            None => Ok(ops::string(&identifier(t)?)),
        },
        Tag::List(items) => {
            let mut out = Vec::with_capacity(items.len());
            for i in items {
                out.push(ops::string(&identifier(i)?));
            }
            Ok(compact(out))
        }
        _ => decode_err("holder set"),
    }
}

/// The `lock` component (`ItemPredicate.CODEC`): the items, the count bounds, and the
/// component matchers. The components and partial predicates in it are NBT whose
/// schema is each type's codec; they are written as the NBT is (see the module notes).
pub(super) fn item_predicate(t: &Tag, _cx: &Cx<'_>) -> Res {
    let Tag::Compound(c) = t else {
        return decode_err("item predicate");
    };
    let mut rec = Rec::new();
    for (k, v) in c.iter() {
        match k {
            "items" => {
                rec.put("items", holder_set(v)?);
            }
            "count" | "components" | "predicates" => {
                rec.put(k, generic(v));
            }
            // Unknown fields are ignored by the decoder, and so not written back.
            _ => {}
        }
    }
    Ok(rec.build())
}

/// A partial predicate of type `name` sent as NBT (`DataComponentPredicate`'s
/// `singleStreamCodec`). Its codec's own output came across, so it is written as the
/// NBT is (booleans and int lists aside, see the module notes).
pub(super) fn predicate(_name: &str, t: &Tag, _cx: &Cx<'_>) -> Res {
    Ok(generic(t))
}

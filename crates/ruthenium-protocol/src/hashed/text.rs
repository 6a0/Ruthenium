//! Text components (`ComponentSerialization.CODEC`), read from the NBT the network
//! carries and hashed the way the codec writes them again.
//!
//! The NBT was written by the same codec, but the client decodes it into a `Component`
//! and encodes that, so the hash follows the decoder's rules: a plain string or a
//! compound; a list is its first element with the rest appended as siblings; a
//! compound's contents are found by its `type` field or else by trying each content
//! type in turn (text, translatable, keybind, score, selector, nbt, object); NBT has no
//! booleans, so the style's flags are numbers that read as `!= 0`. Encoding drops the
//! `type` field, writes a component with literal contents, no style and no siblings as
//! a bare string (`tryCollapseToString`), and spells colours out canonically.

use super::ops::{self, Hash, Rec};
use super::values::{Res, player_name, uuid_hash};
use super::{Cx, HashError, Reg, nbt_form};
use crate::nbt::{Compound, Tag};
use crate::wire::{full_identifier, valid_identifier};

/// How deeply components may nest (siblings, arguments, hover texts).
const MAX_DEPTH: u32 = 128;

fn bad<T>(what: &'static str) -> Result<T, HashError> {
    Err(HashError::Unsupported(what))
}

/// A decoded component: its contents' fields, the literal text if the contents are
/// plain text, its style's fields, and its siblings, each already hashed.
struct Node {
    contents: Rec,
    literal: Option<String>,
    style: Rec,
    style_empty: bool,
    extra: Vec<Hash>,
}

impl Node {
    fn hash(self) -> Hash {
        if let Some(text) = &self.literal
            && self.style_empty
            && self.extra.is_empty()
        {
            return ops::string(text);
        }
        let mut rec = self.contents;
        if !self.extra.is_empty() {
            rec.put("extra", ops::list(self.extra));
        }
        rec.extend(self.style);
        rec.build()
    }
}

/// The hash of the component this NBT decodes to.
pub(super) fn hash(t: &Tag, cx: &Cx<'_>) -> Res {
    component(t, cx, 0)
}

fn component(t: &Tag, cx: &Cx<'_>, depth: u32) -> Res {
    Ok(node(t, cx, depth)?.hash())
}

fn node(t: &Tag, cx: &Cx<'_>, depth: u32) -> Result<Node, HashError> {
    if depth > MAX_DEPTH {
        return bad("text nested too deeply");
    }
    match t {
        Tag::String(s) => Ok(Node {
            contents: literal_rec(s),
            literal: Some(s.clone()),
            style: Rec::new(),
            style_empty: true,
            extra: Vec::new(),
        }),
        Tag::List(items) => {
            let Some((first, rest)) = items.split_first() else {
                return bad("an empty text list");
            };
            let mut n = node(first, cx, depth + 1)?;
            for i in rest {
                n.extra.push(component(i, cx, depth + 1)?);
            }
            Ok(n)
        }
        Tag::Compound(c) => compound(c, cx, depth),
        _ => bad("text that is neither a string, a list nor a compound"),
    }
}

fn literal_rec(s: &str) -> Rec {
    let mut rec = Rec::new();
    rec.put("text", ops::string(s));
    rec
}

fn string_of(t: Option<&Tag>) -> Option<&str> {
    match t {
        Some(Tag::String(s)) => Some(s),
        _ => None,
    }
}

/// `NbtOps.getNumberValue` then `intValue`.
fn int_of(t: &Tag) -> Option<i32> {
    Some(match t {
        Tag::Byte(v) => *v as i32,
        Tag::Short(v) => *v as i32,
        Tag::Int(v) => *v,
        Tag::Long(v) => *v as i32,
        Tag::Float(v) => *v as i32,
        Tag::Double(v) => *v as i32,
        _ => return None,
    })
}

fn float_of(t: &Tag) -> Option<f32> {
    Some(match t {
        Tag::Byte(v) => *v as f32,
        Tag::Short(v) => *v as f32,
        Tag::Int(v) => *v as f32,
        Tag::Long(v) => *v as f32,
        Tag::Float(v) => *v,
        Tag::Double(v) => *v as f32,
        _ => return None,
    })
}

/// `NbtOps.getBooleanValue`: a number other than zero.
fn bool_of(t: &Tag) -> Option<bool> {
    Some(match t {
        Tag::Byte(v) => *v != 0,
        Tag::Short(v) => *v != 0,
        Tag::Int(v) => *v != 0,
        Tag::Long(v) => *v != 0,
        Tag::Float(v) => *v != 0.0,
        Tag::Double(v) => *v != 0.0,
        _ => return None,
    })
}

fn identifier_of(t: Option<&Tag>) -> Option<String> {
    let s = string_of(t)?;
    valid_identifier(s).then(|| full_identifier(s))
}

fn compound(c: &Compound, cx: &Cx<'_>, depth: u32) -> Result<Node, HashError> {
    let (contents, literal) = contents(c, cx, depth)?;
    let mut extra = Vec::new();
    if let Some(e) = c.get("extra") {
        let Tag::List(items) = e else {
            return bad("text siblings that aren't a list");
        };
        if items.is_empty() {
            return bad("an empty list of text siblings");
        }
        for i in items {
            extra.push(component(i, cx, depth + 1)?);
        }
    }
    let (style, style_empty) = style(c, cx, depth)?;
    Ok(Node {
        contents,
        literal,
        style,
        style_empty,
        extra,
    })
}

/// The contents: by the `type` field, else the first content codec that decodes.
fn contents(c: &Compound, cx: &Cx<'_>, depth: u32) -> Result<(Rec, Option<String>), HashError> {
    const TYPES: [&str; 7] = [
        "text",
        "translatable",
        "keybind",
        "score",
        "selector",
        "nbt",
        "object",
    ];
    if let Some(t) = c.get("type") {
        let Some(name) = string_of(Some(t)) else {
            return bad("text type");
        };
        return match content_of(name, c, cx, depth)? {
            Some(r) => Ok(r),
            None => bad("text contents that don't match their type"),
        };
    }
    for name in TYPES {
        if let Some(r) = content_of(name, c, cx, depth)? {
            return Ok(r);
        }
    }
    bad("text with no contents")
}

/// One content type's map codec: `None` if the compound doesn't decode as it.
fn content_of(
    name: &str,
    c: &Compound,
    cx: &Cx<'_>,
    depth: u32,
) -> Result<Option<(Rec, Option<String>)>, HashError> {
    let mut rec = Rec::new();
    match name {
        "text" => {
            let Some(text) = string_of(c.get("text")) else {
                return Ok(None);
            };
            rec.put("text", ops::string(text));
            return Ok(Some((rec, Some(text.to_owned()))));
        }
        "translatable" => {
            let Some(key) = string_of(c.get("translate")) else {
                return Ok(None);
            };
            rec.put("translate", ops::string(key));
            // Lenient: a fallback that isn't a string is dropped.
            rec.opt("fallback", string_of(c.get("fallback")).map(ops::string));
            if let Some(with) = c.get("with") {
                let Tag::List(args) = with else {
                    return Ok(None);
                };
                let mut out = Vec::with_capacity(args.len());
                for a in args {
                    out.push(match a {
                        // Primitives keep their type (`ExtraCodecs.JAVA`).
                        Tag::String(s) => ops::string(s),
                        Tag::Byte(_)
                        | Tag::Short(_)
                        | Tag::Int(_)
                        | Tag::Long(_)
                        | Tag::Float(_)
                        | Tag::Double(_) => nbt_form::generic(a),
                        other => match component(other, cx, depth + 1) {
                            Ok(h) => h,
                            Err(_) => return Ok(None),
                        },
                    });
                }
                if !out.is_empty() {
                    rec.put("with", ops::list(out));
                }
            }
        }
        "keybind" => {
            let Some(key) = string_of(c.get("keybind")) else {
                return Ok(None);
            };
            rec.put("keybind", ops::string(key));
        }
        "score" => {
            let Some(Tag::Compound(score)) = c.get("score") else {
                return Ok(None);
            };
            let (Some(n), Some(o)) = (
                string_of(score.get("name")),
                string_of(score.get("objective")),
            ) else {
                return Ok(None);
            };
            let mut s = Rec::new();
            s.put("name", ops::string(n))
                .put("objective", ops::string(o));
            rec.put("score", s.build());
        }
        "selector" => {
            let Some(sel) = string_of(c.get("selector")) else {
                return Ok(None);
            };
            rec.put("selector", ops::string(sel));
            if let Some(sep) = c.get("separator") {
                match component(sep, cx, depth + 1) {
                    Ok(h) => rec.put("separator", h),
                    Err(_) => return Ok(None),
                };
            }
        }
        "nbt" => {
            let Some(path) = string_of(c.get("nbt")) else {
                return Ok(None);
            };
            rec.put("nbt", ops::string(path));
            // Lenient flags and separator.
            if c.get("interpret").and_then(bool_of) == Some(true) {
                rec.put("interpret", ops::boolean(true));
            }
            if c.get("plain").and_then(bool_of) == Some(true) {
                rec.put("plain", ops::boolean(true));
            }
            if let Some(sep) = c.get("separator")
                && let Ok(h) = component(sep, cx, depth + 1)
            {
                rec.put("separator", h);
            }
            let source = |name: &str, rec: &mut Rec| -> bool {
                match name {
                    "entity" | "block" => match string_of(c.get(name)) {
                        Some(v) => {
                            rec.put(name, ops::string(v));
                            true
                        }
                        None => false,
                    },
                    _ => match identifier_of(c.get("storage")) {
                        Some(v) => {
                            rec.put("storage", ops::string(&v));
                            true
                        }
                        None => false,
                    },
                }
            };
            let found = match string_of(c.get("source")) {
                Some(s) => source(s, &mut rec),
                None => ["entity", "block", "storage"]
                    .iter()
                    .any(|s| source(s, &mut rec)),
            };
            if !found {
                return Ok(None);
            }
        }
        "object" => {
            let atlas = |rec: &mut Rec| -> bool {
                let Some(sprite) = identifier_of(c.get("sprite")) else {
                    return false;
                };
                if let Some(a) = identifier_of(c.get("atlas"))
                    && a != "minecraft:blocks"
                {
                    rec.put("atlas", ops::string(&a));
                }
                rec.put("sprite", ops::string(&sprite));
                true
            };
            let player = |rec: &mut Rec| -> Result<bool, HashError> {
                let Some(p) = c.get("player") else {
                    return Ok(false);
                };
                rec.put("player", profile(p, cx)?);
                if c.get("hat").and_then(bool_of) == Some(false) {
                    rec.put("hat", ops::boolean(false));
                }
                Ok(true)
            };
            let found = match string_of(c.get("object")) {
                Some("atlas") => atlas(&mut rec),
                Some("player") => player(&mut rec)?,
                Some(_) => false,
                None => atlas(&mut rec) || player(&mut rec)?,
            };
            if !found {
                return Ok(None);
            }
            if let Some(f) = c.get("fallback") {
                rec.put("fallback", component(f, cx, depth + 1)?);
            }
        }
        _ => return bad("unknown text type"),
    }
    Ok(Some((rec, None)))
}

/// `ResolvableProfile.CODEC` from NBT: a name alone, or the full form.
fn profile(t: &Tag, _cx: &Cx<'_>) -> Res {
    match t {
        Tag::String(name) => {
            let mut rec = Rec::new();
            rec.put("name", player_name(name)?);
            Ok(rec.build())
        }
        Tag::Compound(c) => {
            let mut rec = Rec::new();
            if let Some(n) = string_of(c.get("name")) {
                rec.put("name", player_name(n)?);
            }
            if let Some(Tag::IntArray(id)) = c.get("id")
                && id.len() == 4
            {
                let u = id.iter().fold(0u128, |a, &w| (a << 32) | w as u32 as u128);
                rec.put("id", uuid_hash(u));
            }
            for k in ["properties", "texture", "cape", "elytra", "model"] {
                if let Some(v) = c.get(k) {
                    rec.put(k, nbt_form::generic(v));
                }
            }
            Ok(rec.build())
        }
        _ => bad("profile"),
    }
}

/// `Style.Serializer.MAP_CODEC`: every field optional, written when set.
fn style(c: &Compound, cx: &Cx<'_>, depth: u32) -> Result<(Rec, bool), HashError> {
    let mut rec = Rec::new();
    let mut empty = true;
    let mut put = |rec: &mut Rec, name: &str, h: Hash| {
        rec.put(name, h);
        empty = false;
    };
    if let Some(t) = c.get("color") {
        let Some(name) = string_of(Some(t)) else {
            return bad("text colour");
        };
        put(&mut rec, "color", ops::string(&color(name)?));
    }
    if let Some(t) = c.get("shadow_color") {
        let argb = match t {
            Tag::List(v) if v.len() == 4 => {
                let f: Vec<f32> = v.iter().filter_map(float_of).collect();
                if f.len() != 4 {
                    return bad("shadow colour");
                }
                // `ARGB.colorFromFloat(w, x, y, z)`.
                let c = |x: f32| ((x * 255.0).floor() as i32) & 0xFF;
                (c(f[3]) << 24) | (c(f[0]) << 16) | (c(f[1]) << 8) | c(f[2])
            }
            other => int_of(other).ok_or(HashError::Unsupported("shadow colour"))?,
        };
        put(&mut rec, "shadow_color", ops::int(argb));
    }
    for flag in [
        "bold",
        "italic",
        "underlined",
        "strikethrough",
        "obfuscated",
    ] {
        if let Some(t) = c.get(flag) {
            let v = bool_of(t).ok_or(HashError::Unsupported("text style flag"))?;
            put(&mut rec, flag, ops::boolean(v));
        }
    }
    if let Some(t) = c.get("click_event") {
        put(&mut rec, "click_event", click_event(t)?);
    }
    if let Some(t) = c.get("hover_event") {
        put(&mut rec, "hover_event", hover_event(t, cx, depth)?);
    }
    if let Some(t) = c.get("insertion") {
        let Some(s) = string_of(Some(t)) else {
            return bad("insertion");
        };
        put(&mut rec, "insertion", ops::string(s));
    }
    if let Some(t) = c.get("font") {
        let Some(id) = identifier_of(Some(t)) else {
            return bad("font");
        };
        put(&mut rec, "font", ops::string(&id));
    }
    Ok((rec, empty))
}

/// `TextColor.parseColor` then `serialize`: a named colour stays named; `#rrggbb` is
/// written as `#RRGGBB`.
fn color(s: &str) -> Result<String, HashError> {
    const NAMED: [&str; 16] = [
        "black",
        "dark_blue",
        "dark_green",
        "dark_aqua",
        "dark_red",
        "dark_purple",
        "gold",
        "gray",
        "dark_gray",
        "blue",
        "green",
        "aqua",
        "red",
        "light_purple",
        "yellow",
        "white",
    ];
    if let Some(hex) = s.strip_prefix('#') {
        // `Integer.parseInt(hex, 16)` takes a sign.
        let v = i64::from_str_radix(hex, 16).map_err(|_| HashError::Unsupported("text colour"))?;
        if !(0..=0xFF_FFFF).contains(&v) || hex.is_empty() || hex.len() > 12 {
            return bad("text colour");
        }
        return Ok(format!("#{v:06X}"));
    }
    if NAMED.contains(&s) {
        Ok(s.to_owned())
    } else {
        bad("text colour")
    }
}

/// `ClickEvent.CODEC`: dispatched on `action`.
fn click_event(t: &Tag) -> Res {
    let Tag::Compound(c) = t else {
        return bad("click event");
    };
    let Some(action) = string_of(c.get("action")) else {
        return bad("click event action");
    };
    let mut rec = Rec::new();
    rec.put("action", ops::string(action));
    let field =
        |name: &str| string_of(c.get(name)).ok_or(HashError::Unsupported("click event field"));
    match action {
        "open_url" => {
            rec.put("url", ops::string(field("url")?));
        }
        "run_command" | "suggest_command" => {
            rec.put("command", ops::string(field("command")?));
        }
        "change_page" => {
            let page = c
                .get("page")
                .and_then(int_of)
                .ok_or(HashError::Unsupported("click event page"))?;
            rec.put("page", super::values::positive(page)?);
        }
        "copy_to_clipboard" => {
            rec.put("value", ops::string(field("value")?));
        }
        "show_dialog" => match c.get("dialog") {
            Some(Tag::String(id)) if valid_identifier(id) => {
                rec.put("dialog", ops::string(&full_identifier(id)));
            }
            _ => return bad("an inline dialog"),
        },
        "custom" => {
            let id = identifier_of(c.get("id")).ok_or(HashError::Unsupported("click event id"))?;
            rec.put("id", ops::string(&id));
            rec.opt("payload", c.get("payload").map(nbt_form::generic));
        }
        // `open_file` isn't allowed from a server, and fails to encode.
        _ => return bad("click event action"),
    }
    Ok(rec.build())
}

/// `HoverEvent.CODEC`: dispatched on `action`.
fn hover_event(t: &Tag, cx: &Cx<'_>, depth: u32) -> Res {
    let Tag::Compound(c) = t else {
        return bad("hover event");
    };
    let Some(action) = string_of(c.get("action")) else {
        return bad("hover event action");
    };
    let mut rec = Rec::new();
    rec.put("action", ops::string(action));
    match action {
        "show_text" => {
            let v = c.get("value").ok_or(HashError::Unsupported("hover text"))?;
            rec.put("value", component(v, cx, depth + 1)?);
        }
        "show_item" => {
            // `ItemStackTemplate.MAP_CODEC` inline.
            let id = identifier_of(c.get("id")).ok_or(HashError::Unsupported("hover item"))?;
            cx.key_id(Reg::Item, &id)?;
            rec.put("id", ops::string(&id));
            if let Some(n) = c.get("count").and_then(int_of)
                && n != 1
            {
                rec.put("count", super::values::int_in(n, 1, 99)?);
            }
            if let Some(Tag::Compound(comps)) = c.get("components")
                && !comps.is_empty()
            {
                // Each component's value in its codec's NBT form (see nbt_form).
                rec.put(
                    "components",
                    nbt_form::generic(&Tag::Compound(comps.clone())),
                );
            }
        }
        "show_entity" => {
            let id = identifier_of(c.get("id")).ok_or(HashError::Unsupported("hover entity"))?;
            cx.key_id(Reg::EntityType, &id)?;
            rec.put("id", ops::string(&id));
            let uuid = match c.get("uuid") {
                Some(Tag::IntArray(v)) if v.len() == 4 => {
                    v.iter().fold(0u128, |a, &w| (a << 32) | w as u32 as u128)
                }
                Some(Tag::String(s)) => {
                    parse_uuid(s).ok_or(HashError::Unsupported("hover entity uuid"))?
                }
                _ => return bad("hover entity uuid"),
            };
            rec.put("uuid", uuid_hash(uuid));
            if let Some(n) = c.get("name") {
                rec.put("name", component(n, cx, depth + 1)?);
            }
        }
        _ => return bad("hover event action"),
    }
    Ok(rec.build())
}

/// `UUID.fromString` for the usual 8-4-4-4-12 form.
fn parse_uuid(s: &str) -> Option<u128> {
    let parts: Vec<&str> = s.split('-').collect();
    if parts.len() != 5 {
        return None;
    }
    let mut v: u128 = 0;
    for (p, bits) in parts.iter().zip([32u32, 16, 16, 16, 48]) {
        let n = u64::from_str_radix(p, 16).ok()? as u128;
        if n >> bits != 0 {
            return None;
        }
        v = (v << bits) | n;
    }
    Some(v)
}

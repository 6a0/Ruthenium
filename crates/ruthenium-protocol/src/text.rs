//! Text components, flattened into styled spans for drawing.
//!
//! The JSON form (`network/chat/ComponentSerialization.java`): a string is literal text;
//! an array is its first element with the rest appended; an object has one content
//! (`text`, `translate` with `with` and `fallback`, `keybind`, `score`,
//! `selector`, `nbt`, `object`), style fields and `extra` children, which inherit its
//! style. Literal text may hold legacy `§` formatting codes, applied when drawn as
//! vanilla's font does (`util/StringDecomposer.java`): a colour code sets the colour and
//! clears the other formats, `§r` goes back to the component's own style, and an
//! unknown code is dropped with its `§`.
//!
//! A translation takes its format from the language ([`Lang`]), else its `fallback`,
//! else its key, with `%s` and `%1$s` filled in. Scores, selectors, NBT and objects show nothing (the
//! server list resolves them without a source). Nesting past 16 levels is dropped, as the
//! server list's sanitising does (`multiplayer/ServerStatusPinger.java`).
//!
//! The NBT form (play and configuration packets) is the same structure in tags: a string
//! tag is literal text, a list is an array, a compound an object. Booleans are bytes
//! there, and vanilla's codecs read any number as a boolean (non-zero is true) in either
//! form.

use serde_json::Value;

use crate::nbt::Tag;

const MAX_DEPTH: usize = 16;

/// Characters one component may expand to: a translation's `%1$s` repeated, nested in
/// arguments that repeat it again, grows as a power of the nesting (vanilla's own
/// formatter has no such bound). Chat lines, titles, names and death and disconnect
/// messages are far shorter; the rest is dropped.
pub const MAX_EXPANDED_CHARS: usize = 32 * 1024;
/// Runs of one style one component may expand to.
pub const MAX_EXPANDED_SPANS: usize = 4096;
/// Components and format pieces one expansion may go through, whatever they produce: a
/// tree of empty arguments, each used many times, makes nothing and costs as much.
pub const MAX_EXPANSION_STEPS: usize = 64 * 1024;
/// Bytes of literal text and translation formats one expansion may read, whatever they
/// produce: a format of nothing but `§` codes, named many times through nested
/// arguments, makes nothing and is read anew each time. Each byte read counts once per
/// reading.
pub const MAX_SCANNED_BYTES: usize = 1 << 20;

/// What is left of one expansion's [`MAX_EXPANDED_CHARS`], [`MAX_EXPANDED_SPANS`],
/// [`MAX_EXPANSION_STEPS`] and [`MAX_SCANNED_BYTES`]; once any is used up, nothing more
/// is added.
#[derive(Debug, Clone, Copy)]
struct Budget {
    chars: usize,
    steps: usize,
    scanned: usize,
}

impl Budget {
    fn new() -> Budget {
        Budget {
            chars: MAX_EXPANDED_CHARS,
            steps: MAX_EXPANSION_STEPS,
            scanned: MAX_SCANNED_BYTES,
        }
    }

    /// Unbounded: for text already within bounds being split up.
    fn unbounded() -> Budget {
        Budget {
            chars: usize::MAX,
            steps: usize::MAX,
            scanned: usize::MAX,
        }
    }

    fn spent(&self) -> bool {
        self.chars == 0 || self.steps == 0 || self.scanned == 0
    }

    /// One step taken; false if there were none left.
    fn step(&mut self) -> bool {
        if self.spent() {
            return false;
        }
        self.steps -= 1;
        true
    }

    /// `bytes` to be read; false (and nothing left at all) if there aren't that many.
    fn scan(&mut self, bytes: usize) -> bool {
        if self.spent() || bytes > self.scanned {
            self.scanned = 0;
            return false;
        }
        self.scanned -= bytes;
        true
    }
}

/// Translation formats by key: a language file.
pub trait Lang {
    fn get(&self, key: &str) -> Option<&str>;

    /// The key the player has bound to the key binding `binding` (`key.sneak`), where
    /// this knows the player's bindings; else text shows a binding's default key.
    fn bound_key(&self, _binding: &str) -> Option<BoundKey<'_>> {
        None
    }
}

/// A key as a binding has it: its name (`key.keyboard.e`, `key.mouse.4`), its value
/// (SDL scancode or button), and for a keyboard key what the keyboard's layout calls it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BoundKey<'a> {
    pub name: &'a str,
    pub value: i32,
    pub mouse: bool,
    pub system: Option<&'a str>,
}

/// The player's key bindings, for text that names them.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BoundKeys {
    keys: Vec<Bound>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Bound {
    binding: String,
    name: String,
    value: i32,
    mouse: bool,
    system: Option<String>,
}

impl BoundKeys {
    pub fn push(
        &mut self,
        binding: &str,
        name: &str,
        value: i32,
        mouse: bool,
        system: Option<String>,
    ) {
        self.keys.push(Bound {
            binding: binding.to_owned(),
            name: name.to_owned(),
            value,
            mouse,
            system,
        });
    }

    pub fn get(&self, binding: &str) -> Option<BoundKey<'_>> {
        let b = self.keys.iter().find(|b| b.binding == binding)?;
        Some(BoundKey {
            name: &b.name,
            value: b.value,
            mouse: b.mouse,
            system: b.system.as_deref(),
        })
    }
}

/// A language with the player's key bindings.
#[derive(Clone, Copy)]
#[allow(missing_debug_implementations)]
pub struct WithKeys<'a> {
    pub lang: &'a dyn Lang,
    pub keys: &'a BoundKeys,
}

impl Lang for WithKeys<'_> {
    fn get(&self, key: &str) -> Option<&str> {
        self.lang.get(key)
    }

    fn bound_key(&self, binding: &str) -> Option<BoundKey<'_>> {
        self.keys.get(binding)
    }
}

/// No language: translations show their fallback or key.
impl Lang for () {
    fn get(&self, _: &str) -> Option<&str> {
        None
    }
}

impl Lang for std::collections::HashMap<String, String> {
    fn get(&self, key: &str) -> Option<&str> {
        std::collections::HashMap::get(self, key).map(String::as_str)
    }
}

/// A tag as the JSON value with the same structure (numbers stay numbers).
pub fn nbt_to_json(tag: &Tag) -> Value {
    let num = |n: f64| serde_json::Number::from_f64(n).map_or(Value::Null, Value::Number);
    match tag {
        Tag::Byte(v) => Value::from(*v),
        Tag::Short(v) => Value::from(*v),
        Tag::Int(v) => Value::from(*v),
        Tag::Long(v) => Value::from(*v),
        Tag::Float(v) => num(f64::from(*v)),
        Tag::Double(v) => num(*v),
        Tag::ByteArray(v) => Value::Array(v.iter().map(|&b| Value::from(b)).collect()),
        Tag::String(s) => Value::String(s.clone()),
        Tag::List(items) => Value::Array(items.iter().map(nbt_to_json).collect()),
        Tag::Compound(c) => Value::Object(
            c.iter()
                .map(|(k, v)| (k.to_owned(), nbt_to_json(v)))
                .collect(),
        ),
        Tag::IntArray(v) => Value::Array(v.iter().map(|&b| Value::from(b)).collect()),
        Tag::LongArray(v) => Value::Array(v.iter().map(|&b| Value::from(b)).collect()),
    }
}

/// The 16 named colours, by legacy code `0`..`f` (`ChatFormatting.java`).
pub const NAMED_COLORS: [(&str, u32); 16] = [
    ("black", 0x000000),
    ("dark_blue", 0x0000AA),
    ("dark_green", 0x00AA00),
    ("dark_aqua", 0x00AAAA),
    ("dark_red", 0xAA0000),
    ("dark_purple", 0xAA00AA),
    ("gold", 0xFFAA00),
    ("gray", 0xAAAAAA),
    ("dark_gray", 0x555555),
    ("blue", 0x5555FF),
    ("green", 0x55FF55),
    ("aqua", 0x55FFFF),
    ("red", 0xFF5555),
    ("light_purple", 0xFF55FF),
    ("yellow", 0xFFFF55),
    ("white", 0xFFFFFF),
];

/// A span's look. `color` is `0xRRGGBB`; `None` takes the drawer's default.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Style {
    pub color: Option<u32>,
    /// The shadow's own colour, `0xAARRGGBB`; `None` is the drawer's.
    pub shadow: Option<u32>,
    pub bold: bool,
    pub italic: bool,
    pub underlined: bool,
    pub strikethrough: bool,
    pub obfuscated: bool,
}

/// A style as a component states it: unset fields inherit.
#[derive(Debug, Clone, Copy, Default)]
struct Partial {
    color: Option<u32>,
    shadow: Option<u32>,
    bold: Option<bool>,
    italic: Option<bool>,
    underlined: Option<bool>,
    strikethrough: Option<bool>,
    obfuscated: Option<bool>,
}

impl Partial {
    fn from_json(o: &serde_json::Map<String, Value>) -> Partial {
        let flag = |k: &str| {
            o.get(k).and_then(|v| match v {
                Value::Bool(b) => Some(*b),
                Value::Number(n) => n.as_f64().map(|n| n != 0.0),
                _ => None,
            })
        };
        Partial {
            color: o.get("color").and_then(Value::as_str).and_then(parse_color),
            shadow: o.get("shadow_color").and_then(parse_argb),
            bold: flag("bold"),
            italic: flag("italic"),
            underlined: flag("underlined"),
            strikethrough: flag("strikethrough"),
            obfuscated: flag("obfuscated"),
        }
    }

    fn over(self, parent: Style) -> Style {
        Style {
            color: self.color.or(parent.color),
            shadow: self.shadow.or(parent.shadow),
            bold: self.bold.unwrap_or(parent.bold),
            italic: self.italic.unwrap_or(parent.italic),
            underlined: self.underlined.unwrap_or(parent.underlined),
            strikethrough: self.strikethrough.unwrap_or(parent.strikethrough),
            obfuscated: self.obfuscated.unwrap_or(parent.obfuscated),
        }
    }
}

/// A named colour or `#RRGGBB` (`TextColor.parseColor`).
pub fn parse_color(s: &str) -> Option<u32> {
    if let Some(hex) = s.strip_prefix('#') {
        return (hex.len() == 6 && hex.bytes().all(|b| b.is_ascii_hexdigit()))
            .then(|| u32::from_str_radix(hex, 16).ok())
            .flatten();
    }
    NAMED_COLORS.iter().find(|(n, _)| *n == s).map(|&(_, c)| c)
}

/// A colour with its alpha, as a number or as four parts from 0 to 1 in the order red,
/// green, blue, alpha (`ExtraCodecs.ARGB_COLOR_CODEC`).
fn parse_argb(v: &Value) -> Option<u32> {
    match v {
        Value::Number(n) => n.as_i64().map(|n| n as u32),
        Value::Array(parts) => {
            let parts: [f64; 4] = parts
                .iter()
                .map(Value::as_f64)
                .collect::<Option<Vec<_>>>()?
                .try_into()
                .ok()?;
            let [r, g, b, a] = parts.map(|p| ((p as f32 * 255.0).floor() as i32 as u32) & 0xFF);
            Some(a << 24 | r << 16 | g << 8 | b)
        }
        _ => None,
    }
}

/// The key each binding has by default (`Options`), by which a component that names a
/// binding shows it.
const KEYS: [(&str, &str); 39] = [
    ("key.forward", "key.keyboard.w"),
    ("key.left", "key.keyboard.a"),
    ("key.back", "key.keyboard.s"),
    ("key.right", "key.keyboard.d"),
    ("key.jump", "key.keyboard.space"),
    ("key.sneak", "key.keyboard.left.shift"),
    ("key.sprint", "key.keyboard.left.control"),
    ("key.inventory", "key.keyboard.e"),
    ("key.swapOffhand", "key.keyboard.f"),
    ("key.drop", "key.keyboard.q"),
    ("key.use", "key.mouse.right"),
    ("key.attack", "key.mouse.left"),
    ("key.pickItem", "key.mouse.middle"),
    ("key.chat", "key.keyboard.t"),
    ("key.playerlist", "key.keyboard.tab"),
    ("key.command", "key.keyboard.slash"),
    ("key.friends", "key.keyboard.o"),
    ("key.socialInteractions", "key.keyboard.p"),
    ("key.toggleGui", "key.keyboard.f1"),
    ("key.screenshot", "key.keyboard.f2"),
    ("key.togglePerspective", "key.keyboard.f5"),
    ("key.fullscreen", "key.keyboard.f11"),
    ("key.advancements", "key.keyboard.l"),
    ("key.quickActions", "key.keyboard.g"),
    ("key.saveToolbarActivator", "key.keyboard.c"),
    ("key.loadToolbarActivator", "key.keyboard.x"),
    ("key.smoothCamera", "key.keyboard.unknown"),
    ("key.spectatorOutlines", "key.keyboard.unknown"),
    ("key.spectatorHotbar", "key.mouse.middle"),
    ("key.toggleSpectatorShaderEffects", "key.keyboard.f4"),
    ("key.hotbar.1", "key.keyboard.1"),
    ("key.hotbar.2", "key.keyboard.2"),
    ("key.hotbar.3", "key.keyboard.3"),
    ("key.hotbar.4", "key.keyboard.4"),
    ("key.hotbar.5", "key.keyboard.5"),
    ("key.hotbar.6", "key.keyboard.6"),
    ("key.hotbar.7", "key.keyboard.7"),
    ("key.hotbar.8", "key.keyboard.8"),
    ("key.hotbar.9", "key.keyboard.9"),
];

/// The key a binding has by default, as far as this module knows them.
pub fn default_key(binding: &str) -> Option<&'static str> {
    KEYS.iter().find(|(b, _)| *b == binding).map(|(_, k)| *k)
}

/// What a key's name shows (`InputConstants.Key.getDisplayName`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeyText {
    /// What the keyboard's layout calls the key, in capitals.
    Literal(String),
    /// A language key: the key's own name (or the system's override of it).
    Translate(String),
    /// A mouse button the language has no name for: `key.mouse` with its number.
    Button(i32),
}

impl KeyText {
    /// As text, translated with `lang` (a key the language lacks shows itself).
    pub fn resolve(&self, lang: &dyn Lang) -> String {
        match self {
            KeyText::Literal(s) => s.clone(),
            KeyText::Translate(k) => lang.get(k).unwrap_or(k).to_owned(),
            KeyText::Button(n) => StyledText::from_json_in(
                &serde_json::json!({"translate": "key.mouse", "with": [n.to_string()]}),
                lang,
            )
            .plain(),
        }
    }
}

/// How the key named `name` (`value` a scancode, or a button if `mouse`) shows: a
/// mouse button by its name in `lang`, else as `key.mouse` with its number; an
/// unbound key by its own name; a keyboard key by what the keyboard's layout calls it
/// (`system`, SDL's `SDL_GetKeyName`) when that is one character, in capitals, else by
/// its own name (`InputQuirks.keyboardTranslationKey`: the system's word for the
/// Windows keys).
pub fn key_text(
    name: &str,
    value: i32,
    mouse: bool,
    system: Option<&str>,
    lang: &dyn Lang,
) -> KeyText {
    if mouse {
        return if lang.get(name).is_some() {
            KeyText::Translate(name.to_owned())
        } else {
            KeyText::Button(value)
        };
    }
    if name == "key.keyboard.unknown" {
        return KeyText::Translate(name.to_owned());
    }
    if let Some(s) = system.filter(|s| s.chars().count() == 1) {
        return KeyText::Literal(s.to_uppercase());
    }
    KeyText::Translate(keyboard_name(name).to_owned())
}

/// `InputQuirks.keyboardTranslationKey`: the system's word for some keys.
fn keyboard_name(name: &str) -> &str {
    let overrides: &[(&str, &str)] = if cfg!(target_os = "macos") {
        &[
            ("key.keyboard.left.alt", "key.keyboard.left.option"),
            ("key.keyboard.right.alt", "key.keyboard.right.option"),
            ("key.keyboard.left.win", "key.keyboard.left.command"),
            ("key.keyboard.right.win", "key.keyboard.right.command"),
        ]
    } else if cfg!(windows) {
        &[
            ("key.keyboard.left.win", "key.keyboard.left.windows"),
            ("key.keyboard.right.win", "key.keyboard.right.windows"),
        ]
    } else {
        &[
            ("key.keyboard.left.win", "key.keyboard.left.meta"),
            ("key.keyboard.right.win", "key.keyboard.right.meta"),
        ]
    };
    overrides
        .iter()
        .find(|(from, _)| *from == name)
        .map_or(name, |(_, to)| to)
}

/// What a component that names a key binding shows (`KeyMapping.createNameSupplier`): the
/// name of the key it is bound to (the player's, where `lang` knows them, else the
/// default), or the binding's own for one that is not known. Without the player's
/// bindings, keys of one character have no entry in the language.
pub fn key_name(binding: &str, lang: &dyn Lang) -> String {
    if let Some(b) = lang.bound_key(binding) {
        return key_text(b.name, b.value, b.mouse, b.system, lang).resolve(lang);
    }
    let Some((_, key)) = KEYS.iter().find(|(b, _)| *b == binding) else {
        return lang.get(binding).unwrap_or(binding).to_owned();
    };
    match (lang.get(key), key.rsplit('.').next()) {
        (Some(name), _) => name.to_owned(),
        (None, Some(last)) if last.len() <= 3 => last.to_uppercase(),
        _ => (*key).to_owned(),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Span {
    pub text: String,
    pub style: Style,
}

/// Text as runs of one style each, in reading order.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StyledText {
    pub spans: Vec<Span>,
}

impl StyledText {
    /// Plain text in one style (legacy codes in it still apply).
    pub fn literal(text: &str, style: Style) -> StyledText {
        let mut t = StyledText::default();
        t.push_formatted(text, style, &mut Budget::new());
        t
    }

    /// A component in its JSON form, without a language.
    pub fn from_json(v: &Value) -> StyledText {
        StyledText::from_json_in(v, &())
    }

    /// A component in its JSON form, translated with `lang`.
    pub fn from_json_in(v: &Value, lang: &dyn Lang) -> StyledText {
        let mut t = StyledText::default();
        t.component(v, Style::default(), 0, lang, &mut Budget::new());
        t
    }

    /// A component in its NBT form, translated with `lang`.
    pub fn from_nbt_in(tag: &Tag, lang: &dyn Lang) -> StyledText {
        StyledText::from_json_in(&nbt_to_json(tag), lang)
    }

    /// The text without styles.
    pub fn plain(&self) -> String {
        self.spans.iter().map(|s| s.text.as_str()).collect()
    }

    pub fn is_empty(&self) -> bool {
        self.spans.iter().all(|s| s.text.is_empty())
    }

    /// Split at line breaks.
    pub fn lines(&self) -> Vec<StyledText> {
        let mut lines = vec![StyledText::default()];
        let mut b = Budget::unbounded();
        for span in &self.spans {
            for (i, part) in span.text.split('\n').enumerate() {
                if i > 0 {
                    lines.push(StyledText::default());
                }
                lines.last_mut().unwrap().push(part, span.style, &mut b);
            }
        }
        lines
    }

    /// Appends `text` in `style`, as far as `b` allows.
    fn push(&mut self, text: &str, style: Style, b: &mut Budget) {
        if text.is_empty() || b.chars == 0 {
            return;
        }
        let text = match text.char_indices().nth(b.chars) {
            Some((cut, _)) => &text[..cut],
            None => text,
        };
        let new_span = self.spans.last().is_none_or(|last| last.style != style);
        if new_span && self.spans.len() >= MAX_EXPANDED_SPANS {
            b.chars = 0;
            return;
        }
        b.chars = b.chars.saturating_sub(text.chars().count());
        match self.spans.last_mut() {
            Some(last) if last.style == style => last.text.push_str(text),
            _ => self.spans.push(Span {
                text: text.to_owned(),
                style,
            }),
        }
    }

    /// Appends `text`, applying legacy `§` codes on top of `base`.
    fn push_formatted(&mut self, text: &str, base: Style, b: &mut Budget) {
        if !b.scan(text.len()) {
            return;
        }
        let mut style = base;
        let mut rest = text;
        while let Some(at) = rest.find('§') {
            if b.spent() {
                return;
            }
            self.push(&rest[..at], style, b);
            let mut after = rest[at + '§'.len_utf8()..].chars();
            let Some(code) = after.next() else {
                return;
            };
            // Java reads the code as one UTF-16 unit and lowers its case alone
            // (`Character.toLowerCase`): the Kelvin sign is `k`. A code past the first plane is half a
            // pair there, and the half left over shows as U+FFFD.
            let lower = code.to_lowercase().next().unwrap_or(code);
            if code.len_utf16() == 2 {
                self.push("\u{FFFD}", style, b);
            }
            style = match lower {
                c @ ('0'..='9' | 'a'..='f') => Style {
                    color: Some(NAMED_COLORS[c.to_digit(16).unwrap() as usize].1),
                    shadow: style.shadow,
                    ..Style::default()
                },
                'k' => Style {
                    obfuscated: true,
                    ..style
                },
                'l' => Style {
                    bold: true,
                    ..style
                },
                'm' => Style {
                    strikethrough: true,
                    ..style
                },
                'n' => Style {
                    underlined: true,
                    ..style
                },
                'o' => Style {
                    italic: true,
                    ..style
                },
                'r' => base,
                _ => style,
            };
            rest = after.as_str();
        }
        self.push(rest, style, b);
    }

    fn component(
        &mut self,
        v: &Value,
        parent: Style,
        depth: usize,
        lang: &dyn Lang,
        b: &mut Budget,
    ) {
        // (A step even when too deep: a format that names a too-deep argument many
        // times, nested, would otherwise be walked without end and cost nothing.)
        if !b.step() || depth >= MAX_DEPTH {
            return;
        }
        match v {
            Value::String(s) => self.push_formatted(s, parent, b),
            Value::Number(n) => self.push_formatted(&n.to_string(), parent, b),
            Value::Bool(x) => self.push_formatted(if *x { "true" } else { "false" }, parent, b),
            Value::Array(items) => {
                // The first element is the parent of the rest: they inherit its style.
                let Some((first, rest)) = items.split_first() else {
                    return;
                };
                let style = match first {
                    Value::Object(o) => Partial::from_json(o).over(parent),
                    _ => parent,
                };
                self.component(first, parent, depth + 1, lang, b);
                for item in rest {
                    self.component(item, style, depth + 1, lang, b);
                }
            }
            Value::Object(o) => {
                let style = Partial::from_json(o).over(parent);
                if let Some(text) = o.get("text") {
                    match text {
                        Value::String(s) => self.push_formatted(s, style, b),
                        Value::Number(n) => self.push_formatted(&n.to_string(), style, b),
                        Value::Bool(x) => {
                            self.push_formatted(if *x { "true" } else { "false" }, style, b)
                        }
                        _ => {}
                    }
                } else if let Some(key) = o.get("translate").and_then(Value::as_str) {
                    let format = lang
                        .get(key)
                        .or_else(|| o.get("fallback").and_then(Value::as_str))
                        .unwrap_or(key);
                    let args = o.get("with").and_then(Value::as_array);
                    let args = args.map_or(&[][..], Vec::as_slice);
                    self.translate(format, args, style, depth, lang, b);
                } else if let Some(key) = o.get("keybind").and_then(Value::as_str) {
                    self.push(&key_name(key, lang), style, b);
                }
                if let Some(Value::Array(extra)) = o.get("extra") {
                    for child in extra {
                        self.component(child, style, depth + 1, lang, b);
                    }
                }
            }
            Value::Null => {}
        }
    }

    /// Fills `%s` / `%2$s` in `format` with `args`; `%%` is a percent sign. A format
    /// with other specifiers, or an argument index past the end, shows the format as it
    /// is (vanilla shows the key when formatting fails).
    fn translate(
        &mut self,
        format: &str,
        args: &[Value],
        style: Style,
        depth: usize,
        lang: &dyn Lang,
        b: &mut Budget,
    ) {
        let mut pieces: Vec<Result<&str, usize>> = Vec::new();
        let mut next = 0;
        let mut rest = format;
        let ok = loop {
            let Some(at) = rest.find('%') else {
                pieces.push(Ok(rest));
                break true;
            };
            pieces.push(Ok(&rest[..at]));
            let spec = &rest[at + 1..];
            let digits = spec.bytes().take_while(u8::is_ascii_digit).count();
            let (index, conv) = if digits > 0 && spec.as_bytes().get(digits) == Some(&b'$') {
                (
                    spec[..digits]
                        .parse::<usize>()
                        .ok()
                        .map(|n| n.wrapping_sub(1)),
                    &spec[digits + 1..],
                )
            } else {
                (None, spec)
            };
            match conv.chars().next() {
                Some('%') if index.is_none() => {
                    pieces.push(Ok("%"));
                    rest = &conv[1..];
                }
                Some('s') => {
                    let i = index.unwrap_or_else(|| {
                        next += 1;
                        next - 1
                    });
                    if i >= args.len() {
                        break false;
                    }
                    pieces.push(Err(i));
                    rest = &conv[1..];
                }
                // A lone % at the end is dropped.
                None if index.is_none() => break true,
                _ => break false,
            }
        };
        if !ok {
            self.push_formatted(format, style, b);
            return;
        }
        for p in pieces {
            match p {
                Ok(s) => self.push_formatted(s, style, b),
                Err(i) => self.component(&args[i], style, depth + 1, lang, b),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn spans(t: &StyledText) -> Vec<(&str, Option<u32>, bool)> {
        t.spans
            .iter()
            .map(|s| (s.text.as_str(), s.style.color, s.style.bold))
            .collect()
    }

    #[test]
    fn legacy_codes() {
        let t = StyledText::literal("§aGreen §lbold§r plain§cred§", Style::default());
        assert_eq!(
            spans(&t),
            [
                ("Green ", Some(0x55FF55), false),
                ("bold", Some(0x55FF55), true),
                (" plain", None, false),
                ("red", Some(0xFF5555), false),
            ]
        );
        // A colour code clears bold; unknown codes vanish.
        // Codes past the first plane leave half a pair behind; case is lowered as Java does.
        let t = StyledText::literal("§😀x§\u{212A}y", Style::default());
        assert_eq!(t.plain(), "\u{FFFD}xy");
        assert!(t.spans.last().unwrap().style.obfuscated);
        let t = StyledText::literal("§l§6Gold§zx", Style::default());
        assert_eq!(spans(&t), [("Goldx", Some(0xFFAA00), false)]);
    }

    /// A translation that repeats its argument, nested: the output grows as a power of
    /// the nesting. It stops at the budget, quickly.
    #[test]
    fn nested_repeats_stop_at_the_budget() {
        let nest = |repeats: usize, depth: usize, leaf: serde_json::Value| {
            let format = "%1$s".repeat(repeats);
            let mut v = leaf;
            for _ in 0..depth {
                v = json!({"translate": "k", "fallback": format, "with": [v]});
            }
            v
        };
        // 2^14 x 4 characters: past the budget.
        let t = StyledText::from_json(&nest(2, 14, json!("abcd")));
        assert_eq!(t.plain().chars().count(), MAX_EXPANDED_CHARS);
        // 8^14: would never end.
        let started = std::time::Instant::now();
        let t = StyledText::from_json(&nest(8, 14, json!("abcd")));
        assert!(t.plain().chars().count() <= MAX_EXPANDED_CHARS);
        assert!(started.elapsed() < std::time::Duration::from_secs(10));
        // Nothing made, everything walked: the steps run out, and what follows is not
        // added.
        let v = json!(["", nest(4, 9, json!("")), "END"]);
        assert_eq!(StyledText::from_json(&v).plain(), "");
        // Arguments past the depth limit show nothing, and still cost: a long format
        // that names one many times would otherwise be walked for nothing, again and
        // again.
        let started = std::time::Instant::now();
        let v = json!(["", nest(30_000, MAX_DEPTH, json!("x")), "END"]);
        assert_eq!(StyledText::from_json(&v).plain(), "");
        assert!(started.elapsed() < std::time::Duration::from_secs(10));
        let small = json!(["", nest(2, 3, json!("")), "END"]);
        assert_eq!(StyledText::from_json(&small).plain(), "END");
    }

    /// A format of nothing but `§` codes, named 255 times by an argument named 255
    /// times: nothing is made, and reading it again and again stops at the budget (it
    /// took 16 s and longer before).
    #[test]
    fn formats_of_codes_alone_stop_at_the_budget() {
        let codes = "§r".repeat(21_000);
        let names = "%1$s".repeat(255);
        let v = json!({"translate": "a", "fallback": names, "with": [
            {"translate": "b", "fallback": names, "with": [
                {"translate": "c", "fallback": codes}
            ]}
        ]});
        let started = std::time::Instant::now();
        let t = StyledText::from_json(&json!(["", v, "END"]));
        assert_eq!(t.plain(), "");
        assert!(
            started.elapsed() < std::time::Duration::from_secs(2),
            "{:?}",
            started.elapsed()
        );
        // The same codes once, in a literal, read as before.
        assert_eq!(StyledText::from_json(&json!([codes, "END"])).plain(), "END");
    }

    /// Text past the budget is cut, and so are styles past the span cap.
    #[test]
    fn long_text_and_many_styles_are_cut() {
        let long = "y".repeat(MAX_EXPANDED_CHARS * 3);
        assert_eq!(
            StyledText::from_json(&json!(long)).plain().len(),
            MAX_EXPANDED_CHARS
        );
        assert_eq!(
            StyledText::literal(&long, Style::default()).plain().len(),
            MAX_EXPANDED_CHARS
        );
        let many: Vec<serde_json::Value> = (0..MAX_EXPANDED_SPANS * 2)
            .map(|i| json!({"text": "z", "color": if i % 2 == 0 { "red" } else { "blue" }}))
            .collect();
        let t = StyledText::from_json(&json!({"text": "", "extra": many}));
        assert_eq!(t.spans.len(), MAX_EXPANDED_SPANS);
    }

    #[test]
    fn components_inherit_style() {
        let v = json!({
            "text": "A ",
            "color": "gold",
            "bold": true,
            "extra": [
                "b ",
                {"text": "c", "bold": false, "color": "#123456"},
                ["d", {"text": "e", "color": "red"}]
            ]
        });
        let t = StyledText::from_json(&v);
        assert_eq!(
            spans(&t),
            [
                ("A b ", Some(0xFFAA00), true),
                ("c", Some(0x123456), false),
                ("d", Some(0xFFAA00), true),
                ("e", Some(0xFF5555), true),
            ]
        );
        assert_eq!(t.plain(), "A b cde");
    }

    #[test]
    fn arrays_share_the_first_elements_style() {
        let v = json!([{"text": "x", "color": "aqua"}, "y"]);
        assert_eq!(
            spans(&StyledText::from_json(&v)),
            [("xy", Some(0x55FFFF), false)]
        );
    }

    #[test]
    fn translations_fill_arguments() {
        let v = json!({"translate": "k", "fallback": "%2$s and %s, 100%%", "with": ["a", {"text": "b", "color": "red"}]});
        let t = StyledText::from_json(&v);
        assert_eq!(t.plain(), "b and a, 100%");
        let v = json!({"translate": "multiplayer.status.unknown"});
        assert_eq!(
            StyledText::from_json(&v).plain(),
            "multiplayer.status.unknown"
        );
        // A bad format shows as it is.
        let v = json!({"translate": "x", "fallback": "%d %s", "with": ["a"]});
        assert_eq!(StyledText::from_json(&v).plain(), "%d %s");
    }

    #[test]
    fn languages_and_nbt() {
        let mut lang = std::collections::HashMap::new();
        lang.insert(
            "multiplayer.disconnect.kicked".to_owned(),
            "Kicked by an operator".to_owned(),
        );
        lang.insert("disconnect.genericReason".to_owned(), "%s".to_owned());
        let v = json!({"translate": "multiplayer.disconnect.kicked", "fallback": "x"});
        assert_eq!(
            StyledText::from_json_in(&v, &lang).plain(),
            "Kicked by an operator"
        );
        // NBT: a compound with byte booleans and a list of mixed children.
        let mut c = crate::nbt::Compound::new();
        c.put("translate", Tag::String("disconnect.genericReason".into()));
        c.put("bold", Tag::Byte(1));
        c.put("with", Tag::List(vec![Tag::String("Server closed".into())]));
        let t = StyledText::from_nbt_in(&Tag::Compound(c), &lang);
        assert_eq!(t.plain(), "Server closed");
        assert!(t.spans[0].style.bold);
        let s = StyledText::from_nbt_in(&Tag::String("§cBanned".into()), &());
        assert_eq!(spans(&s), [("Banned", Some(0xFF5555), false)]);
    }

    #[test]
    fn shadows_have_a_colour_of_their_own() {
        let shadow = |v: Value| StyledText::from_json(&v).spans[0].style.shadow;
        assert_eq!(shadow(json!({"text": "a"})), None);
        assert_eq!(shadow(json!({"text": "a", "shadow_color": 0})), Some(0));
        assert_eq!(
            shadow(json!({"text": "a", "shadow_color": -16_711_936})),
            Some(0xFF00_FF00)
        );
        assert_eq!(
            shadow(json!({"text": "a", "shadow_color": [1.0, 0.5, 0.0, 0.25]})),
            Some(0x3FFF_7F00)
        );
        assert_eq!(shadow(json!({"text": "a", "shadow_color": [1.0]})), None);
        // Children have it, and legacy colours keep it.
        let t = StyledText::from_json(&json!({
            "text": "", "shadow_color": 7, "extra": ["a§cb", {"text": "c", "bold": true}]
        }));
        assert!(t.spans.len() == 3 && t.spans.iter().all(|s| s.style.shadow == Some(7)));
    }

    #[test]
    fn bindings_are_shown_by_their_keys() {
        let mut lang = std::collections::HashMap::new();
        lang.insert(
            "key.keyboard.left.shift".to_owned(),
            "Left Shift".to_owned(),
        );
        lang.insert("key.other".to_owned(), "Other".to_owned());
        let shown = |key: &str| StyledText::from_json_in(&json!({"keybind": key}), &lang).plain();
        assert_eq!(shown("key.sneak"), "Left Shift");
        assert_eq!(shown("key.inventory"), "E");
        assert_eq!(shown("key.togglePerspective"), "F5");
        assert_eq!(shown("key.other"), "Other");
        assert_eq!(shown("key.mystery"), "key.mystery");
        // (Without its language, a key with a name of its own.)
        assert_eq!(shown("key.jump"), "key.keyboard.space");
    }

    #[test]
    fn lines_and_depth() {
        let t = StyledText::from_json(
            &json!({"text": "one\n", "extra": [{"text": "two", "color": "red"}]}),
        );
        let lines = t.lines();
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].plain(), "one");
        assert_eq!(spans(&lines[1]), [("two", Some(0xFF5555), false)]);
        let mut deep = json!("bottom");
        for _ in 0..40 {
            deep = json!({"text": "", "extra": [deep]});
        }
        assert_eq!(StyledText::from_json(&deep).plain(), "");
        assert_eq!(parse_color("#12345"), None);
        assert_eq!(parse_color("dark_aqua"), Some(0x00AAAA));
    }
}

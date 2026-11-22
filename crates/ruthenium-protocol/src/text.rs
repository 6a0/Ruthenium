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
//! Without language files, a translation shows its `fallback`, else its key, with
//! `%s` and `%1$s` filled in. Scores, selectors, NBT and objects show nothing (the
//! server list resolves them without a source). Nesting past 16 levels is dropped, as the
//! server list's sanitising does (`multiplayer/ServerStatusPinger.java`).

use serde_json::Value;

const MAX_DEPTH: usize = 16;

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
    bold: Option<bool>,
    italic: Option<bool>,
    underlined: Option<bool>,
    strikethrough: Option<bool>,
    obfuscated: Option<bool>,
}

impl Partial {
    fn from_json(o: &serde_json::Map<String, Value>) -> Partial {
        let flag = |k: &str| o.get(k).and_then(Value::as_bool);
        Partial {
            color: o.get("color").and_then(Value::as_str).and_then(parse_color),
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
        t.push_formatted(text, style);
        t
    }

    /// A component in its JSON form.
    pub fn from_json(v: &Value) -> StyledText {
        let mut t = StyledText::default();
        t.component(v, Style::default(), 0);
        t
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
        for span in &self.spans {
            for (i, part) in span.text.split('\n').enumerate() {
                if i > 0 {
                    lines.push(StyledText::default());
                }
                lines.last_mut().unwrap().push(part, span.style);
            }
        }
        lines
    }

    fn push(&mut self, text: &str, style: Style) {
        if text.is_empty() {
            return;
        }
        match self.spans.last_mut() {
            Some(last) if last.style == style => last.text.push_str(text),
            _ => self.spans.push(Span {
                text: text.to_owned(),
                style,
            }),
        }
    }

    /// Appends `text`, applying legacy `§` codes on top of `base`.
    fn push_formatted(&mut self, text: &str, base: Style) {
        let mut style = base;
        let mut rest = text;
        while let Some(at) = rest.find('§') {
            self.push(&rest[..at], style);
            let mut after = rest[at + '§'.len_utf8()..].chars();
            let Some(code) = after.next() else {
                return;
            };
            style = match code.to_ascii_lowercase() {
                c @ ('0'..='9' | 'a'..='f') => Style {
                    color: Some(NAMED_COLORS[c.to_digit(16).unwrap() as usize].1),
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
        self.push(rest, style);
    }

    fn component(&mut self, v: &Value, parent: Style, depth: usize) {
        if depth >= MAX_DEPTH {
            return;
        }
        match v {
            Value::String(s) => self.push_formatted(s, parent),
            Value::Number(n) => self.push_formatted(&n.to_string(), parent),
            Value::Bool(b) => self.push_formatted(if *b { "true" } else { "false" }, parent),
            Value::Array(items) => {
                // The first element is the parent of the rest: they inherit its style.
                let Some((first, rest)) = items.split_first() else {
                    return;
                };
                let style = match first {
                    Value::Object(o) => Partial::from_json(o).over(parent),
                    _ => parent,
                };
                self.component(first, parent, depth + 1);
                for item in rest {
                    self.component(item, style, depth + 1);
                }
            }
            Value::Object(o) => {
                let style = Partial::from_json(o).over(parent);
                if let Some(text) = o.get("text") {
                    match text {
                        Value::String(s) => self.push_formatted(s, style),
                        Value::Number(n) => self.push_formatted(&n.to_string(), style),
                        Value::Bool(b) => {
                            self.push_formatted(if *b { "true" } else { "false" }, style)
                        }
                        _ => {}
                    }
                } else if let Some(key) = o.get("translate").and_then(Value::as_str) {
                    let format = o.get("fallback").and_then(Value::as_str).unwrap_or(key);
                    let args = o.get("with").and_then(Value::as_array);
                    self.translate(format, args.map_or(&[], Vec::as_slice), style, depth);
                } else if let Some(key) = o.get("keybind").and_then(Value::as_str) {
                    self.push(key, style);
                }
                if let Some(Value::Array(extra)) = o.get("extra") {
                    for child in extra {
                        self.component(child, style, depth + 1);
                    }
                }
            }
            Value::Null => {}
        }
    }

    /// Fills `%s` / `%2$s` in `format` with `args`; `%%` is a percent sign. A format
    /// with other specifiers, or an argument index past the end, shows the format as it
    /// is (vanilla shows the key when formatting fails).
    fn translate(&mut self, format: &str, args: &[Value], style: Style, depth: usize) {
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
            self.push_formatted(format, style);
            return;
        }
        for p in pieces {
            match p {
                Ok(s) => self.push_formatted(s, style),
                Err(i) => self.component(&args[i], style, depth + 1),
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
        let t = StyledText::literal("§l§6Gold§zx", Style::default());
        assert_eq!(spans(&t), [("Goldx", Some(0xFFAA00), false)]);
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

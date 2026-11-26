//! The mouse settings and key bindings vanilla players already have: imported once from
//! the newest `options.txt` among the launchers' game folders, so the sensitivity is the
//! exact double vanilla uses (its slider stores any value from 0 to 1, so a value typed
//! again by hand would almost never be the same), and the keys are read as vanilla reads
//! them, from files of old versions too.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use ruthenium_prepare::discover::{self, Env};
use ruthenium_sim::input::MouseOptions;
use ruthenium_sim::keybinds;

use crate::settings::VanillaKeys;

/// Vanilla's mouse options from `options.txt` (`mouseSensitivity`, `invertXMouse`,
/// `invertYMouse`); keys missing or unreadable keep vanilla's defaults, as vanilla
/// reads the file.
pub fn parse_options(text: &str) -> MouseOptions {
    let file = OptionsTxt::parse(text);
    let mut o = MouseOptions::DEFAULT;
    // `OptionInstance.UnitDouble`: a double from 0 to 1.
    if let Some(v) = file
        .get("mouseSensitivity")
        .and_then(|v| v.parse::<f64>().ok())
        && (0.0..=1.0).contains(&v)
    {
        o.sensitivity = v;
    }
    o.invert_x = file.bool("invertXMouse").unwrap_or(o.invert_x);
    o.invert_y = file.bool("invertYMouse").unwrap_or(o.invert_y);
    o
}

/// `options.txt` as `Options.load` reads it: lines as `BufferedReader.lines` splits
/// them (at `\n`, `\r` or `\r\n`), each split at its first colon into a name and a
/// value (a line without one is skipped), into a compound tag, so the last line with a
/// name counts even when its value can't be read.
#[derive(Debug, Default)]
pub(crate) struct OptionsTxt<'a> {
    values: HashMap<&'a str, &'a str>,
}

impl<'a> OptionsTxt<'a> {
    pub(crate) fn parse(text: &'a str) -> OptionsTxt<'a> {
        let mut values = HashMap::new();
        let mut rest = text;
        while !rest.is_empty() {
            let end = rest.find(['\n', '\r']).unwrap_or(rest.len());
            let line = &rest[..end];
            rest = &rest[end..];
            rest = rest
                .strip_prefix("\r\n")
                .or_else(|| rest.strip_prefix(['\n', '\r']))
                .unwrap_or(rest);
            if let Some((name, value)) = line.split_once(':') {
                values.insert(name, value);
            }
        }
        OptionsTxt { values }
    }

    /// The value the file gives `name`, as it is written.
    pub(crate) fn get(&self, name: &str) -> Option<&'a str> {
        self.values.get(name).copied()
    }

    /// A boolean option's value ([`vanilla_bool`]); None where the file has none or it
    /// can't be read, and the option keeps its default.
    pub(crate) fn bool(&self, name: &str) -> Option<bool> {
        self.get(name).and_then(vanilla_bool)
    }
}

/// `options.txt` files in the launchers' game folders: the official launcher's, and
/// each instance's or profile's of the others.
pub(crate) fn options_files(env: &Env) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut add = |p: PathBuf| {
        if p.is_file() && !found.contains(&p) {
            found.push(p);
        }
    };
    for (_, root) in discover::default_roots(env) {
        add(root.join("options.txt"));
        for group in ["instances", "profiles"] {
            let Ok(dirs) = std::fs::read_dir(root.join(group)) else {
                continue;
            };
            for d in dirs.flatten() {
                let d = d.path();
                for game in [".minecraft", "minecraft", "."] {
                    add(d.join(game).join("options.txt"));
                }
            }
        }
    }
    found
}

pub(crate) fn modified(p: &Path) -> SystemTime {
    std::fs::metadata(p)
        .and_then(|m| m.modified())
        .unwrap_or(SystemTime::UNIX_EPOCH)
}

/// The most recently saved `options.txt`, and where it is.
fn newest_options() -> Option<(String, PathBuf)> {
    let newest = options_files(&Env::from_process())
        .into_iter()
        .max_by_key(|p| modified(p))?;
    let text = std::fs::read_to_string(&newest).ok()?;
    Some((text, newest))
}

/// The mouse options of the most recently saved `options.txt`, and where it is.
pub fn vanilla_mouse() -> Option<(MouseOptions, PathBuf)> {
    let (text, path) = newest_options()?;
    Some((parse_options(&text), path))
}

/// Whether sneak and sprint toggle (`toggleCrouch`, `toggleSprint`); a value vanilla
/// can't read keeps the default (off).
pub fn parse_toggles(text: &str) -> (bool, bool) {
    let file = OptionsTxt::parse(text);
    (
        file.bool("toggleCrouch").unwrap_or(false),
        file.bool("toggleSprint").unwrap_or(false),
    )
}

/// A boolean option's value as vanilla reads it (`OptionInstance`'s codec over
/// `LenientJsonParser`, Gson's `JsonParser.parseString`): `true` or `false` in any
/// case. Before it, a byte order mark first of all, then JSON's whitespace and comments
/// (`/* */`, and `//` or `#` to the end of the line), as a lenient reader skips them;
/// after it only whitespace, the check for more being strict (a comment there is an
/// error). Anything else (numbers, quotes, other words, null) is no value, and the option
/// keeps its default.
fn vanilla_bool(value: &str) -> Option<bool> {
    // (`JsonReader.fillBuffer` drops a mark at the very start.)
    let mut s = value.strip_prefix('\u{FEFF}').unwrap_or(value);
    // `nextNonWhitespace`, lenient.
    loop {
        s = s.trim_start_matches([' ', '\t', '\n', '\r']);
        if let Some(rest) = s.strip_prefix("/*") {
            s = &rest[rest.find("*/")? + 2..];
        } else if let Some(rest) = s.strip_prefix("//").or_else(|| s.strip_prefix('#')) {
            s = &rest[rest.find(['\n', '\r'])? + 1..];
        } else {
            break;
        }
    }
    // `peekKeyword`: either case, letter by letter.
    let (word, on) = [("true", true), ("false", false)]
        .into_iter()
        .find(|(w, _)| s.get(..w.len()).is_some_and(|p| p.eq_ignore_ascii_case(w)))?;
    // After it, `isLiteral` ends the word at a character that is not part of one, and
    // the check for more is strict: whitespace alone may follow.
    s[word.len()..]
        .trim_start_matches([' ', '\t', '\n', '\r'])
        .is_empty()
        .then_some(on)
}

/// Whether Save Unsent Chats is on (`saveChatDrafts`, off by default); the last line
/// that names it counts, as vanilla reads the file.
pub fn parse_chat_drafts(text: &str) -> bool {
    OptionsTxt::parse(text)
        .bool("saveChatDrafts")
        .unwrap_or(false)
}

/// Save Unsent Chats in the most recently saved `options.txt`, and where it is.
pub fn vanilla_chat_drafts() -> Option<(bool, PathBuf)> {
    let (text, path) = newest_options()?;
    Some((parse_chat_drafts(&text), path))
}

/// The key bindings of the most recently saved `options.txt`, and whether sneak and
/// sprint toggle there, if it has any bindings; and where it is.
pub fn vanilla_keys() -> Option<(VanillaKeys, PathBuf)> {
    let (text, path) = newest_options()?;
    let (keys, any) = keybinds::read_options(&text);
    let (toggle_sneak, toggle_sprint) = parse_toggles(&text);
    any.then_some((
        VanillaKeys {
            keys,
            toggle_sneak,
            toggle_sprint,
        },
        path,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_vanilla_options_exactly() {
        let o = parse_options(
            "version:4671\nmouseSensitivity:0.27464788732394363\ninvertYMouse:true\n\
             key_key.forward:key.keyboard.w\n",
        );
        assert_eq!(o.sensitivity.to_bits(), 0.27464788732394363f64.to_bits());
        assert!(o.invert_y && !o.invert_x);
        assert_eq!(parse_options("mouseSensitivity:1.0E-4").sensitivity, 1.0e-4);
        // Out of range or broken values keep the default.
        assert_eq!(parse_options("mouseSensitivity:1.5").sensitivity, 0.5);
        assert_eq!(parse_options("mouseSensitivity:fast").sensitivity, 0.5);
        assert_eq!(parse_options("invertXMouse:yes"), MouseOptions::DEFAULT);
    }

    /// What vanilla 26.3's own classes make of each value (`OptionInstance.createBoolean`'s
    /// codec and the toggles' `BOOLEAN_VALUES`, which agree, over `LenientJsonParser`):
    /// Some for a value, None where it fails and the option keeps its default.
    const VANILLA_BOOLS: [(&str, Option<bool>); 100] = [
        ("true", Some(true)),
        ("false", Some(false)),
        ("TRUE", Some(true)),
        ("True", Some(true)),
        ("tRuE", Some(true)),
        ("FALSE", Some(false)),
        ("fAlSe", Some(false)),
        ("1", None),
        ("0", None),
        ("2", None),
        ("-1", None),
        ("1.5", None),
        ("-0", None),
        ("NaN", None),
        ("yes", None),
        ("no", None),
        ("t", None),
        ("", None),
        (" ", None),
        ("\"true\"", None),
        ("'true'", None),
        (" true", Some(true)),
        ("true ", Some(true)),
        ("\ttrue", Some(true)),
        ("true\t", Some(true)),
        (" \t true \t ", Some(true)),
        ("true x", None),
        ("true false", None),
        ("true:x", None),
        (":true", None),
        ("true,false", None),
        ("true,", None),
        (",true", None),
        ("[true]", None),
        ("[true", None),
        ("true]", None),
        ("{}", None),
        ("null", None),
        ("NULL", None),
        ("/*c*/true", Some(true)),
        ("/*c*/ /*d*/ FALSE", Some(false)),
        ("/**/true/**/", None),
        ("true/*c*/", None),
        ("true/*c", None),
        ("/*c true", None),
        ("/*/true", None),
        ("true//c", None),
        ("// true", None),
        ("true#c", None),
        ("#true", None),
        ("# c", None),
        ("true;", None),
        (";true", None),
        ("true/", None),
        ("/true", None),
        ("true\\", None),
        ("true=", None),
        ("=true", None),
        ("true}", None),
        ("true\"", None),
        ("\u{c}true", None),
        ("true\u{c}", None),
        ("\u{a0}true", None),
        ("true\u{a0}", None),
        ("\u{3000}true", None),
        ("tru", None),
        ("truee", None),
        ("true1", None),
        ("true.", None),
        ("true-", None),
        ("true_", None),
        ("true$", None),
        ("true'", None),
        ("true(", None),
        ("true)", None),
        ("nul", None),
        ("\u{feff}true", Some(true)),
        ("TRUE//", None),
        ("false#", None),
        ("true/**/false", None),
        ("true*", None),
        ("/*true*/", None),
        ("true/*/", None),
        ("/*a*//*b*/true//c", None),
        (" \u{feff}true", None),
        ("\u{feff}\u{feff}true", None),
        ("\u{feff}/*c*/true", Some(true)),
        ("\u{feff}", None),
        ("/*c*/\u{feff}true", None),
        ("true \t\r", Some(true)),
        ("\u{feff} false ", Some(false)),
        ("/*c*/", None),
        ("/*", None),
        ("*/true", None),
        ("/*c*/tRUE ", Some(true)),
        ("/* a /* b */true", Some(true)),
        ("/*c**/true", Some(true)),
        ("/***/false", Some(false)),
        ("\u{0}true", None),
        ("true\u{0}", None),
    ];

    /// Whole files through `Options.load`'s reading, with vanilla's outcome for
    /// invertXMouse, invertYMouse, toggleCrouch, toggleSprint and saveChatDrafts (1: on).
    const VANILLA_FILES: [(&str, &str); 15] = [
        (
            "invertXMouse:true\ninvert YMouse:TRUE\ninvertYMouse:TRUE\ntoggleCrouch:/*c*/true\ntoggleSprint: true\nsaveChatDrafts:True\n",
            "11111",
        ),
        ("toggleCrouch:true\ntoggleCrouch:yes\n", "00000"),
        ("toggleCrouch:true\ntoggleCrouch\n", "00100"),
        ("toggleCrouch:true\rtoggleSprint:true\r", "00110"),
        ("toggleCrouch:true\r\ntoggleSprint:true\r\n", "00110"),
        ("toggleCrouch:true\rx\ntoggleSprint:true\n\r", "00110"),
        ("toggleCrouch::true\ntoggleSprint:true:\n", "00000"),
        ("toggleCrouch :true\n toggleSprint:true\n", "00000"),
        ("TOGGLECROUCH:true\n", "00000"),
        ("invertYMouse:true\ninvertYMouse:\n", "00000"),
        (
            "saveChatDrafts:false\nsaveChatDrafts:true\nsaveChatDraft:false\nsaveChatDrafts\n",
            "00001",
        ),
        ("\u{feff}invertXMouse:true\ninvertYMouse:true", "01000"),
        ("toggleSprint:true\u{2028}toggleCrouch:true\n", "00000"),
        ("toggleSprint:true\u{85}\n", "00000"),
        ("invertXMouse:true\n\ninvertXMouse:false//true\n", "00000"),
    ];

    #[test]
    fn reads_boolean_options_as_vanilla_does() {
        for (value, want) in VANILLA_BOOLS {
            assert_eq!(vanilla_bool(value), want, "{value:?}");
            for name in ["invertXMouse", "toggleSprint", "saveChatDrafts"] {
                let line = format!("{name}:{value}\n");
                assert_eq!(OptionsTxt::parse(&line).bool(name), want, "{line:?}");
            }
        }
        for (text, want) in VANILLA_FILES {
            let mouse = parse_options(text);
            let toggles = parse_toggles(text);
            let got: String = [
                mouse.invert_x,
                mouse.invert_y,
                toggles.0,
                toggles.1,
                parse_chat_drafts(text),
            ]
            .map(|on| if on { '1' } else { '0' })
            .into_iter()
            .collect();
            assert_eq!(got, want, "{text:?}");
        }
    }

    #[test]
    fn options_txt_splits_lines_and_names_as_vanilla_does() {
        let f = OptionsTxt::parse("a:1\rb:2\r\nc:3:4\n\nd\ne:\na:5\u{2028}f:6\r");
        let got = |k| f.get(k);
        assert_eq!(
            ["a", "b", "c", "d", "e", "f"].map(got),
            [
                Some("5\u{2028}f:6"),
                Some("2"),
                Some("3:4"),
                None,
                Some(""),
                None
            ]
        );
        // The last line wins, even one that can't be read.
        let f = OptionsTxt::parse("mouseSensitivity:0.25\nmouseSensitivity:fast\n");
        assert_eq!(f.get("mouseSensitivity"), Some("fast"));
        assert_eq!(
            parse_options("mouseSensitivity:0.25\nmouseSensitivity:fast").sensitivity,
            0.5
        );
        assert_eq!(
            parse_options("mouseSensitivity:fast\nmouseSensitivity:0.25").sensitivity,
            0.25
        );
        // A comment line that ends where the value doesn't.
        assert_eq!(vanilla_bool("// c\ntrue"), Some(true));
        assert_eq!(vanilla_bool("# c\rfalse"), Some(false));
    }

    #[test]
    fn reads_whether_sneak_and_sprint_toggle() {
        assert_eq!(
            parse_toggles("toggleCrouch:true\ntoggleSprint:false\n"),
            (true, false)
        );
        assert_eq!(parse_toggles("toggleSprint:true\n"), (false, true));
        assert_eq!(
            parse_toggles("toggleCrouch:yes\ntoggleSprint:1\n"),
            (false, false)
        );
        assert_eq!(parse_toggles(""), (false, false));
        // As vanilla's boolean options read: any case, a comment before.
        assert_eq!(
            parse_toggles("toggleCrouch:TRUE\ntoggleSprint:/*on*/ True\n"),
            (true, true)
        );
        assert_eq!(
            parse_toggles("toggleCrouch:true\rtoggleCrouch:on\rtoggleSprint:True\r"),
            (false, true)
        );
    }
}

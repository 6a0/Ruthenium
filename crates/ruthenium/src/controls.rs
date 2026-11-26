//! The mouse settings and key bindings vanilla players already have: imported once from
//! the newest `options.txt` among the launchers' game folders, so the sensitivity is the
//! exact double vanilla uses (its slider stores any value from 0 to 1, so a value typed
//! again by hand would almost never be the same), and the keys are read as vanilla reads
//! them, from files of old versions too.

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
    let mut o = MouseOptions::DEFAULT;
    for line in text.lines() {
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        match key {
            // `OptionInstance.UnitDouble`: a double from 0 to 1.
            "mouseSensitivity" => {
                if let Ok(v) = value.parse::<f64>()
                    && (0.0..=1.0).contains(&v)
                {
                    o.sensitivity = v;
                }
            }
            "invertXMouse" | "invertYMouse" => {
                let v = match value {
                    "true" => true,
                    "false" => false,
                    _ => continue,
                };
                if key == "invertXMouse" {
                    o.invert_x = v;
                } else {
                    o.invert_y = v;
                }
            }
            _ => {}
        }
    }
    o
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

/// Whether sneak and sprint toggle (`toggleCrouch`, `toggleSprint`); a value that is not
/// `true` or `false` keeps the default (off), as vanilla's boolean options read.
pub fn parse_toggles(text: &str) -> (bool, bool) {
    let mut t = (false, false);
    for line in text.lines() {
        let (field, on) = match line.split_once(':') {
            Some(("toggleCrouch", v)) => (&mut t.0, v),
            Some(("toggleSprint", v)) => (&mut t.1, v),
            _ => continue,
        };
        match on {
            "true" => *field = true,
            "false" => *field = false,
            _ => {}
        }
    }
    t
}

/// A boolean option's value as vanilla reads it (`OptionInstance`'s codec over lenient
/// JSON): `true` or `false` in any case, with JSON's whitespace around it; anything else
/// (numbers, quotes, other words) is no value, and the option keeps its default.
fn vanilla_bool(value: &str) -> Option<bool> {
    let word = value.trim_matches([' ', '\t', '\n', '\r']);
    if word.eq_ignore_ascii_case("true") {
        Some(true)
    } else if word.eq_ignore_ascii_case("false") {
        Some(false)
    } else {
        None
    }
}

/// Whether Save Unsent Chats is on (`saveChatDrafts`, off by default); the last line
/// that names it counts, as vanilla reads the file.
pub fn parse_chat_drafts(text: &str) -> bool {
    let mut last = None;
    for line in text.lines() {
        if let Some(("saveChatDrafts", value)) = line.split_once(':') {
            last = Some(value);
        }
    }
    last.and_then(vanilla_bool).unwrap_or(false)
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

    #[test]
    fn reads_save_unsent_chats_as_vanilla_does() {
        // What vanilla's own codec makes of each value (OptionInstance.createBoolean's,
        // over LenientJsonParser): Some for a value, None where it fails and the option
        // keeps its default.
        let vanilla = [
            ("true", Some(true)),
            ("false", Some(false)),
            ("TRUE", Some(true)),
            ("True", Some(true)),
            ("tRuE", Some(true)),
            ("FALSE", Some(false)),
            (" true", Some(true)),
            ("true ", Some(true)),
            ("\ttrue", Some(true)),
            ("1", None),
            ("0", None),
            ("-0", None),
            ("1.5", None),
            ("NaN", None),
            ("yes", None),
            ("t", None),
            ("", None),
            ("\"true\"", None),
            ("'true'", None),
            ("true x", None),
            ("true:x", None),
            ("true,false", None),
            ("[true]", None),
            ("null", None),
            ("true;", None),
            ("#true", None),
            ("tru", None),
            ("truee", None),
        ];
        for (value, want) in vanilla {
            assert_eq!(vanilla_bool(value), want, "{value:?}");
            let line = format!("saveChatDrafts:{value}\n");
            assert_eq!(parse_chat_drafts(&line), want.unwrap_or(false), "{value:?}");
        }
        // The last line counts; other keys and lines without a colon don't.
        assert!(parse_chat_drafts(
            "saveChatDrafts:false\nsaveChatDrafts:true\nsaveChatDraft:false\nsaveChatDrafts\n"
        ));
        assert!(!parse_chat_drafts("version:4671\nchatLinks:true\n"));
        assert!(!parse_chat_drafts(
            "saveChatDrafts:true\nsaveChatDrafts:false\r\n"
        ));
        // (A later line that can't be read leaves the default, not the earlier value.)
        assert!(!parse_chat_drafts(
            "saveChatDrafts:true\nsaveChatDrafts:yes\n"
        ));
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
    }
}

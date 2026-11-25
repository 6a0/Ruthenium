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
fn options_files(env: &Env) -> Vec<PathBuf> {
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

fn modified(p: &Path) -> SystemTime {
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

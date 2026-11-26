//! `--input`: keys and mouse buttons pressed in the window as if by hand, once the world
//! shows with no screen over it, through the same path as the window's own events
//! (development: checks of the key bindings in the running game). The window losing
//! focus meanwhile neither lets go of keys nor pauses.
//!
//! Steps are separated by spaces: `<key name><+|->@<ms>` presses (+) or lets go of (-)
//! a key or button by its name as `options.txt` has it (`key.keyboard.i+@500`,
//! `key.mouse.5-@50`), `shot@<ms>` takes the `--screenshot` then, `type=<text>@<ms>`
//! types text (`_` for a space, `%XX` for any byte); each waits so many milliseconds
//! after the step before it.

use std::time::{Duration, Instant};

use ruthenium_sim::keybinds::{Key, Parsed};

/// One step.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    /// A key (SDL scancode) down or up.
    Key(u32, bool),
    /// A mouse button (SDL's numbering) down or up.
    Button(u8, bool),
    /// The screenshot.
    Shot,
    /// Typed text.
    Text(String),
}

/// See the module docs.
#[derive(Debug, Clone)]
pub struct InputScript {
    steps: Vec<(Duration, Step)>,
    next: usize,
    /// When the last step was taken (or the world showed).
    last: Option<Instant>,
}

impl InputScript {
    pub fn parse(text: &str) -> Result<InputScript, String> {
        let mut steps = Vec::new();
        for s in text.split_whitespace() {
            let bad = || format!("--input: can't read the step {s:?}");
            let (what, ms) = s.rsplit_once('@').ok_or_else(bad)?;
            let wait = Duration::from_millis(ms.parse().map_err(|_| bad())?);
            let step = if what == "shot" {
                Step::Shot
            } else if let Some(text) = what.strip_prefix("type=") {
                Step::Text(typed(text).ok_or_else(bad)?)
            } else {
                let down = match what.chars().last() {
                    Some('+') => true,
                    Some('-') => false,
                    _ => return Err(bad()),
                };
                match Key::parse(&what[..what.len() - 1]) {
                    Parsed::Key(Key::Keyboard(v)) if v > 0 => {
                        Step::Key(u32::try_from(v).map_err(|_| bad())?, down)
                    }
                    Parsed::Key(Key::Mouse(v)) => {
                        Step::Button(u8::try_from(v).map_err(|_| bad())?, down)
                    }
                    _ => return Err(bad()),
                }
            };
            steps.push((wait, step));
        }
        Ok(InputScript {
            steps,
            next: 0,
            last: None,
        })
    }

    /// Whether the world has shown and the steps are under way.
    pub fn started(&self) -> bool {
        self.last.is_some()
    }

    /// The steps due at `now`; none until the world shows (`in_world`).
    pub fn due(&mut self, now: Instant, in_world: bool) -> Vec<Step> {
        let mut out = Vec::new();
        let Some(mut last) = self.last.or(in_world.then_some(now)) else {
            return out;
        };
        while let Some((wait, step)) = self.steps.get(self.next).cloned() {
            if now < last + wait {
                break;
            }
            last += wait;
            self.next += 1;
            out.push(step);
        }
        self.last = Some(last);
        out
    }
}

/// A `type=` step's text: `_` a space, `%XX` a byte, the rest as it is.
fn typed(s: &str) -> Option<String> {
    let mut bytes = Vec::with_capacity(s.len());
    let mut it = s.bytes();
    while let Some(b) = it.next() {
        match b {
            b'_' => bytes.push(b' '),
            b'%' => {
                let hex = [it.next()?, it.next()?];
                bytes.push(u8::from_str_radix(std::str::from_utf8(&hex).ok()?, 16).ok()?);
            }
            b => bytes.push(b),
        }
    }
    String::from_utf8(bytes).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn steps_wait_for_the_world_and_then_each_other() {
        let mut s =
            InputScript::parse("key.keyboard.i+@100 key.mouse.5+@0 key.keyboard.i-@50 shot@10")
                .unwrap();
        let t = Instant::now();
        assert_eq!(s.due(t + Duration::from_secs(9), false), []);
        assert_eq!(s.due(t, true), []);
        assert_eq!(
            s.due(t + Duration::from_millis(120), true),
            [Step::Key(12, true), Step::Button(5, true)]
        );
        assert_eq!(s.due(t + Duration::from_millis(140), true), []);
        assert_eq!(
            s.due(t + Duration::from_millis(200), true),
            [Step::Key(12, false), Step::Shot]
        );
        let mut t = InputScript::parse("type=/msg_@a_%C3%A9@0").unwrap();
        assert_eq!(
            t.due(Instant::now(), true),
            [Step::Text("/msg @a é".into())]
        );
        assert!(InputScript::parse("type=%Z@5").is_err());
        assert!(InputScript::parse("key.keyboard.i@5").is_err());
        assert!(InputScript::parse("w+@5").is_err());
        assert!(InputScript::parse("key.keyboard.unknown+@5").is_err());
        assert!(InputScript::parse("key.mouse.4+").is_err());
    }
}

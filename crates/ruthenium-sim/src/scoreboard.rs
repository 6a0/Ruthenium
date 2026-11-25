//! What is written over an entity's head: its name as its team shows it, and below it the
//! score of the objective that is shown there (`Scoreboard`, as far as that goes).
//!
//! Texts stay components in their NBT form: what draws them has the language.

use std::collections::HashMap;
use std::sync::Arc;

use ruthenium_protocol::nbt::{Compound, Tag};
use ruthenium_protocol::packets::play::{self, NumberFormat};

/// The names of the sixteen colours, by `ChatFormatting` ordinal.
const COLOURS: [&str; 16] = [
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

/// The style field a `ChatFormatting` sets, by its ordinal: a colour, or one of the five
/// formats. Nothing for reset and for what is no formatting.
fn formatting(ordinal: i32) -> Option<(&'static str, Tag)> {
    let flag = |name| Some((name, Tag::Byte(1)));
    match ordinal {
        0..=15 => Some(("color", Tag::String(COLOURS[ordinal as usize].to_owned()))),
        16 => flag("obfuscated"),
        17 => flag("bold"),
        18 => flag("strikethrough"),
        19 => flag("underlined"),
        20 => flag("italic"),
        _ => None,
    }
}

/// A name between a team's prefix and suffix, all in the team's colour
/// (`PlayerTeam.getFormattedName`).
pub(crate) fn team_name(prefix: &Tag, name: Tag, suffix: &Tag, colour: Option<i32>) -> Tag {
    let mut c = Compound::new();
    c.put("text", Tag::String(String::new()));
    if let Some((field, value)) = colour.and_then(formatting) {
        c.put(field, value);
    }
    c.put(
        "extra",
        Tag::List(vec![prefix.clone(), name, suffix.clone()]),
    );
    Tag::Compound(c)
}

#[derive(Debug, Clone, PartialEq)]
struct Objective {
    display_name: Tag,
    format: Option<NumberFormat>,
}

#[derive(Debug, Clone, PartialEq)]
struct Score {
    value: i32,
    format: Option<NumberFormat>,
}

/// The objectives and scores the server has told of.
#[derive(Debug, Clone, Default)]
pub struct Scoreboard {
    objectives: HashMap<String, Objective>,
    /// By owner, then by objective.
    scores: HashMap<String, HashMap<String, Score>>,
    /// The objective shown below names.
    below_name: Option<String>,
    rev: u64,
}

impl Scoreboard {
    /// Counts the changes: what was made of an older state is out of date.
    pub fn rev(&self) -> u64 {
        self.rev
    }

    /// `handleAddObjective`.
    pub fn set_objective(
        &mut self,
        name: &str,
        method: u8,
        display_name: Option<Tag>,
        format: Option<NumberFormat>,
    ) {
        self.rev += 1;
        let objective = display_name.map(|display_name| Objective {
            display_name,
            format,
        });
        match (method, objective) {
            // (An objective that is there already stays as it is.)
            (0, Some(o)) => {
                self.objectives.entry(name.to_owned()).or_insert(o);
            }
            (1, _) => {
                if self.objectives.remove(name).is_some() {
                    if self.below_name.as_deref() == Some(name) {
                        self.below_name = None;
                    }
                    for scores in self.scores.values_mut() {
                        scores.remove(name);
                    }
                }
            }
            (2, Some(o)) => {
                if let Some(old) = self.objectives.get_mut(name) {
                    *old = o;
                }
            }
            _ => {}
        }
    }

    /// `handleSetDisplayObjective`: only the slot below names is kept.
    pub fn set_display(&mut self, slot: i32, objective: &str) {
        if slot == play::DISPLAY_SLOT_BELOW_NAME {
            self.rev += 1;
            self.below_name = self
                .objectives
                .contains_key(objective)
                .then(|| objective.to_owned());
        }
    }

    /// `handleSetScore`.
    pub fn set_score(
        &mut self,
        owner: &str,
        objective: &str,
        value: i32,
        format: Option<NumberFormat>,
    ) {
        if self.objectives.contains_key(objective) {
            self.rev += 1;
            self.scores
                .entry(owner.to_owned())
                .or_default()
                .insert(objective.to_owned(), Score { value, format });
        }
    }

    /// `handleResetScore`: one of an owner's scores, or all of them.
    pub fn reset_score(&mut self, owner: &str, objective: Option<&str>) {
        self.rev += 1;
        match objective {
            None => {
                self.scores.remove(owner);
            }
            Some(objective) => {
                if let Some(scores) = self.scores.get_mut(owner) {
                    scores.remove(objective);
                    if scores.is_empty() {
                        self.scores.remove(owner);
                    }
                }
            }
        }
    }

    /// What stands below the name of `owner` (`Entity.belowNameDisplay`): its score as the
    /// objective writes it, a space, and the objective's name.
    pub fn below_name(&self, owner: &str) -> Option<Tag> {
        let name = self.below_name.as_ref()?;
        let objective = self.objectives.get(name)?;
        let score = self.scores.get(owner)?.get(name)?;
        let number = || Tag::String(score.value.to_string());
        let value = match score.format.as_ref().or(objective.format.as_ref()) {
            None => number(),
            Some(NumberFormat::Blank) => Tag::String(String::new()),
            Some(NumberFormat::Fixed(text)) => text.clone(),
            Some(NumberFormat::Styled(Tag::Compound(style))) => {
                let mut c = style.clone();
                c.put("text", number());
                Tag::Compound(c)
            }
            // (A style that is no compound styles nothing.)
            Some(NumberFormat::Styled(_)) => number(),
        };
        // The first is the parent of the rest: an empty text, which styles nothing.
        Some(Tag::List(vec![
            Tag::String(String::new()),
            value,
            Tag::String(" ".to_owned()),
            objective.display_name.clone(),
        ]))
    }
}

/// What is written over an entity's head.
#[derive(Debug, Clone, PartialEq)]
pub struct NameText {
    pub name: Tag,
    /// What stands below it, for those who are near.
    pub below: Option<Tag>,
}

/// An entity's name as drawing needs it.
#[derive(Debug, Clone, PartialEq)]
pub struct NameRecord {
    /// The same one for as long as nothing of it changes.
    pub text: Arc<NameText>,
    /// It sneaks: its name is not seen through walls, nor from 32 blocks or more.
    pub discrete: bool,
    /// From how far the name is seen, and what is below it, in blocks.
    pub distance: f32,
    pub below_distance: f32,
    /// How far the text is moved down, in its own units (up over deadmau5's ears).
    pub offset: i32,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(s: &str) -> Tag {
        Tag::String(s.to_owned())
    }

    #[test]
    fn a_team_colours_its_names_and_stands_around_them() {
        let name = team_name(&text("["), text("Kai"), &text("]"), Some(12));
        let c = name.as_compound().unwrap();
        assert_eq!(c.string("color"), Some("red"));
        assert_eq!(c.list("extra"), [text("["), text("Kai"), text("]")]);
        // A format is no colour; reset and what is unknown are nothing.
        let bold = team_name(&text(""), text("Kai"), &text(""), Some(17));
        let c = bold.as_compound().unwrap();
        assert_eq!((c.string("color"), c.boolean("bold")), (None, Some(true)));
        let formats = [
            (16, "obfuscated"),
            (17, "bold"),
            (18, "strikethrough"),
            (19, "underlined"),
            (20, "italic"),
        ];
        for (ordinal, field) in formats {
            let named = team_name(&text(""), text("Kai"), &text(""), Some(ordinal));
            let c = named.as_compound().unwrap();
            assert_eq!((c.boolean(field), c.len()), (Some(true), 3), "{field}");
        }
        for none in [None, Some(21), Some(99), Some(-1)] {
            let plain = team_name(&text(""), text("Kai"), &text(""), none);
            assert_eq!(plain.as_compound().unwrap().len(), 2, "{none:?}");
        }
    }

    #[test]
    fn a_score_is_shown_below_a_name_as_its_objective_says() {
        let mut s = Scoreboard::default();
        // Nothing without the objective, the slot and a score.
        s.set_score("Kai", "hp", 20, None);
        assert_eq!(s.below_name("Kai"), None);
        s.set_objective("hp", 0, Some(text("Health")), None);
        s.set_display(play::DISPLAY_SLOT_BELOW_NAME, "hp");
        assert_eq!(s.below_name("Kai"), None);
        s.set_score("Kai", "hp", 20, None);
        let shown = |s: &Scoreboard| match s.below_name("Kai") {
            Some(Tag::List(items)) => items,
            other => panic!("{other:?}"),
        };
        assert_eq!(shown(&s), [text(""), text("20"), text(" "), text("Health")]);
        assert_eq!(s.below_name("Efe"), None);
        // The objective's way of writing, and the score's own over it.
        let mut red = Compound::new();
        red.put("color", text("red"));
        s.set_objective(
            "hp",
            2,
            Some(text("HP")),
            Some(NumberFormat::Styled(Tag::Compound(red))),
        );
        let items = shown(&s);
        let value = items[1].as_compound().unwrap();
        assert_eq!(
            (value.string("text"), value.string("color")),
            (Some("20"), Some("red"))
        );
        assert_eq!(items[3], text("HP"));
        s.set_score("Kai", "hp", 7, Some(NumberFormat::Fixed(text("seven"))));
        assert_eq!(shown(&s)[1], text("seven"));
        s.set_score("Kai", "hp", 7, Some(NumberFormat::Blank));
        assert_eq!(shown(&s)[1], text(""));
        // Another slot shows nothing here; a slot given no objective shows none.
        s.set_display(1, "");
        assert!(s.below_name("Kai").is_some());
        s.set_display(play::DISPLAY_SLOT_BELOW_NAME, "");
        assert_eq!(s.below_name("Kai"), None);
    }

    #[test]
    fn scores_go_with_their_owner_or_their_objective() {
        let mut s = Scoreboard::default();
        for name in ["hp", "kills"] {
            s.set_objective(name, 0, Some(text(name)), None);
            s.set_score("Kai", name, 1, None);
        }
        s.set_display(play::DISPLAY_SLOT_BELOW_NAME, "hp");
        // An objective that is there is not made again.
        s.set_objective("hp", 0, Some(text("other")), None);
        assert!(matches!(s.below_name("Kai"), Some(Tag::List(i)) if i[3] == text("hp")));
        s.reset_score("Kai", Some("kills"));
        assert!(s.below_name("Kai").is_some());
        s.reset_score("Kai", Some("hp"));
        assert_eq!(s.below_name("Kai"), None);
        s.set_score("Kai", "hp", 3, None);
        s.reset_score("Kai", None);
        assert_eq!(s.below_name("Kai"), None);
        // A removed objective takes its scores and its slot along.
        s.set_score("Kai", "hp", 3, None);
        s.set_objective("hp", 1, None, None);
        s.set_objective("hp", 0, Some(text("hp")), None);
        assert_eq!(s.below_name("Kai"), None);
        s.set_display(play::DISPLAY_SLOT_BELOW_NAME, "hp");
        assert_eq!(s.below_name("Kai"), None);
        // A change to one that is not there makes none.
        s.set_objective("gone", 2, Some(text("gone")), None);
        s.set_score("Kai", "gone", 3, None);
        s.set_display(play::DISPLAY_SLOT_BELOW_NAME, "gone");
        assert_eq!(s.below_name("Kai"), None);
    }
}

//! The scoreboard (`Scoreboard`): what is written over an entity's head (its name as its
//! team shows it, and below it the score of the objective that is shown there), the
//! sidebar, and the scores in the player list.
//!
//! Texts stay components in their NBT form: what draws them has the language.

use std::collections::HashMap;
use std::sync::Arc;

use ruthenium_protocol::nbt::{Compound, Tag};
use ruthenium_protocol::packets::play::{self, NumberFormat};

use crate::entity::Teams;

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
    /// The player list shows it as hearts.
    hearts: bool,
    format: Option<NumberFormat>,
}

#[derive(Debug, Clone, PartialEq)]
struct Score {
    value: i32,
    /// What its owner is called.
    display: Option<Tag>,
    format: Option<NumberFormat>,
}

/// A score as it is written: as it says itself, else as its objective says, else in
/// `colour`.
fn written(value: i32, formats: [Option<&NumberFormat>; 2], colour: Option<&str>) -> Tag {
    let number = || Tag::String(value.to_string());
    let styled = |mut style: Compound| {
        style.put("text", number());
        Tag::Compound(style)
    };
    let [own, objective] = formats;
    match own.or(objective) {
        Some(NumberFormat::Blank) => Tag::String(String::new()),
        Some(NumberFormat::Fixed(text)) => text.clone(),
        Some(NumberFormat::Styled(Tag::Compound(style))) => styled(style.clone()),
        // (A style that is no compound styles nothing.)
        Some(NumberFormat::Styled(_)) => number(),
        None => match colour {
            Some(colour) => {
                let mut style = Compound::new();
                style.put("color", Tag::String(colour.to_owned()));
                styled(style)
            }
            None => number(),
        },
    }
}

/// Lines of the sidebar at most.
const SIDEBAR_LINES: usize = 15;

/// What the sidebar shows (`Hud.displayScoreboardSidebar`).
#[derive(Debug, Clone, PartialEq)]
pub struct Sidebar {
    pub title: Arc<Tag>,
    /// Each line's name and score, from the top.
    pub lines: Vec<(Arc<Tag>, Arc<Tag>)>,
}

/// `String.CASE_INSENSITIVE_ORDER`.
pub(crate) fn ignoring_case(a: &str, b: &str) -> std::cmp::Ordering {
    let folded = |s: &str| -> Vec<u32> {
        s.chars()
            .map(|c| {
                let one = |mut i: std::char::ToUppercase| match (i.next(), i.next()) {
                    (Some(u), None) => u,
                    _ => c,
                };
                let upper = one(c.to_uppercase());
                let mut lower = upper.to_lowercase();
                match (lower.next(), lower.next()) {
                    (Some(l), None) => l as u32,
                    _ => upper as u32,
                }
            })
            .collect()
    };
    folded(a).cmp(&folded(b))
}

/// The objectives and scores the server has told of.
#[derive(Debug, Clone, Default)]
pub struct Scoreboard {
    objectives: HashMap<String, Objective>,
    /// By owner, then by objective.
    scores: HashMap<String, HashMap<String, Score>>,
    /// The objective each `DisplaySlot` shows.
    displayed: HashMap<i32, String>,
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
        hearts: bool,
        format: Option<NumberFormat>,
    ) {
        self.rev += 1;
        let objective = display_name.map(|display_name| Objective {
            display_name,
            hearts,
            format,
        });
        match (method, objective) {
            // (An objective that is there already stays as it is.)
            (0, Some(o)) => {
                self.objectives.entry(name.to_owned()).or_insert(o);
            }
            (1, _) => {
                if self.objectives.remove(name).is_some() {
                    self.displayed.retain(|_, o| o != name);
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

    /// `handleSetDisplayObjective`.
    pub fn set_display(&mut self, slot: i32, objective: &str) {
        self.rev += 1;
        // (Unknown slots read as the first.)
        let slot = if (0..19).contains(&slot) { slot } else { 0 };
        if self.objectives.contains_key(objective) {
            self.displayed.insert(slot, objective.to_owned());
        } else {
            self.displayed.remove(&slot);
        }
    }

    /// The objective a slot shows, and its name.
    fn shown(&self, slot: i32) -> Option<(&str, &Objective)> {
        let name = self.displayed.get(&slot)?;
        Some((name, self.objectives.get(name)?))
    }

    /// `handleSetScore`.
    pub fn set_score(
        &mut self,
        owner: &str,
        objective: &str,
        value: i32,
        display: Option<Tag>,
        format: Option<NumberFormat>,
    ) {
        if self.objectives.contains_key(objective) {
            self.rev += 1;
            let score = Score {
                value,
                display,
                format,
            };
            self.scores
                .entry(owner.to_owned())
                .or_default()
                .insert(objective.to_owned(), score);
        }
    }

    /// The sidebar as the player `owner` sees it (`extractScoreboardSidebar`): its team's
    /// colour's own, else the one for all.
    pub fn sidebar(&self, owner: &str, teams: &Teams) -> Option<Sidebar> {
        let own = teams
            .team_of(owner)
            .and_then(|t| t.colour)
            .and_then(|colour| self.shown(play::DISPLAY_SLOT_TEAM + colour));
        let (name, objective) = own.or_else(|| self.shown(play::DISPLAY_SLOT_SIDEBAR))?;
        let mut scores: Vec<(&str, &Score)> = self
            .scores
            .iter()
            .filter(|(owner, _)| !owner.starts_with('#'))
            .filter_map(|(owner, scores)| Some((owner.as_str(), scores.get(name)?)))
            .collect();
        scores.sort_by(|(a, x), (b, y)| {
            y.value
                .cmp(&x.value)
                .then_with(|| ignoring_case(a, b))
                // (Which of two that differ in case alone comes first is not said: one
                // and the same every time.)
                .then_with(|| a.cmp(b))
        });
        scores.truncate(SIDEBAR_LINES);
        let lines = scores
            .into_iter()
            .map(|(owner, score)| {
                let name = score
                    .display
                    .clone()
                    .unwrap_or_else(|| Tag::String(owner.to_owned()));
                let name = teams.decorated(owner, name);
                let formats = [score.format.as_ref(), objective.format.as_ref()];
                let value = written(score.value, formats, Some("red"));
                (Arc::new(name), Arc::new(value))
            })
            .collect();
        Some(Sidebar {
            title: Arc::new(objective.display_name.clone()),
            lines,
        })
    }

    /// Whether the player list shows scores, and as hearts.
    pub fn list_hearts(&self) -> Option<bool> {
        Some(self.shown(play::DISPLAY_SLOT_LIST)?.1.hearts)
    }

    /// The score of `owner` in the player list: its value, and how it is written.
    pub fn list_score(&self, owner: &str) -> Option<(i32, Tag)> {
        let (name, objective) = self.shown(play::DISPLAY_SLOT_LIST)?;
        let score = self.scores.get(owner)?.get(name)?;
        let formats = [score.format.as_ref(), objective.format.as_ref()];
        Some((score.value, written(score.value, formats, Some("yellow"))))
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
        let (name, objective) = self.shown(play::DISPLAY_SLOT_BELOW_NAME)?;
        let score = self.scores.get(owner)?.get(name)?;
        let formats = [score.format.as_ref(), objective.format.as_ref()];
        let value = written(score.value, formats, None);
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
        s.set_score("Kai", "hp", 20, None, None);
        assert_eq!(s.below_name("Kai"), None);
        s.set_objective("hp", 0, Some(text("Health")), false, None);
        s.set_display(play::DISPLAY_SLOT_BELOW_NAME, "hp");
        assert_eq!(s.below_name("Kai"), None);
        s.set_score("Kai", "hp", 20, None, None);
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
            false,
            Some(NumberFormat::Styled(Tag::Compound(red))),
        );
        let items = shown(&s);
        let value = items[1].as_compound().unwrap();
        assert_eq!(
            (value.string("text"), value.string("color")),
            (Some("20"), Some("red"))
        );
        assert_eq!(items[3], text("HP"));
        s.set_score(
            "Kai",
            "hp",
            7,
            None,
            Some(NumberFormat::Fixed(text("seven"))),
        );
        assert_eq!(shown(&s)[1], text("seven"));
        s.set_score("Kai", "hp", 7, None, Some(NumberFormat::Blank));
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
            s.set_objective(name, 0, Some(text(name)), false, None);
            s.set_score("Kai", name, 1, None, None);
        }
        s.set_display(play::DISPLAY_SLOT_BELOW_NAME, "hp");
        // An objective that is there is not made again.
        s.set_objective("hp", 0, Some(text("other")), false, None);
        assert!(matches!(s.below_name("Kai"), Some(Tag::List(i)) if i[3] == text("hp")));
        s.reset_score("Kai", Some("kills"));
        assert!(s.below_name("Kai").is_some());
        s.reset_score("Kai", Some("hp"));
        assert_eq!(s.below_name("Kai"), None);
        s.set_score("Kai", "hp", 3, None, None);
        s.reset_score("Kai", None);
        assert_eq!(s.below_name("Kai"), None);
        // A removed objective takes its scores and its slot along.
        s.set_score("Kai", "hp", 3, None, None);
        s.set_objective("hp", 1, None, false, None);
        s.set_objective("hp", 0, Some(text("hp")), false, None);
        assert_eq!(s.below_name("Kai"), None);
        s.set_display(play::DISPLAY_SLOT_BELOW_NAME, "hp");
        assert_eq!(s.below_name("Kai"), None);
        // A change to one that is not there makes none.
        s.set_objective("gone", 2, Some(text("gone")), false, None);
        s.set_score("Kai", "gone", 3, None, None);
        s.set_display(play::DISPLAY_SLOT_BELOW_NAME, "gone");
        assert_eq!(s.below_name("Kai"), None);
    }

    #[test]
    fn the_sidebar_shows_the_highest_scores_first() {
        let mut s = Scoreboard::default();
        let mut teams = Teams::default();
        let plain = |b: &Sidebar| -> Vec<(Tag, Tag)> {
            b.lines
                .iter()
                .map(|(n, s)| ((**n).clone(), (**s).clone()))
                .collect()
        };
        let red = |n: i32| {
            let mut c = Compound::new();
            c.put("color", text("red"));
            c.put("text", text(&n.to_string()));
            Tag::Compound(c)
        };
        assert_eq!(s.sidebar("Kai", &teams), None);
        s.set_objective("o", 0, Some(text("Points")), false, None);
        s.set_display(play::DISPLAY_SLOT_SIDEBAR, "o");
        // Shown even without a score in it.
        assert!(s.sidebar("Kai", &teams).unwrap().lines.is_empty());
        for (owner, value) in [("bea", 2), ("Abe", 2), ("cy", 9), ("#hid", 50), ("dan", -1)] {
            s.set_score(owner, "o", value, None, None);
        }
        let bar = s.sidebar("Kai", &teams).unwrap();
        assert_eq!(*bar.title, text("Points"));
        assert_eq!(
            plain(&bar),
            [
                (text("cy"), red(9)),
                (text("Abe"), red(2)),
                (text("bea"), red(2)),
                (text("dan"), red(-1)),
            ]
        );
        // What an owner is called, how a number is written, and the owner's team.
        s.set_score(
            "cy",
            "o",
            9,
            Some(text("Boss")),
            Some(NumberFormat::Fixed(text("top"))),
        );
        let params = play::TeamParameters {
            display_name: text(""),
            prefix: text("["),
            suffix: text("]"),
            name_tag_visibility: 0,
            collision_rule: 0,
            color: Some(12),
            options: 0,
        };
        teams.apply("t", 0, Some(&params), &["cy".to_owned(), "Kai".to_owned()]);
        let bar = plain(&s.sidebar("Kai", &teams).unwrap());
        let boss = team_name(&text("["), text("Boss"), &text("]"), Some(12));
        assert_eq!(bar[0], (boss, text("top")));
        // A team's colour has a sidebar of its own, which only its members see.
        s.set_objective("own", 0, Some(text("Team")), false, None);
        s.set_display(play::DISPLAY_SLOT_TEAM + 12, "own");
        assert_eq!(*s.sidebar("Kai", &teams).unwrap().title, text("Team"));
        assert_eq!(*s.sidebar("Efe", &teams).unwrap().title, text("Points"));
        s.set_display(play::DISPLAY_SLOT_TEAM + 12, "");
        assert_eq!(*s.sidebar("Kai", &teams).unwrap().title, text("Points"));
        // Fifteen at most.
        for i in 0..20 {
            s.set_score(&format!("p{i:02}"), "o", 100 + i, None, None);
        }
        let bar = s.sidebar("Kai", &teams).unwrap();
        assert_eq!(bar.lines.len(), 15);
        assert_eq!(*bar.lines[14].0, text("p05"));
        // Removed and made again, the objective is shown nowhere.
        s.set_objective("o", 1, None, false, None);
        s.set_objective("o", 0, Some(text("Points")), false, None);
        assert_eq!(s.sidebar("Kai", &teams), None);
    }

    #[test]
    fn the_player_list_shows_scores_as_numbers_or_hearts() {
        let mut s = Scoreboard::default();
        assert_eq!((s.list_hearts(), s.list_score("Kai")), (None, None));
        s.set_objective("o", 0, Some(text("")), true, None);
        s.set_display(play::DISPLAY_SLOT_LIST, "o");
        assert_eq!((s.list_hearts(), s.list_score("Kai")), (Some(true), None));
        s.set_objective("o", 2, Some(text("")), false, None);
        s.set_score("Kai", "o", 7, None, None);
        assert_eq!(s.list_hearts(), Some(false));
        let (value, shown) = s.list_score("Kai").unwrap();
        let c = shown.as_compound().unwrap();
        assert_eq!(
            (value, c.string("text"), c.string("color")),
            (7, Some("7"), Some("yellow"))
        );
        s.set_objective("o", 1, None, false, None);
        assert_eq!(s.list_hearts(), None);
        // A slot that is not known is the list's.
        s.set_objective("o", 0, Some(text("")), true, None);
        s.set_display(99, "o");
        assert_eq!(s.list_hearts(), Some(true));
    }

    #[test]
    fn names_are_ordered_as_java_does_ignoring_case() {
        use std::cmp::Ordering::{Equal, Greater, Less};
        assert_eq!(ignoring_case("abe", "ABE"), Equal);
        assert_eq!(ignoring_case("Abe", "bea"), Less);
        // Case is folded up and then down: an underscore comes before the letters.
        assert_eq!(ignoring_case("_a", "Aa"), Less);
        assert_eq!(ignoring_case("ab", "a"), Greater);
        assert_eq!(ignoring_case("ß", "ẞ"), Equal);
    }
}

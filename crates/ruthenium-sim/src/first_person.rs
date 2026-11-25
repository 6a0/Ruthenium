//! What of the local player moves only to be drawn from its own eyes: the swing of its
//! hands, how its view bobs with its steps and tilts when it is hurt, and what its hands
//! hold up (`FirstPersonHandsAndItems`, `ClientAvatarState`, `LivingEntity.SwingState`
//! and the hurt fields of `LivingEntity`).
//!
//! Nothing here reaches the server: the swing's packet is the click's own
//! ([`crate::interact`]).

use std::sync::Arc;

use crate::avatar::{SkinSource, Swing, SwingState};
use crate::item::Stack;

/// How far a hand's height moves in a tick at most.
const HEIGHT_STEP: f32 = 0.4;
/// Below this height a hand takes up what it is to hold next.
const HEIGHT_SWAP: f32 = 0.1;

/// Which hands are drawn (`HandRenderSelection`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Hands {
    #[default]
    Both,
    MainOnly,
    OffOnly,
}

impl Hands {
    pub fn main(self) -> bool {
        self != Hands::OffOnly
    }

    pub fn off(self) -> bool {
        self != Hands::MainOnly
    }
}

/// `FirstPersonHandsAndItems.evaluateWhichHandsToRender`: a bow or a crossbow in use
/// leaves the other hand out, and a loaded crossbow in the main hand the off hand.
/// `using` is the hand that uses an item, if one does.
pub fn hands_drawn(main: &Stack, off: &Stack, using: Option<bool>) -> Hands {
    use crate::item::Class;
    let is = |s: &Stack, class: Class| !s.is_empty() && s.c.class == class;
    let loaded = |s: &Stack| is(s, Class::Crossbow) && s.c.charged;
    let bow = is(main, Class::Bow) || is(off, Class::Bow);
    let crossbow = is(main, Class::Crossbow) || is(off, Class::Crossbow);
    if !bow && !crossbow {
        return Hands::Both;
    }
    match using {
        None if loaded(main) => Hands::MainOnly,
        None => Hands::Both,
        Some(off_hand) => {
            let used = if off_hand { off } else { main };
            if is(used, Class::Bow) || is(used, Class::Crossbow) {
                if off_hand {
                    Hands::OffOnly
                } else {
                    Hands::MainOnly
                }
            } else if !off_hand && loaded(off) {
                Hands::MainOnly
            } else {
                Hands::Both
            }
        }
    }
}

/// See the module docs.
#[derive(Debug, Clone, PartialEq)]
pub struct FirstPerson {
    swing: SwingState,
    pub(crate) last_kinetic_hit: Option<i64>,
    /// `hurtTime`, `hurtDuration` and `hurtDir`: the ticks a flinch has left, how long
    /// it is, and where what hurt came from.
    hurt_time: i32,
    hurt_duration: i32,
    hurt_dir: f32,
    /// `LocalPlayer.flashOnSetHealth`: the first health the server sends is no hurt.
    flash_on_set_health: bool,
    /// `ClientAvatarState`: how far the player has walked, and how much its view
    /// bobs; each as the last tick began and now.
    walk: (f32, f32),
    bob: (f32, f32),
    /// `xBob` and `yBob`: the pitch and yaw the hands trail behind the view with.
    x_bob: (f32, f32),
    y_bob: (f32, f32),
    /// What the main hand and the other hand show, and how far up each is held (0: out
    /// of sight below; 1: where it rests), as the last tick began and now.
    held: [Stack; 2],
    heights: [(f32, f32); 2],
}

impl Default for FirstPerson {
    fn default() -> FirstPerson {
        FirstPerson {
            swing: SwingState::default(),
            last_kinetic_hit: None,
            hurt_time: 0,
            hurt_duration: 0,
            hurt_dir: 0.0,
            flash_on_set_health: false,
            walk: (0.0, 0.0),
            bob: (0.0, 0.0),
            x_bob: (0.0, 0.0),
            y_bob: (0.0, 0.0),
            held: [Stack::empty(), Stack::empty()],
            heights: [(0.0, 0.0); 2],
        }
    }
}

/// The local player's own view of itself, as drawing needs it: what its last two ticks
/// left.
#[derive(Debug, Clone, PartialEq)]
pub struct FirstPersonRecord {
    pub skin: Arc<SkinSource>,
    /// `PlayerModelPart` bits, as an avatar's.
    pub parts: u8,
    pub left_handed: bool,
    pub invisible: bool,
    pub on_fire: bool,
    pub sleeping: bool,
    pub spin_attack: bool,
    pub ticks_since_kinetic_hit: Option<i32>,
    /// Looking through a spyglass.
    pub scoping: bool,
    pub swing: Option<Swing>,
    pub swing_progress: (f32, f32),
    pub hurt_time: i32,
    pub hurt_duration: i32,
    pub hurt_dir: f32,
    pub dead: bool,
    pub death_time: i32,
    pub walk: (f32, f32),
    pub bob: (f32, f32),
    pub x_bob: (f32, f32),
    pub y_bob: (f32, f32),
    /// What the main hand and the other hand show.
    pub held: [Stack; 2],
    pub heights: [(f32, f32); 2],
    pub hands: Hands,
    /// The hand that uses an item (the off hand: true), and the ticks its use has left.
    pub using: Option<bool>,
    pub use_remaining: i32,
    /// How long each hand's item takes to use, and a crossbow in it to load, in ticks.
    pub use_duration: [i32; 2],
    pub charge_duration: [i32; 2],
}

impl Default for FirstPersonRecord {
    fn default() -> FirstPersonRecord {
        FirstPersonRecord {
            skin: Arc::new(SkinSource::Profile(Default::default())),
            parts: 0,
            left_handed: false,
            invisible: false,
            on_fire: false,
            sleeping: false,
            spin_attack: false,
            ticks_since_kinetic_hit: None,
            scoping: false,
            swing: None,
            swing_progress: (0.0, 0.0),
            hurt_time: 0,
            hurt_duration: 0,
            hurt_dir: 0.0,
            dead: false,
            death_time: 0,
            walk: (0.0, 0.0),
            bob: (0.0, 0.0),
            x_bob: (0.0, 0.0),
            y_bob: (0.0, 0.0),
            held: [Stack::empty(), Stack::empty()],
            heights: [(0.0, 0.0); 2],
            hands: Hands::Both,
            using: None,
            use_remaining: 0,
            use_duration: [0; 2],
            charge_duration: [0; 2],
        }
    }
}

impl FirstPerson {
    /// `LivingEntity.swing`: a hand swings, unless a swing is in its first half.
    pub fn swing(&mut self, swing: Swing) {
        self.swing.start(swing);
    }

    /// `animateHurt` (with where it came from) and `handleDamageEvent` (without): a
    /// flinch of ten ticks.
    pub fn hurt(&mut self, from: Option<f32>) {
        self.hurt_duration = 10;
        self.hurt_time = 10;
        if let Some(yaw) = from {
            self.hurt_dir = yaw;
        }
    }

    /// `flashOnSetHealth`: the server has set the health before.
    pub fn health_was_set(&self) -> bool {
        self.flash_on_set_health
    }

    /// `LocalPlayer.hurtTo`'s flinch: health the server sets lower than it was, but
    /// for the first it sends.
    pub fn health_set(&mut self, old: f32, new: f32) {
        if !self.flash_on_set_health {
            self.flash_on_set_health = true;
        } else if old - new > 0.0 {
            self.hurt_duration = 10;
            self.hurt_time = 10;
        }
    }

    /// `ClientAvatarState.tick`: the tick begins.
    pub(crate) fn begin_tick(&mut self) {
        self.walk.0 = self.walk.1;
    }

    /// `LivingEntity.baseTick`'s first step: the swing goes on.
    pub(crate) fn tick_swing(&mut self) {
        self.swing.tick();
    }

    /// And its later one: the flinch wears off.
    pub(crate) fn tick_hurt(&mut self) {
        if self.hurt_time > 0 {
            self.hurt_time -= 1;
        }
    }

    /// Whether a swing has begun to show (`swingState.animation > 0`).
    pub fn swinging(&self) -> bool {
        self.swing.now > 0.0
    }

    /// `updateBob`: the bob eases towards `target`.
    pub(crate) fn update_bob(&mut self, target: f32) {
        self.bob.0 = self.bob.1;
        self.bob.1 += (target - self.bob.1) * 0.4;
    }

    /// `resetBob`: none while riding.
    pub(crate) fn reset_bob(&mut self) {
        self.bob = (self.bob.1, 0.0);
    }

    /// `LocalPlayer.applyInput`: the hands follow the view half of the way.
    pub(crate) fn follow_view(&mut self, x_rot: f32, y_rot: f32) {
        self.x_bob.0 = self.x_bob.1;
        self.y_bob.0 = self.y_bob.1;
        self.x_bob.1 += (x_rot - self.x_bob.1) * 0.5;
        self.y_bob.1 += (y_rot - self.y_bob.1) * 0.5;
    }

    /// `addWalkedDistance`.
    pub(crate) fn walked(&mut self, distance: f32) {
        self.walk.1 += distance;
    }

    /// `itemUsed`: the hand's item comes up anew.
    pub fn item_used(&mut self, off_hand: bool) {
        self.heights[usize::from(off_hand)].1 = 0.0;
    }

    /// `FirstPersonHandsAndItems.tick`, after the player's own: a hand that is to hold
    /// something else sinks, takes it up out of sight and rises again; the main hand
    /// rises only as far as the attack has recovered (`swap_scale`:
    /// `getItemSwapScale(1)`). Hands busy paddling stay down.
    pub(crate) fn tick_hands(&mut self, next: [&Stack; 2], hands_busy: bool, swap_scale: f32) {
        for (hand, next) in next.into_iter().enumerate() {
            self.heights[hand].0 = self.heights[hand].1;
            // (Every item's model plays the swap: none of the game's says otherwise.)
            if self.held[hand].matches_but_damage(next) {
                self.held[hand] = next.clone();
            }
        }
        for (hand, next) in next.into_iter().enumerate() {
            let height = &mut self.heights[hand].1;
            if hands_busy {
                *height = (*height - HEIGHT_STEP).clamp(0.0, 1.0);
            } else {
                let target = if !self.held[hand].same_object(next) {
                    0.0
                } else if hand == 0 {
                    swap_scale * swap_scale * swap_scale
                } else {
                    1.0
                };
                *height += (target - *height).clamp(-HEIGHT_STEP, HEIGHT_STEP);
            }
        }
        for (hand, next) in next.into_iter().enumerate() {
            if self.heights[hand].1 < HEIGHT_SWAP {
                self.held[hand] = next.clone();
            }
        }
    }

    /// Fills the parts of `r` this keeps.
    pub(crate) fn record(&self, r: &mut FirstPersonRecord) {
        r.swing = self.swing.current;
        r.swing_progress = (self.swing.old, self.swing.now);
        r.hurt_time = self.hurt_time;
        r.hurt_duration = self.hurt_duration;
        r.hurt_dir = self.hurt_dir;
        r.walk = self.walk;
        r.bob = self.bob;
        r.x_bob = self.x_bob;
        r.y_bob = self.y_bob;
        for hand in 0..2 {
            if r.held[hand] != self.held[hand] {
                r.held[hand] = self.held[hand].clone();
            }
        }
        r.heights = self.heights;
    }
}

/// A block someone is breaking (`BlockDestructionProgress`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Breaking {
    /// Who breaks it: an entity's id.
    id: i32,
    pos: [i32; 3],
    /// The stage, 0 to 9.
    progress: u8,
    /// The game time it was last told.
    updated: i64,
}

/// The blocks being broken, by whom (`ClientLevel.destroyingBlocks`).
#[derive(Debug, Clone, Default)]
pub struct Destruction {
    blocks: Vec<Breaking>,
}

/// Game ticks a block stays cracked without news of it.
const FORGOTTEN_AFTER: i64 = 400;

impl Destruction {
    /// `destroyBlockProgress`: `id` has broken the block at `pos` as far as stage
    /// `progress`; a stage that is none (not 0 to 9) ends it. One breaks one block at a
    /// time.
    pub fn set(&mut self, id: i32, pos: [i32; 3], progress: i32, game_time: i64) {
        self.blocks.retain(|b| b.id != id);
        if (0..10).contains(&progress) {
            self.blocks.push(Breaking {
                id,
                pos,
                progress: progress as u8,
                updated: game_time,
            });
        }
    }

    /// `removeBlockBreakingProgress`: once a second, what has had no news for twenty is
    /// dropped.
    pub fn tick(&mut self, game_time: i64) {
        if game_time % 20 == 0 {
            self.blocks
                .retain(|b| game_time - b.updated <= FORGOTTEN_AFTER);
        }
    }

    pub fn clear(&mut self) {
        self.blocks.clear();
    }

    /// The blocks and their stages: of several breaking one block, the furthest (and of
    /// those, the one with the highest id), as the game's sorted set has its last.
    pub fn stages(&self, out: &mut Vec<([i32; 3], u8)>) {
        out.clear();
        for b in &self.blocks {
            match out.iter_mut().find(|(pos, _)| *pos == b.pos) {
                Some((_, stage)) => *stage = (*stage).max(b.progress),
                None => out.push((b.pos, b.progress)),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stack(identity: u64, item: i32, count: i32) -> Stack {
        Stack {
            identity,
            item,
            count,
            ..Stack::empty()
        }
    }

    #[test]
    fn a_hand_sinks_swaps_out_of_sight_and_rises() {
        let mut f = FirstPerson::default();
        let (sword, none) = (stack(1, 5, 1), Stack::empty());
        // Nothing is held and the hand is down: it takes the sword there, and rises 0.4
        // a tick from the next on.
        f.tick_hands([&sword, &none], false, 1.0);
        assert!(f.held[0].same_object(&sword));
        assert_eq!(f.heights[0], (0.0, 0.0));
        f.tick_hands([&sword, &none], false, 1.0);
        assert_eq!(f.heights[0], (0.0, 0.4));
        f.tick_hands([&sword, &none], false, 1.0);
        f.tick_hands([&sword, &none], false, 1.0);
        assert_eq!(f.heights[0], (0.8, 1.0));
        // Another item: the hand sinks with the old one until it is out of sight.
        let pick = stack(2, 6, 1);
        f.tick_hands([&pick, &none], false, 1.0);
        assert!(f.held[0].same_object(&sword));
        assert_eq!(f.heights[0], (1.0, 0.6));
        f.tick_hands([&pick, &none], false, 1.0);
        assert!(f.held[0].same_object(&sword));
        f.tick_hands([&pick, &none], false, 1.0);
        assert!((f.heights[0].1 - 0.0).abs() < 1e-6);
        assert!(f.held[0].same_object(&pick));
        f.tick_hands([&pick, &none], false, 1.0);
        assert!((f.heights[0].1 - 0.4).abs() < 1e-6);
    }

    #[test]
    fn the_same_item_from_the_server_does_not_sink() {
        let mut f = FirstPerson::default();
        let none = Stack::empty();
        let first = stack(1, 5, 3);
        for _ in 0..5 {
            f.tick_hands([&first, &none], false, 1.0);
        }
        // The same again as a new object: taken at once, the hand stays up.
        let again = stack(2, 5, 3);
        f.tick_hands([&again, &none], false, 1.0);
        assert!(f.held[0].same_object(&again));
        assert_eq!(f.heights[0], (1.0, 1.0));
        // One fewer is something else: the hand sinks.
        let fewer = stack(3, 5, 2);
        f.tick_hands([&fewer, &none], false, 1.0);
        assert!(f.held[0].same_object(&again));
        assert_eq!(f.heights[0].1, 0.6);
    }

    #[test]
    fn the_main_hand_rises_with_the_attacks_recovery_and_busy_hands_stay_down() {
        let mut f = FirstPerson::default();
        let (sword, shield) = (stack(1, 5, 1), stack(2, 7, 1));
        for _ in 0..5 {
            f.tick_hands([&sword, &shield], false, 1.0);
        }
        assert_eq!((f.heights[0].1, f.heights[1].1), (1.0, 1.0));
        // Half recovered: an eighth of the way up is where the main hand goes; the off
        // hand does not care.
        f.tick_hands([&sword, &shield], false, 0.5);
        assert_eq!((f.heights[0].1, f.heights[1].1), (0.6, 1.0));
        f.tick_hands([&sword, &shield], false, 0.5);
        f.tick_hands([&sword, &shield], false, 0.5);
        assert!((f.heights[0].1 - 0.125).abs() < 1e-6);
        // Used: it comes up anew.
        f.item_used(true);
        assert_eq!(f.heights[1].1, 0.0);
        // Paddling: both go down and stay there.
        for _ in 0..4 {
            f.tick_hands([&sword, &shield], true, 1.0);
        }
        assert_eq!((f.heights[0].1, f.heights[1].1), (0.0, 0.0));
    }

    #[test]
    fn the_view_bobs_towards_its_target_and_the_hands_trail_the_view() {
        let mut f = FirstPerson::default();
        f.update_bob(0.1);
        assert_eq!(f.bob, (0.0, 0.1 * 0.4));
        f.update_bob(0.1);
        assert!((f.bob.1 - 0.064).abs() < 1e-6 && f.bob.0 == 0.1 * 0.4);
        f.reset_bob();
        assert!((f.bob.0 - 0.064).abs() < 1e-6 && f.bob.1 == 0.0);
        f.follow_view(10.0, -40.0);
        f.follow_view(10.0, -40.0);
        assert_eq!((f.x_bob, f.y_bob), ((5.0, 7.5), (-20.0, -30.0)));
        f.begin_tick();
        f.walked(0.25);
        f.walked(0.5);
        assert_eq!(f.walk, (0.0, 0.75));
        f.begin_tick();
        assert_eq!(f.walk, (0.75, 0.75));
    }

    #[test]
    fn a_flinch_lasts_ten_ticks_and_the_first_health_is_none() {
        let mut f = FirstPerson::default();
        f.health_set(0.0, 20.0);
        f.health_set(20.0, 20.0);
        assert_eq!(f.hurt_time, 0);
        // Healed is no hurt; less health is.
        f.health_set(10.0, 12.0);
        assert_eq!(f.hurt_time, 0);
        f.health_set(12.0, 9.0);
        assert_eq!((f.hurt_time, f.hurt_duration), (10, 10));
        for _ in 0..12 {
            f.tick_hurt();
        }
        assert_eq!(f.hurt_time, 0);
        // The packets: one says where from, the other keeps what was.
        f.hurt(Some(90.0));
        assert_eq!((f.hurt_time, f.hurt_dir), (10, 90.0));
        f.tick_hurt();
        f.hurt(None);
        assert_eq!((f.hurt_time, f.hurt_dir), (10, 90.0));
    }

    #[test]
    fn the_furthest_stage_of_a_block_is_shown_and_old_news_is_dropped() {
        let mut d = Destruction::default();
        let mut out = Vec::new();
        d.set(1, [0, 64, 0], 3, 100);
        d.set(2, [0, 64, 0], 5, 100);
        d.set(3, [1, 64, 0], 0, 100);
        d.stages(&mut out);
        assert_eq!(out, [([0, 64, 0], 5), ([1, 64, 0], 0)]);
        // One breaks one block at a time: the old one is given up.
        d.set(2, [5, 64, 0], 1, 120);
        d.stages(&mut out);
        assert_eq!(out, [([0, 64, 0], 3), ([1, 64, 0], 0), ([5, 64, 0], 1)]);
        // A stage that is none ends it.
        d.set(1, [0, 64, 0], 255, 130);
        d.set(3, [1, 64, 0], 10, 130);
        d.stages(&mut out);
        assert_eq!(out, [([5, 64, 0], 1)]);
        // Forgotten 400 ticks after the last news, looked at once a second.
        d.tick(519);
        d.tick(520);
        d.stages(&mut out);
        assert_eq!(out.len(), 1);
        d.tick(521);
        d.tick(540);
        d.stages(&mut out);
        assert!(out.is_empty());
    }

    #[test]
    fn a_bow_in_use_hides_the_other_hand() {
        use crate::item::{Class, Components};
        let of = |class: Class, charged: bool| Stack {
            identity: 9,
            item: 1,
            count: 1,
            c: Arc::new(Components {
                class,
                charged,
                ..Components::default()
            }),
            ..Stack::empty()
        };
        let (bow, crossbow, loaded) = (
            of(Class::Bow, false),
            of(Class::Crossbow, false),
            of(Class::Crossbow, true),
        );
        let (sword, none) = (of(Class::Plain, false), Stack::empty());
        assert_eq!(hands_drawn(&sword, &none, None), Hands::Both);
        assert_eq!(hands_drawn(&bow, &sword, None), Hands::Both);
        assert_eq!(hands_drawn(&bow, &sword, Some(false)), Hands::MainOnly);
        assert_eq!(hands_drawn(&sword, &bow, Some(true)), Hands::OffOnly);
        assert_eq!(hands_drawn(&crossbow, &sword, Some(false)), Hands::MainOnly);
        // A loaded crossbow in the main hand is held in both.
        assert_eq!(hands_drawn(&loaded, &sword, None), Hands::MainOnly);
        assert_eq!(hands_drawn(&sword, &loaded, None), Hands::Both);
        // Using something else in the main hand over a loaded crossbow in the other.
        assert_eq!(hands_drawn(&sword, &loaded, Some(false)), Hands::MainOnly);
        assert_eq!(hands_drawn(&loaded, &sword, Some(true)), Hands::Both);
    }
}

//! How far the view widens and narrows with the player's speed, flight and items
//! (`AbstractClientPlayer.getFieldOfViewModifier`), eased tick by tick as the camera does
//! (`Camera.tickFov`).
//!
//! The camera ticks before the entities (`GameRenderer.tick` comes before
//! `level.tickEntities` in `Minecraft.tick`), so a tick's target is taken from the player
//! as the last tick and this tick's key binds left it. The FOV Effects option scales the
//! target, not the eased value, and a spyglass in first person ignores it.

use ruthenium_javamath::mth;

use crate::player::LocalPlayer;

/// What the field of view's modifier reads from the player.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FovInputs {
    /// Flying (`getAbilities().flying`).
    pub flying: bool,
    /// The walking speed the server gave (`getAbilities().getWalkingSpeed()`).
    pub walking_speed: f32,
    /// The movement speed attribute's value, sprinting and effects included.
    pub movement_speed: f64,
    /// Ticks spent drawing a bow, while one is drawn.
    pub bow_ticks: Option<i32>,
    /// Looking through a spyglass (`isScoping`).
    pub scoping: bool,
}

impl FovInputs {
    /// The player's, as of now.
    pub fn of(p: &LocalPlayer) -> FovInputs {
        let using = p.is_using_item();
        let item = p.use_item();
        FovInputs {
            flying: p.abilities.flying,
            walking_speed: p.abilities.walking_speed,
            movement_speed: p.attributes.movement_speed.value(),
            // `getTicksUsingItem`: the use's duration less the ticks left.
            bow_ticks: (using && item.c.name == "minecraft:bow")
                .then(|| item.c.use_duration - p.use_remaining()),
            scoping: p.is_scoping(),
        }
    }
}

/// `getFieldOfViewModifier(firstPerson, effectScale)`: the factor the field of view eases
/// towards.
pub fn modifier(i: &FovInputs, first_person: bool, effect_scale: f32) -> f32 {
    let mut modifier = 1.0f32;
    if i.flying {
        modifier *= 1.1;
    }
    if i.walking_speed != 0.0 {
        let speed_factor = i.movement_speed as f32 / i.walking_speed;
        modifier *= (speed_factor + 1.0) / 2.0;
    }
    if let Some(ticks) = i.bow_ticks {
        let scale = (ticks as f32 / 20.0).min(1.0);
        modifier *= 1.0 - scale * scale * 0.15;
    } else if first_person && i.scoping {
        return 0.1;
    }
    mth::lerp(effect_scale, 1.0, modifier)
}

/// The camera's eased modifier (`fovModifier` and `oldFovModifier`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FovEase {
    pub old: f32,
    pub now: f32,
}

impl Default for FovEase {
    /// Vanilla's camera starts at 0 when the game starts and keeps its value from one
    /// world to the next; the level's loading screen hides either. A session starts at
    /// rest instead, so that no frame can be drawn with a field of view of nothing.
    fn default() -> FovEase {
        FovEase { old: 1.0, now: 1.0 }
    }
}

impl FovEase {
    /// The least and the most the eased modifier may be.
    pub const MIN: f32 = 0.1;
    pub const MAX: f32 = 1.5;

    /// One tick: half the way to `target`, clamped.
    pub fn tick(&mut self, target: f32) {
        self.old = self.now;
        let eased = self.now + (target - self.now) * 0.5;
        self.now = eased.clamp(Self::MIN, Self::MAX);
    }

    /// The modifier `partial_tick` of the way through the tick.
    pub fn at(&self, partial_tick: f32) -> f32 {
        mth::lerp(partial_tick, self.old, self.now)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn walking() -> FovInputs {
        FovInputs {
            flying: false,
            walking_speed: 0.1,
            movement_speed: 0.10000000149011612,
            bow_ticks: None,
            scoping: false,
        }
    }

    #[test]
    fn walking_is_one_and_sprinting_widens() {
        assert_eq!(modifier(&walking(), true, 1.0), 1.0);
        // The sprint modifier: +30% of the base, multiplied in.
        let sprint = FovInputs {
            movement_speed: 0.10000000149011612 * 1.3,
            ..walking()
        };
        let m = modifier(&sprint, true, 1.0);
        assert_eq!(
            m,
            (((0.10000000149011612f64 * 1.3) as f32 / 0.1) + 1.0) / 2.0
        );
        assert!((m - 1.15).abs() < 1e-6);
        // Half the option: half the change; none at 0.
        assert_eq!(modifier(&sprint, true, 0.5), 1.0 + 0.5 * (m - 1.0));
        assert_eq!(modifier(&sprint, true, 0.0), 1.0);
    }

    #[test]
    fn flying_and_a_zero_walking_speed() {
        let flying = FovInputs {
            flying: true,
            ..walking()
        };
        assert_eq!(modifier(&flying, true, 1.0), 1.1);
        let still = FovInputs {
            walking_speed: 0.0,
            ..walking()
        };
        assert_eq!(modifier(&still, true, 1.0), 1.0);
    }

    #[test]
    fn a_bow_narrows_over_twenty_ticks() {
        let bow = |t| FovInputs {
            bow_ticks: Some(t),
            ..walking()
        };
        assert_eq!(modifier(&bow(0), true, 1.0), 1.0);
        assert_eq!(modifier(&bow(10), true, 1.0), 1.0 - 0.25 * 0.15);
        assert_eq!(modifier(&bow(20), true, 1.0), 0.85);
        assert_eq!(modifier(&bow(60), true, 1.0), 0.85);
    }

    #[test]
    fn a_spyglass_ignores_the_option_in_first_person_only() {
        let scoping = FovInputs {
            scoping: true,
            ..walking()
        };
        assert_eq!(modifier(&scoping, true, 0.0), 0.1);
        assert_eq!(modifier(&scoping, false, 1.0), 1.0);
    }

    #[test]
    fn the_ease_goes_half_way_and_is_clamped() {
        let mut e = FovEase::default();
        e.tick(1.15);
        assert_eq!((e.old, e.now), (1.0, 1.075));
        assert_eq!(e.at(0.5), 1.0375);
        e.tick(3.0);
        assert_eq!(e.now, 1.5);
        let mut low = FovEase::default();
        low.tick(-1.0);
        assert_eq!(low.now, 0.1);
    }
}

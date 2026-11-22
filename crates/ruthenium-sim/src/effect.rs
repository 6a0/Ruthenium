//! Mob effects as the client keeps them (`LivingEntity.activeEffects` on the client):
//! what the server sent last for each effect, the durations counted down each tick for
//! display (`MobEffectInstance.tickClient`). The client never ends an effect itself; the
//! server removes it, and its attribute modifiers arrive as attribute updates.

use std::collections::BTreeMap;

use ruthenium_data::v26_3::registries::MobEffect;

/// `MobEffectInstance.INFINITE_DURATION`.
pub const INFINITE: i32 = -1;

/// One effect (`MobEffectInstance`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Effect {
    /// The `minecraft:mob_effect` registry id.
    pub id: i32,
    pub amplifier: i32,
    /// Ticks left, or [`INFINITE`].
    pub duration: i32,
    /// `ClientboundUpdateMobEffectPacket`'s flags (`EFFECT_*`).
    pub flags: u8,
}

/// An entity's effects, by registry id.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Effects {
    active: BTreeMap<i32, Effect>,
}

impl Effects {
    /// `forceAddEffect` from `handleUpdateMobEffect`: the effect replaced as sent.
    pub fn update(&mut self, e: Effect) {
        self.active.insert(e.id, e);
    }

    /// `removeEffectNoUpdate`.
    pub fn remove(&mut self, id: i32) {
        self.active.remove(&id);
    }

    pub fn get(&self, effect: MobEffect) -> Option<&Effect> {
        self.active.get(&(effect.id() as i32))
    }

    pub fn has(&self, effect: MobEffect) -> bool {
        self.get(effect).is_some()
    }

    pub fn iter(&self) -> impl Iterator<Item = &Effect> {
        self.active.values()
    }

    pub fn is_empty(&self) -> bool {
        self.active.is_empty()
    }

    /// `tickEffects` on the client: every effect with time left counts down (an
    /// infinite one stays).
    pub fn tick(&mut self) {
        for e in self.active.values_mut() {
            if e.duration > 0 {
                e.duration -= 1;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn durations_count_down_and_stop_at_zero() {
        let mut effects = Effects::default();
        let jump = MobEffect::JumpBoost.id() as i32;
        effects.update(Effect {
            id: jump,
            amplifier: 1,
            duration: 2,
            flags: 0,
        });
        effects.update(Effect {
            id: MobEffect::NightVision.id() as i32,
            amplifier: 0,
            duration: INFINITE,
            flags: 0,
        });
        for _ in 0..3 {
            effects.tick();
        }
        // Expired effects stay until the server removes them.
        assert_eq!(effects.get(MobEffect::JumpBoost).unwrap().duration, 0);
        assert_eq!(
            effects.get(MobEffect::NightVision).unwrap().duration,
            INFINITE
        );
        effects.remove(jump);
        assert!(!effects.has(MobEffect::JumpBoost));
    }
}

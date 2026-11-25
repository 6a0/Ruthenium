//! What the game thread plays (`ClientLevel.playSound` and friends): the server's
//! sounds (`handleSoundEvent`, `handleSoundEntityEvent`, `handleStopSoundEvent`), and the
//! sounds the client plays by itself for the local player.
//!
//! A server's sound picks its variant with `RandomSource.create(seed)` from the packet,
//! so every client hears the same one. The client's own sounds take their seeds from
//! the level's random (`ClientLevel.playLocalSound`), which nothing else here depends
//! on, so any generator does.

use std::borrow::Cow;
use std::path::Path;
use std::sync::Arc;

use ruthenium_audio::{EntityState, SoundHandle, SoundInstance, SoundSource};
use ruthenium_data::v26_3::registries::SoundEvent;
use ruthenium_javamath::random::JavaRandom;
use ruthenium_prepare::exporters::{
    BLOCK_SOUNDS, BlockSoundsData, SoundTypeData, read_block_sounds,
};
use ruthenium_protocol::packets::play::SoundEventRef;

use crate::entity::Entities;
use crate::player::LocalPlayer;

/// Every block state's `SoundType` (from the `block_sounds` export).
#[derive(Debug, Clone, Default)]
pub struct BlockSounds(BlockSoundsData);

impl BlockSounds {
    pub fn load(exports: &Path) -> Result<BlockSounds, String> {
        let bytes = std::fs::read(exports.join(BLOCK_SOUNDS))
            .map_err(|e| format!("{BLOCK_SOUNDS}: {e}"))?;
        read_block_sounds(&bytes).map(BlockSounds)
    }

    pub fn new(data: BlockSoundsData) -> BlockSounds {
        BlockSounds(data)
    }

    /// `BlockState.getSoundType`.
    pub fn of(&self, state: u32) -> Option<&SoundTypeData> {
        let t = *self.0.states.get(state as usize)?;
        self.0.types.get(t as usize)
    }
}

/// The game thread's way to sound; does nothing without an engine.
#[derive(Debug, Clone)]
pub struct GameSounds {
    handle: SoundHandle,
    blocks: Option<Arc<BlockSounds>>,
    /// `ClientLevel.random`, for the seeds of the client's own sounds.
    random: JavaRandom,
}

impl Default for GameSounds {
    fn default() -> GameSounds {
        GameSounds::new(SoundHandle::off(), None)
    }
}

/// A sound event's name, as the engine's events are keyed.
pub fn event_name(event: &SoundEventRef) -> Option<Cow<'static, str>> {
    match event {
        SoundEventRef::Registry(id) => u32::try_from(*id)
            .ok()
            .and_then(|id| SoundEvent::NAMES.get(id as usize))
            .map(|n| Cow::Borrowed(*n)),
        SoundEventRef::Direct { id, .. } => Some(Cow::Owned(if id.contains(':') {
            id.clone()
        } else {
            format!("minecraft:{id}")
        })),
    }
}

/// `ClientboundSoundPacket.getX`: eighths of a block, divided as floats.
pub fn packet_position(p: [i32; 3]) -> [f64; 3] {
    p.map(|v| f64::from(v as f32 / 8.0))
}

impl GameSounds {
    pub fn new(handle: SoundHandle, blocks: Option<Arc<BlockSounds>>) -> GameSounds {
        let seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos() as i64);
        GameSounds {
            handle,
            blocks,
            random: JavaRandom::new(seed),
        }
    }

    pub fn handle(&self) -> &SoundHandle {
        &self.handle
    }

    pub fn blocks(&self) -> Option<&BlockSounds> {
        self.blocks.as_deref()
    }

    /// A seed for a sound the client plays by itself (`level.random.nextLong()`).
    pub fn seed(&mut self) -> i64 {
        (i64::from(self.random.next(32)) << 32).wrapping_add(i64::from(self.random.next(32)))
    }

    /// A float from the level's random (`nextFloat`), for pitches.
    pub fn next_float(&mut self) -> f32 {
        self.random.next(24) as f32 * (1.0 / (1 << 24) as f32)
    }

    /// `handleSoundEvent`: `playSeededSound` at the packet's place.
    pub fn sound(
        &self,
        event: &SoundEventRef,
        source: i32,
        position: [i32; 3],
        volume: f32,
        pitch: f32,
        seed: i64,
    ) {
        let (Some(event), Some(source)) = (event_name(event), SoundSource::from_index(source))
        else {
            return;
        };
        if ruthenium_audio::trace() {
            eprintln!("[sound] {event} at {:?}", packet_position(position));
        }
        self.handle.play(SoundInstance::at(
            event,
            source,
            volume,
            pitch,
            seed,
            packet_position(position),
        ));
    }

    /// `handleSoundEntityEvent`: a sound that follows the entity, if the client has it.
    #[allow(clippy::too_many_arguments)]
    pub fn sound_entity(
        &self,
        event: &SoundEventRef,
        source: i32,
        entity_id: i32,
        volume: f32,
        pitch: f32,
        seed: i64,
        player: &LocalPlayer,
        entities: &Entities,
    ) {
        let (Some(event), Some(source)) = (event_name(event), SoundSource::from_index(source))
        else {
            return;
        };
        let Some(state) = entity_state(entity_id, player, entities) else {
            return;
        };
        if ruthenium_audio::trace() {
            eprintln!(
                "[sound] {event} follows entity {entity_id} at {:?}, volume {volume}, pitch {pitch}",
                state.position
            );
        }
        self.handle.play(SoundInstance::entity(
            event,
            source,
            volume,
            pitch,
            seed,
            entity_id,
            state.position,
        ));
    }

    /// `handleStopSoundEvent`.
    pub fn stop(&self, source: Option<i32>, name: Option<&str>) {
        let source = match source {
            Some(s) => match SoundSource::from_index(s) {
                Some(s) => Some(s),
                None => return,
            },
            None => None,
        };
        let name = name.map(|n| {
            if n.contains(':') {
                Cow::Borrowed(n)
            } else {
                Cow::Owned(format!("minecraft:{n}"))
            }
        });
        self.handle.stop_matching(name.as_deref(), source);
    }

    /// `ClientLevel.playLocalSound(x, y, z, ...)`: a sound only this client hears,
    /// delayed by its distance (40 blocks a second) if asked and over 10 blocks away.
    pub fn local_at(
        &mut self,
        event: &'static str,
        source: SoundSource,
        volume: f32,
        pitch: f32,
        position: [f64; 3],
        distance_delay: bool,
    ) {
        if !self.handle.is_on() {
            return;
        }
        let seed = self.seed();
        let sound = SoundInstance::at(event, source, volume, pitch, seed, position);
        if distance_delay && let Some(camera) = self.handle.listener_position() {
            let d2: f64 = (0..3)
                .map(|i| {
                    let d = f64::from(camera[i]) - position[i];
                    d * d
                })
                .sum();
            if d2 > 100.0 {
                let delay = (d2.sqrt() / 40.0 * 20.0) as i32;
                self.handle.play_delayed(sound, delay);
                return;
            }
        }
        self.handle.play(sound);
    }

    /// `LocalPlayer.playSound`: at the player's feet, as a player's sound.
    pub fn player(&mut self, player: &LocalPlayer, event: &'static str, volume: f32, pitch: f32) {
        self.local_at(
            event,
            SoundSource::Players,
            volume,
            pitch,
            player.position,
            false,
        );
    }

    /// `playLocalSound(BlockPos, ...)`: at the block's centre.
    pub fn at_block(
        &mut self,
        pos: [i32; 3],
        event: &'static str,
        source: SoundSource,
        volume: f32,
        pitch: f32,
    ) {
        let p = pos.map(|v| f64::from(v) + 0.5);
        self.local_at(event, source, volume, pitch, p, false);
    }

    /// `SoundManager.tick` for a client tick: sounds that follow entities move with
    /// them, or stop once they are gone.
    pub fn tick(&self, player: &LocalPlayer, entities: &Entities) {
        if self.handle.is_on() {
            self.handle
                .tick(false, &|id| entity_state(id, player, entities));
        }
    }

    pub fn stop_all(&self) {
        self.handle.stop_all();
    }
}

/// Where an entity is, for a sound that follows it (`level.getEntity`).
fn entity_state(id: i32, player: &LocalPlayer, entities: &Entities) -> Option<EntityState> {
    if id == player.entity_id {
        return Some(EntityState {
            position: player.position,
            silent: false,
        });
    }
    entities
        .get(id)
        .filter(|e| !e.removed)
        .map(|e| EntityState {
            position: e.position,
            silent: false,
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_events_as_the_registry_and_the_wire_do() {
        assert_eq!(
            event_name(&SoundEventRef::Direct {
                id: "custom.thing".into(),
                range: Some(4.0)
            })
            .as_deref(),
            Some("minecraft:custom.thing")
        );
        let id = SoundEvent::NAMES
            .iter()
            .position(|n| *n == "minecraft:entity.player.hurt")
            .unwrap();
        assert_eq!(
            event_name(&SoundEventRef::Registry(id as i32)).as_deref(),
            Some("minecraft:entity.player.hurt")
        );
        assert_eq!(event_name(&SoundEventRef::Registry(-1)), None);
        assert_eq!(event_name(&SoundEventRef::Registry(1 << 20)), None);
    }

    fn varint(out: &mut Vec<u8>, mut v: u32) {
        loop {
            if v < 0x80 {
                out.push(v as u8);
                return;
            }
            out.push((v as u8 & 0x7f) | 0x80);
            v >>= 7;
        }
    }

    fn decode(
        id: ruthenium_data::v26_3::packets::play::Clientbound,
        body: &[u8],
    ) -> play::Clientbound {
        play::Clientbound::decode(id as u32, body).unwrap()
    }

    use ruthenium_protocol::packets::play;

    #[test]
    fn reads_the_sound_packets_as_their_codecs_write_them() {
        use ruthenium_data::v26_3::packets::play::Clientbound as C;
        // `sound`: holder id 7 (+1), source 4 (blocks), x y z, volume, pitch, seed.
        let mut b = Vec::new();
        varint(&mut b, 8);
        varint(&mut b, 4);
        for v in [-12i32, 516, 40] {
            b.extend(v.to_be_bytes());
        }
        b.extend(0.5f32.to_be_bytes());
        b.extend(1.25f32.to_be_bytes());
        b.extend((-5i64).to_be_bytes());
        assert_eq!(
            decode(C::Sound, &b),
            play::Clientbound::Sound {
                event: SoundEventRef::Registry(7),
                source: 4,
                position: [-12, 516, 40],
                volume: 0.5,
                pitch: 1.25,
                seed: -5,
            }
        );
        // `sound_entity` with an event written out and a fixed range.
        let mut b = Vec::new();
        varint(&mut b, 0);
        varint(&mut b, 7);
        b.extend(b"mod:a.b");
        b.push(1);
        b.extend(32.0f32.to_be_bytes());
        varint(&mut b, 7);
        varint(&mut b, 300);
        b.extend(1.0f32.to_be_bytes());
        b.extend(0.5f32.to_be_bytes());
        b.extend(9i64.to_be_bytes());
        assert_eq!(
            decode(C::SoundEntity, &b),
            play::Clientbound::SoundEntity {
                event: SoundEventRef::Direct {
                    id: "mod:a.b".into(),
                    range: Some(32.0)
                },
                source: 7,
                entity_id: 300,
                volume: 1.0,
                pitch: 0.5,
                seed: 9,
            }
        );
        // `stop_sound`: flags 3 (source, then name), 1, 2, 0.
        let mut b = vec![3];
        varint(&mut b, 8);
        varint(&mut b, 3);
        b.extend(b"x.y");
        assert_eq!(
            decode(C::StopSound, &b),
            play::Clientbound::StopSound {
                source: Some(8),
                name: Some("minecraft:x.y".into())
            }
        );
        assert_eq!(
            decode(C::StopSound, &[0]),
            play::Clientbound::StopSound {
                source: None,
                name: None
            }
        );
        assert_eq!(
            decode(C::StopSound, &[1, 2]),
            play::Clientbound::StopSound {
                source: Some(2),
                name: None
            }
        );
    }

    #[test]
    fn packet_positions_are_eighths_divided_as_floats() {
        assert_eq!(packet_position([8, -12, 1]), [1.0, -1.5, 0.125]);
        // A coordinate float can't hold exactly: the float's value, not the double's.
        let big = 268_435_457;
        assert_eq!(packet_position([big, 0, 0])[0], f64::from(big as f32 / 8.0));
        assert_ne!(packet_position([big, 0, 0])[0], f64::from(big) / 8.0);
    }
}

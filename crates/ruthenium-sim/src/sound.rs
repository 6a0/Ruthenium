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

use crate::blocks::Blocks;
use crate::entity::Entities;
use crate::player::LocalPlayer;

/// A block's `SoundType`, its events kept for the whole run (there are a few hundred).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SoundType {
    pub volume: f32,
    pub pitch: f32,
    pub break_sound: &'static str,
    pub step: &'static str,
    pub place: &'static str,
    pub hit: &'static str,
    pub fall: &'static str,
}

/// Every block state's `SoundType` (from the `block_sounds` export).
#[derive(Debug, Clone, Default)]
pub struct BlockSounds {
    types: Vec<SoundType>,
    states: Vec<u32>,
}

impl BlockSounds {
    pub fn load(exports: &Path) -> Result<BlockSounds, String> {
        let bytes = std::fs::read(exports.join(BLOCK_SOUNDS))
            .map_err(|e| format!("{BLOCK_SOUNDS}: {e}"))?;
        read_block_sounds(&bytes).map(BlockSounds::new)
    }

    /// The events' names live as long as the program: loaded once, played often.
    pub fn new(data: BlockSoundsData) -> BlockSounds {
        let keep = |s: String| -> &'static str { Box::leak(s.into_boxed_str()) };
        BlockSounds {
            types: data
                .types
                .into_iter()
                .map(|t: SoundTypeData| SoundType {
                    volume: t.volume,
                    pitch: t.pitch,
                    break_sound: keep(t.break_sound),
                    step: keep(t.step),
                    place: keep(t.place),
                    hit: keep(t.hit),
                    fall: keep(t.fall),
                })
                .collect(),
            states: data.states,
        }
    }

    /// `BlockState.getSoundType`.
    pub fn of(&self, state: u32) -> Option<&SoundType> {
        let t = *self.states.get(state as usize)?;
        self.types.get(t as usize)
    }
}

/// A block sound the local player's own actions make, for [`GameSounds::block`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockSound {
    /// Breaking begins (`addBreakingBlockEffects` with its sound).
    Hit { pos: [i32; 3], state: u32 },
    /// The client breaks the block (`playerWillDestroy`'s level event 2001).
    Break { pos: [i32; 3], state: u32 },
    /// A block item placed it (`BlockItem.place`).
    Place { pos: [i32; 3], state: u32 },
}

/// A sound the local player's movement made (`LocalPlayer.playSound`: at its feet, as
/// a player's sound), for [`GameSounds::player_sound`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PlayerSound {
    /// A block's step sound, at `volume` and `pitch` times its type's.
    Step {
        state: u32,
        volume: f32,
        pitch: f32,
        at: [f64; 3],
    },
    /// `waterSwimSound`.
    Swim { volume: f32, at: [f64; 3] },
    /// `doWaterSplashEffect`.
    Splash {
        volume: f32,
        high_speed: bool,
        at: [f64; 3],
    },
    /// `playAmethystStepSound`: its pitch is drawn from the intensity.
    Chime {
        volume: f32,
        intensity: f32,
        at: [f64; 3],
    },
}

/// A block sound's event, volume and pitch from the block's type: breaking's hit
/// (`ClientLevel.playBreakingSound`), the break (level event 2001) and the place
/// (`BlockItem.place`).
pub fn block_params(sound: BlockSound, t: &SoundType) -> (&'static str, f32, f32) {
    match sound {
        BlockSound::Hit { .. } => (t.hit, (t.volume + 1.0) / 8.0, t.pitch * 0.5),
        BlockSound::Break { .. } => (t.break_sound, (t.volume + 1.0) / 2.0, t.pitch * 0.8),
        BlockSound::Place { .. } => (t.place, (t.volume + 1.0) / 2.0, t.pitch * 0.8),
    }
}

/// How a level event's pitch is drawn from the level's random.
#[derive(Debug, Clone, Copy)]
enum Pitch {
    Fixed(f32),
    /// `base + (nextFloat - nextFloat) * spread`.
    Spread(f32, f32),
    /// `nextFloat * range + base`.
    Range(f32, f32),
    /// The wind charge's `0.4 / (nextFloat * 0.4 + 0.8)`.
    WindCharge,
    /// The end gateway's `(1 + (nextFloat - nextFloat) * 0.2) * 0.7`.
    Gateway,
}

const R1: Pitch = Pitch::Spread(1.0, 0.2);
const R2: Pitch = Pitch::Range(0.9, 0.1);

/// `LevelEventHandler.levelEvent`'s sounds that are only a sound at the block:
/// event, sound, category, volume, pitch.
const LEVEL_EVENTS: &[(i32, &str, SoundSource, f32, Pitch)] = &[
    (
        1000,
        "minecraft:block.dispenser.dispense",
        SoundSource::Blocks,
        1.0,
        Pitch::Fixed(1.0),
    ),
    (
        1001,
        "minecraft:block.dispenser.fail",
        SoundSource::Blocks,
        1.0,
        Pitch::Fixed(1.2),
    ),
    (
        1002,
        "minecraft:block.dispenser.launch",
        SoundSource::Blocks,
        1.0,
        Pitch::Fixed(1.2),
    ),
    (
        1004,
        "minecraft:entity.firework_rocket.shoot",
        SoundSource::Neutral,
        1.0,
        Pitch::Fixed(1.2),
    ),
    (
        1015,
        "minecraft:entity.ghast.warn",
        SoundSource::Hostile,
        10.0,
        R1,
    ),
    (
        1016,
        "minecraft:entity.ghast.shoot",
        SoundSource::Hostile,
        10.0,
        R1,
    ),
    (
        1017,
        "minecraft:entity.ender_dragon.shoot",
        SoundSource::Hostile,
        10.0,
        R1,
    ),
    (
        1018,
        "minecraft:entity.blaze.shoot",
        SoundSource::Hostile,
        2.0,
        R1,
    ),
    (
        1019,
        "minecraft:entity.zombie.attack_wooden_door",
        SoundSource::Hostile,
        2.0,
        R1,
    ),
    (
        1020,
        "minecraft:entity.zombie.attack_iron_door",
        SoundSource::Hostile,
        2.0,
        R1,
    ),
    (
        1021,
        "minecraft:entity.zombie.break_wooden_door",
        SoundSource::Hostile,
        2.0,
        R1,
    ),
    (
        1022,
        "minecraft:entity.wither.break_block",
        SoundSource::Hostile,
        2.0,
        R1,
    ),
    (
        1024,
        "minecraft:entity.wither.shoot",
        SoundSource::Hostile,
        2.0,
        R1,
    ),
    (
        1025,
        "minecraft:entity.bat.takeoff",
        SoundSource::Neutral,
        0.05,
        R1,
    ),
    (
        1026,
        "minecraft:entity.zombie.infect",
        SoundSource::Hostile,
        2.0,
        R1,
    ),
    (
        1027,
        "minecraft:entity.zombie_villager.converted",
        SoundSource::Hostile,
        2.0,
        R1,
    ),
    (
        1029,
        "minecraft:block.anvil.destroy",
        SoundSource::Blocks,
        1.0,
        R2,
    ),
    (
        1030,
        "minecraft:block.anvil.use",
        SoundSource::Blocks,
        1.0,
        R2,
    ),
    (
        1031,
        "minecraft:block.anvil.land",
        SoundSource::Blocks,
        0.3,
        R2,
    ),
    (
        1033,
        "minecraft:block.chorus_flower.grow",
        SoundSource::Blocks,
        1.0,
        Pitch::Fixed(1.0),
    ),
    (
        1034,
        "minecraft:block.chorus_flower.death",
        SoundSource::Blocks,
        1.0,
        Pitch::Fixed(1.0),
    ),
    (
        1035,
        "minecraft:block.brewing_stand.brew",
        SoundSource::Blocks,
        1.0,
        Pitch::Fixed(1.0),
    ),
    (
        1039,
        "minecraft:entity.phantom.bite",
        SoundSource::Hostile,
        0.3,
        R2,
    ),
    (
        1040,
        "minecraft:entity.zombie.converted_to_drowned",
        SoundSource::Hostile,
        2.0,
        R1,
    ),
    (
        1041,
        "minecraft:entity.husk.converted_to_zombie",
        SoundSource::Hostile,
        2.0,
        R1,
    ),
    (
        1042,
        "minecraft:block.grindstone.use",
        SoundSource::Blocks,
        1.0,
        R2,
    ),
    (
        1043,
        "minecraft:item.book.page_turn",
        SoundSource::Blocks,
        1.0,
        R2,
    ),
    (
        1044,
        "minecraft:block.smithing_table.use",
        SoundSource::Blocks,
        1.0,
        R2,
    ),
    (
        1045,
        "minecraft:block.pointed_dripstone.land",
        SoundSource::Blocks,
        2.0,
        R2,
    ),
    (
        1046,
        "minecraft:block.pointed_dripstone.drip_lava_into_cauldron",
        SoundSource::Blocks,
        2.0,
        R2,
    ),
    (
        1047,
        "minecraft:block.pointed_dripstone.drip_water_into_cauldron",
        SoundSource::Blocks,
        2.0,
        R2,
    ),
    (
        1048,
        "minecraft:entity.skeleton.converted_to_stray",
        SoundSource::Hostile,
        2.0,
        R1,
    ),
    (
        1049,
        "minecraft:block.crafter.craft",
        SoundSource::Blocks,
        1.0,
        Pitch::Fixed(1.0),
    ),
    (
        1050,
        "minecraft:block.crafter.fail",
        SoundSource::Blocks,
        1.0,
        Pitch::Fixed(1.0),
    ),
    (
        1051,
        "minecraft:entity.wind_charge.throw",
        SoundSource::Blocks,
        0.5,
        Pitch::WindCharge,
    ),
    (
        1052,
        "minecraft:block.sulfur_spike.land",
        SoundSource::Blocks,
        2.0,
        R2,
    ),
    (
        1053,
        "minecraft:entity.splash_potion.break",
        SoundSource::Neutral,
        1.0,
        R2,
    ),
    (
        1054,
        "minecraft:entity.splash_potion.break",
        SoundSource::Neutral,
        1.0,
        R2,
    ),
    (
        1501,
        "minecraft:block.lava.extinguish",
        SoundSource::Blocks,
        0.5,
        Pitch::Spread(2.6, 0.8),
    ),
    (
        1502,
        "minecraft:block.redstone_torch.burnout",
        SoundSource::Blocks,
        0.5,
        Pitch::Spread(2.6, 0.8),
    ),
    (
        1503,
        "minecraft:block.end_portal_frame.fill",
        SoundSource::Blocks,
        1.0,
        Pitch::Fixed(1.0),
    ),
    (
        1505,
        "minecraft:item.bone_meal.use",
        SoundSource::Blocks,
        1.0,
        Pitch::Fixed(1.0),
    ),
    (
        3000,
        "minecraft:block.end_gateway.spawn",
        SoundSource::Blocks,
        10.0,
        Pitch::Gateway,
    ),
    (
        3001,
        "minecraft:entity.ender_dragon.growl",
        SoundSource::Hostile,
        64.0,
        Pitch::Range(0.8, 0.3),
    ),
];

/// `DamageEffects` of vanilla's damage types other than `hurt`, for the types a server
/// names from the vanilla pack without sending them.
const VANILLA_DAMAGE_EFFECTS: [(&str, &str); 12] = [
    ("minecraft:drown", "drowning"),
    ("minecraft:on_fire", "burning"),
    ("minecraft:sweet_berry_bush", "poking"),
    ("minecraft:sulfur_cube_hot", "burning"),
    ("minecraft:lava", "burning"),
    ("minecraft:freeze", "freezing"),
    ("minecraft:fireball", "burning"),
    ("minecraft:campfire", "burning"),
    ("minecraft:hot_floor", "burning"),
    ("minecraft:in_fire", "burning"),
    ("minecraft:thorns", "thorns"),
    ("minecraft:unattributed_fireball", "burning"),
];

/// A damage type's effects: its data's, else vanilla's for its name, else `hurt`.
pub fn damage_effects<'a>(name: &str, data: Option<&'a str>) -> &'a str {
    if let Some(d) = data {
        return d;
    }
    VANILLA_DAMAGE_EFFECTS
        .iter()
        .find(|(n, _)| *n == name)
        .map_or("hurt", |(_, e)| e)
}

/// `DamageEffects.sound`: what the player hears when hurt so.
pub fn hurt_sound(effects: &str) -> &'static str {
    match effects {
        "drowning" => "minecraft:entity.player.hurt_drown",
        "burning" => "minecraft:entity.player.hurt_on_fire",
        "poking" => "minecraft:entity.player.hurt_sweet_berry_bush",
        "freezing" => "minecraft:entity.player.hurt_freeze",
        _ => "minecraft:entity.player.hurt",
    }
}

/// `ClientboundGameEventPacket` types that play a sound.
pub const GAME_EVENT_ARROW_HIT: u8 = 6;
pub const GAME_EVENT_PUFFER_FISH_STING: u8 = 9;
pub const GAME_EVENT_ELDER_GUARDIAN: u8 = 10;

/// `EntityEvent`s that play a sound here.
pub const ENTITY_EVENT_DEATH: u8 = 3;
pub const ENTITY_EVENT_TOTEM: u8 = 35;

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

    fn pitch(&mut self, p: Pitch) -> f32 {
        match p {
            Pitch::Fixed(v) => v,
            Pitch::Spread(base, spread) => {
                let a = self.next_float();
                base + (a - self.next_float()) * spread
            }
            Pitch::Range(base, range) => self.next_float() * range + base,
            Pitch::WindCharge => 0.4 / (self.next_float() * 0.4 + 0.8),
            Pitch::Gateway => {
                let a = self.next_float();
                (1.0 + (a - self.next_float()) * 0.2) * 0.7
            }
        }
    }

    /// A block sound of the player's own actions (see [`BlockSound`]); nothing for air.
    pub fn block(&mut self, sound: BlockSound, blocks: &Blocks) {
        let (BlockSound::Hit { pos, state }
        | BlockSound::Break { pos, state }
        | BlockSound::Place { pos, state }) = sound;
        if blocks.is_air(state) && !matches!(sound, BlockSound::Place { .. }) {
            return;
        }
        let Some(t) = self.blocks.as_ref().and_then(|b| b.of(state)).copied() else {
            return;
        };
        let (event, volume, pitch) = block_params(sound, &t);
        self.at_block(pos, event, SoundSource::Blocks, volume, pitch);
    }

    /// `LevelEventHandler.levelEvent`'s sounds: a block broken (2001, by its state),
    /// a block being broken (2020: the hit sound of what is there), and the events
    /// that are a sound at the block. `state_at` reads the level.
    pub fn level_event(
        &mut self,
        event: i32,
        pos: [i32; 3],
        data: i32,
        blocks: Option<&Blocks>,
        state_at: impl Fn([i32; 3]) -> u32,
    ) {
        if !self.handle.is_on() {
            return;
        }
        match event {
            2001 => {
                if let (Some(b), Ok(state)) = (blocks, u32::try_from(data)) {
                    self.block(BlockSound::Break { pos, state }, b);
                }
            }
            2020 => {
                if let Some(b) = blocks {
                    self.block(
                        BlockSound::Hit {
                            pos,
                            state: state_at(pos),
                        },
                        b,
                    );
                }
            }
            1009 => {
                let (event, volume, pitch) = match data {
                    0 => (
                        "minecraft:block.fire.extinguish",
                        0.5,
                        Pitch::Spread(2.6, 0.8),
                    ),
                    1 => (
                        "minecraft:entity.generic.extinguish_fire",
                        0.7,
                        Pitch::Spread(1.6, 0.4),
                    ),
                    _ => return,
                };
                let pitch = self.pitch(pitch);
                self.at_block(pos, event, SoundSource::Blocks, volume, pitch);
            }
            1032 => {
                // `SimpleSoundInstance.forLocalAmbience`: at the listener.
                let pitch = self.next_float() * 0.4 + 0.8;
                let seed = self.seed();
                let mut s = SoundInstance::ui("minecraft:block.portal.travel", pitch, 0.25, seed);
                s.source = SoundSource::Ambient;
                self.handle.play(s);
            }
            _ => {
                if let Some(&(_, sound, source, volume, pitch)) =
                    LEVEL_EVENTS.iter().find(|e| e.0 == event)
                {
                    let pitch = self.pitch(pitch);
                    self.at_block(pos, sound, source, volume, pitch);
                }
            }
        }
    }

    /// A sound of the local player's movement (its pitch drawn here: the player's own
    /// random isn't followed).
    pub fn player_sound(&mut self, sound: PlayerSound) {
        let s = SoundSource::Players;
        match sound {
            PlayerSound::Step {
                state,
                volume,
                pitch,
                at,
            } => {
                if let Some(t) = self.blocks.as_ref().and_then(|b| b.of(state)).copied() {
                    self.local_at(t.step, s, t.volume * volume, t.pitch * pitch, at, false);
                }
            }
            PlayerSound::Swim { volume, at } => {
                let pitch = self.pitch(Pitch::Spread(1.0, 0.4));
                self.local_at("minecraft:entity.player.swim", s, volume, pitch, at, false);
            }
            PlayerSound::Splash {
                volume,
                high_speed,
                at,
            } => {
                let pitch = self.pitch(Pitch::Spread(1.0, 0.4));
                let event = if high_speed {
                    "minecraft:entity.player.splash.high_speed"
                } else {
                    "minecraft:entity.player.splash"
                };
                self.local_at(event, s, volume, pitch, at, false);
            }
            PlayerSound::Chime {
                volume,
                intensity,
                at,
            } => {
                let pitch = 0.5 + intensity * self.next_float() * 1.2;
                self.local_at(
                    "minecraft:block.amethyst_block.chime",
                    s,
                    volume,
                    pitch,
                    at,
                    false,
                );
            }
        }
    }

    /// `handleTakeItemEntity`: an orb's or an item's pickup sound where it was.
    pub fn pickup(&mut self, at: [f64; 3], orb: bool) {
        if orb {
            let pitch = self.pitch(Pitch::Spread(0.9, 0.35));
            self.local_at(
                "minecraft:entity.experience_orb.pickup",
                SoundSource::Players,
                0.1,
                pitch,
                at,
                false,
            );
        } else {
            let pitch = self.pitch(Pitch::Spread(2.0, 1.4));
            self.local_at(
                "minecraft:entity.item.pickup",
                SoundSource::Players,
                0.2,
                pitch,
                at,
                false,
            );
        }
    }

    /// Entity event 35 (`handleEntityEvent`): the totem's sound where the entity is.
    pub fn totem(&mut self, position: [f64; 3], source: SoundSource) {
        self.local_at(
            "minecraft:item.totem.use",
            source,
            1.0,
            1.0,
            position,
            false,
        );
    }

    /// `LivingEntity.handleDamageEvent` for the local player: its hurt sound at
    /// `getSoundVolume` (1) and a pitch about 1.
    pub fn hurt(&mut self, player: &LocalPlayer, effects: &str) {
        let pitch = self.pitch(Pitch::Spread(1.0, 0.2));
        self.player(player, hurt_sound(effects), 1.0, pitch);
    }

    /// Entity event 3 for the local player: its death sound.
    pub fn death(&mut self, player: &LocalPlayer) {
        let pitch = self.pitch(Pitch::Spread(1.0, 0.2));
        self.player(player, "minecraft:entity.player.death", 1.0, pitch);
    }

    /// `handleGameEvent`'s sounds, at the player (`level.playSound(player, ...)`).
    pub fn game_event(&mut self, player: &LocalPlayer, event: u8, param: f32) {
        let p = player.position;
        match event {
            GAME_EVENT_ARROW_HIT => {
                let at = [p[0], player.eye_position()[1], p[2]];
                self.local_at(
                    "minecraft:entity.arrow.hit_player",
                    SoundSource::Players,
                    0.18,
                    0.45,
                    at,
                    false,
                );
            }
            GAME_EVENT_PUFFER_FISH_STING => {
                self.local_at(
                    "minecraft:entity.puffer_fish.sting",
                    SoundSource::Neutral,
                    1.0,
                    1.0,
                    p,
                    false,
                );
            }
            GAME_EVENT_ELDER_GUARDIAN if (param + 0.5).floor() as i32 == 1 => {
                self.local_at(
                    "minecraft:entity.elder_guardian.curse",
                    SoundSource::Hostile,
                    1.0,
                    1.0,
                    p,
                    false,
                );
            }
            _ => {}
        }
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
    fn every_event_named_here_is_in_the_registry() {
        let known = |e: &str| SoundEvent::NAMES.contains(&e);
        for (id, event, ..) in LEVEL_EVENTS {
            assert!(known(event), "{id} {event}");
        }
        for effects in [
            "hurt", "thorns", "drowning", "burning", "poking", "freezing",
        ] {
            assert!(known(hurt_sound(effects)), "{effects}");
        }
        for e in [
            "minecraft:entity.player.swim",
            "minecraft:entity.player.splash",
            "minecraft:entity.player.splash.high_speed",
            "minecraft:block.amethyst_block.chime",
            "minecraft:item.totem.use",
            "minecraft:entity.player.death",
            "minecraft:entity.arrow.hit_player",
            "minecraft:entity.puffer_fish.sting",
            "minecraft:entity.elder_guardian.curse",
            "minecraft:entity.experience_orb.pickup",
            "minecraft:entity.item.pickup",
            "minecraft:block.portal.travel",
            "minecraft:block.fire.extinguish",
            "minecraft:entity.generic.extinguish_fire",
        ] {
            assert!(known(e), "{e}");
        }
        // Each event at most once.
        for (i, (id, ..)) in LEVEL_EVENTS.iter().enumerate() {
            assert!(LEVEL_EVENTS[..i].iter().all(|e| e.0 != *id), "{id}");
        }
    }

    #[test]
    fn hurt_sounds_follow_the_damage_types_effects() {
        assert_eq!(damage_effects("minecraft:drown", None), "drowning");
        assert_eq!(damage_effects("minecraft:player_attack", None), "hurt");
        assert_eq!(
            damage_effects("minecraft:drown", Some("freezing")),
            "freezing"
        );
        assert_eq!(hurt_sound("thorns"), "minecraft:entity.player.hurt");
        assert_eq!(
            hurt_sound("burning"),
            "minecraft:entity.player.hurt_on_fire"
        );
        assert_eq!(hurt_sound("anything else"), "minecraft:entity.player.hurt");
    }

    #[test]
    fn block_sounds_scale_their_types_volume_and_pitch() {
        let stone = SoundType {
            volume: 1.0,
            pitch: 1.0,
            break_sound: "b",
            step: "s",
            place: "p",
            hit: "h",
            fall: "f",
        };
        let wool = SoundType {
            volume: 0.8,
            pitch: 1.5,
            ..stone
        };
        let at = |f: fn([i32; 3], u32) -> BlockSound| f([0; 3], 1);
        let hit = at(|pos, state| BlockSound::Hit { pos, state });
        let broken = at(|pos, state| BlockSound::Break { pos, state });
        let placed = at(|pos, state| BlockSound::Place { pos, state });
        assert_eq!(block_params(hit, &stone), ("h", 0.25, 0.5));
        assert_eq!(block_params(broken, &stone), ("b", 1.0, 0.8));
        assert_eq!(block_params(placed, &stone), ("p", 1.0, 0.8));
        assert_eq!(block_params(hit, &wool), ("h", 1.8 / 8.0, 0.75));
        assert_eq!(block_params(placed, &wool), ("p", 0.9, 1.5 * 0.8));
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

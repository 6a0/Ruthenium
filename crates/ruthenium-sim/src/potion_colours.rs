//! The colour of every potion, by its id in the potion registry: what the game mixes
//! from the colours of the potion's effects, 0xRRGGBB; nothing for a potion without
//! effects, which takes the colour its item's model gives.
//!
//! Written by the game's own classes (`research/tools/PotionGolden.java`).

pub(crate) const POTION_COLOURS: [Option<u32>; 46] = [
    None,           // water
    None,           // mundane
    None,           // thick
    None,           // awkward
    Some(0xC2FF66), // night_vision
    Some(0xC2FF66), // long_night_vision
    Some(0xF6F6F6), // invisibility
    Some(0xF6F6F6), // long_invisibility
    Some(0xFDFF84), // leaping
    Some(0xFDFF84), // long_leaping
    Some(0xFDFF84), // strong_leaping
    Some(0xFF9900), // fire_resistance
    Some(0xFF9900), // long_fire_resistance
    Some(0x33EBFF), // swiftness
    Some(0x33EBFF), // long_swiftness
    Some(0x33EBFF), // strong_swiftness
    Some(0x8BAFE0), // slowness
    Some(0x8BAFE0), // long_slowness
    Some(0x8BAFE0), // strong_slowness
    Some(0x8D82E6), // turtle_master
    Some(0x8D82E6), // long_turtle_master
    Some(0x8D85E6), // strong_turtle_master
    Some(0x98DAC0), // water_breathing
    Some(0x98DAC0), // long_water_breathing
    Some(0xF82423), // healing
    Some(0xF82423), // strong_healing
    Some(0xA9656A), // harming
    Some(0xA9656A), // strong_harming
    Some(0x87A363), // poison
    Some(0x87A363), // long_poison
    Some(0x87A363), // strong_poison
    Some(0xCD5CAB), // regeneration
    Some(0xCD5CAB), // long_regeneration
    Some(0xCD5CAB), // strong_regeneration
    Some(0xFFC700), // strength
    Some(0xFFC700), // long_strength
    Some(0xFFC700), // strong_strength
    Some(0x484D48), // weakness
    Some(0x484D48), // long_weakness
    Some(0x59C106), // luck
    Some(0xF3CFB9), // slow_falling
    Some(0xF3CFB9), // long_slow_falling
    Some(0xBDC9FF), // wind_charged
    Some(0x78695A), // weaving
    Some(0x99FFA3), // oozing
    Some(0x8C9B8C), // infested
];

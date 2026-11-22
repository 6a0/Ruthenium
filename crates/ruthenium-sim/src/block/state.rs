//! Reading and changing block states by property (`getValue`, `setValue`,
//! `cycle`), on the palette ids the rest of the client uses.

use ruthenium_data::v26_3::Block;
use ruthenium_data::v26_3::blocks::BlockState;

use crate::direction::Direction;

/// The air block's state.
pub const AIR: u32 = 0;

/// The block a state is of.
pub fn block(state: u32) -> Option<Block> {
    BlockState::from_id(state).map(BlockState::block)
}

/// The block's registry id (`u32::MAX` for an unknown state).
pub fn block_id(state: u32) -> u32 {
    block(state).map_or(u32::MAX, |b| b.id())
}

/// The block's name (`minecraft:stone`).
pub fn name(state: u32) -> &'static str {
    block(state).map_or("minecraft:air", |b| b.name())
}

/// `BlockState.is(block)` by name.
pub fn is(state: u32, block_name: &str) -> bool {
    name(state) == block_name
}

/// Two states of the same block.
pub fn same_block(a: u32, b: u32) -> bool {
    block(a).is_some() && block(a) == block(b)
}

/// A block's default state, by registry id (air if there is no such block).
pub fn default_of_block(block: u32) -> u32 {
    Block::from_id(block).map_or(AIR, |b| b.default_state().id())
}

/// A block's default state, by name (air if there is no such block).
pub fn default_of(name: &str) -> u32 {
    Block::from_name(name).map_or(AIR, |b| b.default_state().id())
}

/// The block's registry id, by name.
pub fn block_named(name: &str) -> Option<u32> {
    Block::from_name(name).map(|b| b.id())
}

/// The state's block has the property (`hasProperty`).
pub fn has(state: u32, property: &str) -> bool {
    get(state, property).is_some()
}

/// A property's value as text (`None` if the block hasn't the property).
pub fn get(state: u32, property: &str) -> Option<&'static str> {
    BlockState::from_id(state).and_then(|s| s.value(property))
}

/// `setValue`: the state with one property changed. The game throws for a property
/// the block doesn't have; here the state comes back unchanged (and debug builds
/// fail).
pub fn set(state: u32, property: &str, value: &str) -> u32 {
    let changed = BlockState::from_id(state)
        .and_then(|s| s.with(property, value))
        .map(BlockState::id);
    debug_assert!(
        changed.is_some(),
        "{} has no {property}={value}",
        BlockState::from_id(state).map_or_else(|| state.to_string(), |s| s.to_string())
    );
    changed.unwrap_or(state)
}

/// `trySetValue`: like [`set`], but a block without the property stays as it is.
pub fn try_set(state: u32, property: &str, value: &str) -> u32 {
    BlockState::from_id(state)
        .and_then(|s| s.with(property, value))
        .map_or(state, BlockState::id)
}

/// A boolean property (false when absent).
pub fn get_bool(state: u32, property: &str) -> bool {
    get(state, property) == Some("true")
}

pub fn set_bool(state: u32, property: &str, value: bool) -> u32 {
    set(state, property, if value { "true" } else { "false" })
}

/// An integer property (0 when absent).
pub fn get_int(state: u32, property: &str) -> i32 {
    get(state, property)
        .and_then(|v| v.parse().ok())
        .unwrap_or(0)
}

pub fn set_int(state: u32, property: &str, value: i32) -> u32 {
    set(state, property, &value.to_string())
}

/// A direction property (`facing`, `horizontal_facing`...).
pub fn get_dir(state: u32, property: &str) -> Option<Direction> {
    get(state, property).and_then(Direction::from_name)
}

pub fn set_dir(state: u32, property: &str, value: Direction) -> u32 {
    set(state, property, value.name())
}

/// `cycle`: the property's next value in the order the game lists them, wrapping
/// round.
pub fn cycle(state: u32, property: &str) -> u32 {
    let Some(s) = BlockState::from_id(state) else {
        return state;
    };
    let info = s.block().info();
    let Some((_, p)) = info.property(property) else {
        debug_assert!(false, "{s} has no {property}");
        return state;
    };
    let current = s.value(property).and_then(|v| p.index_of(v)).unwrap_or(0);
    let next = p.values[(current + 1) % p.values.len()];
    set(state, property, next)
}

/// The values a property may take, in the game's order (empty if the block hasn't
/// it).
pub fn values(state: u32, property: &str) -> &'static [&'static str] {
    BlockState::from_id(state)
        .and_then(|s| s.block().info().property(property))
        .map_or(&[], |(_, p)| p.values)
}

/// Every property the state has, with its value.
pub fn properties(state: u32) -> Vec<(&'static str, &'static str)> {
    BlockState::from_id(state).map_or_else(Vec::new, |s| s.values().collect())
}

/// `withPropertiesOf`: `target` (any state of a block) with every property it
/// shares with `from` copied over.
pub fn with_properties_of(target: u32, from: u32) -> u32 {
    properties(from)
        .into_iter()
        .fold(target, |s, (k, v)| try_set(s, k, v))
}

/// A state from its printed form (`minecraft:oak_stairs[facing=east,half=top]`);
/// properties left out keep the block's defaults.
pub fn parse(text: &str) -> Option<u32> {
    let (name, props) = match text.split_once('[') {
        Some((n, rest)) => (n, rest.strip_suffix(']')?),
        None => (text, ""),
    };
    let mut s = Block::from_name(name)?.default_state();
    for kv in props.split(',').filter(|kv| !kv.is_empty()) {
        let (k, v) = kv.split_once('=')?;
        s = s.with(k, v)?;
    }
    Some(s.id())
}

/// The state as the game prints it (`minecraft:oak_stairs[facing=north,...]`).
pub fn describe(state: u32) -> String {
    BlockState::from_id(state).map_or_else(|| format!("#{state}"), |s| s.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn properties_change_by_name() {
        let door = default_of("minecraft:oak_door");
        assert!(is(door, "minecraft:oak_door") && has(door, "open"));
        assert!(!get_bool(door, "open"));
        let open = cycle(door, "open");
        assert!(get_bool(open, "open"));
        assert_eq!(cycle(open, "open"), door);
        let east = set_dir(door, "facing", Direction::East);
        assert_eq!(get_dir(east, "facing"), Some(Direction::East));
        assert_eq!(try_set(door, "waterlogged", "true"), door);
        let stairs = default_of("minecraft:oak_stairs");
        assert_eq!(get(stairs, "half"), Some("bottom"));
        let snow = default_of("minecraft:snow");
        assert_eq!(get_int(set_int(snow, "layers", 3), "layers"), 3);
        assert_eq!(values(snow, "layers").len(), 8);
        let lever = set_dir(default_of("minecraft:lever"), "facing", Direction::West);
        let gate = with_properties_of(default_of("minecraft:oak_fence_gate"), lever);
        assert_eq!(get_dir(gate, "facing"), Some(Direction::West));
    }
}

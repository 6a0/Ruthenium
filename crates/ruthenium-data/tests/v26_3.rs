//! Checks of the generated 26.3 tables against facts about the game, and of
//! the block-state API over every state.

use ruthenium_data::registry::Registry;
use ruthenium_data::v26_3::entity_data::EntityDataSerializer;
use ruthenium_data::v26_3::packets::{configuration, handshake, login, play, status};
use ruthenium_data::v26_3::registries::{self, EntityType, Item, REGISTRIES};
use ruthenium_data::v26_3::{Block, BlockState, blocks, version};

/// `ALL`, `NAMES`, `id`, `from_id` and `from_name` agree for an id enum.
macro_rules! check_enum {
    ($t:ty) => {{
        for (i, v) in <$t>::ALL.iter().enumerate() {
            assert_eq!(v.id() as usize, i);
            assert_eq!(<$t>::from_id(i as u32), Some(*v));
            assert_eq!(<$t>::from_name(v.name()), Some(*v));
            assert_eq!(v.name(), <$t>::NAMES[i]);
        }
        assert_eq!(<$t>::from_id(<$t>::ALL.len() as u32), None);
        assert_eq!(<$t>::from_name("minecraft:not_a_thing"), None);
    }};
}

#[test]
fn version_facts() {
    assert_eq!(version::ID, "26.3");
    assert_eq!(version::PROTOCOL_VERSION, 777);
    assert_eq!(version::WORLD_VERSION, 5023);
}

#[test]
fn packets() {
    check_enum!(handshake::Serverbound);
    check_enum!(status::Clientbound);
    check_enum!(status::Serverbound);
    check_enum!(login::Clientbound);
    check_enum!(login::Serverbound);
    check_enum!(configuration::Clientbound);
    check_enum!(configuration::Serverbound);
    check_enum!(play::Clientbound);
    check_enum!(play::Serverbound);

    let counts = [
        handshake::Serverbound::ALL.len(),
        status::Clientbound::ALL.len(),
        status::Serverbound::ALL.len(),
        login::Clientbound::ALL.len(),
        login::Serverbound::ALL.len(),
        configuration::Clientbound::ALL.len(),
        configuration::Serverbound::ALL.len(),
        play::Clientbound::ALL.len(),
        play::Serverbound::ALL.len(),
    ];
    assert_eq!(counts, [1, 2, 2, 6, 5, 21, 10, 144, 69]);
    assert_eq!(handshake::Serverbound::Intention.id(), 0);
    assert_eq!(
        play::Serverbound::ClientTickEnd.name(),
        "minecraft:client_tick_end"
    );
    assert!(play::Serverbound::from_name("minecraft:attack").is_some());
    assert!(play::Serverbound::from_name("minecraft:swing").is_none());
}

#[test]
fn registries() {
    assert_eq!(REGISTRIES.len(), 95);
    for (i, r) in REGISTRIES.iter().enumerate() {
        assert_eq!(r.id as usize, i);
        assert_eq!(registries::registry(r.name).map(|r| r.id), Some(r.id));
        if let Some(d) = r.default {
            assert!(r.id_of(d).is_some(), "{}: default {d}", r.name);
        }
    }
    let item: &Registry = registries::registry("minecraft:item").unwrap();
    assert_eq!(item.entries.len(), 1658);
    assert_eq!(
        item.name_of(Item::DiamondSword.id()),
        Some("minecraft:diamond_sword")
    );
    assert_eq!(
        registries::registry("minecraft:entity_type")
            .unwrap()
            .entries
            .len(),
        161
    );

    check_enum!(Block);
    check_enum!(Item);
    check_enum!(EntityType);
    assert_eq!(Block::DEFAULT, Block::Air);
    assert_eq!(Block::Air.id(), 0);
    assert_eq!(Block::from_name("minecraft:stone"), Some(Block::Stone));
    assert_eq!(EntityType::Player.name(), "minecraft:player");
}

#[test]
fn entity_data_serializers() {
    check_enum!(EntityDataSerializer);
    assert_eq!(EntityDataSerializer::ALL.len(), 44);
    assert_eq!(EntityDataSerializer::Byte.id(), 0);
    assert_eq!(EntityDataSerializer::Int.id(), 1);
    assert_eq!(EntityDataSerializer::Pose.id(), 20);
    // Registered after pig, though declared before cow.
    assert_eq!(EntityDataSerializer::ChickenVariant.id(), 30);
    assert_eq!(EntityDataSerializer::DyeColor.id(), 43);
}

#[test]
fn block_state_facts() {
    assert_eq!(blocks::STATE_COUNT, 35723);
    assert_eq!(BlockState::from_id(35723), None);
    assert_eq!(Block::Air.default_state().id(), 0);
    assert_eq!(Block::Stone.default_state().id(), 1);

    let stairs = Block::OakStairs.default_state();
    assert_eq!(
        stairs.to_string(),
        "minecraft:oak_stairs[facing=north,half=bottom,shape=straight,waterlogged=false]"
    );
    assert_eq!(Block::Stone.default_state().to_string(), "minecraft:stone");

    // Declared (type, facing, short) but numbered by name: type varies fastest.
    let head = BlockState::from_id(Block::PistonHead.info().first_state).unwrap();
    assert_eq!(head.id(), 2385);
    assert_eq!(
        head.to_string(),
        "minecraft:piston_head[facing=north,short=true,type=normal]"
    );
    let next = BlockState::from_id(2386).unwrap();
    assert_eq!(next.value("type"), Some("sticky"));
    assert_eq!(head.with("type", "sticky"), Some(next));
    assert_eq!(head.with("type", "nope"), None);
    assert_eq!(head.with("nope", "sticky"), None);
}

#[test]
fn every_block_state() {
    let mut next = 0;
    for &block in Block::ALL {
        let info = block.info();
        assert_eq!(info.first_state, next, "{block}: states are not contiguous");
        next += info.state_count;
        assert!(info.states().contains(&info.default_state));
        assert_eq!(block.states().filter(|s| s.is_default()).count(), 1);

        for state in block.states() {
            assert_eq!(state.block(), block);
            let values: Vec<_> = state.values().collect();
            assert_eq!(values.len(), info.properties.len());
            for (property, value) in &values {
                assert_eq!(state.value(property), Some(*value));
                assert_eq!(state.with(property, value), Some(state));
            }
            // Changing one property keeps the block and every other value.
            for p in info.properties {
                for v in p.values {
                    let other = state.with(p.name, v).unwrap();
                    assert_eq!(other.block(), block);
                    assert_eq!(other.value(p.name), Some(*v));
                    for (q, w) in &values {
                        if *q != p.name {
                            assert_eq!(other.value(q), Some(*w));
                        }
                    }
                }
            }
        }
    }
    assert_eq!(next as usize, blocks::STATE_COUNT);
}

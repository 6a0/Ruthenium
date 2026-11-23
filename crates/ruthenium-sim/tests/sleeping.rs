//! Where a sleeper lies: needs the prepared game (the bed's shape and class).

use ruthenium_data::v26_3::Block;
use ruthenium_protocol::packets::play::DataValue;
use ruthenium_sim::blocks::{BlockTags, Blocks};
use ruthenium_sim::entity::EntityTypes;
use ruthenium_sim::player::{Env, LocalPlayer};
use ruthenium_sim::world::Level;

#[test]
fn a_sleeper_lies_an_eighth_above_the_bed() {
    let Some(game) = std::env::var_os("RUTHENIUM_GAME_DIR") else {
        eprintln!("RUTHENIUM_GAME_DIR not set; skipped");
        return;
    };
    let exports = std::path::PathBuf::from(game).join("versions/26.3/exports");
    let blocks = Blocks::load(&exports).unwrap();
    let types = EntityTypes::load(&exports).unwrap();
    let tags = BlockTags::new();
    let mut level = Level::new(-64, 384);
    level.add_empty_chunk(0, 0);
    let bed = Block::from_name("minecraft:red_bed")
        .unwrap()
        .default_state()
        .with("part", "head")
        .unwrap();
    level.set_block(3, 70, 5, bed.id());
    level.set_block(
        8,
        70,
        5,
        Block::from_name("minecraft:stone")
            .unwrap()
            .default_state()
            .id(),
    );
    let env = Env {
        level: &level,
        blocks: &blocks,
        tags: &tags,
        colliders: &[],
        others: None,
    };
    let field = types
        .player_fields()
        .sleeping_pos
        .expect("the export has SLEEPING_POS_ID");
    let mut p = LocalPlayer::new();
    p.position = [1.0, 2.0, 3.0];
    // A bed: the middle of its block, an eighth above its 9/16 top.
    assert!(p.set_data(
        &[(field, DataValue::OptionalBlockPos(Some([3, 70, 5])))],
        &types
    ));
    p.move_to_bed(&env);
    assert_eq!(p.position, [3.5, 70.0 + 0.5625 + 0.125, 5.5]);
    assert!(p.is_sleeping());
    // No bed there: the player stays where it is.
    assert!(p.set_data(
        &[(field, DataValue::OptionalBlockPos(Some([8, 70, 5])))],
        &types
    ));
    p.move_to_bed(&env);
    assert_eq!(p.position, [3.5, 70.6875, 5.5]);
    // Out of bed: nothing moves.
    assert!(!p.set_data(&[(field, DataValue::OptionalBlockPos(None))], &types));
    assert!(!p.is_sleeping());
}

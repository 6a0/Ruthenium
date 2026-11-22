//! The prepared game's entity types, as the exporter read them from the jar: the
//! facts entity tracking relies on. Needs `RUTHENIUM_GAME_DIR` (see `golden.rs`);
//! skipped without it.

use std::path::PathBuf;

use ruthenium_prepare::exporters::read_entity_types;
use ruthenium_sim::entity::EntityTypes;

#[test]
fn exported_entity_types() {
    let Some(game) = std::env::var_os("RUTHENIUM_GAME_DIR").map(PathBuf::from) else {
        eprintln!("RUTHENIUM_GAME_DIR not set; skipped");
        return;
    };
    let exports = game.join("versions/26.3/exports");
    let bytes = std::fs::read(exports.join("entity_dimensions.bin")).unwrap();
    let data = read_entity_types(&bytes).unwrap();
    let by = |key: &str| data.iter().find(|d| d.key == key).unwrap();

    let zombie = by("minecraft:zombie");
    assert!(zombie.is("LivingEntity") && zombie.is("Zombie") && zombie.is("Mob"));
    assert_eq!(zombie.update_interval, 3);
    assert_eq!(zombie.field("Entity.DATA_POSE"), Some(6));
    assert_eq!(zombie.field("LivingEntity.DATA_HEALTH_ID"), Some(9));
    // Zombie's own baby flag, not AgeableMob's.
    assert_eq!(
        zombie.nearest_field("DATA_BABY_ID"),
        zombie.field("Zombie.DATA_BABY_ID")
    );
    assert_eq!(zombie.baby, Some((0.49, 0.98, false)));

    let player = by("minecraft:player");
    assert!(player.is("Avatar") && player.is("Player"));
    assert_eq!(player.update_interval, 2);

    let slime = by("minecraft:slime");
    assert!(slime.field("AbstractCubeMob.ID_SIZE").is_some());

    let cow = by("minecraft:cow");
    assert!(cow.field("AgeableMob.DATA_BABY_ID").is_some() && cow.baby.is_some());

    assert!(by("minecraft:oak_boat").is("AbstractBoat"));
    assert!(!by("minecraft:oak_boat").is("LivingEntity"));
    assert!(by("minecraft:bat").is("Bat"));
    // Every type's class was found.
    let missing: Vec<&str> = data
        .iter()
        .filter(|d| d.classes.is_empty())
        .map(|d| d.key.as_str())
        .collect();
    assert!(missing.is_empty(), "no class for {missing:?}");

    let types = EntityTypes::load(&exports).unwrap();
    assert_eq!(types.len(), data.len());
}

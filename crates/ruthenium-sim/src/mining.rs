//! How fast the player breaks blocks, and what it may break (`Player.getDestroySpeed`,
//! `BlockBehaviour.getDestroyProgress`, the `tool` component's rules, the adventure-mode
//! `can_break` predicates, `Player.blockActionRestricted`).

use ruthenium_data::v26_3::BlockState;
use ruthenium_data::v26_3::registries::MobEffect;
use ruthenium_protocol::item::{BlockPredicate, HolderSet, PropertyMatcher, ValueMatcher};
use ruthenium_protocol::packets::play;

use crate::blocks::{BlockTags, Blocks, block_of};
use crate::inventory::Hand;
use crate::item::{BlockSet, Stack, Tool};
use crate::player::LocalPlayer;

impl BlockSet {
    /// `BlockState.is(HolderSet)`.
    pub fn contains(&self, state: u32, tags: &BlockTags) -> bool {
        match self {
            BlockSet::Tag(t) => tags.is(state, t),
            BlockSet::Blocks(ids) => block_of(state).is_some_and(|b| ids.contains(&b)),
        }
    }
}

impl Tool {
    /// `getMiningSpeed`: the first rule with a speed that covers the block.
    pub fn mining_speed(&self, state: u32, tags: &BlockTags) -> f32 {
        self.rules
            .iter()
            .find(|r| r.speed.is_some() && r.blocks.contains(state, tags))
            .and_then(|r| r.speed)
            .unwrap_or(self.default_mining_speed)
    }

    /// `isCorrectForDrops`: the first rule that says, for the block.
    pub fn is_correct_for_drops(&self, state: u32, tags: &BlockTags) -> bool {
        self.rules
            .iter()
            .find(|r| r.correct_for_drops.is_some() && r.blocks.contains(state, tags))
            .and_then(|r| r.correct_for_drops)
            .unwrap_or(false)
    }
}

/// `ItemStack.getDestroySpeed`: the tool's speed on the block, 1 without a tool.
pub fn item_destroy_speed(stack: &Stack, state: u32, tags: &BlockTags) -> f32 {
    stack
        .c
        .tool
        .as_ref()
        .map_or(1.0, |t| t.mining_speed(state, tags))
}

/// `ItemStack.canDestroyBlock`: a tool that can't break blocks in creative mode
/// (swords, tridents, maces, spears) doesn't for a player that builds instantly; the
/// debug stick never does on the client (the server cycles the block's properties).
pub fn can_destroy_block(stack: &Stack, instabuild: bool) -> bool {
    if stack.c.name == "minecraft:debug_stick" {
        return false;
    }
    match &stack.c.tool {
        Some(t) if !t.can_destroy_blocks_in_creative => !instabuild,
        _ => true,
    }
}

/// `AdventureModePredicate.test`: any of the stack's `can_break` entries matches the
/// block. Entries with block entity conditions (NBT, components) never match: block
/// entities aren't tracked, and a block without one fails them in vanilla too.
pub fn can_break_in_adventure(stack: &Stack, state: u32, tags: &BlockTags) -> bool {
    stack
        .c
        .can_break
        .as_ref()
        .is_some_and(|p| p.iter().any(|p| predicate_matches(p, state, tags)))
}

/// `canPlaceOnBlockInAdventureMode`: any of the stack's `can_place_on` entries
/// matches the block (block entity conditions never do, as for `can_break`).
pub fn can_place_on_in_adventure(stack: &Stack, state: u32, tags: &BlockTags) -> bool {
    stack
        .c
        .can_place_on
        .as_ref()
        .is_some_and(|p| p.iter().any(|p| predicate_matches(p, state, tags)))
}

/// `BlockPredicate.matches`.
fn predicate_matches(p: &BlockPredicate, state: u32, tags: &BlockTags) -> bool {
    if let Some(blocks) = &p.blocks {
        let set = match blocks {
            HolderSet::Tag(t) => BlockSet::Tag(t.clone()),
            HolderSet::Ids(ids) => {
                BlockSet::Blocks(ids.iter().filter_map(|&i| u32::try_from(i).ok()).collect())
            }
        };
        if !set.contains(state, tags) {
            return false;
        }
    }
    if let Some(props) = &p.properties
        && !props.iter().all(|m| property_matches(m, state))
    {
        return false;
    }
    !p.nbt && !p.components
}

/// `StatePropertiesPredicate.PropertyMatcher.match`: the block has the property and
/// its value is the one named, or within the range (values compare as the property's
/// type does: numbers, then false before true, then in the property's order).
fn property_matches(m: &PropertyMatcher, state: u32) -> bool {
    let Some(info) = BlockState::from_id(state).map(|s| s.block().info()) else {
        return false;
    };
    let Some((i, prop)) = info.property(&m.name) else {
        return false;
    };
    let actual = info.value_index(state, i);
    let rank = |index: usize| -> i64 {
        let v = prop.values[index];
        if let Ok(n) = v.parse::<i64>() {
            n
        } else if v == "true" || v == "false" {
            i64::from(v == "true")
        } else {
            index as i64
        }
    };
    let parse = |v: &str| prop.index_of(v).map(rank);
    let value = rank(actual);
    match &m.value {
        ValueMatcher::Exact(v) => parse(v) == Some(value),
        ValueMatcher::Range { min, max } => {
            if let Some(min) = min {
                match parse(min) {
                    Some(lo) if value >= lo => {}
                    _ => return false,
                }
            }
            if let Some(max) = max {
                match parse(max) {
                    Some(hi) if value <= hi => {}
                    _ => return false,
                }
            }
            true
        }
    }
}

impl LocalPlayer {
    /// `Player.getDestroySpeed`: the held item's speed on the block, with efficiency,
    /// haste or a conduit's power, mining fatigue, the break speed attribute, the
    /// eyes under water, and off the ground.
    pub fn destroy_speed(&self, state: u32, tags: &BlockTags) -> f32 {
        let mut speed = item_destroy_speed(self.hand(Hand::Main), state, tags);
        if speed > 1.0 {
            speed += self.attributes.mining_efficiency.value() as f32;
        }
        let haste = self.effects.get(MobEffect::Haste).map(|e| e.amplifier);
        let conduit = self
            .effects
            .get(MobEffect::ConduitPower)
            .map(|e| e.amplifier);
        if haste.is_some() || conduit.is_some() {
            // `MobEffectUtil.getDigSpeedAmplification`.
            let amplifier = haste.unwrap_or(0).max(conduit.unwrap_or(0));
            speed *= 1.0 + (amplifier + 1) as f32 * 0.2;
        }
        if let Some(fatigue) = self.effects.get(MobEffect::MiningFatigue) {
            speed *= 0.3f64.powf(f64::from(fatigue.amplifier + 1)) as f32;
        }
        speed *= self.attributes.block_break_speed.value() as f32;
        if self.is_eye_in_water() {
            speed *= self.attributes.submerged_mining_speed.value() as f32;
        }
        if !self.on_ground {
            speed /= 5.0;
        }
        speed
    }

    /// `Player.hasCorrectToolForDrops`.
    pub fn has_correct_tool_for_drops(
        &self,
        state: u32,
        blocks: &Blocks,
        tags: &BlockTags,
    ) -> bool {
        !blocks.requires_correct_tool(state)
            || self
                .hand(Hand::Main)
                .c
                .tool
                .as_ref()
                .is_some_and(|t| t.is_correct_for_drops(state, tags))
    }

    /// `BlockBehaviour.getDestroyProgress`: the share of the block one tick of mining
    /// breaks (0 for unbreakable blocks; infinite for blocks that break at once).
    pub fn destroy_progress(&self, state: u32, blocks: &Blocks, tags: &BlockTags) -> f32 {
        let time = blocks.destroy_time(state);
        if time == -1.0 {
            return 0.0;
        }
        let modifier = if self.has_correct_tool_for_drops(state, blocks, tags) {
            30.0
        } else {
            100.0
        };
        self.destroy_speed(state, tags) / time / modifier
    }

    /// `Player.blockActionRestricted`: spectators may not break blocks, nor may
    /// adventure-mode players, except with an item whose `can_break` names the block.
    pub fn block_action_restricted(&self, state: u32, tags: &BlockTags) -> bool {
        let game_type = self.game_type();
        if game_type != play::ADVENTURE && game_type != play::SPECTATOR {
            return false;
        }
        if game_type == play::SPECTATOR {
            return true;
        }
        if self.abilities.may_build {
            return false;
        }
        let held = self.hand(Hand::Main);
        held.is_empty() || !can_break_in_adventure(held, state, tags)
    }

    /// `Player.canUseGameMasterBlocks`: in creative mode, as an operator of level 2
    /// or more.
    pub fn can_use_game_master_blocks(&self) -> bool {
        self.abilities.instabuild && self.permission_level >= 2
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::item::{Components, ToolRule};
    use ruthenium_data::v26_3::Block;
    use std::sync::Arc;

    fn state(name: &str) -> u32 {
        Block::from_name(name).unwrap().default_state().id()
    }

    fn tags() -> BlockTags {
        let mut t = BlockTags::new();
        t.set(
            "minecraft:mineable/pickaxe",
            ["minecraft:stone", "minecraft:iron_ore"],
        );
        t.set(
            "minecraft:incorrect_for_wooden_tool",
            ["minecraft:iron_ore"],
        );
        t
    }

    /// A wooden pickaxe's rules, as its item report has them.
    fn wooden_pickaxe() -> Tool {
        Tool {
            rules: vec![
                ToolRule {
                    blocks: BlockSet::Tag("minecraft:incorrect_for_wooden_tool".into()),
                    speed: None,
                    correct_for_drops: Some(false),
                },
                ToolRule {
                    blocks: BlockSet::Tag("minecraft:mineable/pickaxe".into()),
                    speed: Some(2.0),
                    correct_for_drops: Some(true),
                },
            ],
            default_mining_speed: 1.0,
            can_destroy_blocks_in_creative: true,
        }
    }

    #[test]
    fn rules_apply_in_order_each_for_what_it_says() {
        let (t, tool) = (tags(), wooden_pickaxe());
        let (stone, iron, dirt) = (
            state("minecraft:stone"),
            state("minecraft:iron_ore"),
            state("minecraft:dirt"),
        );
        // Iron ore: the first rule says nothing of speed, so the second gives it; the
        // first says it doesn't drop.
        assert_eq!(tool.mining_speed(iron, &t), 2.0);
        assert!(!tool.is_correct_for_drops(iron, &t));
        assert_eq!(tool.mining_speed(stone, &t), 2.0);
        assert!(tool.is_correct_for_drops(stone, &t));
        assert_eq!(tool.mining_speed(dirt, &t), 1.0);
        assert!(!tool.is_correct_for_drops(dirt, &t));
        let by_id = BlockSet::Blocks(vec![Block::from_name("minecraft:dirt").unwrap().id()]);
        assert!(by_id.contains(dirt, &t) && !by_id.contains(stone, &t));
    }

    fn holding(c: Components) -> Stack {
        Stack {
            c: Arc::new(c),
            count: 1,
            item: 1,
            ..Stack::empty()
        }
    }

    #[test]
    fn creative_tools_and_the_debug_stick() {
        let sword = holding(Components {
            tool: Some(Arc::new(Tool {
                can_destroy_blocks_in_creative: false,
                ..wooden_pickaxe()
            })),
            ..Components::default()
        });
        assert!(can_destroy_block(&sword, false) && !can_destroy_block(&sword, true));
        assert!(can_destroy_block(&Stack::empty(), true));
        let stick = holding(Components {
            name: "minecraft:debug_stick",
            ..Components::default()
        });
        assert!(!can_destroy_block(&stick, false));
    }

    #[test]
    fn adventure_predicates_match_blocks_and_properties() {
        let t = tags();
        let wheat = Block::from_name("minecraft:wheat").unwrap();
        let age = |n: &str| wheat.default_state().with("age", n).unwrap().id();
        let pred = |properties: Option<Vec<PropertyMatcher>>, nbt| BlockPredicate {
            blocks: Some(HolderSet::Ids(vec![wheat.id() as i32])),
            properties,
            nbt,
            components: false,
        };
        let range = |min: Option<&str>, max: Option<&str>| {
            Some(vec![PropertyMatcher {
                name: "age".into(),
                value: ValueMatcher::Range {
                    min: min.map(Into::into),
                    max: max.map(Into::into),
                },
            }])
        };
        let stack = |p: Vec<BlockPredicate>| {
            holding(Components {
                can_break: Some(Arc::new(p)),
                ..Components::default()
            })
        };
        // Grown wheat only (age 3 and up, as numbers: "10" would sort before "3").
        let grown = stack(vec![pred(range(Some("3"), None), false)]);
        assert!(!can_break_in_adventure(&grown, age("2"), &t));
        assert!(can_break_in_adventure(&grown, age("3"), &t));
        assert!(can_break_in_adventure(&grown, age("7"), &t));
        assert!(!can_break_in_adventure(
            &grown,
            state("minecraft:stone"),
            &t
        ));
        let exact = stack(vec![pred(
            Some(vec![PropertyMatcher {
                name: "age".into(),
                value: ValueMatcher::Exact("5".into()),
            }]),
            false,
        )]);
        assert!(can_break_in_adventure(&exact, age("5"), &t));
        assert!(!can_break_in_adventure(&exact, age("6"), &t));
        // An unknown property, a value the property doesn't have, a block entity
        // condition: no.
        let unknown = stack(vec![pred(
            Some(vec![PropertyMatcher {
                name: "facing".into(),
                value: ValueMatcher::Exact("north".into()),
            }]),
            false,
        )]);
        assert!(!can_break_in_adventure(&unknown, age("5"), &t));
        assert!(!can_break_in_adventure(
            &stack(vec![pred(range(Some("9"), None), false)]),
            age("7"),
            &t
        ));
        assert!(!can_break_in_adventure(
            &stack(vec![pred(None, true)]),
            age("1"),
            &t
        ));
        // A tag: the block is in it.
        let by_tag = stack(vec![BlockPredicate {
            blocks: Some(HolderSet::Tag("minecraft:mineable/pickaxe".into())),
            properties: None,
            nbt: false,
            components: false,
        }]);
        assert!(can_break_in_adventure(
            &by_tag,
            state("minecraft:iron_ore"),
            &t
        ));
        assert!(!can_break_in_adventure(
            &holding(Components::default()),
            age("1"),
            &t
        ));
    }
}

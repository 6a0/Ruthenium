//! What blocks themselves do on the client when a player places or uses them: the
//! state a block is placed in (`getStateForPlacement`), whether it may stay where it
//! is (`canSurvive`), how it answers a neighbour changing (`updateShape`), what it
//! may be placed over (`canBeReplaced`), and what a right click on it does
//! (`useItemOn`, `useWithoutItem`), each as vanilla's client runs it.
//!
//! Every block names the classes it is made of, and for each of these behaviours
//! the class whose version it uses ([`Kind`], from the `block_behaviour` export).
//! The functions here look that class up and run its port; a class without one
//! falls back to what `Block` and `BlockBehaviour` do (the default state, always
//! survives, never changes, replaceable by its `replaceable` flag, uses nothing).
//!
//! The ports live in groups, each owning whole classes: `building` (slabs, stairs,
//! doors, fences, walls, scaffolding and other structural blocks), `facing` (blocks
//! that face the player or hang on a face, signs, banners, skulls), `plants`,
//! `attached` (blocks that need support: torches, rails, fire, corals, dripstone and
//! the like), `redstone` and `usable` (blocks with a right click of their own).

pub mod class;
pub mod level;
pub mod state;

mod attached;
mod building;
mod facing;
mod plants;
mod redstone;
mod usable;

#[cfg(test)]
pub(crate) mod testing;

use std::sync::Arc;

use ruthenium_prepare::exporters::{BlockBehaviourData, BlockBehaviours, StateBehaviour};

pub use class::Class;
pub use level::{CollisionMode, Support, flags};
pub(crate) use usable::spectator_outline;

use crate::direction::Direction;
use crate::interact::{Outcome, World};
use crate::inventory::Hand;
use crate::pick::BlockHit;
use crate::place::PlaceContext;
use crate::player::LocalPlayer;

/// A block position.
pub type Pos = [i32; 3];

/// The behaviours each block's classes may declare, in the order [`Kind`] keeps
/// them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Behaviour {
    GetStateForPlacement,
    UpdateShape,
    CanSurvive,
    CanBeReplaced,
    CanBeReplacedByFluid,
    UseItemOn,
    UseWithoutItem,
    UpdateIndirectNeighbourShapes,
    SetPlacedBy,
    Attack,
    MayPlaceOn,
    IsValidBonemealTarget,
}

impl Behaviour {
    pub const ALL: [Behaviour; 12] = [
        Behaviour::GetStateForPlacement,
        Behaviour::UpdateShape,
        Behaviour::CanSurvive,
        Behaviour::CanBeReplaced,
        Behaviour::CanBeReplacedByFluid,
        Behaviour::UseItemOn,
        Behaviour::UseWithoutItem,
        Behaviour::UpdateIndirectNeighbourShapes,
        Behaviour::SetPlacedBy,
        Behaviour::Attack,
        Behaviour::MayPlaceOn,
        Behaviour::IsValidBonemealTarget,
    ];

    /// The name the export lists it under.
    pub fn name(self) -> &'static str {
        match self {
            Behaviour::GetStateForPlacement => "getStateForPlacement",
            Behaviour::UpdateShape => "updateShape",
            Behaviour::CanSurvive => "canSurvive",
            Behaviour::CanBeReplaced => "canBeReplaced",
            Behaviour::CanBeReplacedByFluid => "canBeReplacedByFluid",
            Behaviour::UseItemOn => "useItemOn",
            Behaviour::UseWithoutItem => "useWithoutItem",
            Behaviour::UpdateIndirectNeighbourShapes => "updateIndirectNeighbourShapes",
            Behaviour::SetPlacedBy => "setPlacedBy",
            Behaviour::Attack => "attack",
            Behaviour::MayPlaceOn => "mayPlaceOn",
            Behaviour::IsValidBonemealTarget => "isValidBonemealTarget",
        }
    }
}

/// What the export says of one block: its classes, which class's version of each
/// behaviour it runs, the item it gives and the settings its constructor took.
#[derive(Debug, Clone)]
pub struct Kind {
    /// The block's class and its superclasses, nearest first.
    pub classes: Vec<Class>,
    declared: [Option<Class>; Behaviour::ALL.len()],
    /// `asItem`: the item registry id (0, air, for blocks without one).
    pub item: u32,
    /// Fire's odds for the block: catching (`igniteOdds`) and burning away.
    pub ignite_odds: u8,
    pub burn_odds: u8,
    fields: Vec<(String, String)>,
}

impl Kind {
    /// A plain `Block`, for blocks the export didn't describe.
    fn plain() -> Kind {
        let mut declared = [Some(Class::BlockBehaviour); Behaviour::ALL.len()];
        declared[Behaviour::GetStateForPlacement as usize] = Some(Class::Block);
        declared[Behaviour::SetPlacedBy as usize] = Some(Class::Block);
        declared[Behaviour::IsValidBonemealTarget as usize] = None;
        declared[Behaviour::MayPlaceOn as usize] = None;
        Kind {
            classes: vec![Class::Block, Class::BlockBehaviour],
            declared,
            item: 0,
            ignite_odds: 0,
            burn_odds: 0,
            fields: Vec::new(),
        }
    }

    fn new(data: &BlockBehaviourData, names: &[String]) -> Result<Kind, String> {
        let classes = data
            .classes
            .iter()
            .map(|c| Class::from_name(c).ok_or_else(|| format!("{}: unknown class {c}", data.key)))
            .collect::<Result<Vec<_>, _>>()?;
        let mut declared = [None; Behaviour::ALL.len()];
        for b in Behaviour::ALL {
            let Some(i) = names.iter().position(|n| n == b.name()) else {
                return Err(format!("the export lists no {}", b.name()));
            };
            let index = data.declared[i];
            declared[b as usize] = if index == BlockBehaviourData::NONE {
                None
            } else {
                Some(classes[usize::from(index)])
            };
        }
        Ok(Kind {
            classes,
            declared,
            item: data.item,
            ignite_odds: data.ignite_odds,
            burn_odds: data.burn_odds,
            fields: data.fields.clone(),
        })
    }

    /// `instanceof`: the block's class is or extends `class`.
    pub fn is(&self, class: Class) -> bool {
        self.classes.contains(&class)
    }

    /// The class whose version of a behaviour the block runs (`None`: no class has
    /// one, for `isValidBonemealTarget` on blocks that can't be bone-mealed).
    pub fn declared(&self, b: Behaviour) -> Option<Class> {
        self.declared[b as usize]
    }

    /// A setting the block's constructor took (`Class.field`), as the exporter
    /// wrote it: `DoorBlock.type` is
    /// `BlockSetType[name=oak,canOpenByHand=true,...]`, a block is its key, a state
    /// `state:<id>`.
    pub fn field(&self, name: &str) -> Option<&str> {
        self.fields
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    }

    /// One part of a record-valued field: `record_part("DoorBlock.type",
    /// "canOpenByHand")` is `Some("true")` for oak doors.
    pub fn record_part(&self, name: &str, part: &str) -> Option<&str> {
        let v = self.field(name)?;
        let inner = v.split_once('[')?.1.strip_suffix(']')?;
        inner.split(',').find_map(|kv| {
            let (k, v) = kv.split_once('=')?;
            (k == part).then_some(v)
        })
    }
}

/// Every block's [`Kind`] and every state's flags, supporting faces and signals.
#[derive(Debug, Clone)]
pub struct Kinds {
    kinds: Vec<Kind>,
    states: Vec<StateBehaviour>,
    /// The note block instruments: name, and whether it plays from above.
    instruments: Vec<(String, bool)>,
    plain: Kind,
}

impl Default for Kinds {
    fn default() -> Kinds {
        Kinds {
            kinds: Vec::new(),
            states: Vec::new(),
            instruments: Vec::new(),
            plain: Kind::plain(),
        }
    }
}

impl Kinds {
    pub fn new(data: &BlockBehaviours) -> Result<Kinds, String> {
        let kinds = data
            .blocks
            .iter()
            .map(|b| Kind::new(b, &data.behaviours))
            .collect::<Result<_, _>>()?;
        Ok(Kinds {
            kinds,
            states: data.states.clone(),
            instruments: data.instruments.clone(),
            plain: Kind::plain(),
        })
    }

    /// `BlockState.instrument()`: the note block instrument a state gives (its
    /// `instrument` property's value, `harp` for an unknown state), and whether it
    /// plays from above a note block (`worksAboveNoteBlock`: mob heads).
    pub fn instrument(&self, state: u32) -> (&str, bool) {
        self.instruments
            .get(usize::from(self.state(state).instrument))
            .map_or(("harp", false), |(n, above)| (n.as_str(), *above))
    }

    /// Reads the export from the prepared game's exports folder.
    pub fn load(exports: &std::path::Path) -> Result<Kinds, String> {
        let path = exports.join(ruthenium_prepare::exporters::BLOCK_BEHAVIOUR);
        let bytes = std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        Kinds::new(&ruthenium_prepare::exporters::read_block_behaviour(&bytes)?)
    }

    /// A block's kind, by block registry id.
    pub fn of_block(&self, block: u32) -> &Kind {
        self.kinds.get(block as usize).unwrap_or(&self.plain)
    }

    /// The kind of the block a state belongs to.
    pub fn of(&self, state: u32) -> &Kind {
        match crate::blocks::block_of(state) {
            Some(b) => self.of_block(b),
            None => &self.plain,
        }
    }

    /// A state's flags and faces (all clear for an unknown state).
    pub fn state(&self, state: u32) -> StateBehaviour {
        self.states.get(state as usize).copied().unwrap_or_default()
    }
}

/// Shared, so cloning [`crate::blocks::Blocks`] stays cheap.
pub type SharedKinds = Arc<Kinds>;

/// The class whose version of `b` a state's block runs.
fn declared(w: &World<'_>, state: u32, b: Behaviour) -> Option<Class> {
    w.blocks.kind(state).declared(b)
}

/// Asks each group in turn; the first that ports the class answers.
macro_rules! ask_groups {
    ($f:ident($($arg:expr),*)) => {
        building::$f($($arg),*)
            .or_else(|| facing::$f($($arg),*))
            .or_else(|| plants::$f($($arg),*))
            .or_else(|| attached::$f($($arg),*))
            .or_else(|| redstone::$f($($arg),*))
            .or_else(|| usable::$f($($arg),*))
    };
}

/// `Block.getStateForPlacement`: the state `block` (a block registry id) would be
/// placed in, or `None` if it can't be placed there.
pub fn state_for_placement(w: &World<'_>, ctx: &PlaceContext, block: u32) -> Option<u32> {
    let default = state::default_of_block(block);
    let class = w
        .blocks
        .kinds()
        .of_block(block)
        .declared(Behaviour::GetStateForPlacement);
    match class {
        Some(Class::Block) | None => Some(default),
        Some(c) => ask_groups!(state_for_placement(c, w, ctx, default))
            .unwrap_or_else(|| unported(c, Behaviour::GetStateForPlacement, Some(default))),
    }
}

/// `updateShape`: what `state` at `pos` becomes when its neighbour towards
/// `direction` (at `neighbour_pos`) is now `neighbour`. Air means the block
/// can't stay (which the client leaves to the server).
pub fn update_shape(
    w: &World<'_>,
    state: u32,
    pos: Pos,
    direction: Direction,
    neighbour_pos: Pos,
    neighbour: u32,
) -> u32 {
    let u = Update {
        state,
        pos,
        direction,
        neighbour_pos,
        neighbour,
    };
    update_shape_of(w, &u)
}

/// [`update_shape`] for an [`Update`].
pub fn update_shape_of(w: &World<'_>, u: &Update) -> u32 {
    match declared(w, u.state, Behaviour::UpdateShape) {
        Some(Class::BlockBehaviour) | None => u.state,
        Some(c) => ask_groups!(update_shape(c, w, u))
            .unwrap_or_else(|| unported(c, Behaviour::UpdateShape, u.state)),
    }
}

/// One neighbour's change, as `updateShape` hears of it.
#[derive(Debug, Clone, Copy)]
pub struct Update {
    /// The block being told, and where it is.
    pub state: u32,
    pub pos: Pos,
    /// Where the neighbour is, seen from the block.
    pub direction: Direction,
    pub neighbour_pos: Pos,
    /// The neighbour's new state.
    pub neighbour: u32,
}

impl Update {
    /// The same update for another state of the block (a superclass's version run on
    /// what a subclass made of it).
    pub fn with_state(&self, state: u32) -> Update {
        Update { state, ..*self }
    }
}

/// `canSurvive`: whether `state` may be at `pos` (its support is there).
pub fn can_survive(w: &World<'_>, state: u32, pos: Pos) -> bool {
    match declared(w, state, Behaviour::CanSurvive) {
        Some(Class::BlockBehaviour) | None => true,
        Some(c) => ask_groups!(can_survive(c, w, state, pos))
            .unwrap_or_else(|| unported(c, Behaviour::CanSurvive, true)),
    }
}

/// `canBeReplaced(context)`: whether placing with `ctx` may put a block where
/// `state` is. By default: the state is replaceable and the item isn't the block's
/// own.
pub fn can_be_replaced(w: &World<'_>, state: u32, ctx: &PlaceContext) -> bool {
    match declared(w, state, Behaviour::CanBeReplaced) {
        Some(Class::BlockBehaviour) | None => default_can_be_replaced(w, state, ctx),
        Some(c) => ask_groups!(can_be_replaced(c, w, state, ctx)).unwrap_or_else(|| {
            unported(
                c,
                Behaviour::CanBeReplaced,
                default_can_be_replaced(w, state, ctx),
            )
        }),
    }
}

/// `BlockBehaviour.canBeReplaced(state, context)`.
pub fn default_can_be_replaced(w: &World<'_>, state: u32, ctx: &PlaceContext) -> bool {
    let item = ctx.item();
    w.blocks.replaceable(state) && (item.is_empty() || item_id(item) != w.blocks.kind(state).item)
}

/// The item registry id of a stack's item.
pub fn item_id(stack: &crate::item::Stack) -> u32 {
    ruthenium_data::v26_3::registries::Item::from_name(stack.c.name).map_or(0, |i| i.id())
}

/// `canBeReplaced(fluid)`: whether flowing `fluid` (`water`, else lava) washes the
/// state away (by default: it is replaceable, or not solid).
pub fn can_be_replaced_by_fluid(w: &World<'_>, state: u32, water: bool) -> bool {
    let default = || w.blocks.replaceable(state) || !w.blocks.solid(state);
    match declared(w, state, Behaviour::CanBeReplacedByFluid) {
        Some(Class::BlockBehaviour) | None => default(),
        Some(c) => ask_groups!(can_be_replaced_by_fluid(c, w, state, water))
            .unwrap_or_else(|| unported(c, Behaviour::CanBeReplacedByFluid, default())),
    }
}

/// A right click on the block at `pos` with the item in `hand` (`useItemOn`).
/// [`Outcome::TryEmptyHand`] (the default) passes the click on to
/// [`use_without_item`] when it is the main hand.
pub fn use_item_on(
    w: &mut World<'_>,
    p: &mut LocalPlayer,
    hand: Hand,
    state: u32,
    pos: Pos,
    hit: &BlockHit,
) -> Outcome {
    let u = Use {
        hand,
        state,
        pos,
        hit,
    };
    match declared(w, state, Behaviour::UseItemOn) {
        Some(Class::BlockBehaviour) | None => Outcome::TryEmptyHand,
        Some(c) => ask_groups!(use_item_on(c, w, p, &u))
            .unwrap_or_else(|| unported(c, Behaviour::UseItemOn, Outcome::Success)),
    }
}

/// A right click, as a block hears it.
#[derive(Debug, Clone, Copy)]
pub struct Use<'h> {
    pub hand: Hand,
    pub state: u32,
    pub pos: Pos,
    pub hit: &'h BlockHit,
}

/// A right click on the block with nothing used on it (`useWithoutItem`; the main
/// hand only).
pub fn use_without_item(
    w: &mut World<'_>,
    p: &mut LocalPlayer,
    state: u32,
    pos: Pos,
    hit: &BlockHit,
) -> Outcome {
    let u = Use {
        hand: Hand::Main,
        state,
        pos,
        hit,
    };
    match declared(w, state, Behaviour::UseWithoutItem) {
        Some(Class::BlockBehaviour) | None => Outcome::Pass,
        Some(c) => ask_groups!(use_without_item(c, w, p, &u))
            .unwrap_or_else(|| unported(c, Behaviour::UseWithoutItem, Outcome::Success)),
    }
}

/// `updateIndirectNeighbourShapes`: blocks that also tell blocks diagonal to them
/// (redstone wire).
pub fn update_indirect_neighbour_shapes(
    w: &mut World<'_>,
    state: u32,
    pos: Pos,
    flags: u32,
    limit: i32,
) {
    match declared(w, state, Behaviour::UpdateIndirectNeighbourShapes) {
        Some(Class::BlockBehaviour) | None => {}
        Some(c) => {
            if ask_groups!(update_indirect_neighbour_shapes(
                c, w, state, pos, flags, limit
            ))
            .is_none()
            {
                unported(c, Behaviour::UpdateIndirectNeighbourShapes, ());
            }
        }
    }
}

/// `setPlacedBy`: what a block does once a player has placed it (a door or tall
/// plant puts its top half above it).
pub fn set_placed_by(w: &mut World<'_>, p: &LocalPlayer, state: u32, pos: Pos, hand: Hand) {
    match declared(w, state, Behaviour::SetPlacedBy) {
        Some(Class::Block) | None => {}
        Some(c) => {
            if ask_groups!(set_placed_by(c, w, p, state, pos, hand)).is_none() {
                unported(c, Behaviour::SetPlacedBy, ());
            }
        }
    }
}

/// `attack`: a left click starting to break the block (redstone ore lights up).
pub fn attack(w: &mut World<'_>, p: &LocalPlayer, state: u32, pos: Pos) {
    match declared(w, state, Behaviour::Attack) {
        Some(Class::BlockBehaviour) | None => {}
        Some(c) => {
            if ask_groups!(attack(c, w, p, state, pos)).is_none() {
                unported(c, Behaviour::Attack, ());
            }
        }
    }
}

/// `isValidBonemealTarget`: whether bone meal does something to the block (false
/// for blocks that can't be bone-mealed).
pub fn is_valid_bonemeal_target(w: &World<'_>, state: u32, pos: Pos) -> bool {
    match declared(w, state, Behaviour::IsValidBonemealTarget) {
        None => false,
        Some(c) => ask_groups!(is_valid_bonemeal_target(c, w, state, pos))
            .unwrap_or_else(|| unported(c, Behaviour::IsValidBonemealTarget, false)),
    }
}

/// `mayPlaceOn`: whether the plant `state` may stand on `below` (the block under
/// it, at `below_pos`). Blocks that aren't plants never ask.
pub fn may_place_on(w: &World<'_>, state: u32, below: u32, below_pos: Pos) -> bool {
    match declared(w, state, Behaviour::MayPlaceOn) {
        Some(c) => ask_groups!(may_place_on(c, w, state, below, below_pos))
            .unwrap_or_else(|| unported(c, Behaviour::MayPlaceOn, false)),
        None => false,
    }
}

/// `ScaffoldingBlock.getDistance`.
pub fn scaffolding_distance(w: &World<'_>, pos: Pos) -> i32 {
    building::scaffolding_distance(w, pos)
}

/// `WallHangingSignBlock.canPlace`.
pub fn wall_hanging_sign_can_place(w: &World<'_>, state: u32, pos: Pos) -> bool {
    facing::wall_hanging_sign_can_place(w, state, pos)
}

/// A class whose version of a behaviour isn't ported: the given fallback, and a
/// failing debug assertion so tests find it.
fn unported<T>(class: Class, b: Behaviour, fallback: T) -> T {
    debug_assert!(
        UNPORTED_OK.contains(&(class, b)),
        "{}.{} isn't ported",
        class.name(),
        b.name()
    );
    fallback
}

/// Versions that are deliberately left to the server (their client side does
/// nothing the client keeps).
const UNPORTED_OK: &[(Class, Behaviour)] = &[];

#[cfg(test)]
mod tests {
    use super::*;

    /// Every (class, behaviour) pair a block of the export runs is ported by some
    /// group, or deliberately left alone.
    #[test]
    fn every_declared_behaviour_is_ported() {
        let Some(dir) = std::env::var_os("RUTHENIUM_GAME_DIR") else {
            return;
        };
        let exports = std::path::PathBuf::from(dir).join("versions/26.3/exports");
        let kinds = Kinds::load(&exports).unwrap();
        let ported: Vec<(Class, Behaviour)> = [
            building::PORTED,
            facing::PORTED,
            plants::PORTED,
            attached::PORTED,
            redstone::PORTED,
            usable::PORTED,
            UNPORTED_OK,
        ]
        .concat();
        let mut missing = std::collections::BTreeSet::new();
        for k in &kinds.kinds {
            for b in Behaviour::ALL {
                let Some(c) = k.declared(b) else { continue };
                let default = c == Class::BlockBehaviour
                    || c == Class::Block
                        && matches!(b, Behaviour::GetStateForPlacement | Behaviour::SetPlacedBy);
                if !default && !ported.contains(&(c, b)) {
                    missing.insert(format!("{}.{}", c.name(), b.name()));
                }
            }
        }
        let missing: Vec<_> = missing.into_iter().collect();
        if std::env::var_os("RUTHENIUM_REQUIRE_PORTS").is_some() {
            assert!(missing.is_empty(), "not ported: {missing:?}");
        } else if !missing.is_empty() {
            eprintln!("{} behaviours not ported yet", missing.len());
        }
    }

    #[test]
    fn record_parts_read_block_set_types() {
        let mut k = Kind::plain();
        k.fields.push((
            "DoorBlock.type".into(),
            "BlockSetType[name=iron,canOpenByHand=false,canOpenByWindCharge=false]".into(),
        ));
        assert_eq!(
            k.record_part("DoorBlock.type", "canOpenByHand"),
            Some("false")
        );
        assert_eq!(k.record_part("DoorBlock.type", "name"), Some("iron"));
        assert_eq!(k.record_part("DoorBlock.type", "missing"), None);
        assert!(k.is(Class::Block) && !k.is(Class::DoorBlock));
    }
}

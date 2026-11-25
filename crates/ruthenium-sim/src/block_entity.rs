//! Block entities as the client level keeps them, for the two kinds that move players:
//! moving pistons (`PistonMovingBlockEntity`: a block on its way, with the shape it has
//! part of the way there) and shulker boxes (`ShulkerBoxBlockEntity`: the lid, which
//! changes the box's shape as it opens). Other block entities don't change how anything
//! collides, so they aren't kept.
//!
//! Where they live and tick follows `LevelChunk` and `Level`: setting a block removes
//! the block entity of the block it replaced and makes a new one for a block that has
//! one by itself (shulker boxes; a moving piston gets its block entity from the piston
//! that moves it); each block entity has a ticker in the level's list, in the order they
//! were registered (a block entity replaced at the same position keeps its predecessor's
//! place: `RebindableTickingBlockEntityWrapper`), and tickers added while the list
//! ticks wait for the next tick.

use std::collections::HashMap;
use std::sync::OnceLock;

use crate::direction::Direction;
use crate::shape::{Aabb, Axis, Shape};

pub type Pos = [i32; 3];

/// The kinds of block entity the client keeps.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    MovingPiston,
    ShulkerBox,
}

/// The kind of block entity a state's block has, of those kept.
pub fn kind_of(state: u32) -> Option<Kind> {
    static KINDS: OnceLock<Vec<u8>> = OnceLock::new();
    let kinds = KINDS.get_or_init(|| {
        (0..ruthenium_data::v26_3::blocks::STATE_COUNT as u32)
            .map(|id| {
                let name = ruthenium_data::v26_3::blocks::BlockState::from_id(id)
                    .map_or("", |s| s.block().name());
                if name == "minecraft:moving_piston" {
                    1
                } else if name.ends_with("shulker_box") {
                    2
                } else {
                    0
                }
            })
            .collect()
    });
    match kinds.get(state as usize) {
        Some(1) => Some(Kind::MovingPiston),
        Some(2) => Some(Kind::ShulkerBox),
        _ => None,
    }
}

/// `PistonMovingBlockEntity`.
#[derive(Debug, Clone)]
pub struct MovingPiston {
    /// The state on its way (`movedState`): the moved block, or for the piston itself
    /// its head (extending) or its base (retracting).
    pub moved: u32,
    /// The piston's facing.
    pub direction: Direction,
    pub extending: bool,
    /// The piston's own head or base (`isSourcePiston`).
    pub source: bool,
    pub progress: f32,
    pub progress_o: f32,
    /// The client's ticks past the end, before it puts the block down itself.
    pub death_ticks: i32,
    /// `getCollisionShape`, and what it is for an entity a piston moves this block's
    /// way (`NOCLIP`): only a retracting base's platform.
    pub(crate) shape: Shape,
    pub(crate) noclip_shape: Shape,
}

impl MovingPiston {
    /// A block entity at its start; its shapes are made by [`crate::piston`].
    pub fn new(moved: u32, direction: Direction, extending: bool, source: bool) -> MovingPiston {
        MovingPiston {
            moved,
            direction,
            extending,
            source,
            progress: 0.0,
            progress_o: 0.0,
            death_ticks: 0,
            shape: Shape::empty(),
            noclip_shape: Shape::empty(),
        }
    }

    /// `getMovementDirection`: the way the block goes.
    pub fn movement_direction(&self) -> Direction {
        if self.extending {
            self.direction
        } else {
            self.direction.opposite()
        }
    }

    /// `getExtendedProgress`: where the block is along the piston's facing, relative to
    /// the position it is going to (from -1 to 0 extending, 1 to 0 retracting).
    pub fn extended_progress(&self, progress: f32) -> f32 {
        if self.extending {
            progress - 1.0
        } else {
            1.0 - progress
        }
    }
}

/// `ShulkerBoxBlockEntity.AnimationStatus`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lid {
    Closed,
    Opening,
    Opened,
    Closing,
}

/// `ShulkerBoxBlockEntity`: the lid.
#[derive(Debug, Clone)]
pub struct ShulkerBox {
    pub lid: Lid,
    pub progress: f32,
    pub progress_old: f32,
    /// The players with it open, as the server's block event says.
    pub open_count: i32,
    /// `getShape`: the box with its lid where it is now.
    pub(crate) shape: Shape,
}

impl ShulkerBox {
    pub fn new(facing: Direction) -> ShulkerBox {
        let mut b = ShulkerBox {
            lid: Lid::Closed,
            progress: 0.0,
            progress_old: 0.0,
            open_count: 0,
            shape: Shape::empty(),
        };
        b.refresh(facing);
        b
    }

    /// `getProgress`: the lid between the last tick and this one.
    pub fn progress_at(&self, a: f32) -> f32 {
        self.progress_old + a * (self.progress - self.progress_old)
    }

    /// `getBoundingBox`: the box with its lid raised half a block at most, at the
    /// block's origin.
    pub fn bounding_box(&self, facing: Direction) -> Aabb {
        progress_aabb(1.0, facing, 0.5 * self.progress_at(1.0), [0.5, 0.0, 0.5])
    }

    /// Makes the shape again (after the lid moved or the box turned).
    pub fn refresh(&mut self, facing: Direction) {
        self.shape = Shape::from_boxes(&[self.bounding_box(facing)]);
    }

    pub fn is_closed(&self) -> bool {
        self.lid == Lid::Closed
    }

    /// `triggerEvent`: the server's count of players with the box open.
    pub fn trigger(&mut self, b0: u8, b1: u8) {
        if b0 == 1 {
            self.open_count = i32::from(b1);
            if b1 == 0 {
                self.lid = Lid::Closing;
            }
            if b1 == 1 {
                self.lid = Lid::Opening;
            }
        }
    }
}

/// `Shulker.getProgressAabb`.
pub fn progress_aabb(size: f32, d: Direction, to: f32, position: [f64; 3]) -> Aabb {
    progress_delta_aabb(size, d, -1.0, to, position)
}

/// `Shulker.getProgressDeltaAabb`: the space a lid (or a shulker's shell) of `size`
/// sweeps going from `from` to `to` towards `d`, with a box's base at `position`.
pub fn progress_delta_aabb(
    size: f32,
    d: Direction,
    from: f32,
    to: f32,
    position: [f64; 3],
) -> Aabb {
    let s = f64::from(size);
    let bounds = Aabb::new([-s * 0.5, 0.0, -s * 0.5], [s * 0.5, s, s * 0.5]);
    let max = f64::from(from.max(to));
    let min = f64::from(from.min(to));
    let n = d.normal();
    let grown = bounds.expand_towards(std::array::from_fn(|i| f64::from(n[i]) * max * s));
    let shrunk = grown.contract(std::array::from_fn(|i| f64::from(-n[i]) * (1.0 + min) * s));
    shrunk.moved(position)
}

/// `ChestLidController`: how far a chest's lid is open. Lids move no one: they are kept
/// for drawing, only while they are not shut.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct ChestLid {
    /// Where it is going.
    pub open: bool,
    pub openness: f32,
    pub openness_old: f32,
}

impl ChestLid {
    /// `tickLid`.
    fn tick(&mut self) {
        self.openness_old = self.openness;
        if !self.open && self.openness > 0.0 {
            self.openness = (self.openness - 0.1).max(0.0);
        } else if self.open && self.openness < 1.0 {
            self.openness = (self.openness + 0.1).min(1.0);
        }
    }

    fn is_shut(&self) -> bool {
        !self.open && self.openness == 0.0 && self.openness_old == 0.0
    }
}

/// Whether a state is a chest's, of any kind.
pub fn is_chest(state: u32) -> bool {
    ruthenium_data::v26_3::blocks::BlockState::from_id(state)
        .is_some_and(|s| s.block().name().ends_with("chest"))
}

/// A lid that is not shut, for drawing: a chest's or a shulker box's, from how far it was
/// open at the last tick to how far it is now.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LidRecord {
    pub pos: Pos,
    pub old: f32,
    pub now: f32,
}

/// A block entity's own state.
#[derive(Debug, Clone)]
pub enum Data {
    Piston(MovingPiston),
    Shulker(ShulkerBox),
}

impl Data {
    pub fn kind(&self) -> Kind {
        match self {
            Data::Piston(_) => Kind::MovingPiston,
            Data::Shulker(_) => Kind::ShulkerBox,
        }
    }
}

/// One block entity: an identity (a replaced one is a different object to its ticker)
/// and its data.
#[derive(Debug, Clone)]
struct Entry {
    id: u64,
    data: Data,
}

/// The collision shape a block entity gives its block.
#[derive(Debug, Clone, Copy)]
pub enum BlockEntityShape<'a> {
    /// No block entity there.
    Missing,
    /// None at all.
    Empty,
    Shape(&'a Shape),
}

/// The level's block entities and their tickers.
#[derive(Debug, Clone, Default)]
pub struct BlockEntities {
    entries: HashMap<Pos, Entry>,
    /// The chests whose lids are not shut.
    lids: HashMap<Pos, ChestLid>,
    next_id: u64,
    /// `LevelChunk.tickersInLevel`: the ticker of each position.
    tickers_at: HashMap<Pos, u64>,
    /// Each ticker's block entity (position and identity), none once unbound.
    bound: HashMap<u64, Option<(Pos, u64)>>,
    next_ticker: u64,
    /// `Level.blockEntityTickers` and the ones added while they tick.
    tickers: Vec<u64>,
    pending: Vec<u64>,
    ticking: bool,
}

impl BlockEntities {
    pub fn get(&self, pos: Pos) -> Option<&Data> {
        self.entries.get(&pos).map(|e| &e.data)
    }

    pub fn get_mut(&mut self, pos: Pos) -> Option<&mut Data> {
        self.entries.get_mut(&pos).map(|e| &mut e.data)
    }

    pub fn piston(&self, pos: Pos) -> Option<&MovingPiston> {
        match self.get(pos) {
            Some(Data::Piston(p)) => Some(p),
            _ => None,
        }
    }

    pub fn piston_mut(&mut self, pos: Pos) -> Option<&mut MovingPiston> {
        match self.get_mut(pos) {
            Some(Data::Piston(p)) => Some(p),
            _ => None,
        }
    }

    pub fn shulker(&self, pos: Pos) -> Option<&ShulkerBox> {
        match self.get(pos) {
            Some(Data::Shulker(s)) => Some(s),
            _ => None,
        }
    }

    pub fn shulker_mut(&mut self, pos: Pos) -> Option<&mut ShulkerBox> {
        match self.get_mut(pos) {
            Some(Data::Shulker(s)) => Some(s),
            _ => None,
        }
    }

    /// `ChestBlockEntity.triggerEvent` at a chest: the server's count of players with it
    /// open.
    pub fn chest_event(&mut self, pos: Pos, b0: u8, b1: u8) {
        if b0 != 1 {
            return;
        }
        if b1 > 0 {
            self.lids.entry(pos).or_default().open = true;
        } else if let Some(lid) = self.lids.get_mut(&pos) {
            lid.open = false;
        }
    }

    /// `lidAnimateTick` of every chest.
    pub(crate) fn tick_lids(&mut self) {
        self.lids.retain(|_, lid| {
            lid.tick();
            !lid.is_shut()
        });
    }

    /// Every lid that is not shut, by position.
    pub fn lids(&self, out: &mut Vec<LidRecord>) {
        out.clear();
        out.extend(self.lids.iter().map(|(&pos, lid)| LidRecord {
            pos,
            old: lid.openness_old,
            now: lid.openness,
        }));
        out.extend(self.entries.iter().filter_map(|(&pos, e)| match &e.data {
            Data::Shulker(s) if s.progress > 0.0 || s.progress_old > 0.0 => Some(LidRecord {
                pos,
                old: s.progress_old,
                now: s.progress,
            }),
            _ => None,
        }));
        out.sort_unstable_by_key(|l| l.pos);
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// `addAndRegisterBlockEntity`: puts a block entity at `pos` (one already there is
    /// replaced) and binds the position's ticker to it, or adds a ticker. The caller has
    /// checked that the block there has this kind.
    pub fn add(&mut self, pos: Pos, data: Data) {
        let id = self.next_id;
        self.next_id += 1;
        self.entries.insert(pos, Entry { id, data });
        self.update_ticker(pos, id);
    }

    /// `updateBlockEntityTicker` (both kinds tick).
    fn update_ticker(&mut self, pos: Pos, id: u64) {
        if let Some(&t) = self.tickers_at.get(&pos) {
            self.bound.insert(t, Some((pos, id)));
            return;
        }
        let t = self.next_ticker;
        self.next_ticker += 1;
        self.bound.insert(t, Some((pos, id)));
        self.tickers_at.insert(pos, t);
        if self.ticking {
            self.pending.push(t);
        } else {
            self.tickers.push(t);
        }
    }

    /// `removeBlockEntity`: the block entity goes, and its ticker with it.
    pub fn remove(&mut self, pos: Pos) {
        self.entries.remove(&pos);
        if let Some(t) = self.tickers_at.remove(&pos) {
            self.bound.insert(t, None);
        }
    }

    /// `clearAllBlockEntities` for one chunk (loaded again, or forgotten).
    pub fn clear_chunk(&mut self, cx: i32, cz: i32) {
        let in_chunk = |p: &Pos| p[0] >> 4 == cx && p[2] >> 4 == cz;
        self.lids.retain(|p, _| !in_chunk(p));
        let gone: Vec<Pos> = self
            .entries
            .keys()
            .chain(self.tickers_at.keys())
            .filter(|p| in_chunk(p))
            .copied()
            .collect();
        for p in gone {
            self.remove(p);
        }
    }

    /// `LevelChunk.setBlockState`'s part for block entities, after the block at `pos`
    /// went from `old` to `new`: a different block loses the old one's block entity;
    /// a block that has one gets one if it has none (shulker boxes; a moving piston waits
    /// for its piston) or keeps it with the new state.
    pub fn on_block_changed(&mut self, pos: Pos, old: u32, new: u32) {
        let block_changed = crate::blocks::block_of(old) != crate::blocks::block_of(new);
        if block_changed {
            self.lids.remove(&pos);
        }
        if block_changed && kind_of(old).is_some() {
            self.remove(pos);
        }
        let Some(kind) = kind_of(new) else {
            return;
        };
        match self.entries.get(&pos).map(|e| (e.id, e.data.kind())) {
            Some((id, k)) if k == kind => {
                if let (Some(Data::Shulker(s)), Some(facing)) = (
                    self.get_mut(pos),
                    crate::block::state::get_dir(new, "facing"),
                ) {
                    s.refresh(facing);
                }
                self.update_ticker(pos, id);
            }
            existing => {
                if existing.is_some() {
                    self.remove(pos);
                }
                if kind == Kind::ShulkerBox {
                    let facing =
                        crate::block::state::get_dir(new, "facing").unwrap_or(Direction::Up);
                    self.add(pos, Data::Shulker(ShulkerBox::new(facing)));
                }
            }
        }
    }

    /// The collision shape of the block entity at `pos`, which has `kind`: a moving
    /// piston's for an entity moved by a piston going `noclip`'s way, if any.
    pub fn collision_shape(
        &self,
        pos: Pos,
        kind: Kind,
        noclip: Option<Direction>,
    ) -> BlockEntityShape<'_> {
        let shape = match (kind, self.get(pos)) {
            (Kind::MovingPiston, Some(Data::Piston(p))) => {
                if p.progress < 1.0 && noclip == Some(p.movement_direction()) {
                    &p.noclip_shape
                } else {
                    &p.shape
                }
            }
            (Kind::ShulkerBox, Some(Data::Shulker(s))) => &s.shape,
            _ => return BlockEntityShape::Missing,
        };
        if shape.is_empty() {
            BlockEntityShape::Empty
        } else {
            BlockEntityShape::Shape(shape)
        }
    }

    /// Starts `tickBlockEntities`: the tickers added since the last time join the list.
    pub(crate) fn begin_ticking(&mut self) {
        self.ticking = true;
        let pending = std::mem::take(&mut self.pending);
        self.tickers.extend(pending);
    }

    pub(crate) fn end_ticking(&mut self) {
        self.ticking = false;
    }

    /// The `i`th ticker's block entity position, `Some(None)` for one whose block
    /// entity is gone (taken out of the list), `None` past the end.
    pub(crate) fn ticker(&mut self, i: usize) -> Option<Option<Pos>> {
        let &t = self.tickers.get(i)?;
        let live = self
            .bound
            .get(&t)
            .copied()
            .flatten()
            .filter(|&(pos, id)| self.entries.get(&pos).is_some_and(|e| e.id == id));
        match live {
            Some((pos, _)) => Some(Some(pos)),
            None => {
                self.tickers.remove(i);
                self.bound.remove(&t);
                Some(None)
            }
        }
    }
}

/// The `max` of a shape along an axis (`VoxelShape.max`): its bounds' far side.
pub(crate) fn shape_max(shape: &Shape, axis: Axis) -> f64 {
    shape
        .bounds()
        .map_or(f64::NEG_INFINITY, |b| b.max[axis as usize])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lids_sweep_like_shulkers() {
        // A closed box is the full block; open, the lid stands half a block higher.
        let b = progress_aabb(1.0, Direction::Up, 0.0, [0.5, 0.0, 0.5]);
        assert_eq!((b.min, b.max), ([0.0, 0.0, 0.0], [1.0, 1.0, 1.0]));
        let open = progress_aabb(1.0, Direction::Up, 0.5, [0.5, 0.0, 0.5]);
        assert_eq!(open.max[1], 1.5);
        // What the lid sweeps in a tick: above the box, from the old to the new progress.
        let d = progress_delta_aabb(1.0, Direction::East, 0.1, 0.2, [0.5, 0.0, 0.5]);
        // (The progress is a float: 0.1F widens to 0.10000000149011612.)
        let (old, new) = (f64::from(0.1f32), f64::from(0.2f32));
        assert_eq!(
            (d.min[0], d.max[0]),
            ((-0.5 + (1.0 + old)) + 0.5, (0.5 + new) + 0.5)
        );
        assert_eq!((d.min[1], d.max[1]), (0.0, 1.0));
    }

    #[test]
    fn chest_lids_open_and_shut_over_ten_ticks() {
        let mut b = BlockEntities::default();
        let mut out = Vec::new();
        // Shutting a shut chest, and other events, are nothing.
        b.chest_event([1, 2, 3], 1, 0);
        b.chest_event([1, 2, 3], 0, 1);
        b.lids(&mut out);
        assert!(out.is_empty());
        b.chest_event([1, 2, 3], 1, 2);
        b.tick_lids();
        b.lids(&mut out);
        assert_eq!(
            out,
            [LidRecord {
                pos: [1, 2, 3],
                old: 0.0,
                now: 0.1
            }]
        );
        for _ in 0..12 {
            b.tick_lids();
        }
        b.lids(&mut out);
        assert_eq!((out[0].old, out[0].now), (1.0, 1.0));
        b.chest_event([1, 2, 3], 1, 0);
        b.tick_lids();
        b.lids(&mut out);
        assert_eq!((out[0].old, out[0].now), (1.0, 0.9));
        for _ in 0..12 {
            b.tick_lids();
        }
        b.lids(&mut out);
        assert!(out.is_empty());
        // Another block there, or the chunk going, takes the lid away.
        let chest = (0..).find(|&s| is_chest(s)).expect("there are chests");
        for gone in [true, false] {
            b.chest_event([1, 2, 3], 1, 1);
            b.tick_lids();
            if gone {
                b.on_block_changed([1, 2, 3], chest, 0);
            } else {
                b.clear_chunk(0, 0);
            }
            b.lids(&mut out);
            assert!(out.is_empty());
        }
    }

    #[test]
    fn tickers_keep_their_place() {
        let mut b = BlockEntities::default();
        let shulker =
            |b: &mut BlockEntities, p| b.add(p, Data::Shulker(ShulkerBox::new(Direction::Up)));
        shulker(&mut b, [0, 0, 0]);
        shulker(&mut b, [1, 0, 0]);
        // Replaced in place: the same ticker, still first.
        shulker(&mut b, [0, 0, 0]);
        b.begin_ticking();
        assert_eq!(b.ticker(0), Some(Some([0, 0, 0])));
        // Removed while ticking, then added again: a new ticker, waiting for the next tick.
        b.remove([1, 0, 0]);
        shulker(&mut b, [1, 0, 0]);
        assert_eq!(b.ticker(1), Some(None));
        assert_eq!(b.ticker(1), None);
        b.end_ticking();
        b.begin_ticking();
        assert_eq!(b.ticker(1), Some(Some([1, 0, 0])));
    }
}

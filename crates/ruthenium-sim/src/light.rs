//! Light as the client computes it: the game's light engine, run on the game thread.
//!
//! The server sends light with a chunk and for chunks on the edge of the view. Light
//! that changes inside the view (a torch placed, a roof built) is never sent: the client
//! works it out with the same rules the server uses, and the two agree because the rules
//! and the blocks are the same.
//!
//! Two layers work alike, block light and sky light ([\`LightEngine\`] runs block light
//! first). Each keeps:
//!
//! - **Which sections hold light.** A section holds a layer of 4096 levels while its
//!   chunk section, or one of the 26 around it, has a block that isn't air. Light from
//!   the server waits per section until the section holds light, and is dropped when its
//!   chunk is forgotten.
//! - **Checks.** A block change that matters for light queues its position. Once per
//!   frame the checks turn into *decreases* (light that lost its source is taken back,
//!   outwards, until something brighter is met) and *increases* (light spreads from
//!   emitters and from what the decreases met), each a FIFO queue, decreases first.
//!
//! Sky light also keeps, per column of blocks, the lowest y the sky reaches straight
//! down: everything from there up is a source at level 15.
//!
//! What comes out is the level of every block of every section that holds light, as
//! [\`LightPlane\`]s the level copies into its chunks, and the sections whose geometry
//! has to be made again.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;

use ruthenium_protocol::packets::play::{LIGHT_BYTES, LightData};

use crate::shape::Shape;
use crate::world::LightPlane;

/// The brightest level, and the level of every sky source.
pub const MAX_LEVEL: u8 = 15;

/// \`Direction\` ordinals: a direction's opposite is its ordinal with the lowest bit
/// flipped.
const UP: usize = 1;
const NORTH: usize = 2;
const SOUTH: usize = 3;
const WEST: usize = 4;
const EAST: usize = 5;
/// The step of each direction, in ordinal order (down, up, north, south, west, east):
/// the order light spreads in.
const STEPS: [[i32; 3]; 6] = [
    [0, -1, 0],
    [0, 1, 0],
    [0, 0, -1],
    [0, 0, 1],
    [-1, 0, 0],
    [1, 0, 0],
];
const EVERY_WAY: u8 = 0b11_1111;

/// Every direction but one.
const fn all_but(direction: usize) -> u8 {
    EVERY_WAY & !(1 << direction)
}

/// What a block state does to light, as the table's maker gives it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct StateLight {
    /// \`getLightEmission\`.
    pub emission: u8,
    /// \`getLightDampening\`.
    pub dampening: u8,
    /// \`useShapeForLightOcclusion\`.
    pub uses_shape: bool,
    /// The shape of its light occlusion face towards each direction, in ordinal order
    /// (\`LightEngine.getOcclusionShape\`), each the id of a shape.
    pub faces: [u32; 6],
}

/// A state as the engine reads it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Props {
    emission: u8,
    dampening: u8,
    uses_shape: bool,
    /// Every face is empty: the state's shape never stops light.
    hollow: bool,
    /// The kind of each face ([\`LightTable::stops\`]).
    faces: [u16; 6],
}

impl Props {
    const AIR: Props = Props {
        emission: 0,
        dampening: 0,
        uses_shape: false,
        hollow: true,
        faces: [0; 6],
    };
    /// A block of a chunk that isn't loaded: the engine takes it for bedrock.
    const UNLOADED: Props = Props {
        dampening: MAX_LEVEL,
        ..Props::AIR
    };

    /// What passing into the block costs: at least 1.
    fn opacity(&self) -> i32 {
        i32::from(self.dampening.max(1))
    }
}

/// Every block state's light properties, by state id.
#[derive(Debug, Clone)]
pub struct LightTable {
    states: Vec<Props>,
    /// How many kinds of face there are; kind 0 is the empty face.
    kinds: usize,
    /// For each pair of face kinds, whether the two faces between neighbouring blocks
    /// stop light (\`Shapes.faceShapeOccludes\`).
    stops: Vec<bool>,
}

impl LightTable {
    /// A table of \`states\` in id order, with \`shape\` giving the shape of a face id.
    pub fn new<'a>(
        states: impl IntoIterator<Item = StateLight>,
        shape: impl Fn(u32) -> &'a Shape,
    ) -> LightTable {
        let empty = Shape::empty();
        let mut kinds: Vec<&Shape> = vec![&empty];
        let mut kind_of: HashMap<u32, u16> = HashMap::new();
        let states = states
            .into_iter()
            .map(|s| {
                let faces = s.faces.map(|id| {
                    *kind_of.entry(id).or_insert_with(|| {
                        let face = shape(id);
                        if face.is_empty() {
                            return 0;
                        }
                        // The same shape under another id is the same kind.
                        match kinds.iter().position(|k| std::ptr::eq(*k, face)) {
                            Some(k) => k as u16,
                            None => {
                                kinds.push(face);
                                (kinds.len() - 1) as u16
                            }
                        }
                    })
                });
                Props {
                    emission: s.emission.min(MAX_LEVEL),
                    dampening: s.dampening.min(MAX_LEVEL),
                    uses_shape: s.uses_shape,
                    hollow: faces == [0; 6],
                    faces,
                }
            })
            .collect();
        let n = kinds.len();
        let mut stops = vec![false; n * n];
        for a in 0..n {
            for b in a..n {
                let s = Shape::faces_occlude(kinds[a], kinds[b]);
                stops[a * n + b] = s;
                stops[b * n + a] = s;
            }
        }
        LightTable {
            states,
            kinds: n,
            stops,
        }
    }

    /// The table of the game's own blocks.
    pub fn of(blocks: &crate::blocks::Blocks) -> LightTable {
        LightTable::new(
            (0..blocks.len() as u32).map(|id| {
                let s = blocks.state(id);
                StateLight {
                    emission: s.light_emission,
                    dampening: s.light_dampening,
                    uses_shape: s.light_shape,
                    faces: s.occlusion,
                }
            }),
            |id| blocks.shape_at(id),
        )
    }

    /// A state's properties: \`None\` is a block of a chunk that isn't loaded, an id the
    /// table lacks is air.
    fn props(&self, state: Option<u32>) -> Props {
        match state {
            None => Props::UNLOADED,
            Some(id) => self.states.get(id as usize).copied().unwrap_or(Props::AIR),
        }
    }

    /// Whether the faces between \`from\` and its neighbour \`to\` in a direction stop
    /// light (\`shapeOccludes\`).
    fn faces_stop(&self, from: &Props, to: &Props, direction: usize) -> bool {
        let a = usize::from(from.faces[direction]);
        let b = usize::from(to.faces[direction ^ 1]);
        self.stops[a * self.kinds + b]
    }

    /// \`LightEngine.hasDifferentLightProperties\`: whether replacing one state by the
    /// other can change light.
    pub fn differ(&self, old: u32, new: u32) -> bool {
        if old == new {
            return false;
        }
        let (a, b) = (self.props(Some(old)), self.props(Some(new)));
        a.dampening != b.dampening || a.emission != b.emission || a.uses_shape || b.uses_shape
    }

    /// \`ChunkSkyLightSources.isEdgeOccluded\`: sky light can't come straight down from
    /// \`above\` into \`below\`.
    pub fn edge_occluded(&self, above: u32, below: u32) -> bool {
        let (a, b) = (self.props(Some(above)), self.props(Some(below)));
        b.dampening != 0 || self.faces_stop(&a, &b, 0)
    }

    pub fn emission(&self, state: u32) -> u8 {
        self.props(Some(state)).emission
    }
}

/// The lowest sky source of each column of blocks of a chunk (index \`x + 16 * z\`): the
/// y from which everything up is lit straight from the sky. [\`OPEN_BELOW\`] where the
/// sky reaches under the level.
pub type SkySources = [i32; 256];

/// A column of blocks with no occluded edge at all (\`ChunkSkyLightSources\` extends
/// its sources below the world).
pub const OPEN_BELOW: i32 = i32::MIN;

/// The lowest source of a column of blocks, looking down from \`from\` (the y above the
/// highest block looked at) with \`above\` the state there: the y above the first
/// occluded edge (\`findLowestSourceY\`). \`below(y)\` gives the state at y, or \`Err\` with
/// the bottom y of an all-air section containing y, which is skipped whole: the edges
/// at its top and inside it are not looked at, and the block above its bottom edge
/// counts as air.
pub(crate) fn lowest_source(
    table: &LightTable,
    from: i32,
    min_y: i32,
    mut above: u32,
    below: impl Fn(i32) -> Result<u32, i32>,
) -> i32 {
    let mut edge = from;
    while edge > min_y {
        match below(edge - 1) {
            Ok(state) => {
                if table.edge_occluded(above, state) {
                    return edge;
                }
                above = state;
                edge -= 1;
            }
            Err(bottom) => {
                above = 0;
                edge = bottom;
            }
        }
    }
    OPEN_BELOW
}

/// \`ChunkSkyLightSources.update\`: the new lowest source of a column of blocks after
/// the block at \`y\` changed, when it is another than \`current\`. \`state(y)\` is the
/// block there now (air outside the level). Only the edges above and below the block
/// are looked at, and the column below them when the edge that held the source opened.
pub(crate) fn updated_source(
    table: &LightTable,
    current: i32,
    y: i32,
    min_y: i32,
    state: impl Fn(i32) -> u32,
) -> Option<i32> {
    if y + 1 < current {
        return None;
    }
    let exact = |y: i32| -> Result<u32, i32> { Ok(state(y)) };
    let (top, middle) = (state(y + 1), state(y));
    if table.edge_occluded(top, middle) {
        if y + 1 > current {
            return Some(y + 1);
        }
    } else if y + 1 == current {
        return Some(lowest_source(table, y, min_y - 1, middle, exact));
    }
    let bottom = state(y - 1);
    if table.edge_occluded(middle, bottom) {
        if y > current {
            return Some(y);
        }
    } else if y == current {
        return Some(lowest_source(table, y - 1, min_y - 1, bottom, exact));
    }
    None
}

/// Where the engine reads blocks.
pub trait BlockSource {
    /// The state at a block position: \`None\` where no chunk is loaded, air outside the
    /// level's height and in a section counted as all air.
    fn state(&self, x: i32, y: i32, z: i32) -> Option<u32>;
}

/// What a queue entry carries besides its position.
#[derive(Debug, Clone, Copy)]
struct Wave {
    /// An increase: the level the source must still have. A decrease: the level that
    /// was taken away.
    level: u8,
    /// The directions to go, a bit per ordinal.
    ways: u8,
    /// An increase whose source's faces count as empty.
    hollow: bool,
    /// An increase from an emitter, which may first raise the source itself.
    emission: bool,
}

impl Wave {
    fn decrease(level: u8, ways: u8) -> Wave {
        Wave {
            level,
            ways,
            hollow: false,
            emission: false,
        }
    }

    fn increase(level: u8, ways: u8, hollow: bool) -> Wave {
        Wave {
            level,
            ways,
            hollow,
            emission: false,
        }
    }

    fn emit(level: u8, hollow: bool) -> Wave {
        Wave {
            level,
            ways: EVERY_WAY,
            hollow,
            emission: true,
        }
    }
}

/// A section's status bit: its chunk section has a block that isn't air. The bits
/// below count the neighbours that do.
const HAS_BLOCKS: u8 = 32;
const NEIGHBOURS: u8 = 31;

/// One chunk's light sections of a layer, from the one below the level up.
#[derive(Debug, Clone)]
struct Stack {
    status: Box<[u8]>,
    /// The layers held ([\`LightPlane::Absent\`]: none).
    held: Box<[LightPlane]>,
    /// The server's data waiting for a section to hold light.
    waiting: Box<[LightPlane]>,
    /// Whether light is on for the chunk.
    lit: bool,
    /// Sky light: the section y above the highest section that holds light.
    top: Option<i32>,
}

impl Stack {
    fn new(count: usize) -> Stack {
        Stack {
            status: vec![0; count].into(),
            held: vec![LightPlane::Absent; count].into(),
            waiting: vec![LightPlane::Absent; count].into(),
            lit: false,
            top: None,
        }
    }

    fn idle(&self) -> bool {
        !self.lit
            && self.status.iter().all(|s| *s == 0)
            && self.held.iter().all(|p| matches!(p, LightPlane::Absent))
            && self.waiting.iter().all(|p| matches!(p, LightPlane::Absent))
    }
}

fn section_of(p: [i32; 3]) -> [i32; 3] {
    [p[0] >> 4, p[1] >> 4, p[2] >> 4]
}

fn step(p: [i32; 3], direction: usize) -> [i32; 3] {
    let s = STEPS[direction];
    [p[0] + s[0], p[1] + s[1], p[2] + s[2]]
}

/// What a run reads: the blocks, their properties and the chunks' sky sources.
struct Scope<'a, B: BlockSource> {
    blocks: &'a B,
    table: &'a LightTable,
    sources: &'a HashMap<(i32, i32), Box<SkySources>>,
}

impl<B: BlockSource> Scope<'_, B> {
    fn props(&self, p: [i32; 3]) -> Props {
        self.table.props(self.blocks.state(p[0], p[1], p[2]))
    }

    /// The lowest sky source of a column of blocks, where its chunk is loaded.
    fn source(&self, x: i32, z: i32) -> Option<i32> {
        self.sources
            .get(&(x >> 4, z >> 4))
            .map(|s| s[((x & 15) | (z & 15) << 4) as usize])
    }
}

/// One of the two layers.
#[derive(Debug, Clone)]
struct Layer {
    sky: bool,
    /// The section y of a stack's first light section, and how many it has.
    bottom: i32,
    count: usize,
    stacks: HashMap<(i32, i32), Stack>,
    /// Sky light: the lowest section y that ever held light (it never rises).
    floor: i32,
    checks: Vec<[i32; 3]>,
    queued: HashSet<[i32; 3]>,
    decreases: VecDeque<([i32; 3], Wave)>,
    increases: VecDeque<([i32; 3], Wave)>,
    /// Sections whose status fell to zero: they lose their layer when the run ends.
    doomed: HashSet<[i32; 3]>,
    /// Sections whose layer is another than at the end of the last run.
    changed: HashSet<[i32; 3]>,
    /// Sections whose geometry has to be made again.
    touched: HashSet<[i32; 3]>,
    /// Sections came or went, or data waits: there is something to settle.
    unsettled: bool,
}

impl Layer {
    fn new(sky: bool, bottom: i32, count: usize) -> Layer {
        Layer {
            sky,
            bottom,
            count,
            stacks: HashMap::new(),
            floor: i32::MAX,
            checks: Vec::new(),
            queued: HashSet::new(),
            decreases: VecDeque::new(),
            increases: VecDeque::new(),
            doomed: HashSet::new(),
            changed: HashSet::new(),
            touched: HashSet::new(),
            unsettled: false,
        }
    }

    fn index(&self, section_y: i32) -> Option<usize> {
        usize::try_from(section_y - self.bottom)
            .ok()
            .filter(|i| *i < self.count)
    }

    fn stack_mut(&mut self, x: i32, z: i32) -> &mut Stack {
        let count = self.count;
        self.stacks
            .entry((x, z))
            .or_insert_with(|| Stack::new(count))
    }

    /// The layer a section holds.
    fn plane(&self, s: [i32; 3]) -> Option<&LightPlane> {
        let i = self.index(s[1])?;
        Some(&self.stacks.get(&(s[0], s[2]))?.held[i]).filter(|p| !matches!(p, LightPlane::Absent))
    }

    /// Whether a section holds light (\`storingLightForSection\`).
    fn holds(&self, s: [i32; 3]) -> bool {
        self.plane(s).is_some()
    }

    fn lit(&self, x: i32, z: i32) -> bool {
        self.stacks.get(&(x, z)).is_some_and(|s| s.lit)
    }

    /// The stored level of a block; 0 where no light is held.
    fn get(&self, p: [i32; 3]) -> u8 {
        self.plane(section_of(p))
            .and_then(|l| l.get(p[0] & 15, p[1] & 15, p[2] & 15))
            .unwrap_or(0)
    }

    /// Stores a level, and notes the sections within one block for meshing.
    fn set(&mut self, p: [i32; 3], level: u8) {
        let s = section_of(p);
        let Some(i) = self.index(s[1]) else {
            return;
        };
        let Some(stack) = self.stacks.get_mut(&(s[0], s[2])) else {
            return;
        };
        if matches!(stack.held[i], LightPlane::Absent) {
            return;
        }
        stack.held[i].set(p[0] & 15, p[1] & 15, p[2] & 15, level);
        self.changed.insert(s);
        for x in (p[0] - 1) >> 4..=(p[0] + 1) >> 4 {
            for y in (p[1] - 1) >> 4..=(p[1] + 1) >> 4 {
                for z in (p[2] - 1) >> 4..=(p[2] + 1) >> 4 {
                    self.touched.insert([x, y, z]);
                }
            }
        }
    }

    fn status(&self, s: [i32; 3]) -> u8 {
        match (self.index(s[1]), self.stacks.get(&(s[0], s[2]))) {
            (Some(i), Some(stack)) => stack.status[i],
            _ => 0,
        }
    }

    /// \`updateSectionStatus\`: a chunk section became all air, or stopped being so.
    /// The section's own status changes first, then its neighbours', x outermost and z
    /// innermost: the order new layers are made in.
    fn set_empty(&mut self, s: [i32; 3], empty: bool) {
        let old = self.status(s);
        let new = if empty {
            old & !HAS_BLOCKS
        } else {
            old | HAS_BLOCKS
        };
        if new == old {
            return;
        }
        self.put_status(s, new);
        for dx in -1..=1 {
            for dy in -1..=1 {
                for dz in -1..=1 {
                    if (dx, dy, dz) == (0, 0, 0) {
                        continue;
                    }
                    let n = [s[0] + dx, s[1] + dy, s[2] + dz];
                    let status = self.status(n);
                    let count = status & NEIGHBOURS;
                    let count = if empty {
                        count.saturating_sub(1)
                    } else {
                        (count + 1).min(26)
                    };
                    self.put_status(n, status & HAS_BLOCKS | count);
                }
            }
        }
    }

    fn put_status(&mut self, s: [i32; 3], new: u8) {
        let Some(i) = self.index(s[1]) else {
            return;
        };
        let old = std::mem::replace(&mut self.stack_mut(s[0], s[2]).status[i], new);
        if old == 0 && new != 0 {
            self.appear(s, i);
        } else if old != 0 && new == 0 {
            self.doomed.insert(s);
            self.unsettled = true;
        }
    }

    /// A section starts holding light (\`initializeSection\`), unless it was only about
    /// to stop: then it keeps the layer it has.
    fn appear(&mut self, s: [i32; 3], i: usize) {
        if self.doomed.remove(&s) {
            return;
        }
        let plane = self.fresh(s, i);
        let sky = self.sky;
        let stack = self.stack_mut(s[0], s[2]);
        stack.held[i] = plane;
        if sky && stack.top.is_none_or(|top| top < s[1] + 1) {
            stack.top = Some(s[1] + 1);
        }
        if sky {
            self.floor = self.floor.min(s[1]);
        }
        self.changed.insert(s);
        for dx in -1..=1 {
            for dy in -1..=1 {
                for dz in -1..=1 {
                    self.touched.insert([s[0] + dx, s[1] + dy, s[2] + dz]);
                }
            }
        }
        self.unsettled = true;
    }

    /// The layer a section starts with (\`createDataLayer\`): the server's if one waits.
    /// Sky light below the chunk's highest layer continues the bottom of the next layer
    /// above; at the top it is full where light is on. Everything else starts dark.
    fn fresh(&self, s: [i32; 3], i: usize) -> LightPlane {
        let Some(stack) = self.stacks.get(&(s[0], s[2])) else {
            return LightPlane::Dark;
        };
        if !matches!(stack.waiting[i], LightPlane::Absent) {
            return stack.waiting[i].clone();
        }
        if !self.sky {
            return LightPlane::Dark;
        }
        if stack.top.is_some_and(|top| s[1] < top)
            && let Some(above) = stack.held[i + 1..]
                .iter()
                .find(|p| !matches!(p, LightPlane::Absent))
        {
            return above.bottom_repeated();
        }
        if stack.lit {
            LightPlane::Full
        } else {
            LightPlane::Dark
        }
    }

    /// \`queueSectionData\`.
    fn wait(&mut self, s: [i32; 3], plane: Option<LightPlane>) {
        let Some(i) = self.index(s[1]) else {
            return;
        };
        match plane {
            Some(p) => {
                self.stack_mut(s[0], s[2]).waiting[i] = p;
                self.unsettled = true;
            }
            None => {
                if let Some(stack) = self.stacks.get_mut(&(s[0], s[2])) {
                    stack.waiting[i] = LightPlane::Absent;
                }
            }
        }
    }

    /// Turns light on or off for a chunk. Sky light turned on fills, from the chunk's
    /// top layer down to the section above the highest block the sky doesn't reach,
    /// every layer that is all dark (\`SkyLightEngine.setLightEnabled\`). \`highest\` is
    /// the highest of the chunk's lowest sources.
    fn set_lit(&mut self, x: i32, z: i32, on: bool, highest: i32) {
        if !on {
            if let Some(stack) = self.stacks.get_mut(&(x, z)) {
                stack.lit = false;
            }
            return;
        }
        let (sky, floor, bottom, count) = (self.sky, self.floor, self.bottom, self.count);
        let stack = self
            .stacks
            .entry((x, z))
            .or_insert_with(|| Stack::new(count));
        stack.lit = true;
        let Some(top) = stack.top.filter(|_| sky) else {
            return;
        };
        // The game computes this in 32 bits: a chunk open below everywhere wraps
        // around to the far top, and nothing is filled.
        let first_full = (highest.wrapping_sub(1) >> 4).wrapping_add(1);
        for y in (floor.max(first_full)..top).rev() {
            let i = usize::try_from(y - bottom).ok().filter(|i| *i < count);
            if let Some(i) = i
                && !matches!(stack.held[i], LightPlane::Absent)
            {
                if matches!(stack.held[i], LightPlane::Dark) {
                    stack.held[i] = LightPlane::Full;
                }
                self.changed.insert([x, y, z]);
            }
        }
    }

    fn check(&mut self, p: [i32; 3]) {
        if self.queued.insert(p) {
            self.checks.push(p);
        }
    }

    fn has_work(&self) -> bool {
        self.unsettled
            || !self.checks.is_empty()
            || !self.decreases.is_empty()
            || !self.increases.is_empty()
            || !self.changed.is_empty()
            || !self.touched.is_empty()
    }

    /// \`runLightUpdates\`: the checks, the decreases, the increases, then what came,
    /// went and waited.
    fn run<B: BlockSource>(&mut self, scope: Option<&Scope<'_, B>>) {
        let checks = std::mem::take(&mut self.checks);
        self.queued.clear();
        if let Some(scope) = scope {
            for p in checks {
                if self.sky {
                    self.check_sky(p, scope);
                } else {
                    self.check_block(p, scope);
                }
            }
            while let Some((p, wave)) = self.decreases.pop_front() {
                self.take_back(p, wave, scope);
            }
            while let Some((p, wave)) = self.increases.pop_front() {
                let mut level = self.get(p);
                if wave.emission && level < wave.level {
                    self.set(p, wave.level);
                    level = wave.level;
                }
                if level == wave.level {
                    self.spread(p, wave, scope);
                }
            }
        }
        self.settle();
    }

    /// The emission of the block at \`p\`: none where light is off for its chunk.
    fn emission(&self, p: [i32; 3], props: &Props) -> u8 {
        if props.emission > 0 && self.lit(p[0] >> 4, p[2] >> 4) {
            props.emission
        } else {
            0
        }
    }

    /// \`BlockLightEngine.checkNode\`.
    fn check_block<B: BlockSource>(&mut self, p: [i32; 3], scope: &Scope<'_, B>) {
        if !self.holds(section_of(p)) {
            return;
        }
        let props = scope.props(p);
        let emission = self.emission(p, &props);
        let old = self.get(p);
        if emission < old {
            self.set(p, 0);
            self.decreases
                .push_back((p, Wave::decrease(old, EVERY_WAY)));
        } else {
            // Taking back level 1 takes nothing: every lit neighbour sends its light
            // in again.
            self.decreases.push_back((p, Wave::decrease(1, EVERY_WAY)));
        }
        if emission > 0 {
            self.increases
                .push_back((p, Wave::emit(emission, props.hollow)));
        }
    }

    /// \`SkyLightEngine.checkNode\`.
    fn check_sky<B: BlockSource>(&mut self, p: [i32; 3], scope: &Scope<'_, B>) {
        let lowest = if self.lit(p[0] >> 4, p[2] >> 4) {
            scope.source(p[0], p[2])
        } else {
            None
        };
        if let Some(lowest) = lowest {
            self.remove_sources_below(p[0], p[2], lowest);
            self.add_sources_from(p[0], p[2], lowest, scope);
        }
        if !self.holds(section_of(p)) {
            return;
        }
        if lowest.is_some_and(|lowest| p[1] >= lowest) {
            self.decreases
                .push_back((p, Wave::decrease(MAX_LEVEL, all_but(UP))));
            self.increases
                .push_back((p, Wave::increase(MAX_LEVEL, all_but(UP), false)));
        } else {
            let old = self.get(p);
            if old > 0 {
                self.set(p, 0);
                self.decreases
                    .push_back((p, Wave::decrease(old, EVERY_WAY)));
            } else {
                self.decreases.push_back((p, Wave::decrease(1, EVERY_WAY)));
            }
        }
    }

    /// The bottom y of the lowest section that ever held sky light, as the game
    /// computes it (in 32 bits, also before any section has).
    fn floor_y(&self) -> i32 {
        self.floor.wrapping_mul(16)
    }

    /// Below a column of blocks' lowest source nothing is a source: every block still
    /// at 15 down from there is taken back, until one that isn't.
    fn remove_sources_below(&mut self, x: i32, z: i32, lowest: i32) {
        if lowest <= self.floor_y() {
            return;
        }
        let start = lowest - 1;
        let mut section = start >> 4;
        while section >= self.floor {
            if self.holds([x >> 4, section, z >> 4]) {
                let base = section * 16;
                for y in (base..=(base + 15).min(start)).rev() {
                    let p = [x, y, z];
                    if self.get(p) != MAX_LEVEL {
                        return;
                    }
                    self.set(p, 0);
                    let ways = if y == start { EVERY_WAY } else { all_but(UP) };
                    self.decreases
                        .push_back((p, Wave::decrease(MAX_LEVEL, ways)));
                }
            }
            section -= 1;
        }
    }

    /// From a column of blocks' lowest source up everything is a source: every block
    /// not yet at 15 becomes one, until one that is. A new source spreads only where
    /// it has something to light: at the lowest source itself, and below a neighbouring
    /// column's.
    fn add_sources_from<B: BlockSource>(
        &mut self,
        x: i32,
        z: i32,
        lowest: i32,
        scope: &Scope<'_, B>,
    ) {
        let beside = |x: i32, z: i32| scope.source(x, z).unwrap_or(OPEN_BELOW);
        let around = beside(x - 1, z)
            .max(beside(x + 1, z))
            .max(beside(x, z - 1))
            .max(beside(x, z + 1));
        let start = lowest.max(self.floor_y());
        let mut section = start >> 4;
        loop {
            let top = self.stacks.get(&(x >> 4, z >> 4)).and_then(|s| s.top);
            if top.is_none_or(|top| section >= top) {
                return;
            }
            if self.holds([x >> 4, section, z >> 4]) {
                let base = section * 16;
                for y in base.max(start)..=base + 15 {
                    let p = [x, y, z];
                    if self.get(p) == MAX_LEVEL {
                        return;
                    }
                    self.set(p, MAX_LEVEL);
                    if y < around || y == lowest {
                        self.increases
                            .push_back((p, Wave::increase(MAX_LEVEL, all_but(UP), false)));
                    }
                }
            }
            section += 1;
        }
    }

    /// Light goes from a block at \`wave.level\` into its neighbours.
    fn spread<B: BlockSource>(&mut self, from: [i32; 3], wave: Wave, scope: &Scope<'_, B>) {
        let level = i32::from(wave.level);
        let open_below = if self.sky {
            self.open_sections_below(from)
        } else {
            0
        };
        let mut source = None;
        for direction in 0..6 {
            if wave.ways & 1 << direction == 0 {
                continue;
            }
            let to = step(from, direction);
            if !self.holds(section_of(to)) {
                continue;
            }
            let has = i32::from(self.get(to));
            if level - 1 <= has {
                continue;
            }
            let target = scope.props(to);
            let new = level - target.opacity();
            if new <= has {
                continue;
            }
            let source = *source.get_or_insert_with(|| {
                if wave.hollow {
                    Props::AIR
                } else {
                    scope.props(from)
                }
            });
            if scope.table.faces_stop(&source, &target, direction) {
                continue;
            }
            let new = new as u8;
            self.set(to, new);
            if new > 1 {
                self.increases.push_back((
                    to,
                    Wave::increase(new, all_but(direction ^ 1), target.hollow),
                ));
            }
            if self.sky {
                self.copy_down(to, direction, new, true, open_below);
            }
        }
    }

    /// Light of level \`wave.level\` was taken from a block: neighbours it lit lose
    /// theirs and pass the loss on, brighter ones send their light back.
    fn take_back<B: BlockSource>(&mut self, from: [i32; 3], wave: Wave, scope: &Scope<'_, B>) {
        let open_below = if self.sky {
            self.open_sections_below(from)
        } else {
            0
        };
        for direction in 0..6 {
            if wave.ways & 1 << direction == 0 {
                continue;
            }
            let to = step(from, direction);
            if !self.holds(section_of(to)) {
                continue;
            }
            let has = self.get(to);
            if has == 0 {
                continue;
            }
            let back = direction ^ 1;
            if has >= wave.level {
                self.increases
                    .push_back((to, Wave::increase(has, 1 << back, false)));
                continue;
            }
            if self.sky {
                self.set(to, 0);
                self.decreases
                    .push_back((to, Wave::decrease(has, all_but(back))));
                self.copy_down(to, direction, has, false, open_below);
            } else {
                let props = scope.props(to);
                let emission = self.emission(to, &props);
                self.set(to, 0);
                if emission < has {
                    self.decreases
                        .push_back((to, Wave::decrease(has, all_but(back))));
                }
                if emission > 0 {
                    self.increases
                        .push_back((to, Wave::emit(emission, props.hollow)));
                }
            }
        }
    }

    /// For a block on the bottom layer of its section and on the edge of its chunk:
    /// how many sections straight below hold no sky light (down to the lowest that ever
    /// did). Their light is read from the layer above, so nothing spreads sideways out
    /// of them by itself (\`countEmptySectionsBelowIfAtBorder\`).
    fn open_sections_below(&self, p: [i32; 3]) -> i32 {
        let (x, z) = (p[0] & 15, p[2] & 15);
        if p[1] & 15 != 0 || (x != 0 && x != 15 && z != 0 && z != 15) {
            return 0;
        }
        let s = section_of(p);
        let mut open = 0;
        while !self.holds([s[0], s[1] - open - 1, s[2]]) && s[1] - open > self.floor {
            open += 1;
        }
        open
    }

    /// What crossed into another chunk above \`open_below\` sections without light is
    /// repeated in that chunk's sections beside them (\`propagateFromEmptySections\`).
    fn copy_down(
        &mut self,
        to: [i32; 3],
        direction: usize,
        level: u8,
        increase: bool,
        open_below: i32,
    ) {
        if open_below == 0 {
            return;
        }
        let (x, z) = (to[0] & 15, to[2] & 15);
        let crossed = match direction {
            NORTH => z == 15,
            SOUTH => z == 0,
            WEST => x == 15,
            EAST => x == 0,
            _ => false,
        };
        if !crossed {
            return;
        }
        let s = section_of(to);
        for section in (s[1] - open_below..s[1]).rev() {
            if !self.holds([s[0], section, s[2]]) {
                continue;
            }
            for y in (section * 16..section * 16 + 16).rev() {
                let p = [to[0], y, to[2]];
                if increase {
                    self.set(p, level);
                    if level > 1 {
                        self.increases
                            .push_back((p, Wave::increase(level, all_but(direction ^ 1), true)));
                    }
                } else {
                    self.set(p, 0);
                    self.decreases
                        .push_back((p, Wave::decrease(level, all_but(direction ^ 1))));
                }
            }
        }
    }

    /// \`markNewInconsistencies\`: doomed sections lose their layer and what waited for
    /// them, then every waiting layer whose section holds light replaces what is there.
    fn settle(&mut self) {
        if !self.unsettled {
            return;
        }
        self.unsettled = false;
        let doomed: Vec<[i32; 3]> = self.doomed.drain().collect();
        for s in &doomed {
            if let (Some(i), Some(stack)) = (self.index(s[1]), self.stacks.get_mut(&(s[0], s[2]))) {
                stack.waiting[i] = LightPlane::Absent;
                stack.held[i] = LightPlane::Absent;
            }
        }
        for s in &doomed {
            if self.sky {
                self.lower_top(*s);
            }
            self.changed.insert(*s);
        }
        let bottom = self.bottom;
        for (&(x, z), stack) in &mut self.stacks {
            for i in 0..stack.held.len() {
                if matches!(stack.waiting[i], LightPlane::Absent)
                    || matches!(stack.held[i], LightPlane::Absent)
                {
                    continue;
                }
                let plane = std::mem::take(&mut stack.waiting[i]);
                if !stack.held[i].shares(&plane) {
                    stack.held[i] = plane;
                    self.changed.insert([x, bottom + i as i32, z]);
                }
            }
        }
        self.stacks.retain(|_, s| !s.idle());
    }

    /// A chunk's highest layer went: the next one down is the highest now
    /// (\`SkyLightSectionStorage.onNodeRemoved\`).
    fn lower_top(&mut self, s: [i32; 3]) {
        if self.stacks.get(&(s[0], s[2])).and_then(|k| k.top) != Some(s[1] + 1) {
            return;
        }
        let mut y = s[1];
        while !self.holds([s[0], y, s[2]]) && y >= self.floor {
            y -= 1;
        }
        let top = self.holds([s[0], y, s[2]]).then_some(y + 1);
        if let Some(stack) = self.stacks.get_mut(&(s[0], s[2])) {
            stack.top = top;
        }
    }
}

/// What a run changed.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Outcome {
    /// Sections whose sky or block layer may be another than before.
    pub changed: Vec<[i32; 3]>,
    /// Sections whose geometry has to be made again (\`onLightUpdate\`).
    pub touched: Vec<[i32; 3]>,
}

/// The light of a level: block light, and sky light where the dimension has it.
#[derive(Debug, Clone)]
pub struct LightEngine {
    /// The section y of the light section below the level's lowest section.
    bottom: i32,
    /// Light sections per chunk: two more than it has sections.
    count: usize,
    block: Layer,
    sky: Option<Layer>,
    /// The loaded chunks' sky sources, where something has computed them.
    sources: HashMap<(i32, i32), Box<SkySources>>,
}

impl LightEngine {
    /// An engine for a level whose lowest section is at section y \`min_section\` and
    /// whose chunks have \`sections\` sections.
    pub fn new(min_section: i32, sections: i32, has_skylight: bool) -> LightEngine {
        let (bottom, count) = (min_section - 1, sections.max(0) as usize + 2);
        LightEngine {
            bottom,
            count,
            block: Layer::new(false, bottom, count),
            sky: has_skylight.then(|| Layer::new(true, bottom, count)),
            sources: HashMap::new(),
        }
    }

    pub fn has_skylight(&self) -> bool {
        self.sky.is_some()
    }

    fn layers(&mut self) -> impl Iterator<Item = &mut Layer> {
        std::iter::once(&mut self.block).chain(self.sky.as_mut())
    }

    /// The server's light for some of a chunk's light sections (\`readSectionList\`): it
    /// waits until its section holds light.
    pub fn queue(&mut self, x: i32, z: i32, light: &LightData) {
        let (bottom, count) = (self.bottom, self.count);
        let put =
            |layer: &mut Layer, mask: &[u64], empty: &[u64], arrays: &[Arc<[u8; LIGHT_BYTES]>]| {
                let mut arrays = arrays.iter();
                for i in 0..count {
                    let plane = if LightData::bit(mask, i) {
                        // A mask bit without its array: the packet is short, nothing waits.
                        let Some(a) = arrays.next() else {
                            return;
                        };
                        LightPlane::Dense(a.clone())
                    } else if LightData::bit(empty, i) {
                        LightPlane::Dark
                    } else {
                        continue;
                    };
                    layer.wait([x, bottom + i as i32, z], Some(plane));
                }
            };
        if let Some(sky) = &mut self.sky {
            put(sky, &light.sky_mask, &light.empty_sky_mask, &light.sky);
        }
        put(
            &mut self.block,
            &light.block_mask,
            &light.empty_block_mask,
            &light.block,
        );
    }

    /// Turns light on or off for a chunk (\`setLightEnabled\`).
    pub fn set_lit(&mut self, x: i32, z: i32, on: bool) {
        let highest = self.sources.get(&(x, z)).map_or(OPEN_BELOW, |s| {
            s.iter().copied().max().unwrap_or(OPEN_BELOW)
        });
        for layer in self.layers() {
            layer.set_lit(x, z, on, highest);
        }
    }

    /// A chunk section became all air or stopped being so (\`updateSectionStatus\`).
    pub fn set_section_empty(&mut self, section: [i32; 3], empty: bool) {
        for layer in self.layers() {
            layer.set_empty(section, empty);
        }
    }

    /// A chunk was forgotten: light off, what waited for it discarded, its sections
    /// all air (\`queueLightRemoval\`).
    pub fn forget(&mut self, x: i32, z: i32) {
        self.sources.remove(&(x, z));
        let (bottom, count) = (self.bottom, self.count as i32);
        for layer in self.layers() {
            layer.set_lit(x, z, false, OPEN_BELOW);
            for y in bottom..bottom + count {
                layer.wait([x, y, z], None);
            }
            for y in bottom + 1..bottom + count - 1 {
                layer.set_empty([x, y, z], true);
            }
        }
    }

    /// A loaded chunk's sky sources, or that it has none any more.
    pub fn set_sources(&mut self, x: i32, z: i32, sources: Option<Box<SkySources>>) {
        match sources {
            Some(s) => self.sources.insert((x, z), s),
            None => self.sources.remove(&(x, z)),
        };
    }

    /// The lowest sky source of a column of blocks, where its chunk has sources.
    pub fn source(&self, x: i32, z: i32) -> Option<i32> {
        self.sources
            .get(&(x >> 4, z >> 4))
            .map(|s| s[((x & 15) | (z & 15) << 4) as usize])
    }

    pub(crate) fn set_source(&mut self, x: i32, z: i32, y: i32) {
        if let Some(s) = self.sources.get_mut(&(x >> 4, z >> 4)) {
            s[((x & 15) | (z & 15) << 4) as usize] = y;
        }
    }

    /// Queues a block position for a check in both layers (\`checkBlock\`).
    pub fn check(&mut self, p: [i32; 3]) {
        for layer in self.layers() {
            layer.check(p);
        }
    }

    /// Notes every light section of a chunk as changed: its copy outside the engine
    /// was made anew.
    pub fn refresh(&mut self, x: i32, z: i32) {
        let (bottom, count) = (self.bottom, self.count as i32);
        for layer in self.layers() {
            layer
                .changed
                .extend((bottom..bottom + count).map(|y| [x, y, z]));
        }
    }

    pub fn has_work(&self) -> bool {
        self.block.has_work() || self.sky.as_ref().is_some_and(Layer::has_work)
    }

    /// One frame's light (\`runLightUpdates\`): block light, then sky light. Without a
    /// table nothing is computed: sections come and go and the server's light is
    /// installed, as sent.
    pub fn run(&mut self, blocks: &impl BlockSource, table: Option<&LightTable>) -> Outcome {
        let scope = table.map(|table| Scope {
            blocks,
            table,
            sources: &self.sources,
        });
        self.block.run(scope.as_ref());
        if let Some(sky) = &mut self.sky {
            sky.run(scope.as_ref());
        }
        let mut changed = std::mem::take(&mut self.block.changed);
        let mut touched = std::mem::take(&mut self.block.touched);
        if let Some(sky) = &mut self.sky {
            changed.extend(sky.changed.drain());
            touched.extend(sky.touched.drain());
        }
        let mut out = Outcome {
            changed: changed.into_iter().collect(),
            touched: touched.into_iter().collect(),
        };
        out.changed.sort_unstable();
        out.touched.sort_unstable();
        out
    }

    /// The block light layer a section holds, [\`LightPlane::Absent\`] if none.
    pub fn block_plane(&self, section: [i32; 3]) -> LightPlane {
        self.block.plane(section).cloned().unwrap_or_default()
    }

    /// The sky light layer a section holds.
    pub fn sky_plane(&self, section: [i32; 3]) -> LightPlane {
        self.sky
            .as_ref()
            .and_then(|l| l.plane(section))
            .cloned()
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::dirty;
    use crate::world::Level;
    use ruthenium_prepare::exporters::ShapeData;

    // State ids the level takes for blocks that aren't air; the table below says what
    // each does to light.
    const AIR: u32 = 0;
    const STONE: u32 = 1;
    const TORCH: u32 = 2;
    const GLASS: u32 = 3;
    const WATER: u32 = 4;
    const SLAB: u32 = 5;

    /// Faces: 0 none, 1 the whole face, 2 the lower half of a side face.
    fn table() -> LightTable {
        let shapes = [
            Shape::empty(),
            Shape::block(),
            Shape::new(&ShapeData {
                coords: [vec![0.0, 1.0], vec![0.0, 0.5, 1.0], vec![0.0, 1.0]],
                full: vec![true, false],
            }),
        ];
        let state = |emission, dampening, uses_shape, faces| StateLight {
            emission,
            dampening,
            uses_shape,
            faces,
        };
        LightTable::new(
            [
                state(0, 0, false, [0; 6]),
                state(0, 15, false, [0; 6]),
                state(14, 0, false, [0; 6]),
                state(0, 0, false, [0; 6]),
                state(0, 1, false, [0; 6]),
                // A bottom slab: its lower face is whole, its sides half, its top open.
                state(0, 0, true, [1, 0, 2, 2, 2, 2]),
            ],
            |id| &shapes[id as usize],
        )
    }

    /// Nine chunks of a level 64 high from y -16, with a floor of stone at y -1 and
    /// the sky above it.
    fn world() -> Level {
        let mut level = Level::new(-16, 64);
        level.set_light_table(Arc::new(table()));
        for x in -1..=1 {
            for z in -1..=1 {
                level.add_empty_chunk(x, z);
            }
        }
        for x in -16..32 {
            for z in -16..32 {
                level.set_block(x, -1, z, STONE);
            }
        }
        level.run_light_updates();
        level
    }

    #[test]
    fn the_sky_lights_everything_above_the_floor_and_nothing_below() {
        let level = world();
        for (x, z) in [(8, 8), (-16, -16), (31, 31), (0, 15), (16, -1)] {
            assert_eq!(level.sky_light(x, 0, z), 15, "{x} {z}");
            assert_eq!(level.sky_light(x, 30, z), 15);
            assert_eq!(level.sky_light(x, -1, z), 0);
            assert_eq!(level.sky_light(x, -16, z), 0);
            assert_eq!(level.block_light(x, 0, z), 0);
        }
    }

    #[test]
    fn a_torch_lights_a_diamond_and_takes_it_away_again() {
        let mut level = world();
        level.track_changes(4096);
        level.take_changes();
        level.set_block(8, 0, 8, TORCH);
        // Nothing until the frame's run.
        assert_eq!(level.block_light(8, 0, 8), 0);
        level.run_light_updates();
        let lit = |level: &Level, x: i32, y: i32, z: i32| level.block_light(x, y, z);
        assert_eq!(lit(&level, 8, 0, 8), 14);
        assert_eq!(lit(&level, 9, 0, 8), 13);
        assert_eq!(lit(&level, 8, 1, 8), 13);
        assert_eq!(lit(&level, 9, 0, 9), 12);
        assert_eq!(lit(&level, 11, 0, 8), 11);
        assert_eq!(lit(&level, 8, 5, 8), 9);
        // Into the next chunk, and to where it ends.
        assert_eq!(lit(&level, 16, 0, 8), 6);
        assert_eq!((lit(&level, 21, 0, 8), lit(&level, 22, 0, 8)), (1, 0));
        // Stone takes none.
        assert_eq!(lit(&level, 8, -1, 8), 0);
        // The sky's light is another layer.
        assert_eq!(level.sky_light(9, 0, 8), 15);
        // The sections the light reached are noted for drawing, each as its own.
        let changes = level.take_changes().unwrap();
        let keys: HashMap<_, _> = changes.dirty().collect();
        assert_eq!(keys[&[0, 0, 0]] & dirty::LIGHT, dirty::LIGHT);
        assert_eq!(keys[&[1, 0, 0]], dirty::LIGHT);
        // The stone below is within a block of lit air: its faces are drawn with it.
        assert_eq!(keys[&[0, -1, 0]], dirty::NEIGHBOUR);
        // The torch is its section's first block: the section above holds light now,
        // and the ones around that are noted with it.
        assert_eq!(keys[&[0, 2, 0]], dirty::NEIGHBOUR);

        level.set_block(8, 0, 8, AIR);
        level.run_light_updates();
        for (x, y, z) in [(8, 0, 8), (9, 0, 8), (8, 5, 8), (16, 0, 8), (21, 0, 8)] {
            assert_eq!(lit(&level, x, y, z), 0, "{x} {y} {z}");
        }
    }

    #[test]
    fn two_torches_keep_each_other_s_light_when_one_goes() {
        let mut level = world();
        level.set_block(4, 0, 8, TORCH);
        level.set_block(12, 0, 8, TORCH);
        level.run_light_updates();
        // Between them, eight blocks apart: 10 from either.
        assert_eq!(level.block_light(8, 0, 8), 10);
        level.set_block(4, 0, 8, AIR);
        level.run_light_updates();
        assert_eq!(level.block_light(8, 0, 8), 10);
        assert_eq!(level.block_light(4, 0, 8), 6);
        assert_eq!(level.block_light(0, 0, 8), 2);
        assert_eq!(level.block_light(12, 0, 8), 14);
    }

    #[test]
    fn a_wall_stops_light_and_light_goes_around_it() {
        let mut level = world();
        // A wall across x 10, three high and wide, beside a torch.
        for y in 0..3 {
            for z in 7..=9 {
                level.set_block(10, y, z, STONE);
            }
        }
        level.set_block(8, 0, 8, TORCH);
        level.run_light_updates();
        assert_eq!(level.block_light(9, 0, 8), 13);
        assert_eq!(level.block_light(10, 0, 8), 0);
        // Behind the wall: around its end at z 6 (9,0,6 is 11) and back.
        assert_eq!(level.block_light(11, 0, 8), 7);
    }

    #[test]
    fn a_roof_casts_a_shadow_the_sky_fills_from_the_sides() {
        let mut level = world();
        for x in 7..=9 {
            for z in 7..=9 {
                level.set_block(x, 3, z, STONE);
            }
        }
        level.run_light_updates();
        let sky = |level: &Level, x: i32, y: i32, z: i32| level.sky_light(x, y, z);
        assert_eq!(sky(&level, 8, 4, 8), 15);
        assert_eq!(sky(&level, 8, 3, 8), 0);
        assert_eq!(sky(&level, 6, 2, 8), 15);
        assert_eq!(sky(&level, 7, 2, 8), 14);
        assert_eq!(sky(&level, 8, 2, 8), 13);
        assert_eq!(sky(&level, 8, 0, 8), 13);
        // One block of the roof goes: the sky is back under it, and brighter beside.
        level.set_block(8, 3, 8, AIR);
        level.run_light_updates();
        assert_eq!(sky(&level, 8, 3, 8), 15);
        assert_eq!(sky(&level, 8, 0, 8), 15);
        assert_eq!(sky(&level, 7, 2, 7), 14);
        // And the rest.
        for x in 7..=9 {
            for z in 7..=9 {
                level.set_block(x, 3, z, AIR);
            }
        }
        level.run_light_updates();
        for x in 6..=10 {
            for y in 0..5 {
                assert_eq!(sky(&level, x, y, 8), 15, "{x} {y}");
            }
        }
    }

    #[test]
    fn dampening_and_faces_decide_what_the_sky_passes() {
        let mut level = world();
        // Glass changes nothing; a check isn't even queued.
        level.set_block(2, 3, 2, GLASS);
        level.run_light_updates();
        assert_eq!(
            (level.sky_light(2, 3, 2), level.sky_light(2, 2, 2)),
            (15, 15)
        );
        // Water takes one level, and what is below is lit from the sides as brightly.
        level.set_block(8, 3, 8, WATER);
        level.run_light_updates();
        assert_eq!(
            (level.sky_light(8, 3, 8), level.sky_light(8, 2, 8)),
            (14, 14)
        );
        // A slab's whole lower face ends the straight way down: the slab is in the sky,
        // the block below is lit from beside.
        level.set_block(12, 3, 12, SLAB);
        level.run_light_updates();
        assert_eq!(
            (level.sky_light(12, 3, 12), level.sky_light(12, 2, 12)),
            (15, 14)
        );
    }

    #[test]
    fn a_whole_face_stops_block_light_and_a_half_face_does_not() {
        let mut level = world();
        level.set_block(8, 2, 8, SLAB);
        level.set_block(8, 3, 8, TORCH);
        level.run_light_updates();
        // Into the slab from above, out of it sideways, not down through its base.
        assert_eq!(level.block_light(8, 2, 8), 13);
        assert_eq!(level.block_light(7, 2, 8), 12);
        assert_eq!(level.block_light(8, 1, 8), 10);
    }

    #[test]
    fn sections_hold_light_while_blocks_are_near() {
        let mut level = world();
        let column = |level: &Level| level.column(0, 0).unwrap().clone();
        // Light sections: index 0 is y -32, then every 16. Stone is in index 1, so 0
        // to 2 hold light and the rest don't.
        let c = column(&level);
        assert!(matches!(c.light()[3].block, LightPlane::Absent));
        assert!(matches!(c.light()[3].sky, LightPlane::Absent));
        assert!(!matches!(c.light()[2].sky, LightPlane::Absent));
        // Sky light above all layers is full without one.
        assert_eq!(level.sky_light(8, 40, 8), 15);
        // A torch high up: its section (index 4, y 32) and the two around it hold
        // light, here and in the chunks beside.
        level.set_block(8, 40, 8, TORCH);
        level.run_light_updates();
        let c = column(&level);
        assert_eq!(level.block_light(8, 41, 8), 13);
        assert!(!matches!(c.light()[3].block, LightPlane::Absent));
        assert!(!matches!(c.light()[5].block, LightPlane::Absent));
        assert!(!matches!(
            level.column(1, 1).unwrap().light()[4].sky,
            LightPlane::Absent
        ));
        // Light reaches down into the section below, which holds light too.
        assert_eq!(level.block_light(8, 30, 8), 4);
        assert_eq!(level.sky_light(8, 40, 8), 15);
        // The torch goes, and with the next run the layers.
        level.set_block(8, 40, 8, AIR);
        level.run_light_updates();
        let c = column(&level);
        assert!(matches!(c.light()[3].block, LightPlane::Absent));
        assert!(matches!(c.light()[4].sky, LightPlane::Absent));
        assert_eq!(level.block_light(8, 30, 8), 0);
        assert_eq!(level.sky_light(8, 40, 8), 15);
    }

    #[test]
    fn the_server_s_light_is_kept_and_computed_light_goes_on_top() {
        use crate::world::ChunkPacket;
        use crate::world::testing::{chunk_data, light};
        let mut level = Level::new(-16, 64);
        level.set_light_table(Arc::new(table()));
        // Stone in the bottom section; the server's light: sky 15 above it, dark in
        // it, block light 0 above it.
        let data = chunk_data(4, |i| u16::from(i == 0));
        let sent = light(&[(1, None), (2, Some(15))], &[(1, None), (2, Some(0))]);
        level
            .load_chunk_packet(
                0,
                0,
                ChunkPacket {
                    heightmaps: &[],
                    data: &data,
                    block_entities: &[],
                    light: &sent,
                },
                6,
            )
            .unwrap();
        level.run_light_updates();
        let c = level.column(0, 0).unwrap().clone();
        // The very arrays the packet had.
        assert!(
            c.light()[2]
                .sky
                .shares(&LightPlane::Dense(sent.sky[0].clone()))
        );
        assert!(matches!(c.light()[1].sky, LightPlane::Dark));
        assert_eq!(level.sky_light(8, 0, 8), 15);
        level.set_block(8, 0, 8, TORCH);
        level.run_light_updates();
        assert_eq!(level.block_light(9, 0, 8), 13);
        // The chunk beside isn't loaded: light stops at it.
        assert_eq!(level.block_light(16, 0, 8), 0);
        assert_eq!(level.block_light(15, 0, 8), 7);
        // The array the server sent was copied, not written to.
        assert_eq!(sent.block[0][0], 0);
        // A new array from the server replaces what was computed.
        level.update_light(0, 0, &light(&[], &[(2, Some(3))]));
        level.run_light_updates();
        assert_eq!(level.block_light(9, 0, 8), 3);
    }

    #[test]
    fn sky_sources_follow_the_highest_occluded_edge() {
        let t = table();
        // A column of blocks from y 0: stone at 2, a slab at 5, air elsewhere.
        let column = |y: i32| -> u32 {
            match y {
                2 => STONE,
                5 => SLAB,
                _ => AIR,
            }
        };
        let all = |y: i32| -> Result<u32, i32> { Ok(column(y)) };
        // From the top: the slab's lower face is the first edge that stops the sky.
        assert_eq!(lowest_source(&t, 16, 0, AIR, all), 5);
        // Below the slab: the stone.
        assert_eq!(lowest_source(&t, 5, 0, AIR, all), 3);
        assert_eq!(lowest_source(&t, 2, 0, AIR, all), OPEN_BELOW);
        // A section counted as all air is skipped with the edge at its top: a slab
        // right above one doesn't stop the sky there.
        let skipping = |y: i32| -> Result<u32, i32> {
            if (0..5).contains(&y) {
                Err(0)
            } else {
                Ok(column(y))
            }
        };
        assert_eq!(lowest_source(&t, 16, 0, AIR, skipping), OPEN_BELOW);

        // Updates look at the two edges of the changed block only.
        // The stone at 2 is under the source at 5: nothing changes.
        assert_eq!(updated_source(&t, 5, 2, 0, column), None);
        // A block placed above the source raises it.
        let with = |y: i32| if y == 9 { STONE } else { column(y) };
        assert_eq!(updated_source(&t, 5, 9, 0, with), Some(10));
        // The slab goes: the next edge down is the stone's.
        let without = |y: i32| if y == 5 { AIR } else { column(y) };
        assert_eq!(updated_source(&t, 5, 5, 0, without), Some(3));
        // And then the stone: open to below the level.
        assert_eq!(updated_source(&t, 3, 2, 0, |_| AIR), Some(OPEN_BELOW));
    }

    #[test]
    fn states_differ_for_light_by_what_light_reads() {
        let t = table();
        assert!(!t.differ(AIR, GLASS));
        assert!(t.differ(AIR, STONE) && t.differ(AIR, TORCH) && t.differ(AIR, WATER));
        // A state that uses its shape always counts.
        assert!(t.differ(AIR, SLAB) && t.differ(SLAB, GLASS));
        assert!(!t.differ(STONE, STONE));
        // An edge is occluded by dampening below or by the faces between.
        assert!(t.edge_occluded(AIR, STONE) && t.edge_occluded(AIR, WATER));
        assert!(t.edge_occluded(SLAB, AIR) && !t.edge_occluded(AIR, SLAB));
        assert!(!t.edge_occluded(AIR, GLASS));
    }

    /// With the prepared game (\`RUTHENIUM_GAME_DIR\`): the table of the game's own
    /// blocks says what the game's blocks are known to do. Skipped without it.
    #[test]
    fn the_game_s_blocks_light_as_they_are_known_to() {
        use ruthenium_data::v26_3::Block;
        let Some(game) = std::env::var_os("RUTHENIUM_GAME_DIR") else {
            return;
        };
        let exports = std::path::PathBuf::from(game).join("versions/26.3/exports");
        let blocks = crate::blocks::Blocks::load(&exports).expect("the block exports");
        let t = LightTable::of(&blocks);
        let id = |name: &str| Block::from_name(name).unwrap().default_state().id();
        let (air, stone) = (id("minecraft:air"), id("minecraft:stone"));
        for (name, emission) in [
            ("minecraft:torch", 14),
            ("minecraft:glowstone", 15),
            ("minecraft:lava", 15),
            ("minecraft:redstone_torch", 7),
            ("minecraft:stone", 0),
        ] {
            assert_eq!(t.emission(id(name)), emission, "{name}");
        }
        let dampening = |name: &str| t.props(Some(id(name))).dampening;
        assert_eq!(dampening("minecraft:stone"), 15);
        assert_eq!(dampening("minecraft:water"), 1);
        assert_eq!(dampening("minecraft:oak_leaves"), 1);
        assert_eq!(dampening("minecraft:glass"), 0);
        assert_eq!(dampening("minecraft:torch"), 0);
        // A bottom slab's base is whole and stops light; its top doesn't.
        let slab = Block::from_name("minecraft:oak_slab")
            .unwrap()
            .default_state();
        assert_eq!(slab.value("type"), Some("bottom"));
        assert!(t.edge_occluded(slab.id(), air) && !t.edge_occluded(air, slab.id()));
        assert!(t.differ(air, slab.id()) && t.differ(air, stone));
        assert!(!t.differ(air, id("minecraft:glass")));
        // Stone doesn't use its shape for light: it is opaque by dampening alone.
        assert!(t.props(Some(stone)).hollow);
    }
}

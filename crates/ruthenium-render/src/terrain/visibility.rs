//! Which sections can be seen from where the camera is: cave culling.
//!
//! Two parts, as in 26.3 (`VisGraph`, `SectionOcclusionGraph`):
//!
//! - A section's [`Connectivity`]: which of its six faces can be seen from which, through
//!   the blocks in it that are not solid. The mesher works it out with every mesh.
//! - The [`Graph`]: a walk from the camera's section to its neighbours, breadth first,
//!   through the faces each section connects. It never turns back in a direction opposite
//!   to one it has travelled, a section that has no mesh yet lets nothing through, and far
//!   from the camera a step is only taken if the sections on the straight line back to
//!   the camera were reached before it. What the walk reaches is what gets a mesh and is
//!   drawn; the frustum is applied to that every frame, and plays no part here.
//!
//! The walk runs on a thread of its own ([`Visibility`]): the render thread sends what
//! it learns ([`Event`]s, with the camera) and takes back which sections became reached
//! or stopped being so ([`Delta`]). A walk from the start is due when the camera enters
//! another 8-block cell, as in vanilla, or when a section's connectivity loses a pair;
//! everything else (a first mesh, a chunk arriving) goes on from where the last walk
//! stopped.

use std::collections::{HashMap, VecDeque};
use std::hash::{BuildHasherDefault, Hasher};
use std::sync::mpsc;
use std::thread::JoinHandle;
use std::time::Instant;

use ruthenium_sim::scene::SectionKey;

use super::mesher::STEPS;

const DOWN: usize = 0;
const UP: usize = 1;
const NORTH: usize = 2;
const SOUTH: usize = 3;
const WEST: usize = 4;
const EAST: usize = 5;

/// Sections from the camera's, on any axis, beyond which a step needs the line back to
/// the camera to be reached (60 blocks; `SectionOcclusionGraph.java:49`).
const NEAR_SECTIONS: i32 = 60 >> 4;
/// Blocks from the camera within which that line is not looked at, squared (`:320`).
const NEAR_BLOCKS_SQUARED: f64 = 3600.0;
/// The line's step in blocks: a section's diagonal, rounded up (`:51`).
const LINE_STEP: f64 = 28.0;
/// Blocks along one side of the cell whose change makes a walk from the start.
const CELL: f64 = 8.0;

/// For each face of a section (by direction ordinal: down, up, north, south, west,
/// east), the faces that can be seen from it, a bit each.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Connectivity([u8; 6]);

impl Default for Connectivity {
    /// Everything: what hides nothing.
    fn default() -> Connectivity {
        Connectivity::ALL
    }
}

impl Connectivity {
    pub const NONE: Connectivity = Connectivity([0; 6]);
    pub const ALL: Connectivity = Connectivity([0x3F; 6]);

    pub fn sees(self, from: usize, to: usize) -> bool {
        self.0[from] >> to & 1 != 0
    }

    /// Every pair of either.
    pub fn union(self, other: Connectivity) -> Connectivity {
        Connectivity(std::array::from_fn(|i| self.0[i] | other.0[i]))
    }

    /// Whether every pair of `other` is one of its own.
    pub fn contains(self, other: Connectivity) -> bool {
        (0..6).all(|i| other.0[i] & !self.0[i] == 0)
    }

    /// Pairs every face of `faces` with every one of them, itself included.
    fn join(&mut self, faces: u8) {
        for (f, seen) in self.0.iter_mut().enumerate() {
            if faces >> f & 1 != 0 {
                *seen |= faces;
            }
        }
    }

    /// From a section's solid blocks (`isSolidRender`): bit `x + 16 * z + 256 * y` of
    /// `solid` is the block at x, y, z. Fewer than 256 of them hide nothing and 4096
    /// everything; in between, every region of other blocks that touches the section's
    /// surface connects the faces it touches (`VisGraph.resolve`).
    pub fn of(solid: &[u64; 64]) -> Connectivity {
        let count: u32 = solid.iter().map(|w| w.count_ones()).sum();
        if count < 256 {
            return Connectivity::ALL;
        }
        if count == 4096 {
            return Connectivity::NONE;
        }
        let mut filled = *solid;
        let mut out = Connectivity::NONE;
        let mut stack = [0u16; 4096];
        for start in 0..4096_usize {
            let (x, z, y) = (start & 15, start >> 4 & 15, start >> 8);
            let surface = x == 0 || x == 15 || y == 0 || y == 15 || z == 0 || z == 15;
            if !surface || filled[start >> 6] >> (start & 63) & 1 != 0 {
                continue;
            }
            filled[start >> 6] |= 1 << (start & 63);
            stack[0] = start as u16;
            let mut top = 1;
            let mut faces = 0u8;
            while top > 0 {
                top -= 1;
                let cell = usize::from(stack[top]);
                let (x, z, y) = (cell & 15, cell >> 4 & 15, cell >> 8);
                let mut visit = |on_low: bool, on_high: bool, face: usize, step: usize| {
                    if on_low {
                        faces |= 1 << face;
                    } else if on_high {
                        faces |= 2 << face;
                    }
                    for (inside, next) in
                        [(!on_low, cell.wrapping_sub(step)), (!on_high, cell + step)]
                    {
                        if inside && filled[next >> 6] >> (next & 63) & 1 == 0 {
                            filled[next >> 6] |= 1 << (next & 63);
                            stack[top] = next as u16;
                            top += 1;
                        }
                    }
                };
                visit(y == 0, y == 15, DOWN, 256);
                visit(z == 0, z == 15, NORTH, 16);
                visit(x == 0, x == 15, WEST, 1);
            }
            out.join(faces);
        }
        out
    }
}

/// Where the walk starts and what bounds it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Camera {
    /// The eye, in blocks.
    pub position: [f64; 3],
    /// Off: connectivity is not asked, everything in range is reached (a spectator
    /// inside a solid block; `Camera.java:121-124`).
    pub smart: bool,
    /// The view distance in chunks.
    pub view_distance: i32,
}

impl Camera {
    /// What a walk from the start depends on: the 8-block cell the eye is in
    /// (`SectionOcclusionGraph.java:97-100`) and the two options. The walk a camera
    /// gives stays good while this stays the same.
    pub fn cell(&self) -> ([i64; 3], bool, i32) {
        (
            self.position.map(|p| (p / CELL).floor() as i64),
            self.smart,
            self.view_distance,
        )
    }
}

/// What the render thread learns that the graph has to know.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Event {
    /// Another level, whose chunks have `count` sections from section y `lowest`:
    /// nothing known so far holds any more.
    Level {
        lowest: i32,
        count: i32,
    },
    /// A chunk is loaded, or loaded anew. For each of its sections, bottom up: its
    /// connectivity if it has one (an empty section connects everything); `None`
    /// leaves what is known of it, which for a new chunk is nothing.
    Chunk {
        x: i32,
        z: i32,
        sections: Vec<Option<Connectivity>>,
    },
    Unloaded {
        x: i32,
        z: i32,
    },
    /// A section's connectivity, from its mesh.
    Section {
        key: SectionKey,
        connectivity: Connectivity,
    },
}

/// One frame's events. Batches are numbered from 1, each one higher than the last.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Batch {
    pub seq: u64,
    pub camera: Camera,
    pub events: Vec<Event>,
}

/// What changed in what the walk reaches, after the batches up to `seq`.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct Delta {
    pub seq: u64,
    pub added: Vec<SectionKey>,
    pub removed: Vec<SectionKey>,
    /// The walk began again at the camera.
    pub full: bool,
    /// How long the walk took.
    pub micros: u32,
    /// Sections of loaded chunks reached now.
    pub reached: u32,
}

/// Hashes a chunk's two coordinates: fast and good enough for positions that lie in a
/// square next to each other.
#[derive(Default)]
struct PairHasher(u64);

impl Hasher for PairHasher {
    fn finish(&self) -> u64 {
        self.0
    }

    fn write(&mut self, bytes: &[u8]) {
        for b in bytes {
            self.write_u32(u32::from(*b));
        }
    }

    fn write_u32(&mut self, v: u32) {
        self.0 = (self.0.rotate_left(5) ^ u64::from(v)).wrapping_mul(0x517c_c1b7_2722_0a95);
    }

    fn write_i32(&mut self, v: i32) {
        self.write_u32(v as u32);
    }
}

const NO_COLUMN: u32 = u32::MAX;

#[derive(Debug, Clone, Copy)]
struct Node {
    connectivity: Connectivity,
    /// It has a connectivity: a section with blocks that was never meshed has none and
    /// lets nothing through.
    known: bool,
    /// What the render thread was last told: reached or not.
    sent: bool,
    /// The directions it was entered by travelling in.
    sources: u8,
    /// Every direction travelled on the way to it.
    directions: u8,
    /// The walk it was last reached in; the graph's own: reached.
    pass: u32,
}

const UNREACHED: Node = Node {
    connectivity: Connectivity::NONE,
    known: false,
    sent: false,
    sources: 0,
    directions: 0,
    pass: 0,
};

#[derive(Debug, Clone, Copy)]
struct Column {
    at: (i32, i32),
    /// Its chunk is loaded. A column that is not stands where the walk wanted to go: its
    /// nodes are reached and wait for the chunk.
    loaded: bool,
    live: bool,
    /// The columns to the north, south, west and east.
    beside: [u32; 4],
}

/// See the module docs. Plain data and no thread: [`Visibility`] runs one.
pub(crate) struct Graph {
    lowest: i32,
    count: i32,
    slots: HashMap<(i32, i32), u32, BuildHasherDefault<PairHasher>>,
    columns: Vec<Column>,
    free: Vec<u32>,
    /// `count` nodes a column, bottom up.
    nodes: Vec<Node>,
    pass: u32,
    camera: Option<Camera>,
    cell: Option<([i64; 3], bool, i32)>,
    seq: u64,
    full_due: bool,
    /// Nodes the walk goes on from.
    from: Vec<u32>,
    /// Nodes whose state may not be what the render thread was told.
    touched: Vec<u32>,
    queue: VecDeque<u32>,
    reached: u32,
}

impl std::fmt::Debug for Graph {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Graph")
            .field("columns", &self.slots.len())
            .field("reached", &self.reached)
            .finish_non_exhaustive()
    }
}

/// `ChunkTrackingView.isInViewDistance`: a chunk counts as in view one chunk further
/// than the distance says, and by a circle beyond that.
fn in_view(centre: (i32, i32), view_distance: i32, x: i32, z: i32) -> bool {
    let off = |a: i32, b: i32| ((i64::from(a) - i64::from(b)).abs() - 1).max(0);
    let (dx, dz) = (off(x, centre.0), off(z, centre.1));
    dx * dx + dz * dz < i64::from(view_distance) * i64::from(view_distance)
}

impl Graph {
    pub fn new() -> Graph {
        Graph {
            lowest: 0,
            count: 0,
            slots: HashMap::default(),
            columns: Vec::new(),
            free: Vec::new(),
            nodes: Vec::new(),
            pass: 1,
            camera: None,
            cell: None,
            seq: 0,
            full_due: false,
            from: Vec::new(),
            touched: Vec::new(),
            queue: VecDeque::new(),
            reached: 0,
        }
    }

    fn key(&self, node: u32) -> SectionKey {
        let c = &self.columns[(node / self.count as u32) as usize];
        [
            c.at.0,
            self.lowest + (node % self.count as u32) as i32,
            c.at.1,
        ]
    }

    fn visited(&self, node: u32) -> bool {
        self.nodes[node as usize].pass == self.pass
    }

    /// The column at a chunk position, made if there is none: not loaded, its nodes
    /// unreached.
    fn column(&mut self, x: i32, z: i32) -> u32 {
        if let Some(slot) = self.slots.get(&(x, z)) {
            return *slot;
        }
        let column = Column {
            at: (x, z),
            loaded: false,
            live: true,
            beside: [NO_COLUMN; 4],
        };
        let slot = match self.free.pop() {
            Some(slot) => {
                self.columns[slot as usize] = column;
                slot
            }
            None => {
                self.columns.push(column);
                self.nodes
                    .resize(self.columns.len() * self.count as usize, UNREACHED);
                self.columns.len() as u32 - 1
            }
        };
        self.slots.insert((x, z), slot);
        for d in NORTH..=EAST {
            let step = STEPS[d];
            if let Some(&other) = self.slots.get(&(x + step[0], z + step[2])) {
                self.columns[slot as usize].beside[d - 2] = other;
                self.columns[other as usize].beside[(d ^ 1) - 2] = slot;
            }
        }
        slot
    }

    fn free_column(&mut self, slot: u32) {
        let column = self.columns[slot as usize];
        for d in NORTH..=EAST {
            let other = column.beside[d - 2];
            if other != NO_COLUMN {
                self.columns[other as usize].beside[(d ^ 1) - 2] = NO_COLUMN;
            }
        }
        self.slots.remove(&column.at);
        self.columns[slot as usize].live = false;
        self.columns[slot as usize].loaded = false;
        let first = slot as usize * self.count as usize;
        for node in &mut self.nodes[first..first + self.count as usize] {
            self.reached -= u32::from(node.sent);
            *node = UNREACHED;
        }
        self.free.push(slot);
    }

    fn connect(&mut self, node: u32, connectivity: Connectivity) {
        let visited = self.visited(node);
        let n = &mut self.nodes[node as usize];
        if n.known && n.connectivity == connectivity {
            return;
        }
        let lost = n.known && !connectivity.contains(n.connectivity);
        n.connectivity = connectivity;
        n.known = true;
        if visited {
            if lost {
                // What was reached through the pairs that are gone is found again or not.
                self.full_due = true;
            } else {
                self.from.push(node);
            }
        }
    }

    /// Takes in a batch: its events, and where the camera is now.
    pub fn take(&mut self, batch: Batch) {
        for event in batch.events {
            match event {
                Event::Level { lowest, count } => {
                    *self = Graph {
                        lowest,
                        count: count.max(0),
                        ..Graph::new()
                    };
                    self.full_due = true;
                }
                Event::Chunk { x, z, sections } => {
                    if self.count == 0 {
                        continue;
                    }
                    let slot = self.column(x, z);
                    let first = slot * self.count as u32;
                    if !std::mem::replace(&mut self.columns[slot as usize].loaded, true) {
                        // Where the walk waited for it, it goes on.
                        for node in first..first + self.count as u32 {
                            if self.visited(node) {
                                self.from.push(node);
                                self.touched.push(node);
                            }
                        }
                    }
                    for (node, known) in (first..first + self.count as u32).zip(sections) {
                        if let Some(connectivity) = known {
                            self.connect(node, connectivity);
                        }
                    }
                }
                Event::Unloaded { x, z } => {
                    let Some(&slot) = self.slots.get(&(x, z)) else {
                        continue;
                    };
                    // It stays as a place the walk has been, for the lines that cross
                    // it, until the next walk from the start.
                    self.columns[slot as usize].loaded = false;
                    let first = slot as usize * self.count as usize;
                    for node in &mut self.nodes[first..first + self.count as usize] {
                        self.reached -= u32::from(node.sent);
                        node.sent = false;
                        node.known = false;
                        node.connectivity = Connectivity::NONE;
                    }
                }
                Event::Section { key, connectivity } => {
                    let y = key[1] - self.lowest;
                    if let Some(&slot) = self.slots.get(&(key[0], key[2]))
                        && (0..self.count).contains(&y)
                        && self.columns[slot as usize].loaded
                    {
                        self.connect(slot * self.count as u32 + y as u32, connectivity);
                    }
                }
            }
        }
        let c = batch.camera;
        let cell = c.cell();
        if self.cell != Some(cell) {
            self.cell = Some(cell);
            self.full_due = true;
        }
        self.camera = Some(c);
        self.seq = batch.seq;
    }

    /// Walks as far as what was taken in allows, and says what changed.
    pub fn settle(&mut self) -> Delta {
        let started = Instant::now();
        let mut delta = Delta {
            seq: self.seq,
            ..Delta::default()
        };
        let (Some(camera), true) = (self.camera, self.count > 0) else {
            return delta;
        };
        let mut report = |graph: &mut Graph, node: u32| {
            let column = &graph.columns[(node / graph.count as u32) as usize];
            let now = column.live && column.loaded && graph.visited(node);
            let n = &mut graph.nodes[node as usize];
            if now == n.sent {
                return;
            }
            n.sent = now;
            let key = graph.key(node);
            if now {
                graph.reached += 1;
                delta.added.push(key);
            } else {
                graph.reached -= 1;
                delta.removed.push(key);
            }
        };
        if std::mem::take(&mut self.full_due) {
            self.from.clear();
            self.touched.clear();
            self.walk_from_camera(&camera);
            self.touched.clear();
            for node in 0..self.nodes.len() as u32 {
                report(self, node);
            }
            delta.full = true;
        } else {
            if !self.from.is_empty() {
                let mut from = std::mem::take(&mut self.from);
                self.queue.clear();
                for node in from.drain(..) {
                    if self.visited(node) {
                        self.queue.push_back(node);
                    }
                }
                self.from = from;
                self.walk(&camera);
            }
            let mut touched = std::mem::take(&mut self.touched);
            for node in touched.drain(..) {
                report(self, node);
            }
            self.touched = touched;
        }
        delta.reached = self.reached;
        delta.micros = started.elapsed().as_micros().min(u128::from(u32::MAX)) as u32;
        delta
    }

    fn reach(&mut self, node: u32, sources: u8, directions: u8) {
        let n = &mut self.nodes[node as usize];
        n.pass = self.pass;
        n.sources = sources;
        n.directions = directions;
        self.queue.push_back(node);
        self.touched.push(node);
    }

    /// A walk from the start: from the camera's section, or from the level's top or
    /// bottom layer for a camera above or below it.
    fn walk_from_camera(&mut self, camera: &Camera) {
        if self.pass == u32::MAX {
            for n in &mut self.nodes {
                n.pass = 0;
            }
            self.pass = 0;
        }
        self.pass += 1;
        // The places the last walk waited at are made again where this one wants to go.
        for slot in 0..self.columns.len() as u32 {
            let c = self.columns[slot as usize];
            if c.live && !c.loaded {
                self.free_column(slot);
            }
        }
        self.queue.clear();
        let block = camera.position.map(|p| p.floor() as i32);
        let section = block.map(|b| b >> 4);
        let y = section[1] - self.lowest;
        if (0..self.count).contains(&y) {
            let slot = self.column(section[0], section[2]);
            self.reach(slot * self.count as u32 + y as u32, 0, 0);
        } else {
            // Outside the level's height: every section of the nearest layer in view,
            // entered from outside, nearest first (`:203-236`).
            let (layer, towards) = if y < 0 {
                (0, UP)
            } else {
                (self.count - 1, DOWN)
            };
            let vd = camera.view_distance;
            let mut seeds = Vec::new();
            for dx in -vd..=vd {
                for dz in -vd..=vd {
                    let (x, z) = (section[0] + dx, section[2] + dz);
                    if !in_view((section[0], section[2]), vd, x, z) {
                        continue;
                    }
                    let mut directions = 1u8 << towards;
                    if dx != 0 {
                        directions |= 1 << if dx > 0 { EAST } else { WEST };
                    }
                    if dz != 0 {
                        directions |= 1 << if dz > 0 { SOUTH } else { NORTH };
                    }
                    let centre = [x * 16 + 8, (self.lowest + layer) * 16 + 8, z * 16 + 8];
                    let distance: i64 = (0..3)
                        .map(|i| {
                            let d = i64::from(block[i]) - i64::from(centre[i]);
                            d * d
                        })
                        .sum();
                    seeds.push((distance, x, z, directions));
                }
            }
            seeds.sort_by_key(|s| s.0);
            for (_, x, z, directions) in seeds {
                let slot = self.column(x, z);
                self.reach(
                    slot * self.count as u32 + layer as u32,
                    1 << towards,
                    directions,
                );
            }
        }
        self.walk(camera);
    }

    /// Whether the sections on the line from a far section's corner back to the camera
    /// are reached (`:303-333`). `direction` is the step being considered from the
    /// section at `section`.
    fn line_is_reached(&self, camera: &Camera, section: [i32; 3], direction: usize) -> bool {
        let centre = camera.position.map(|p| ((p.floor() as i32) >> 4) * 16 + 8);
        // The direction's axis among x, y, z.
        let along = [1, 1, 2, 2, 0, 0][direction];
        let mut point: [f64; 3] = std::array::from_fn(|a| {
            let origin = section[a] * 16;
            let high = if a == along {
                centre[a] > origin
            } else {
                centre[a] < origin
            };
            f64::from(origin + if high { 16 } else { 0 })
        });
        let eye = camera.position;
        // The game's vector arithmetic, term by term.
        let d = [eye[0] - point[0], eye[1] - point[1], eye[2] - point[2]];
        let scale = 1.0 / (d[0] * d[0] + (d[1] * d[1] + d[2] * d[2])).sqrt();
        let step = d.map(|c| c * scale * LINE_STEP);
        let far = |p: &[f64; 3]| {
            let d = [p[0] - eye[0], p[1] - eye[1], p[2] - eye[2]];
            d[0] * d[0] + (d[1] * d[1] + d[2] * d[2]) > NEAR_BLOCKS_SQUARED
        };
        let (lowest, highest) = (
            f64::from(self.lowest * 16),
            f64::from((self.lowest + self.count) * 16 - 1),
        );
        while far(&point) {
            for a in 0..3 {
                point[a] += step[a];
            }
            if point[1] > highest || point[1] < lowest {
                break;
            }
            let at = point.map(|p| (p.floor() as i32) >> 4);
            let reached = self.slots.get(&(at[0], at[2])).is_some_and(|slot| {
                let y = at[1] - self.lowest;
                (0..self.count).contains(&y) && self.visited(slot * self.count as u32 + y as u32)
            });
            if !reached {
                return false;
            }
        }
        true
    }

    /// Goes on from the queued nodes (`:244-340`).
    fn walk(&mut self, camera: &Camera) {
        let origin = camera.position.map(|p| (p.floor() as i32) >> 4);
        let (smart, vd) = (camera.smart, camera.view_distance);
        let count = self.count as u32;
        while let Some(node) = self.queue.pop_front() {
            let slot = node / count;
            let column = self.columns[slot as usize];
            if !column.loaded {
                // It waits for its chunk.
                continue;
            }
            let y = (node % count) as i32;
            let section = [column.at.0, self.lowest + y, column.at.1];
            let n = self.nodes[node as usize];
            let distant = (0..3).any(|a| (section[a] - origin[a]).abs() > NEAR_SECTIONS);
            for (d, step) in STEPS.iter().enumerate() {
                let to = [
                    section[0] + step[0],
                    section[1] + step[1],
                    section[2] + step[2],
                ];
                let to_y = y + step[1];
                if !(0..self.count).contains(&to_y)
                    || (to[1] - origin[1]).abs() > vd
                    || !in_view((origin[0], origin[2]), vd, to[0], to[2])
                {
                    continue;
                }
                if smart {
                    if n.directions >> (d ^ 1) & 1 != 0 {
                        continue;
                    }
                    if n.sources != 0
                        && !(n.known
                            && (0..6)
                                .any(|s| n.sources >> s & 1 != 0 && n.connectivity.sees(s ^ 1, d)))
                    {
                        continue;
                    }
                    if distant && !self.line_is_reached(camera, section, d) {
                        continue;
                    }
                }
                let next = if d < NORTH {
                    slot * count + to_y as u32
                } else {
                    let mut beside = self.columns[slot as usize].beside[d - 2];
                    if beside == NO_COLUMN {
                        beside = self.column(to[0], to[2]);
                    }
                    beside * count + to_y as u32
                };
                if self.visited(next) {
                    self.nodes[next as usize].sources |= 1 << d;
                } else {
                    self.reach(next, 1 << d, n.directions | 1 << d);
                }
            }
        }
    }
}

/// The graph on its thread.
pub(crate) struct Visibility {
    batches: Option<mpsc::Sender<Batch>>,
    deltas: mpsc::Receiver<Delta>,
    thread: Option<JoinHandle<()>>,
}

impl std::fmt::Debug for Visibility {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Visibility").finish_non_exhaustive()
    }
}

impl Visibility {
    pub fn start() -> Visibility {
        let (batches, taken) = mpsc::channel::<Batch>();
        let (answers, deltas) = mpsc::channel();
        let thread = std::thread::Builder::new()
            .name("visibility".into())
            .spawn(move || {
                let mut graph = Graph::new();
                while let Ok(batch) = taken.recv() {
                    graph.take(batch);
                    // Whatever else has come meanwhile: one walk for all of it.
                    while let Ok(more) = taken.try_recv() {
                        graph.take(more);
                    }
                    profiling::scope!("visibility");
                    if answers.send(graph.settle()).is_err() {
                        return;
                    }
                }
            })
            .ok();
        Visibility {
            batches: Some(batches),
            deltas,
            thread,
        }
    }

    pub fn send(&self, batch: Batch) {
        if let Some(b) = &self.batches {
            let _ = b.send(batch);
        }
    }

    /// The next answer, if one has come.
    pub fn poll(&self) -> Option<Delta> {
        self.deltas.try_recv().ok()
    }

    pub fn stop(&mut self) {
        self.batches = None;
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

impl Drop for Visibility {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;

    /// A section's solid blocks, from a rule.
    fn solid(is: impl Fn(usize, usize, usize) -> bool) -> [u64; 64] {
        let mut bits = [0u64; 64];
        for y in 0..16 {
            for z in 0..16 {
                for x in 0..16 {
                    if is(x, y, z) {
                        let i = x + 16 * z + 256 * y;
                        bits[i >> 6] |= 1 << (i & 63);
                    }
                }
            }
        }
        bits
    }

    /// Connects exactly the given faces with each other (and themselves).
    fn only(faces: &[usize]) -> Connectivity {
        let mut c = Connectivity::NONE;
        c.join(faces.iter().fold(0, |m, f| m | 1 << f));
        c
    }

    #[test]
    fn few_solid_blocks_hide_nothing_and_a_full_section_everything() {
        assert_eq!(Connectivity::of(&[0; 64]), Connectivity::ALL);
        assert_eq!(Connectivity::of(&[u64::MAX; 64]), Connectivity::NONE);
        // 255 blocks of a floor with one missing: not enough to count, whatever they
        // would separate.
        let nearly = solid(|x, y, z| y == 8 && (x, z) != (0, 0));
        assert_eq!(Connectivity::of(&nearly), Connectivity::ALL);
    }

    #[test]
    fn a_floor_separates_what_is_above_it_from_what_is_below() {
        let c = Connectivity::of(&solid(|_, y, _| y == 8));
        assert!(!c.sees(DOWN, UP) && !c.sees(UP, DOWN));
        for side in NORTH..=EAST {
            // The sides reach above and below the floor.
            assert!(c.sees(DOWN, side) && c.sees(side, DOWN));
            assert!(c.sees(UP, side) && c.sees(side, UP));
            assert!(c.sees(side, side ^ 1));
        }
        // A face is paired with itself.
        assert!(c.sees(UP, UP) && c.sees(DOWN, DOWN));
        // A hole in the floor connects the two.
        let holed = Connectivity::of(&solid(|x, y, z| y == 8 && (x, z) != (5, 9)));
        assert_eq!(holed, Connectivity::ALL);
    }

    #[test]
    fn a_tunnel_connects_its_two_ends_only() {
        // Solid but for a tunnel from west to east, and a pocket that touches no face.
        let c = Connectivity::of(&solid(|x, y, z| {
            let tunnel = y == 3 && z == 4;
            let pocket = (5..8).contains(&x) && (9..12).contains(&y) && (9..12).contains(&z);
            !tunnel && !pocket
        }));
        assert_eq!(c, only(&[WEST, EAST]));
        assert!(c.sees(WEST, EAST) && c.sees(EAST, WEST) && c.sees(WEST, WEST));
        assert!(!c.sees(WEST, UP) && !c.sees(NORTH, SOUTH) && !c.sees(UP, UP));
        // A tunnel with a bend: west to up. And both at once stay apart.
        let bend = Connectivity::of(&solid(|x, y, z| {
            !((z == 4 && y == 3 && x <= 6) || (z == 4 && x == 6 && y >= 3))
        }));
        assert_eq!(bend, only(&[WEST, UP]));
        let two = Connectivity::of(&solid(|x, y, z| {
            !(y == 3 && z == 4) && !(x == 10 && z == 12)
        }));
        assert_eq!(two, only(&[WEST, EAST]).union(only(&[DOWN, UP])));
        assert!(two.contains(only(&[DOWN, UP])) && !only(&[DOWN, UP]).contains(two));
    }

    /// A graph and what it has said is reached.
    struct Sim {
        graph: Graph,
        seq: u64,
        reached: BTreeSet<SectionKey>,
    }

    fn eye(section: SectionKey) -> [f64; 3] {
        section.map(|s| f64::from(s * 16) + 8.5)
    }

    fn camera(section: SectionKey) -> Camera {
        Camera {
            position: eye(section),
            smart: true,
            view_distance: 8,
        }
    }

    fn chunk(x: i32, z: i32, sections: &[Option<Connectivity>]) -> Event {
        Event::Chunk {
            x,
            z,
            sections: sections.to_vec(),
        }
    }

    impl Sim {
        fn new(lowest: i32, count: i32) -> Sim {
            let mut sim = Sim {
                graph: Graph::new(),
                seq: 0,
                reached: BTreeSet::new(),
            };
            sim.graph.take(Batch {
                seq: 0,
                camera: camera([0; 3]),
                events: vec![Event::Level { lowest, count }],
            });
            sim
        }

        fn step(&mut self, camera: Camera, events: Vec<Event>) -> Delta {
            self.seq += 1;
            self.graph.take(Batch {
                seq: self.seq,
                camera,
                events,
            });
            let delta = self.graph.settle();
            assert_eq!(delta.seq, self.seq);
            for k in &delta.added {
                assert!(self.reached.insert(*k), "{k:?} added twice");
            }
            for k in &delta.removed {
                assert!(self.reached.remove(k), "{k:?} removed but never added");
            }
            delta
        }

        fn sees(&self, x: i32, y: i32, z: i32) -> bool {
            self.reached.contains(&[x, y, z])
        }
    }

    const OPEN: Option<Connectivity> = Some(Connectivity::ALL);
    const SHUT: Option<Connectivity> = Some(Connectivity::NONE);

    #[test]
    fn open_air_is_reached_as_far_as_chunks_are_loaded() {
        let mut sim = Sim::new(-1, 4);
        let chunks = (-2..=2)
            .flat_map(|z| (-2..=2).map(move |x| chunk(x, z, &[OPEN; 4])))
            .collect();
        let delta = sim.step(camera([0, 0, 0]), chunks);
        assert!(delta.full);
        assert_eq!((sim.reached.len(), delta.reached), (100, 100));
        assert!(delta.removed.is_empty());
        // Nothing new: nothing said.
        let delta = sim.step(camera([0, 0, 0]), Vec::new());
        assert!(!delta.full && delta.added.is_empty() && delta.removed.is_empty());
    }

    #[test]
    fn a_wall_hides_what_is_behind_it_until_it_opens() {
        let mut sim = Sim::new(0, 2);
        let mut chunks = Vec::new();
        for z in -1..=1 {
            for x in -1..=3 {
                chunks.push(chunk(x, z, &[if x == 1 { SHUT } else { OPEN }; 2]));
            }
        }
        sim.step(camera([0, 0, 0]), chunks);
        // The wall itself is reached; nothing beyond it.
        assert!(sim.sees(1, 0, 0) && sim.sees(1, 1, -1));
        assert!(!sim.reached.iter().any(|k| k[0] > 1), "{:?}", sim.reached);
        assert_eq!(sim.reached.len(), 3 * 3 * 2);
        // A hole from west to east in one of its sections: the walk goes on from there,
        // without starting again.
        let delta = sim.step(
            camera([0, 0, 0]),
            vec![Event::Section {
                key: [1, 0, 0],
                connectivity: only(&[WEST, EAST]),
            }],
        );
        assert!(!delta.full);
        assert_eq!(delta.added.len(), 2 * 3 * 2);
        assert!(sim.sees(3, 1, 1));
        // The hole closes: everything behind is taken back.
        let delta = sim.step(
            camera([0, 0, 0]),
            vec![Event::Section {
                key: [1, 0, 0],
                connectivity: Connectivity::NONE,
            }],
        );
        assert!(delta.full);
        assert_eq!((delta.removed.len(), delta.added.len()), (12, 0));
        assert_eq!(delta.reached, 18);
    }

    #[test]
    fn a_section_without_a_mesh_lets_nothing_through() {
        let mut sim = Sim::new(0, 1);
        let row = |known| {
            vec![
                chunk(0, 0, &[OPEN]),
                chunk(1, 0, &[known]),
                chunk(2, 0, &[OPEN]),
            ]
        };
        sim.step(camera([0, 0, 0]), row(None));
        assert!(sim.sees(1, 0, 0) && !sim.sees(2, 0, 0));
        // Saying nothing of it again changes nothing; its mesh does.
        let delta = sim.step(camera([0, 0, 0]), row(None));
        assert!(delta.added.is_empty());
        let delta = sim.step(camera([0, 0, 0]), row(OPEN));
        assert_eq!(delta.added, vec![[2, 0, 0]]);
        assert!(!delta.full);
        // The camera's own section needs none: it is looked out of.
        let mut sim = Sim::new(0, 1);
        sim.step(
            camera([0, 0, 0]),
            vec![chunk(0, 0, &[None]), chunk(1, 0, &[OPEN])],
        );
        assert!(sim.sees(1, 0, 0));
    }

    #[test]
    fn faces_are_left_only_where_the_one_entered_by_connects() {
        let mut sim = Sim::new(0, 1);
        sim.step(
            camera([0, 0, 0]),
            vec![
                chunk(0, 0, &[OPEN]),
                chunk(1, 0, &[Some(only(&[WEST, EAST]))]),
                // Entered from the west, it leads south only.
                chunk(2, 0, &[Some(only(&[WEST, SOUTH]))]),
                chunk(3, 0, &[OPEN]),
                chunk(2, 1, &[OPEN]),
                chunk(1, -1, &[OPEN]),
            ],
        );
        assert!(sim.sees(2, 0, 0) && sim.sees(2, 0, 1));
        assert!(!sim.sees(3, 0, 0));
        // North of the tunnel: not through its wall.
        assert!(!sim.sees(1, 0, -1));
    }

    #[test]
    fn the_walk_never_turns_back() {
        let chunks = || {
            vec![
                chunk(0, 0, &[OPEN]),
                chunk(1, 0, &[Some(only(&[WEST, EAST]))]),
                chunk(2, 0, &[OPEN]),
                chunk(0, 1, &[SHUT]),
                chunk(1, 1, &[OPEN]),
                chunk(2, 1, &[OPEN]),
            ]
        };
        // East, east, south: the section to the west of there would be a step back.
        let mut sim = Sim::new(0, 1);
        sim.step(camera([0, 0, 0]), chunks());
        assert!(sim.sees(2, 0, 1) && !sim.sees(1, 0, 1));
        // Without smart culling everything in range is reached, whatever is solid.
        let mut sim = Sim::new(0, 1);
        let free = Camera {
            smart: false,
            ..camera([0, 0, 0])
        };
        sim.step(free, chunks());
        assert_eq!(sim.reached.len(), 6);
        // Switching it on again is a walk from the start.
        let delta = sim.step(camera([0, 0, 0]), Vec::new());
        assert!(delta.full);
        assert_eq!(delta.removed, vec![[1, 0, 1]]);
    }

    #[test]
    fn a_camera_outside_the_level_looks_in_through_its_nearest_layer() {
        for (camera_y, first, second) in [(5, 1, 0), (-3, 0, 1)] {
            // The layer the camera looks at is shut, but for one section or none.
            for hole in [Some((0, 0)), Some((1, -1)), None] {
                let mut sim = Sim::new(0, 2);
                let mut chunks = Vec::new();
                for z in -1..=1 {
                    for x in -1..=1 {
                        let mut sections = [OPEN; 2];
                        if hole != Some((x, z)) {
                            sections[first as usize] = SHUT;
                        }
                        chunks.push(chunk(x, z, &sections));
                    }
                }
                sim.step(camera([0, camera_y, 0]), chunks);
                for z in -1..=1 {
                    for x in -1..=1 {
                        assert!(sim.sees(x, first, z));
                        // Through a hole under (over) the camera, the whole layer
                        // behind; through one to a side, what is under it and no step
                        // back towards the middle.
                        let through = match hole {
                            Some((0, 0)) => true,
                            Some(at) => at == (x, z),
                            None => false,
                        };
                        assert_eq!(sim.sees(x, second, z), through, "{hole:?} {x} {z}");
                    }
                }
            }
        }
        // Further out than the view distance in sections: the layer, and no step on.
        let mut sim = Sim::new(0, 2);
        sim.step(camera([0, 10, 0]), vec![chunk(0, 0, &[OPEN; 2])]);
        assert!(sim.sees(0, 1, 0) && !sim.sees(0, 0, 0));
    }

    #[test]
    fn the_walk_waits_where_a_chunk_is_missing_and_goes_on_when_it_comes() {
        let mut sim = Sim::new(0, 1);
        sim.step(
            camera([0, 0, 0]),
            vec![
                chunk(0, 0, &[OPEN]),
                chunk(1, 0, &[OPEN]),
                chunk(3, 0, &[OPEN]),
            ],
        );
        assert_eq!(sim.reached.len(), 2);
        let delta = sim.step(camera([0, 0, 0]), vec![chunk(2, 0, &[OPEN])]);
        assert!(!delta.full);
        assert_eq!(delta.added, vec![[2, 0, 0], [3, 0, 0]]);
        // It leaves again: no longer reached (the render thread forgot it itself), and
        // what lies beyond stays until the camera moves on.
        let delta = sim.step(camera([0, 0, 0]), vec![Event::Unloaded { x: 2, z: 0 }]);
        assert!(delta.added.is_empty() && delta.removed.is_empty());
        assert_eq!(delta.reached, 3);
        sim.reached.remove(&[2, 0, 0]);
        // Back, as another chunk whose section has no mesh yet: reached, and a wall.
        let delta = sim.step(camera([0, 0, 0]), vec![chunk(2, 0, &[None])]);
        assert_eq!(delta.added, vec![[2, 0, 0]]);
        assert_eq!(delta.reached, 4);
        // A walk from the start finds what is behind it gone.
        let moved = Camera {
            position: [7.5, 8.5, 8.5],
            ..camera([0, 0, 0])
        };
        let delta = sim.step(moved, Vec::new());
        assert!(delta.full);
        assert_eq!(delta.removed, vec![[3, 0, 0]]);
    }

    #[test]
    fn only_another_cell_makes_the_walk_start_again() {
        let mut sim = Sim::new(0, 1);
        sim.step(camera([0, 0, 0]), vec![chunk(0, 0, &[OPEN])]);
        let at = |x: f64| Camera {
            position: [x, 8.5, 8.5],
            ..camera([0, 0, 0])
        };
        assert!(!sim.step(at(8.1), Vec::new()).full);
        assert!(!sim.step(at(15.9), Vec::new()).full);
        assert!(sim.step(at(7.9), Vec::new()).full);
        assert!(sim.step(at(16.0), Vec::new()).full);
        // The camera's chunk is not loaded: the walk waits there, and nothing is
        // reached.
        assert!(sim.reached.is_empty());
        // Another level forgets everything.
        let delta = sim.step(
            camera([0, 0, 0]),
            vec![Event::Level {
                lowest: -4,
                count: 3,
            }],
        );
        assert!(delta.full && delta.added.is_empty() && delta.removed.is_empty());
        assert_eq!(delta.reached, 0);
    }

    #[test]
    fn the_view_distance_bounds_the_walk() {
        let mut sim = Sim::new(-8, 16);
        let mut chunks = Vec::new();
        for z in -6..=6 {
            for x in -6..=6 {
                chunks.push(chunk(x, z, &[OPEN; 16]));
            }
        }
        let near = Camera {
            view_distance: 3,
            ..camera([0, 0, 0])
        };
        sim.step(near, chunks);
        // Along an axis a chunk more than the distance; by a circle off it; three
        // sections up and down.
        assert!(sim.sees(3, 0, 0) && !sim.sees(4, 0, 0));
        assert!(sim.sees(3, 0, 3) && !sim.sees(4, 0, 3) && sim.sees(3, 0, -3));
        assert!(sim.sees(0, 3, 0) && !sim.sees(0, 4, 0));
        assert!(sim.sees(0, -3, 0) && !sim.sees(0, -4, 0));
        assert_eq!(sim.reached.len(), 7 * 7 * 7);
    }

    #[test]
    fn far_from_the_camera_a_step_needs_the_line_back_to_be_reached() {
        // A tunnel east from the camera, opening five sections out to a section to its
        // south. From there the next step east is far, and the straight line from
        // there to the camera crosses the column (3, 1), which nothing has entered.
        let layout = |third: Option<Connectivity>| {
            let tunnel = Some(only(&[WEST, EAST]));
            vec![
                chunk(0, 0, &[OPEN]),
                chunk(1, 0, &[tunnel]),
                chunk(2, 0, &[tunnel]),
                chunk(3, 0, &[third]),
                chunk(4, 0, &[tunnel]),
                chunk(5, 0, &[OPEN]),
                chunk(5, 1, &[OPEN]),
                chunk(6, 1, &[OPEN]),
            ]
        };
        let mut sim = Sim::new(0, 1);
        sim.step(camera([0, 0, 0]), layout(Some(only(&[WEST, EAST]))));
        assert!(sim.sees(5, 0, 1));
        assert!(!sim.sees(6, 0, 1));
        // With the tunnel open to the south at (3, 0), the walk has been where the
        // line crosses (it waits there for the chunk), and the step is taken.
        let mut sim = Sim::new(0, 1);
        sim.step(camera([0, 0, 0]), layout(OPEN));
        assert!(sim.sees(6, 0, 1));
        // Four sections out is far, three is not.
        let mut sim = Sim::new(0, 1);
        sim.step(camera([1, 0, 0]), layout(Some(only(&[WEST, EAST]))));
        assert!(sim.sees(5, 0, 1) && !sim.sees(6, 0, 1));
        let mut sim = Sim::new(0, 1);
        sim.step(camera([2, 0, 0]), layout(Some(only(&[WEST, EAST]))));
        assert!(sim.sees(6, 0, 1));
        // Near the camera no line is asked for: the same shape three sections closer.
        let mut sim = Sim::new(0, 1);
        let shifted = layout(Some(only(&[WEST, EAST])));
        sim.step(camera([3, 0, 0]), shifted);
        assert!(sim.sees(6, 0, 1));
    }

    #[test]
    fn three_sections_out_no_line_is_asked_for_however_far_the_corner_is() {
        // East through a tunnel, then south through one, to a section three sections
        // from the camera on two axes: more than 60 blocks, and the line back crosses
        // the column (1, 2), which nothing has entered.
        let (east, south) = (Some(only(&[WEST, EAST])), Some(only(&[NORTH, SOUTH])));
        let mut sim = Sim::new(0, 1);
        sim.step(
            camera([0, 0, 0]),
            vec![
                chunk(0, 0, &[OPEN]),
                chunk(1, 0, &[east]),
                chunk(2, 0, &[east]),
                chunk(3, 0, &[Some(only(&[WEST, SOUTH]))]),
                chunk(3, 1, &[south]),
                chunk(3, 2, &[south]),
                chunk(3, 3, &[OPEN]),
                chunk(4, 3, &[OPEN]),
                chunk(3, 4, &[OPEN]),
            ],
        );
        assert!(sim.sees(3, 0, 3));
        assert!(sim.sees(4, 0, 3) && sim.sees(3, 0, 4));
    }

    #[test]
    fn the_walk_starts_when_the_cameras_chunk_arrives() {
        let mut sim = Sim::new(0, 1);
        sim.step(camera([0, 0, 0]), vec![chunk(1, 0, &[OPEN])]);
        assert!(sim.reached.is_empty());
        // Its section has no mesh yet: it is looked out of all the same.
        let delta = sim.step(camera([0, 0, 0]), vec![chunk(0, 0, &[None])]);
        assert!(!delta.full);
        assert_eq!(delta.added, vec![[0, 0, 0], [1, 0, 0]]);
    }
}

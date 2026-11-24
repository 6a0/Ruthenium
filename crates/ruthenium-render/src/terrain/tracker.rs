//! Which sections have a mesh, which need one, and in what order they get it.
//!
//! The level tells what changed through the scene's dirty queue; the tracker turns
//! that into work and keeps, for every section of a loaded chunk, the mesh that is
//! drawn and the [`Stamp`] it was made from. It holds no GPU objects itself: a mesh is
//! whatever the caller stores (`M`), handed back when it is replaced or its chunk goes.
//!
//! Rules it keeps:
//! - A section's chunk left: its mesh goes at once (no geometry outlives its chunk).
//! - A chunk next to a section's own left: its mesh stays as it is, as in vanilla. It is
//!   made again when that chunk is back, which is another load of it.
//! - A new level (another epoch) empties everything.
//! - The dirty queue overflowed, or a level began: every section is compared with the
//!   level again, by its stamp.
//! - A section gets its first mesh only once the eight chunks around its own are
//!   loaded, as in vanilla (`SectionUpdateTracker.hasAllNeighbors`): a chunk at the edge
//!   of what is loaded is not drawn with faces towards chunks that are still to come.
//!   Later changes are meshed whatever is loaded around.
//! - One job per section at a time. A change that arrives while a section is being
//!   meshed keeps it dirty, and it is meshed again afterwards.
//! - The player's own changes go first, then what is nearest the camera.
//!
//! With cave culling ([`Tracker::culled`]) it also keeps what the visibility graph
//! says is reached from the camera, and only those sections are meshed and drawn:
//! - It tells the graph what it learns ([`Event`]s): the chunks that come and go, and
//!   every section's connectivity, as its mesh gives it. A new mesh's connectivity is
//!   added to the old one's as soon as its result is taken (what it opens is reached a
//!   few frames sooner) and replaces it when the mesh is drawn.
//! - A section of nothing but air needs no job: it has nothing to draw and connects
//!   everything.
//! - A dirty section the graph does not reach waits, dirty, until it does. The player's
//!   own changes are meshed wherever they are.

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::time::Instant;

use ruthenium_sim::scene::{Around, Neighbourhood, Scene, SectionKey, Stamp, WHOLE_COLUMN, dirty};
use ruthenium_sim::world::{BlockPlane, Column};

use super::table::BlockTable;
use super::visibility::{Connectivity, Delta, Event};

/// New dirty sections after which the order of work is worked out again.
const RESORT_AFTER: usize = 256;

/// A section the tracker knows. Kept small: every pass over the sections reads these.
#[derive(Debug, Default)]
struct Entry {
    /// Where the mesh drawn is among the meshes (`None`: nothing to draw, or no mesh
    /// yet) and what it was made from (`None`: never meshed).
    mesh: Option<u32>,
    stamp: Option<Box<Stamp>>,
    /// Why it has to be meshed (again); 0: it doesn't.
    dirty: u8,
    /// A job for it is under way.
    meshing: bool,
    /// It waits in the urgent queue.
    urgent: bool,
    /// Since when it has been dirty, and since when the change its job is for was
    /// known.
    since: Option<Since>,
    in_flight: Option<Since>,
    /// The visibility graph reaches it from the camera (always, without culling).
    reached: bool,
    /// It was nothing but air when last looked at: no mesh, and no job for one.
    empty: bool,
    /// What the graph was last told its faces connect.
    connectivity: Option<Connectivity>,
    /// The batch of events that tells the graph of this load of its chunk: what the
    /// graph said before it had that batch is of another chunk.
    born: u64,
    /// When it first had a mesh, or was first found to be air: what its fade into view
    /// counts from, whatever is meshed there later. Forgotten with its chunk.
    first_at: Option<Instant>,
}

/// A mesh to draw.
#[derive(Debug)]
struct Slot<M> {
    key: SectionKey,
    /// Its section is reached.
    shown: bool,
    /// Its section's `first_at`.
    since: Instant,
    mesh: M,
}

/// A moment on the render thread: the time and the number of the frame being recorded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Since {
    pub at: Instant,
    pub frame: u64,
}

/// Counters since the tracker was made.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TrackerStats {
    /// Jobs handed out.
    pub jobs: u64,
    /// Of them, for the player's own changes.
    pub urgent_jobs: u64,
    /// Results thrown away because the level had changed under them.
    pub stale: u64,
    /// Comparisons of everything with the level.
    pub rescans: u64,
    /// Times a section waited because chunks around it were not loaded.
    pub waits_for_neighbours: u64,
    /// Levels entered.
    pub epochs: u64,
    /// Jobs by why their section was dirty, a job counted once for each reason: a
    /// first mesh, blocks, light, biomes, block entities, something next to it.
    pub causes: [u64; 6],
}

/// The names of [`TrackerStats::causes`], in order.
pub const CAUSES: [&str; 6] = [
    "first",
    "blocks",
    "light",
    "biomes",
    "block_entities",
    "neighbour",
];

/// A section to mesh.
#[derive(Debug)]
pub(crate) struct Job {
    pub neighbourhood: Neighbourhood,
    /// The player's own change: its upload is urgent too.
    pub urgent: bool,
    /// The section has never had a mesh.
    pub first: bool,
    /// When the tracker learned of the change the job is for ([`Tracker::set_clock`]).
    pub since: Since,
}

/// See the module docs.
#[derive(Debug)]
pub(crate) struct Tracker<M> {
    epoch: u64,
    sections: HashMap<SectionKey, Entry>,
    /// Every mesh with its section, in no order: what a frame goes through.
    meshes: Vec<Slot<M>>,
    /// Sections whose `dirty` is set and that are reached: the ones waiting for a job.
    dirty_count: usize,
    /// Sections that are reached.
    reached: usize,
    /// With cave culling: the block states, to know air by.
    table: Option<Arc<BlockTable>>,
    /// What the graph has yet to be told, the level it was last told of (its epoch and
    /// its sections' range), and the number of the batch the events being gathered go
    /// in.
    events: Vec<Event>,
    announced: Option<(u64, (i32, i32))>,
    batch: u64,
    urgent: VecDeque<SectionKey>,
    /// Dirty sections, the nearest to the camera last.
    queue: Vec<SectionKey>,
    /// Sections that became dirty (or free for a job again) since the queue was last
    /// put in order.
    fresh: Vec<SectionKey>,
    /// The sections of a chunk, by y: from the level's lowest section, so many.
    range: (i32, i32),
    /// Scratch of [`Self::sync`].
    around: Around,
    /// The camera's section when the queue was last sorted, and how many sections
    /// became dirty since.
    sorted_for: Option<SectionKey>,
    unsorted: usize,
    keys: Vec<(SectionKey, u8)>,
    stats: TrackerStats,
    /// The moment changes taken in now are known from.
    now: Since,
}

fn distance(a: SectionKey, b: SectionKey) -> i64 {
    (0..3)
        .map(|i| i64::from(a[i] - b[i]) * i64::from(a[i] - b[i]))
        .sum()
}

impl<M> Tracker<M> {
    /// A tracker of sections that are all meshed and drawn, reached or not.
    pub fn new() -> Tracker<M> {
        Tracker::with(None)
    }

    /// A tracker of sections that are meshed and drawn only where the visibility graph
    /// reaches them: see the module docs.
    pub fn culled(table: Arc<BlockTable>) -> Tracker<M> {
        Tracker::with(Some(table))
    }

    fn with(table: Option<Arc<BlockTable>>) -> Tracker<M> {
        Tracker {
            epoch: 0,
            sections: HashMap::new(),
            meshes: Vec::new(),
            dirty_count: 0,
            reached: 0,
            table,
            events: Vec::new(),
            announced: None,
            batch: 0,
            urgent: VecDeque::new(),
            queue: Vec::new(),
            fresh: Vec::new(),
            range: (0, 0),
            around: Around::default(),
            sorted_for: None,
            unsorted: 0,
            keys: Vec::new(),
            stats: TrackerStats::default(),
            now: Since {
                at: Instant::now(),
                frame: 0,
            },
        }
    }

    /// Says what moment it is: the changes [`Self::sync`] takes in from here on are
    /// known since then, which is what a job's [`Job::since`] says. With cave culling a
    /// change in a section that nothing reaches counts from the moment the section is
    /// reached ([`Self::reach`]).
    pub fn set_clock(&mut self, now: Since) {
        self.now = now;
    }

    pub fn stats(&self) -> TrackerStats {
        self.stats
    }

    /// Sections of loaded chunks, sections with a mesh, and sections waiting for a
    /// mesh (with cave culling: the reached ones among the dirty).
    pub fn counts(&self) -> (usize, usize, usize) {
        (self.sections.len(), self.meshes.len(), self.dirty_count)
    }

    /// Sections the visibility graph reaches (all of them, without culling).
    pub fn reached(&self) -> usize {
        self.reached
    }

    /// Every mesh to draw: those of reached sections.
    pub fn meshes(&self) -> impl Iterator<Item = (SectionKey, &M)> + '_ {
        self.meshes
            .iter()
            .filter(|s| s.shown)
            .map(|s| (s.key, &s.mesh))
    }

    /// Every mesh to draw, with the moment its section first had one (or was first
    /// found to be air): what a section's fade into view counts from.
    pub fn meshes_since(&self) -> impl Iterator<Item = (SectionKey, &M, Instant)> + '_ {
        self.meshes
            .iter()
            .filter(|s| s.shown)
            .map(|s| (s.key, &s.mesh, s.since))
    }

    /// Every mesh there is, drawn or not.
    pub fn all_meshes(&self) -> impl Iterator<Item = &M> + '_ {
        self.meshes.iter().map(|s| &s.mesh)
    }

    /// Says which batch the events gathered from here on are sent in.
    pub fn set_batch(&mut self, batch: u64) {
        self.batch = batch;
    }

    /// Whether the graph has yet to be told something.
    pub fn has_events(&self) -> bool {
        !self.events.is_empty()
    }

    /// What the graph has to be told since the last call, in order.
    pub fn take_events(&mut self) -> Vec<Event> {
        std::mem::take(&mut self.events)
    }

    /// Takes in what the graph says changed in what it reaches.
    pub fn reach(&mut self, delta: &Delta) {
        for (keys, reached) in [(&delta.added, true), (&delta.removed, false)] {
            for key in keys {
                let Some(e) = self.sections.get_mut(key) else {
                    continue;
                };
                // Said of a chunk that was at this place before.
                if delta.seq < e.born || e.reached == reached {
                    continue;
                }
                e.reached = reached;
                if let Some(index) = e.mesh {
                    self.meshes[index as usize].shown = reached;
                }
                if reached {
                    self.reached += 1;
                } else {
                    self.reached -= 1;
                }
                if e.dirty != 0 {
                    if reached {
                        // Only now does its change wait to be shown.
                        e.since = Some(self.now);
                        self.dirty_count += 1;
                        self.unsorted += 1;
                        self.fresh.push(*key);
                    } else {
                        self.dirty_count -= 1;
                    }
                }
            }
        }
    }

    /// What a mesh says its section's faces connect. `shown`: the mesh is the one
    /// drawn now. Until then what it connects is added to what the mesh before it did.
    pub fn connect(&mut self, key: SectionKey, connectivity: Connectivity, shown: bool) {
        if self.table.is_none() {
            return;
        }
        let Some(e) = self.sections.get_mut(&key) else {
            return;
        };
        let next = match e.connectivity {
            Some(old) if !shown => old.union(connectivity),
            _ => connectivity,
        };
        if e.connectivity != Some(next) {
            e.connectivity = Some(next);
            self.events.push(Event::Section {
                key,
                connectivity: next,
            });
        }
    }

    /// Takes the mesh at `index` out; the mesh that takes its place is told where it
    /// is now.
    fn take_mesh(&mut self, index: u32) -> M {
        let slot = self.meshes.swap_remove(index as usize);
        if let Some(moved) = self.meshes.get(index as usize)
            && let Some(e) = self.sections.get_mut(&moved.key)
        {
            e.mesh = Some(index);
        }
        slot.mesh
    }

    /// A section's entry, made if it has none: reached at once without culling.
    fn entry(&mut self, key: SectionKey) -> &mut Entry {
        let (always, born) = (self.table.is_none(), self.batch);
        let reached = &mut self.reached;
        self.sections.entry(key).or_insert_with(|| {
            *reached += usize::from(always);
            Entry {
                reached: always,
                born,
                ..Entry::default()
            }
        })
    }

    fn mark(&mut self, key: SectionKey, flags: u8) {
        self.entry(key);
        let e = self.sections.get_mut(&key).expect("just made");
        if e.dirty == 0 && e.reached {
            self.dirty_count += 1;
            self.unsorted += 1;
            self.fresh.push(key);
        }
        if e.since.is_none() {
            e.since = Some(self.now);
        }
        e.dirty |= flags.max(1);
        if flags & dirty::URGENT != 0 && !e.urgent {
            e.urgent = true;
            self.urgent.push_back(key);
        }
    }

    /// Forgets everything; the meshes go to `removed`.
    pub fn clear(&mut self, removed: &mut Vec<M>) {
        self.sections.clear();
        removed.extend(self.meshes.drain(..).map(|s| s.mesh));
        self.dirty_count = 0;
        self.reached = 0;
        // The graph is told of the level anew before anything else of it.
        self.announced = None;
        self.urgent.clear();
        self.queue.clear();
        self.fresh.clear();
        self.sorted_for = None;
        self.unsorted = 0;
    }

    /// Forgets a chunk's sections: by their keys, not by going through every section
    /// there is, as chunks leave by the row.
    fn remove_column(&mut self, x: i32, z: i32, removed: &mut Vec<M>) {
        let (lowest, count) = self.range;
        for y in lowest..lowest + count {
            let Some(e) = self.sections.remove(&[x, y, z]) else {
                continue;
            };
            self.dirty_count -= usize::from(e.dirty != 0 && e.reached);
            self.reached -= usize::from(e.reached);
            if let Some(index) = e.mesh {
                removed.push(self.take_mesh(index));
            }
        }
        if self.table.is_some() {
            self.events.push(Event::Unloaded { x, z });
        }
    }

    /// Whether a section of a chunk (by its place from the level's lowest) is nothing
    /// but air, as far as that can be told without looking at every block.
    fn is_air(&self, column: Option<&Arc<Column>>, index: usize) -> bool {
        let (Some(table), Some(column)) = (&self.table, column) else {
            return false;
        };
        column
            .sections()
            .get(index)
            .is_some_and(|p| matches!(p.blocks, BlockPlane::Uniform(s) if table.is_air(s)))
    }

    /// A section turned out to be nothing but air: it has nothing to draw, needs no
    /// job and connects everything.
    fn set_empty(&mut self, key: SectionKey, removed: &mut Vec<M>) {
        let now = self.now.at;
        let e = self.entry(key);
        e.empty = true;
        e.first_at.get_or_insert(now);
        e.stamp = None;
        e.since = None;
        e.connectivity = Some(Connectivity::ALL);
        let waiting = std::mem::take(&mut e.dirty) != 0 && e.reached;
        let mesh = e.mesh.take();
        self.dirty_count -= usize::from(waiting);
        if let Some(index) = mesh {
            removed.push(self.take_mesh(index));
        }
    }

    /// Goes through a loaded chunk's sections. `anew`: it is another load of the chunk,
    /// and all of them are marked dirty. Otherwise those that have a mesh are marked if
    /// the level no longer is what their mesh was made from (a chunk around them that
    /// has left since is no reason to mesh them again), and those without a mesh as
    /// wanting their first; the level around the chunk is read once for all of them.
    /// With cave culling, sections of air are set aside and the graph is told of the
    /// chunk.
    fn refresh_column(
        &mut self,
        scene: &Scene,
        x: i32,
        z: i32,
        flags: u8,
        anew: bool,
        removed: &mut Vec<M>,
    ) {
        let mut around = std::mem::take(&mut self.around);
        if !anew {
            scene.around(x, z, &mut around);
        }
        let culled = self.table.is_some();
        let column = if culled {
            scene.with_terrain(|t| t.column(x, z).cloned())
        } else {
            None
        };
        let (lowest, count) = self.range;
        let mut known = Vec::with_capacity(if culled { count as usize } else { 0 });
        for (index, y) in (lowest..lowest + count).enumerate() {
            let key = [x, y, z];
            if self.is_air(column.as_ref(), index) {
                self.set_empty(key, removed);
                known.push(Some(Connectivity::ALL));
                continue;
            }
            let entry = self.sections.get_mut(&key);
            let stale = anew
                || match entry.as_ref().and_then(|e| e.stamp.as_ref()) {
                    Some(stamp) => !around.is_current_where_loaded(stamp),
                    None => true,
                };
            if let Some(e) = entry {
                e.empty = false;
            }
            if stale {
                self.mark(key, flags);
            }
            if culled {
                known.push(self.sections.get(&key).and_then(|e| e.connectivity));
            }
        }
        if culled {
            self.events.push(Event::Chunk {
                x,
                z,
                sections: known,
            });
        }
        self.around = around;
    }

    /// Takes in what changed in the level since the last call. Meshes that must not
    /// be drawn any more (their chunk left, or the level is another) go to `removed`.
    pub fn sync(&mut self, scene: &Scene, removed: &mut Vec<M>) {
        let mut keys = std::mem::take(&mut self.keys);
        keys.clear();
        let drained = scene.drain_dirty(&mut keys);
        if drained.epoch != self.epoch {
            self.clear(removed);
            self.epoch = drained.epoch;
            self.stats.epochs += 1;
        }
        if drained.rescan.is_some() || !keys.is_empty() {
            let dims = scene.with_terrain(|t| t.dims());
            self.range = (dims.min_section, dims.sections);
            let level = Some((self.epoch, self.range));
            if self.table.is_some() && self.announced != level {
                self.announced = level;
                self.events.push(Event::Level {
                    lowest: self.range.0,
                    count: self.range.1,
                });
            }
        }
        if drained.rescan.is_some() {
            self.stats.rescans += 1;
            let columns =
                scene.with_terrain(|t| t.columns().map(|(k, _)| k).collect::<Vec<(i32, i32)>>());
            // What is tracked of chunks that are gone.
            let loaded: std::collections::HashSet<(i32, i32)> = columns.iter().copied().collect();
            let gone: Vec<(i32, i32)> = self
                .sections
                .keys()
                .map(|k| (k[0], k[2]))
                .filter(|c| !loaded.contains(c))
                .collect();
            for (x, z) in gone {
                self.remove_column(x, z, removed);
            }
            for (x, z) in columns {
                self.refresh_column(scene, x, z, dirty::LOADED, false, removed);
            }
        }
        if !keys.is_empty() {
            let (lowest, count) = self.range;
            let present = |x: i32, z: i32| scene.with_terrain(|t| t.column(x, z).is_some());
            for &(key, flags) in &keys {
                let (x, z) = (key[0], key[2]);
                if key[1] == WHOLE_COLUMN {
                    if !present(x, z) {
                        self.remove_column(x, z, removed);
                        continue;
                    }
                    // Another load of the chunk: whatever was made of the one before
                    // is of a chunk that is gone.
                    let anew = flags & (dirty::LOADED | dirty::UNLOADED) != 0;
                    self.refresh_column(scene, x, z, flags & !dirty::UNLOADED, anew, removed);
                } else if (lowest..lowest + count).contains(&key[1])
                    && (self.sections.contains_key(&key) || present(x, z))
                {
                    if self.sections.get(&key).is_some_and(|e| e.empty) {
                        // Whatever changed in a section of air shows only if it is one
                        // no longer.
                        let column = scene.with_terrain(|t| t.column(x, z).cloned());
                        if self.is_air(column.as_ref(), (key[1] - lowest) as usize) {
                            continue;
                        }
                        if let Some(e) = self.sections.get_mut(&key) {
                            e.empty = false;
                        }
                    }
                    self.mark(key, flags);
                }
            }
        }
        self.keys = keys;
    }

    fn sort(&mut self, camera: SectionKey) {
        // What waited already and what has become dirty since, without those that are
        // dirty no more or have a job out (they come back when it ends).
        self.queue.append(&mut self.fresh);
        let sections = &self.sections;
        self.queue
            .retain(|k| sections.get(k).is_some_and(|e| e.dirty != 0 && !e.meshing));
        // The nearest last, where they are taken from; ties in a fixed order.
        self.queue
            .sort_unstable_by_key(|k| (std::cmp::Reverse(distance(*k, camera)), *k));
        self.queue.dedup();
        self.sorted_for = Some(camera);
        self.unsorted = 0;
    }

    /// The next section to mesh, pinned as the level is now: the player's own changes
    /// first, then the dirty section nearest `camera` (the section the camera is in).
    /// `None` when nothing can be meshed now.
    pub fn next_job(&mut self, scene: &Scene, camera: SectionKey) -> Option<Job> {
        loop {
            let (key, urgent) = match self.urgent.pop_front() {
                Some(key) => (key, true),
                None => {
                    if self.sorted_for != Some(camera)
                        || self.unsorted >= RESORT_AFTER
                        || (self.queue.is_empty() && self.unsorted > 0)
                    {
                        self.sort(camera);
                    }
                    (self.queue.pop()?, false)
                }
            };
            let Some(e) = self.sections.get_mut(&key) else {
                continue;
            };
            if urgent {
                e.urgent = false;
            }
            if e.dirty == 0 || e.meshing {
                continue;
            }
            // Nothing reaches it: it comes back into the order of work when something
            // does. The player's own change is meshed wherever it is.
            if !e.reached && !urgent {
                continue;
            }
            let counted = usize::from(e.reached);
            let Some(neighbourhood) = scene.pin(key) else {
                // Its chunk is gone; the key that says so is on its way.
                e.dirty = 0;
                e.since = None;
                self.dirty_count -= counted;
                continue;
            };
            let surrounded =
                (-1..=1).all(|dz| (-1..=1).all(|dx| neighbourhood.column(dx, dz).is_some()));
            if e.stamp.is_none() && !surrounded {
                // It is marked again when a chunk next to its own arrives.
                e.dirty = 0;
                e.since = None;
                self.dirty_count -= counted;
                self.stats.waits_for_neighbours += 1;
                continue;
            }
            let first = e.stamp.is_none();
            for (cause, flag) in [
                dirty::BLOCKS,
                dirty::LIGHT,
                dirty::BIOMES,
                dirty::BLOCK_ENTITIES,
                dirty::NEIGHBOUR,
            ]
            .into_iter()
            .enumerate()
            {
                // A first mesh is that, whatever else has happened to the section.
                self.stats.causes[cause + 1] += u64::from(!first && e.dirty & flag != 0);
            }
            self.stats.causes[0] += u64::from(first);
            e.dirty = 0;
            self.dirty_count -= counted;
            e.meshing = true;
            let since = e.since.take().unwrap_or(self.now);
            e.in_flight = Some(since);
            self.stats.jobs += 1;
            self.stats.urgent_jobs += u64::from(urgent);
            return Some(Job {
                neighbourhood,
                urgent,
                first,
                since,
            });
        }
    }

    /// Whether a job's result may still be shown: the level is what it was made from,
    /// but for chunks around the section that have left meanwhile. A stale result's
    /// section is free for its next job at once, and dirty.
    pub fn accept(&mut self, scene: &Scene, stamp: &Stamp) -> bool {
        if stamp.epoch == self.epoch && scene.is_current_where_loaded(stamp) {
            return true;
        }
        self.stats.stale += 1;
        self.give_up(stamp.section);
        false
    }

    /// A job came to nothing (its result was stale, or could not be stored): the
    /// section is meshed again.
    pub fn give_up(&mut self, key: SectionKey) {
        if let Some(e) = self.sections.get_mut(&key) {
            e.meshing = false;
            // The change the job was for is still to be shown.
            if let Some(since) = e.in_flight.take() {
                e.since = Some(since);
            }
            let waiting = e.dirty != 0;
            self.mark(key, dirty::BLOCKS);
            // It was left out of the order of work while its job was out.
            self.unsorted += 1;
            if waiting {
                self.fresh.push(key);
            }
        }
    }

    /// Puts a job's result in place: `mesh` (none: the section draws nothing) is what
    /// is drawn from now on. Returns the mesh it replaces. A result for a section that
    /// is no longer tracked is handed back as it came.
    pub fn install(&mut self, stamp: Stamp, mesh: Option<M>) -> Option<M> {
        let now = self.now.at;
        let Some(e) = self.sections.get_mut(&stamp.section) else {
            return mesh;
        };
        let key = stamp.section;
        // A first result is one, with something to draw or without.
        let since = *e.first_at.get_or_insert(now);
        e.meshing = false;
        e.in_flight = None;
        // The stamp's place is kept from one mesh to the next.
        match &mut e.stamp {
            Some(kept) => **kept = stamp,
            None => e.stamp = Some(Box::new(stamp)),
        }
        if e.dirty != 0 {
            // It changed again meanwhile, and was left out of the order of work while
            // its job was out.
            self.unsorted += 1;
            self.fresh.push(key);
        }
        match (e.mesh, mesh) {
            (Some(index), Some(mesh)) => Some(std::mem::replace(
                &mut self.meshes[index as usize].mesh,
                mesh,
            )),
            (Some(index), None) => {
                e.mesh = None;
                Some(self.take_mesh(index))
            }
            (None, Some(mesh)) => {
                e.mesh = Some(self.meshes.len() as u32);
                self.meshes.push(Slot {
                    key,
                    shown: e.reached,
                    since,
                    mesh,
                });
                None
            }
            (None, None) => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::testing::{self, World};
    use super::*;

    const CAMERA: SectionKey = [0, 0, 0];

    /// Runs every job there is, storing the section's key as its mesh.
    fn mesh_all(t: &mut Tracker<SectionKey>, world: &World) -> Vec<SectionKey> {
        let mut done = Vec::new();
        while let Some(job) = t.next_job(&world.scene, CAMERA) {
            let stamp = job.neighbourhood.stamp().clone();
            assert!(t.accept(&world.scene, &stamp));
            // A section meshed again replaces its own mesh.
            let old = t.install(stamp, Some(job.neighbourhood.section()));
            assert!(old.is_none() || old == Some(job.neighbourhood.section()));
            done.push(job.neighbourhood.section());
        }
        done
    }

    fn block(world: &World) -> (i32, i32, i32) {
        let _ = world;
        (3, 3, 3)
    }

    fn surrounded() -> Vec<(i32, i32)> {
        (-1..=1)
            .flat_map(|z| (-1..=1).map(move |x| (x, z)))
            .collect()
    }

    #[test]
    fn a_chunk_is_meshed_once_the_chunks_around_it_are_there() {
        let mut world = World::new(&[(0, 0)]);
        world.publish();
        let mut t: Tracker<SectionKey> = Tracker::new();
        let mut removed = Vec::new();
        t.sync(&world.scene, &mut removed);
        // Its four sections are known and wait for the neighbours.
        assert_eq!(t.counts(), (4, 0, 4));
        assert!(t.next_job(&world.scene, CAMERA).is_none());
        assert_eq!(t.counts(), (4, 0, 0));
        assert_eq!(t.stats().waits_for_neighbours, 4);
        // Seven neighbours are not enough; the eighth is.
        for (x, z) in surrounded()
            .into_iter()
            .filter(|c| *c != (0, 0) && *c != (1, 1))
        {
            world.load(x, z);
        }
        world.publish();
        t.sync(&world.scene, &mut removed);
        assert!(t.next_job(&world.scene, CAMERA).is_none());
        world.load(1, 1);
        world.publish();
        t.sync(&world.scene, &mut removed);
        let done = mesh_all(&mut t, &world);
        // Only the middle chunk has all its neighbours: its four sections, the nearest
        // to the camera's first.
        assert_eq!(done.len(), 4);
        assert!(done.iter().all(|k| (k[0], k[2]) == (0, 0)));
        assert_eq!(done[0], [0, 0, 0]);
        assert_eq!(t.counts().1, 4);
        assert!(removed.is_empty());
    }

    #[test]
    fn a_change_is_meshed_again_with_its_neighbours_and_the_players_own_first() {
        let mut world = World::new(&surrounded());
        world.publish();
        let mut t: Tracker<SectionKey> = Tracker::new();
        let mut removed = Vec::new();
        t.sync(&world.scene, &mut removed);
        mesh_all(&mut t, &world);
        assert!(t.next_job(&world.scene, CAMERA).is_none());

        // The first block of an empty section: that section, and the sections around it
        // that get light planes for it.
        let (x, y, z) = block(&world);
        world.set(x, y, z, testing::stone());
        world.publish();
        t.sync(&world.scene, &mut removed);
        assert!(mesh_all(&mut t, &world).contains(&[0, 0, 0]));

        // Another block inside the section: that section alone.
        world.set(x + 1, y, z, testing::stone());
        world.publish();
        t.sync(&world.scene, &mut removed);
        assert_eq!(mesh_all(&mut t, &world), vec![[0, 0, 0]]);

        // A block in a section's corner: the sections it touches too, of which only the
        // middle chunk's have a mesh to bring up to date (the others still wait for
        // their own neighbours and are not meshed for it).
        world.set(0, 0, 0, testing::stone());
        world.publish();
        t.sync(&world.scene, &mut removed);
        let done = mesh_all(&mut t, &world);
        assert!(done.contains(&[0, 0, 0]) && done.contains(&[0, -1, 0]));
        assert!(done.iter().all(|k| (k[0], k[2]) == (0, 0)));
        assert!(removed.is_empty());
    }

    #[test]
    fn a_result_made_from_an_older_level_is_thrown_away() {
        let mut world = World::new(&surrounded());
        world.publish();
        let mut t: Tracker<SectionKey> = Tracker::new();
        let mut removed = Vec::new();
        t.sync(&world.scene, &mut removed);
        let job = t.next_job(&world.scene, CAMERA).unwrap();
        assert_eq!(job.neighbourhood.section(), [0, 0, 0]);
        // The level changes while the job runs.
        world.set(1, 1, 1, testing::stone());
        world.publish();
        t.sync(&world.scene, &mut removed);
        // No second job for the section while the first is out.
        let others = mesh_all(&mut t, &world);
        assert!(!others.contains(&[0, 0, 0]));
        let stamp = job.neighbourhood.stamp().clone();
        assert!(!t.accept(&world.scene, &stamp));
        assert_eq!(t.stats().stale, 1);
        // It is meshed again, from the level as it is now.
        assert_eq!(mesh_all(&mut t, &world), vec![[0, 0, 0]]);
    }

    #[test]
    fn a_chunk_that_leaves_takes_its_meshes_at_once() {
        let mut world = World::new(&surrounded());
        world.publish();
        let mut t: Tracker<SectionKey> = Tracker::new();
        let mut removed = Vec::new();
        t.sync(&world.scene, &mut removed);
        mesh_all(&mut t, &world);
        world.level.forget_chunk(0, 0);
        world.publish();
        t.sync(&world.scene, &mut removed);
        removed.sort_unstable();
        assert_eq!(removed, vec![[0, -1, 0], [0, 0, 0], [0, 1, 0], [0, 2, 0]]);
        assert_eq!(t.counts().1, 0);
        assert_eq!(t.meshes().count(), 0);
        // It comes back as another chunk: meshed anew.
        removed.clear();
        world.load(0, 0);
        world.publish();
        t.sync(&world.scene, &mut removed);
        assert_eq!(mesh_all(&mut t, &world).len(), 4);
        assert!(removed.is_empty());
    }

    /// Since when a section has been there to draw, if it has a mesh.
    fn since(t: &Tracker<SectionKey>, key: SectionKey) -> Option<Instant> {
        t.meshes_since()
            .find(|(k, ..)| *k == key)
            .map(|(.., at)| at)
    }

    #[test]
    fn a_section_fades_from_its_first_result_whatever_is_meshed_there_later() {
        let mut world = World::new(&surrounded());
        world.publish();
        let mut t: Tracker<SectionKey> = Tracker::new();
        let mut removed = Vec::new();
        let start = Instant::now();
        let at = |ms: u64| Since {
            at: start + std::time::Duration::from_millis(ms),
            frame: ms,
        };
        t.set_clock(at(100));
        t.sync(&world.scene, &mut removed);
        // The camera's section gets a result with nothing to draw, the others meshes.
        let job = t.next_job(&world.scene, CAMERA).unwrap();
        assert_eq!(job.neighbourhood.section(), [0, 0, 0]);
        assert!(t.install(job.neighbourhood.stamp().clone(), None).is_none());
        assert_eq!(since(&t, [0, 0, 0]), None);
        t.set_clock(at(200));
        mesh_all(&mut t, &world);
        assert_eq!(since(&t, [0, 1, 0]), Some(at(200).at));
        // Meshed again later, each counts from its first result: the one that had
        // nothing to draw too.
        t.set_clock(at(900));
        world.set(1, 1, 1, testing::stone());
        world.set(1, 17, 1, testing::stone());
        world.publish();
        t.sync(&world.scene, &mut removed);
        let again = mesh_all(&mut t, &world);
        assert!(again.contains(&[0, 0, 0]) && again.contains(&[0, 1, 0]));
        assert_eq!(since(&t, [0, 0, 0]), Some(at(100).at));
        assert_eq!(since(&t, [0, 1, 0]), Some(at(200).at));
        // A chunk that left and came back fades in anew.
        world.level.forget_chunk(0, 0);
        world.publish();
        t.sync(&world.scene, &mut removed);
        world.load(0, 0);
        world.publish();
        t.set_clock(at(5000));
        t.sync(&world.scene, &mut removed);
        mesh_all(&mut t, &world);
        assert_eq!(since(&t, [0, 0, 0]), Some(at(5000).at));
    }

    #[test]
    fn a_section_of_air_counts_from_when_it_was_found_to_be_air() {
        let mut world = World::new(&surrounded());
        let mut c = Culled::new([0, 0, 0]);
        let start = Instant::now();
        let at = |ms: u64| Since {
            at: start + std::time::Duration::from_millis(ms),
            frame: ms,
        };
        c.t.set_clock(at(100));
        c.sync(&mut world);
        c.mesh_all(&world);
        assert_eq!(since(&c.t, [0, 0, 0]), None);
        // Its first block, much later: the section has been there all along, and the
        // block is not faded in.
        c.t.set_clock(at(4000));
        let (x, y, z) = block(&world);
        world.set(x, y, z, testing::stone());
        c.sync(&mut world);
        assert!(c.mesh_all(&world).contains(&[0, 0, 0]));
        assert_eq!(since(&c.t, [0, 0, 0]), Some(at(100).at));
    }

    #[test]
    fn a_chunk_leaving_next_door_is_no_reason_to_mesh_again_but_its_return_is() {
        let mut world = World::new(&surrounded());
        world.publish();
        let mut t: Tracker<SectionKey> = Tracker::new();
        let mut removed = Vec::new();
        t.sync(&world.scene, &mut removed);
        assert_eq!(mesh_all(&mut t, &world).len(), 4);
        let jobs = t.stats().jobs;
        // The chunk to the east leaves: the meshes made with it stay as they are.
        world.level.forget_chunk(1, 0);
        world.publish();
        t.sync(&world.scene, &mut removed);
        assert!(mesh_all(&mut t, &world).is_empty());
        assert_eq!((t.stats().jobs, t.counts().1), (jobs, 4));
        // A result that was on its way when the chunk left is still good.
        world.set(3, 3, 3, 1);
        world.publish();
        t.sync(&world.scene, &mut removed);
        let job = t.next_job(&world.scene, CAMERA).unwrap();
        world.level.forget_chunk(-1, 0);
        world.publish();
        t.sync(&world.scene, &mut removed);
        let stamp = job.neighbourhood.stamp().clone();
        assert!(t.accept(&world.scene, &stamp));
        t.install(stamp, Some([0, 0, 0]));
        assert_eq!(t.stats().stale, 0);
        mesh_all(&mut t, &world);
        // The eastern chunk comes back, which is another load of it: what was made
        // with the one before is made again.
        let jobs = t.stats().jobs;
        world.load(1, 0);
        world.publish();
        t.sync(&world.scene, &mut removed);
        let again = mesh_all(&mut t, &world);
        assert_eq!(again.len(), 4, "{again:?}");
        assert_eq!(t.stats().jobs, jobs + 4);
        assert!(removed.is_empty());
    }

    #[test]
    fn the_meshes_to_draw_stay_those_of_the_sections_that_have_one() {
        let chunks: Vec<(i32, i32)> = (-2..=2)
            .flat_map(|z| (-2..=2).map(move |x| (x, z)))
            .collect();
        let mut world = World::new(&chunks);
        world.publish();
        let mut t: Tracker<SectionKey> = Tracker::new();
        let mut removed = Vec::new();
        t.sync(&world.scene, &mut removed);
        // The nine chunks in the middle have all their neighbours.
        assert_eq!(mesh_all(&mut t, &world).len(), 9 * 4);
        let drawn = |t: &Tracker<SectionKey>| {
            let mut keys: Vec<SectionKey> = t
                .meshes()
                .map(|(key, mesh)| {
                    assert_eq!(key, *mesh, "a mesh under another section's key");
                    key
                })
                .collect();
            keys.sort_unstable();
            keys
        };
        assert_eq!(drawn(&t).len(), 36);
        // Chunks leave from the middle of the list, one by one; a section's mesh goes
        // for nothing to draw; another comes back.
        for (gone, left) in [((0, 0), 32), ((-1, 1), 28), ((1, -1), 24)] {
            world.level.forget_chunk(gone.0, gone.1);
            world.publish();
            t.sync(&world.scene, &mut removed);
            let keys = drawn(&t);
            assert_eq!(keys.len(), left);
            assert!(keys.iter().all(|k| (k[0], k[2]) != gone));
            assert_eq!(t.counts().1, left);
        }
        assert_eq!(removed.len(), 12);
        // (Whatever the leaving chunks changed in the light next to them.)
        mesh_all(&mut t, &world);
        assert_eq!(drawn(&t).len(), 24);
        world.set(-13, 3, -13, 1);
        world.publish();
        t.sync(&world.scene, &mut removed);
        // The changed section's job, among whatever else the change made dirty (light
        // reaches further than the block).
        let job = loop {
            let job = t.next_job(&world.scene, CAMERA).unwrap();
            if job.neighbourhood.section() == [-1, 0, -1] {
                break job;
            }
            let stamp = job.neighbourhood.stamp().clone();
            t.install(stamp, Some(job.neighbourhood.section()));
        };
        let stamp = job.neighbourhood.stamp().clone();
        assert!(!job.first);
        assert_eq!(t.install(stamp, None), Some([-1, 0, -1]));
        let keys = drawn(&t);
        assert_eq!(keys.len(), 23);
        assert!(!keys.contains(&[-1, 0, -1]));
        // Why the jobs were handed out: 36 first meshes, and a changed block.
        let causes = t.stats().causes;
        assert_eq!(causes[0], 36);
        assert!(causes[1] >= 1);
    }

    #[test]
    fn a_job_says_since_when_its_change_was_known() {
        let mut world = World::new(&surrounded());
        world.publish();
        let mut t: Tracker<SectionKey> = Tracker::new();
        let mut removed = Vec::new();
        let at = Instant::now();
        t.set_clock(Since { at, frame: 7 });
        t.sync(&world.scene, &mut removed);
        t.set_clock(Since { at, frame: 9 });
        let job = t.next_job(&world.scene, CAMERA).unwrap();
        assert!(job.first);
        assert_eq!(job.since.frame, 7, "known since the frame it was taken in");
        let stamp = job.neighbourhood.stamp().clone();
        // The level changes under the job: its result is thrown away, and the next job
        // for the section still counts from the first change.
        world.set(3, job.neighbourhood.section()[1] * 16 + 3, 3, 1);
        world.publish();
        t.set_clock(Since { at, frame: 12 });
        t.sync(&world.scene, &mut removed);
        assert!(!t.accept(&world.scene, &stamp));
        let key = stamp.section;
        let again = loop {
            let job = t.next_job(&world.scene, CAMERA).unwrap();
            if job.neighbourhood.section() == key {
                break job;
            }
        };
        assert_eq!(again.since.frame, 7);
        // Once shown, a later change counts from when it came.
        let stamp = again.neighbourhood.stamp().clone();
        t.install(stamp, Some(key));
        world.set(3, key[1] * 16 + 4, 3, 1);
        world.publish();
        t.set_clock(Since { at, frame: 20 });
        t.sync(&world.scene, &mut removed);
        let later = loop {
            let job = t.next_job(&world.scene, CAMERA).unwrap();
            if job.neighbourhood.section() == key {
                break job;
            }
        };
        assert_eq!((later.since.frame, later.first), (20, false));
    }

    #[test]
    fn a_new_level_empties_the_tracker_and_a_rescan_finds_what_keys_missed() {
        let mut world = World::new(&surrounded());
        world.publish();
        let mut t: Tracker<SectionKey> = Tracker::new();
        let mut removed = Vec::new();
        t.sync(&world.scene, &mut removed);
        mesh_all(&mut t, &world);
        assert_eq!(t.stats().epochs, 1);

        // A job is out when the player changes dimension.
        world.set(1, 1, 1, testing::stone());
        world.publish();
        t.sync(&world.scene, &mut removed);
        let job = t.next_job(&world.scene, CAMERA).unwrap();
        world
            .publisher
            .begin_level(ruthenium_sim::scene::Dims::of(&world.level));
        t.sync(&world.scene, &mut removed);
        assert_eq!(removed.len(), 4);
        assert_eq!(t.counts(), (0, 0, 0));
        assert_eq!(t.stats().epochs, 2);
        // Its result is of the old level.
        let stamp = job.neighbourhood.stamp().clone();
        assert!(!t.accept(&world.scene, &stamp));
        assert_eq!(t.install(stamp, Some([9, 9, 9])), Some([9, 9, 9]));
        assert_eq!(t.meshes().count(), 0);

        // A queue too small for a burst of changes: everything is compared again and
        // exactly the changed sections are found.
        let mut world = World::new(&surrounded());
        world.publish();
        let mut t: Tracker<SectionKey> = Tracker::new();
        let mut removed = Vec::new();
        t.sync(&world.scene, &mut removed);
        mesh_all(&mut t, &world);
        let before = t.stats().rescans;
        let (publisher, scene) = Scene::new(2);
        world.publisher = publisher;
        world.scene = scene;
        world.level.track_changes(4096);
        world
            .publisher
            .begin_level(ruthenium_sim::scene::Dims::of(&world.level));
        world.set(3, 3, 3, testing::stone());
        world.set(3, 20, 3, testing::stone());
        world.set(3, 36, 3, testing::stone());
        world.publish();
        removed.clear();
        t.sync(&world.scene, &mut removed);
        assert!(t.stats().rescans > before);
        // (The new scene is another level to the tracker: everything is meshed anew.)
        let done = mesh_all(&mut t, &world);
        assert_eq!(done.len(), 4);
    }

    #[test]
    fn the_nearest_dirty_section_comes_first_and_urgent_ones_before_it() {
        let chunks: Vec<(i32, i32)> = (-1..=4)
            .flat_map(|z| (-1..=1).map(move |x| (x, z)))
            .collect();
        let mut world = World::new(&chunks);
        world.publish();
        let mut t: Tracker<SectionKey> = Tracker::new();
        let mut removed = Vec::new();
        t.sync(&world.scene, &mut removed);
        // Seen from the chunk at z 3, its sections come before those at z 0.
        let first = t.next_job(&world.scene, [0, 0, 3]).unwrap();
        assert_eq!(first.neighbourhood.section(), [0, 0, 3]);
        assert!(!first.urgent);
        let stamp = first.neighbourhood.stamp().clone();
        t.install(stamp, None);
        let mut order = Vec::new();
        while let Some(job) = t.next_job(&world.scene, [0, 0, 3]) {
            order.push(job.neighbourhood.section());
            t.install(job.neighbourhood.stamp().clone(), None);
        }
        let at = |k: SectionKey| order.iter().position(|o| *o == k).unwrap();
        assert!(at([0, 0, 2]) < at([0, 0, 0]));
        assert!(at([0, 1, 3]) < at([0, 0, 1]));
        assert_eq!(t.stats().urgent_jobs, 0);
        // An empty mesh is a mesh: nothing is dirty any more, and nothing is drawn.
        assert_eq!(t.counts().2, 0);
        assert_eq!(t.meshes().count(), 0);
    }

    use super::super::mesher::{MeshOptions, Mesher};
    use super::super::tint::BiomeTints;
    use super::super::visibility::{Batch, Camera, Graph};

    /// A tracker with cave culling and its graph, which answers at once.
    struct Culled {
        t: Tracker<SectionKey>,
        graph: Graph,
        mesher: Mesher,
        seq: u64,
        camera: SectionKey,
        removed: Vec<SectionKey>,
    }

    impl Culled {
        fn new(camera: SectionKey) -> Culled {
            let table = testing::table();
            let mut t = Tracker::culled(table.clone());
            t.set_batch(1);
            Culled {
                t,
                graph: Graph::new(),
                mesher: Mesher::new(table),
                seq: 0,
                camera,
                removed: Vec::new(),
            }
        }

        /// Sends the graph what the tracker has for it and takes in its answer.
        fn tell(&mut self) {
            self.seq += 1;
            self.graph.take(Batch {
                seq: self.seq,
                camera: Camera {
                    position: self.camera.map(|s| f64::from(s * 16) + 8.5),
                    smart: true,
                    view_distance: 8,
                },
                events: self.t.take_events(),
            });
            let delta = self.graph.settle();
            self.t.reach(&delta);
            self.t.set_batch(self.seq + 1);
        }

        /// Publishes the level and takes in what changed.
        fn sync(&mut self, world: &mut World) {
            world.publish();
            self.t.sync(&world.scene, &mut self.removed);
            self.tell();
        }

        /// Runs jobs, really meshing, until there are none: each result's connectivity
        /// goes to the graph as its mesh is on its way and again as it is drawn.
        fn mesh_all(&mut self, world: &World) -> Vec<SectionKey> {
            let options = MeshOptions {
                shades: [1.0; 6],
                smooth_lighting: true,
                biome_blend: 2,
                cutout_leaves: true,
            };
            let mut done = Vec::new();
            while let Some(job) = self.t.next_job(&world.scene, self.camera) {
                let key = job.neighbourhood.section();
                let mesh = self
                    .mesher
                    .mesh(&job.neighbourhood, &options, &BiomeTints::default());
                let stamp = job.neighbourhood.stamp().clone();
                assert!(self.t.accept(&world.scene, &stamp));
                self.t.connect(key, mesh.connectivity, false);
                self.tell();
                self.t.install(stamp, (mesh.quads() > 0).then_some(key));
                self.t.connect(key, mesh.connectivity, true);
                self.tell();
                done.push(key);
            }
            done.sort_unstable();
            done
        }

        fn drawn(&self) -> Vec<SectionKey> {
            let mut keys: Vec<SectionKey> = self.t.meshes().map(|(k, _)| k).collect();
            keys.sort_unstable();
            keys
        }
    }

    fn fill(world: &mut World, section: SectionKey, state: u32) {
        for y in 0..16 {
            for z in 0..16 {
                for x in 0..16 {
                    world.set(
                        section[0] * 16 + x,
                        section[1] * 16 + y,
                        section[2] * 16 + z,
                        state,
                    );
                }
            }
        }
    }

    /// Five by five chunks; in the middle three by three, a layer of stone sections at
    /// section y 0 with a block in each section under it.
    fn roofed() -> (World, Vec<SectionKey>, Vec<SectionKey>) {
        let chunks: Vec<(i32, i32)> = (-2..=2)
            .flat_map(|z| (-2..=2).map(move |x| (x, z)))
            .collect();
        let mut world = World::new(&chunks);
        let (mut roof, mut under) = (Vec::new(), Vec::new());
        for z in -1..=1 {
            for x in -1..=1 {
                fill(&mut world, [x, 0, z], testing::stone());
                world.set(x * 16 + 5, -10, z * 16 + 5, testing::stone());
                roof.push([x, 0, z]);
                under.push([x, -1, z]);
            }
        }
        roof.sort_unstable();
        under.sort_unstable();
        (world, roof, under)
    }

    #[test]
    fn air_needs_no_mesh_and_lets_the_camera_see_everything() {
        let mut world = World::new(&surrounded());
        let mut c = Culled::new([0, 1, 0]);
        c.sync(&mut world);
        // Nothing to mesh, and everything loaded is reached.
        assert!(c.mesh_all(&world).is_empty());
        assert_eq!(c.t.counts(), (36, 0, 0));
        assert_eq!(c.t.reached(), 36);
        assert_eq!(c.t.stats().jobs, 0);
        // Its first block gives a section a mesh; light changing around it in sections
        // that are still air gives them none.
        world.set(3, 3, 3, testing::stone());
        c.sync(&mut world);
        assert_eq!(c.mesh_all(&world), vec![[0, 0, 0]]);
        assert_eq!(c.drawn(), vec![[0, 0, 0]]);
        // The block goes again: the section's mesh does too.
        world.set(3, 3, 3, 0);
        c.sync(&mut world);
        assert_eq!(c.mesh_all(&world), vec![[0, 0, 0]]);
        assert!(c.drawn().is_empty());
        assert_eq!(c.t.reached(), 36);
        // A chunk sent again with nothing in it where a mesh was: the mesh goes at
        // once, with no job.
        world.set(3, 3, 3, testing::stone());
        c.sync(&mut world);
        c.mesh_all(&world);
        assert_eq!(c.drawn(), vec![[0, 0, 0]]);
        let jobs = c.t.stats().jobs;
        assert!(c.removed.is_empty());
        world.load(0, 0);
        c.sync(&mut world);
        assert_eq!(c.removed, vec![[0, 0, 0]]);
        assert!(c.mesh_all(&world).is_empty() && c.drawn().is_empty());
        assert_eq!((c.t.stats().jobs, c.t.reached()), (jobs, 36));
    }

    #[test]
    fn what_a_solid_layer_hides_is_not_meshed_until_it_opens() {
        let (mut world, roof, under) = roofed();
        let mut c = Culled::new([0, 1, 0]);
        c.sync(&mut world);
        // The roof is seen from above and meshed; under it nothing is reached, from
        // the sides neither (the walk would have to turn back).
        assert_eq!(c.mesh_all(&world), roof);
        assert_eq!(c.drawn(), roof);
        assert_eq!(c.t.counts().2, 0, "nothing reached waits for a mesh");
        // The same world without culling meshes what is under it too.
        let (mut same, ..) = roofed();
        same.publish();
        let mut plain: Tracker<SectionKey> = Tracker::new();
        plain.sync(&same.scene, &mut Vec::new());
        let all = mesh_all(&mut plain, &same);
        assert!(under.iter().all(|k| all.contains(k)));

        // A change under the roof waits, dirty.
        let at = Instant::now();
        c.t.set_clock(Since { at, frame: 3 });
        world.set(21, -12, 5, testing::stone());
        c.sync(&mut world);
        assert!(c.mesh_all(&world).is_empty());
        assert_eq!(c.t.counts().2, 0);
        // It counts as waiting to be shown from the moment it is reached.
        c.t.set_clock(Since { at, frame: 40 });

        // A shaft through the roof's middle section: what is under it is reached from
        // there on, and meshed.
        for y in 0..16 {
            world.set(8, y, 8, 0);
        }
        c.sync(&mut world);
        let shaft = c.t.next_job(&world.scene, c.camera).unwrap();
        assert_eq!(shaft.neighbourhood.section(), [0, 0, 0]);
        let mesh = c.mesher.mesh(
            &shaft.neighbourhood,
            &MeshOptions {
                shades: [1.0; 6],
                smooth_lighting: true,
                biome_blend: 2,
                cutout_leaves: true,
            },
            &BiomeTints::default(),
        );
        c.t.set_clock(Since { at, frame: 41 });
        c.t.connect([0, 0, 0], mesh.connectivity, false);
        c.tell();
        c.t.install(shaft.neighbourhood.stamp().clone(), Some([0, 0, 0]));
        // The section right under the shaft, known to wait since it was reached; the
        // others follow when its mesh says what it connects.
        let below = c.t.next_job(&world.scene, c.camera).unwrap();
        assert_eq!(below.neighbourhood.section(), [0, -1, 0]);
        assert_eq!((below.first, below.since.frame), (true, 41));
        assert!(c.t.next_job(&world.scene, c.camera).is_none());
        c.t.give_up([0, -1, 0]);
        assert_eq!(c.mesh_all(&world), under);
        let mut both = [roof.clone(), under.clone()].concat();
        both.sort_unstable();
        assert_eq!(c.drawn(), both);

        // The shaft is filled again: the meshes under the roof stay, and are not drawn.
        for y in 0..16 {
            world.set(8, y, 8, testing::stone());
        }
        c.sync(&mut world);
        assert_eq!(c.mesh_all(&world), vec![[0, 0, 0]]);
        assert_eq!(c.drawn(), roof);
        assert_eq!(c.t.counts().1, 18);
        assert_eq!(c.t.all_meshes().count(), 18);
        assert!(c.removed.is_empty());
    }

    #[test]
    fn a_new_mesh_opens_at_once_and_closes_when_it_is_drawn() {
        let (mut world, _, under) = roofed();
        let mut c = Culled::new([0, 1, 0]);
        c.sync(&mut world);
        c.mesh_all(&world);
        let reached = c.t.reached();
        // The shaft, step by step: the result alone, not yet drawn, lets the walk go on.
        for y in 0..16 {
            world.set(8, y, 8, 0);
        }
        c.sync(&mut world);
        let job = c.t.next_job(&world.scene, c.camera).unwrap();
        assert_eq!(job.neighbourhood.section(), [0, 0, 0]);
        let open = c
            .mesher
            .mesh(
                &job.neighbourhood,
                &MeshOptions {
                    shades: [1.0; 6],
                    smooth_lighting: true,
                    biome_blend: 2,
                    cutout_leaves: true,
                },
                &BiomeTints::default(),
            )
            .connectivity;
        assert!(open.sees(0, 1));
        c.t.connect([0, 0, 0], open, false);
        c.tell();
        assert!(c.t.reached() > reached);
        c.t.install(job.neighbourhood.stamp().clone(), Some([0, 0, 0]));
        c.t.connect([0, 0, 0], open, true);
        c.tell();
        c.mesh_all(&world);
        let opened = c.t.reached();
        // Filled again: until the new mesh is drawn, the old one's shaft stays open.
        for y in 0..16 {
            world.set(8, y, 8, testing::stone());
        }
        c.sync(&mut world);
        let job = c.t.next_job(&world.scene, c.camera).unwrap();
        c.t.connect([0, 0, 0], Connectivity::NONE, false);
        c.tell();
        assert_eq!(c.t.reached(), opened);
        c.t.install(job.neighbourhood.stamp().clone(), Some([0, 0, 0]));
        c.t.connect([0, 0, 0], Connectivity::NONE, true);
        c.tell();
        assert_eq!(c.t.reached(), reached);
        assert!(under.iter().all(|k| !c.drawn().contains(k)));
    }

    #[test]
    fn the_players_own_change_is_meshed_where_nothing_reaches_it() {
        let (mut world, _, under) = roofed();
        let mut c = Culled::new([0, 1, 0]);
        c.sync(&mut world);
        c.mesh_all(&world);
        c.t.mark(under[4], dirty::BLOCKS | dirty::URGENT);
        let job = c.t.next_job(&world.scene, c.camera).unwrap();
        assert!(job.urgent);
        assert_eq!(job.neighbourhood.section(), under[4]);
        // It was never counted as waiting: nothing reached waited.
        assert_eq!(c.t.counts().2, 0);
    }

    #[test]
    fn what_the_graph_said_of_a_chunk_that_left_is_not_taken_for_its_successor() {
        let mut world = World::new(&surrounded());
        world.set(3, 3, 3, testing::stone());
        let mut c = Culled::new([0, 1, 0]);
        c.sync(&mut world);
        // Under the section that has yet to be meshed, nothing is reached.
        assert_eq!(c.t.reached(), 35);
        assert_eq!(c.mesh_all(&world), vec![[0, 0, 0]]);
        assert_eq!(c.t.reached(), 36);
        // The chunk leaves and comes back between two answers.
        world.level.forget_chunk(0, 0);
        world.publish();
        c.t.sync(&world.scene, &mut c.removed);
        world.load(0, 0);
        world.set(3, 3, 3, testing::stone());
        world.publish();
        c.t.sync(&world.scene, &mut c.removed);
        assert_eq!(c.t.reached(), 32);
        // An answer from before the graph heard of either is of the chunk that left.
        c.t.reach(&Delta {
            seq: c.seq,
            added: vec![[0, 0, 0]],
            ..Delta::default()
        });
        assert_eq!(c.t.reached(), 32);
        // The graph's own answer puts the new one in its place: where the walk had been
        // before the chunk left, it still is.
        c.tell();
        assert_eq!(c.t.reached(), 36);
        assert_eq!(c.mesh_all(&world), vec![[0, 0, 0]]);
        assert_eq!(c.t.reached(), 36);
    }

    #[test]
    fn another_level_starts_the_graph_again() {
        let (mut world, roof, _) = roofed();
        let mut c = Culled::new([0, 1, 0]);
        c.sync(&mut world);
        c.mesh_all(&world);
        world
            .publisher
            .begin_level(ruthenium_sim::scene::Dims::of(&world.level));
        c.t.sync(&world.scene, &mut c.removed);
        assert_eq!((c.t.counts(), c.t.reached()), ((0, 0, 0), 0));
        assert_eq!(c.removed.len(), roof.len());
        // The level is published again as the new one: told from the start.
        let (mut world, roof, _) = roofed();
        let (publisher, scene) = Scene::new(4096);
        world.publisher = publisher;
        world.scene = scene;
        world.level.track_changes(4096);
        world
            .publisher
            .begin_level(ruthenium_sim::scene::Dims::of(&world.level));
        c.sync(&mut world);
        assert_eq!(c.mesh_all(&world), roof);
    }
}

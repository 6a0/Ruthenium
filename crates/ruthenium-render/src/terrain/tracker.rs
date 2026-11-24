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

use std::collections::{HashMap, VecDeque};
use std::time::Instant;

use ruthenium_sim::scene::{Around, Neighbourhood, Scene, SectionKey, Stamp, WHOLE_COLUMN, dirty};

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
    /// Every mesh to draw with its section, in no order: what a frame goes through.
    meshes: Vec<(SectionKey, M)>,
    /// Sections whose `dirty` is set.
    dirty_count: usize,
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
    pub fn new() -> Tracker<M> {
        Tracker {
            epoch: 0,
            sections: HashMap::new(),
            meshes: Vec::new(),
            dirty_count: 0,
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
    /// known since then, which is what a job's [`Job::since`] says.
    pub fn set_clock(&mut self, now: Since) {
        self.now = now;
    }

    pub fn stats(&self) -> TrackerStats {
        self.stats
    }

    /// Sections of loaded chunks, sections with a mesh to draw, and sections waiting
    /// for a mesh.
    pub fn counts(&self) -> (usize, usize, usize) {
        (self.sections.len(), self.meshes.len(), self.dirty_count)
    }

    /// Every mesh to draw.
    pub fn meshes(&self) -> impl Iterator<Item = (SectionKey, &M)> + '_ {
        self.meshes.iter().map(|(k, m)| (*k, m))
    }

    /// Takes the mesh at `index` out; the mesh that takes its place is told where it
    /// is now.
    fn take_mesh(&mut self, index: u32) -> M {
        let (_, mesh) = self.meshes.swap_remove(index as usize);
        if let Some((moved, _)) = self.meshes.get(index as usize)
            && let Some(e) = self.sections.get_mut(moved)
        {
            e.mesh = Some(index);
        }
        mesh
    }

    fn mark(&mut self, key: SectionKey, flags: u8) {
        let e = self.sections.entry(key).or_default();
        if e.dirty == 0 {
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
        removed.extend(self.meshes.drain(..).map(|(_, m)| m));
        self.dirty_count = 0;
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
            self.dirty_count -= usize::from(e.dirty != 0);
            if let Some(index) = e.mesh {
                removed.push(self.take_mesh(index));
            }
        }
    }

    /// Marks the sections of a chunk that have a mesh dirty if the level no longer is
    /// what their mesh was made from (a chunk around them that has left since is no
    /// reason to mesh them again), and those without a mesh as wanting their first. The
    /// level around the chunk is read once for all of them.
    fn mark_stale_in_column(&mut self, scene: &Scene, x: i32, z: i32, flags: u8) {
        let mut around = std::mem::take(&mut self.around);
        scene.around(x, z, &mut around);
        let (lowest, count) = self.range;
        for y in lowest..lowest + count {
            let key = [x, y, z];
            let stale = match self.sections.get(&key).and_then(|e| e.stamp.as_ref()) {
                Some(stamp) => !around.is_current_where_loaded(stamp),
                None => true,
            };
            if stale {
                self.mark(key, flags);
            }
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
                self.mark_stale_in_column(scene, x, z, dirty::LOADED);
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
                    if flags & (dirty::LOADED | dirty::UNLOADED) != 0 {
                        // Another load of the chunk: whatever was made of the one
                        // before is of a chunk that is gone.
                        for y in lowest..lowest + count {
                            self.mark([x, y, z], flags & !dirty::UNLOADED);
                        }
                    } else {
                        self.mark_stale_in_column(scene, x, z, flags);
                    }
                } else if (lowest..lowest + count).contains(&key[1])
                    && (self.sections.contains_key(&key) || present(x, z))
                {
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
            let Some(neighbourhood) = scene.pin(key) else {
                // Its chunk is gone; the key that says so is on its way.
                e.dirty = 0;
                e.since = None;
                self.dirty_count -= 1;
                continue;
            };
            let surrounded =
                (-1..=1).all(|dz| (-1..=1).all(|dx| neighbourhood.column(dx, dz).is_some()));
            if e.stamp.is_none() && !surrounded {
                // It is marked again when a chunk next to its own arrives.
                e.dirty = 0;
                e.since = None;
                self.dirty_count -= 1;
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
            self.dirty_count -= 1;
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
        let Some(e) = self.sections.get_mut(&stamp.section) else {
            return mesh;
        };
        let key = stamp.section;
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
            (Some(index), Some(mesh)) => {
                Some(std::mem::replace(&mut self.meshes[index as usize].1, mesh))
            }
            (Some(index), None) => {
                e.mesh = None;
                Some(self.take_mesh(index))
            }
            (None, Some(mesh)) => {
                e.mesh = Some(self.meshes.len() as u32);
                self.meshes.push((key, mesh));
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
}

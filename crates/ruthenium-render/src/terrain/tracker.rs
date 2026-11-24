//! Which sections have a mesh, which need one, and in what order they get it.
//!
//! The level tells what changed through the scene's dirty queue; the tracker turns
//! that into work and keeps, for every section of a loaded chunk, the mesh that is
//! drawn and the [`Stamp`] it was made from. It holds no GPU objects itself: a mesh is
//! whatever the caller stores (`M`), handed back when it is replaced or its chunk goes.
//!
//! Rules it keeps:
//! - A section's chunk left: its mesh goes at once (no geometry outlives its chunk).
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

use ruthenium_sim::scene::{Neighbourhood, Scene, SectionKey, Stamp, WHOLE_COLUMN, dirty};

/// New dirty sections after which the order of work is worked out again.
const RESORT_AFTER: usize = 256;

/// A section the tracker knows.
#[derive(Debug)]
struct Entry<M> {
    /// The mesh drawn (`None`: nothing to draw, or no mesh yet) and what it was made
    /// from (`None`: never meshed).
    mesh: Option<M>,
    stamp: Option<Stamp>,
    /// Why it has to be meshed (again); 0: it doesn't.
    dirty: u8,
    /// A job for it is under way.
    meshing: bool,
    /// It waits in the urgent queue.
    urgent: bool,
}

impl<M> Default for Entry<M> {
    fn default() -> Entry<M> {
        Entry {
            mesh: None,
            stamp: None,
            dirty: 0,
            meshing: false,
            urgent: false,
        }
    }
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
}

/// A section to mesh.
#[derive(Debug)]
pub(crate) struct Job {
    pub neighbourhood: Neighbourhood,
    /// The player's own change: its upload is urgent too.
    pub urgent: bool,
}

/// See the module docs.
#[derive(Debug)]
pub(crate) struct Tracker<M> {
    epoch: u64,
    sections: HashMap<SectionKey, Entry<M>>,
    /// Sections whose `dirty` is set.
    dirty_count: usize,
    urgent: VecDeque<SectionKey>,
    /// Dirty sections, the nearest to the camera last.
    queue: Vec<SectionKey>,
    /// The camera's section when the queue was last sorted, and how many sections
    /// became dirty since.
    sorted_for: Option<SectionKey>,
    unsorted: usize,
    keys: Vec<(SectionKey, u8)>,
    stats: TrackerStats,
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
            dirty_count: 0,
            urgent: VecDeque::new(),
            queue: Vec::new(),
            sorted_for: None,
            unsorted: 0,
            keys: Vec::new(),
            stats: TrackerStats::default(),
        }
    }

    pub fn stats(&self) -> TrackerStats {
        self.stats
    }

    /// Sections of loaded chunks, sections with a mesh to draw, and sections waiting
    /// for a mesh.
    pub fn counts(&self) -> (usize, usize, usize) {
        (
            self.sections.len(),
            self.sections.values().filter(|e| e.mesh.is_some()).count(),
            self.dirty_count,
        )
    }

    /// Every mesh to draw.
    pub fn meshes(&self) -> impl Iterator<Item = (SectionKey, &M)> + '_ {
        self.sections
            .iter()
            .filter_map(|(k, e)| e.mesh.as_ref().map(|m| (*k, m)))
    }

    fn mark(&mut self, key: SectionKey, flags: u8) {
        let e = self.sections.entry(key).or_default();
        if e.dirty == 0 {
            self.dirty_count += 1;
            self.unsorted += 1;
        }
        e.dirty |= flags.max(1);
        if flags & dirty::URGENT != 0 && !e.urgent {
            e.urgent = true;
            self.urgent.push_back(key);
        }
    }

    /// Forgets everything; the meshes go to `removed`.
    pub fn clear(&mut self, removed: &mut Vec<M>) {
        removed.extend(self.sections.drain().filter_map(|(_, e)| e.mesh));
        self.dirty_count = 0;
        self.urgent.clear();
        self.queue.clear();
        self.sorted_for = None;
        self.unsorted = 0;
    }

    fn remove_column(&mut self, x: i32, z: i32, removed: &mut Vec<M>) {
        let mut gone = 0;
        self.sections.retain(|k, e| {
            if k[0] != x || k[2] != z {
                return true;
            }
            gone += usize::from(e.dirty != 0);
            removed.extend(e.mesh.take());
            false
        });
        self.dirty_count -= gone;
    }

    /// Marks a section that has a mesh dirty if the level no longer is what the mesh
    /// was made from, and one without a mesh as wanting its first.
    fn mark_if_stale(&mut self, scene: &Scene, key: SectionKey, flags: u8) {
        let stale = match self.sections.get(&key).and_then(|e| e.stamp.as_ref()) {
            Some(stamp) => !scene.is_current(stamp),
            None => true,
        };
        if stale {
            self.mark(key, flags);
        }
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
        if drained.rescan.is_some() {
            self.stats.rescans += 1;
            let (dims, columns) = scene.with_terrain(|t| {
                (
                    t.dims(),
                    t.columns().map(|(k, _)| k).collect::<Vec<(i32, i32)>>(),
                )
            });
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
                for y in dims.min_section..dims.min_section + dims.sections {
                    self.mark_if_stale(scene, [x, y, z], dirty::LOADED);
                }
            }
        }
        if !keys.is_empty() {
            let dims = scene.with_terrain(|t| t.dims());
            let present = |x: i32, z: i32| scene.with_terrain(|t| t.column(x, z).is_some());
            for &(key, flags) in &keys {
                let (x, z) = (key[0], key[2]);
                if key[1] == WHOLE_COLUMN {
                    if !present(x, z) {
                        self.remove_column(x, z, removed);
                        continue;
                    }
                    for y in dims.min_section..dims.min_section + dims.sections {
                        if flags & (dirty::LOADED | dirty::UNLOADED) != 0 {
                            // Another load of the chunk: whatever was made of the one
                            // before is of a chunk that is gone.
                            self.mark([x, y, z], flags & !dirty::UNLOADED);
                        } else {
                            self.mark_if_stale(scene, [x, y, z], flags);
                        }
                    }
                } else if (dims.min_section..dims.min_section + dims.sections).contains(&key[1])
                    && (self.sections.contains_key(&key) || present(x, z))
                {
                    self.mark(key, flags);
                }
            }
        }
        self.keys = keys;
    }

    fn sort(&mut self, camera: SectionKey) {
        self.queue.clear();
        self.queue.extend(
            self.sections
                .iter()
                .filter(|(_, e)| e.dirty != 0 && !e.meshing)
                .map(|(k, _)| *k),
        );
        // The nearest last, where they are taken from; ties in a fixed order.
        self.queue
            .sort_unstable_by_key(|k| (std::cmp::Reverse(distance(*k, camera)), *k));
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
                self.dirty_count -= 1;
                continue;
            };
            let surrounded =
                (-1..=1).all(|dz| (-1..=1).all(|dx| neighbourhood.column(dx, dz).is_some()));
            if e.stamp.is_none() && !surrounded {
                // It is marked again when a chunk next to its own arrives.
                e.dirty = 0;
                self.dirty_count -= 1;
                self.stats.waits_for_neighbours += 1;
                continue;
            }
            e.dirty = 0;
            self.dirty_count -= 1;
            e.meshing = true;
            self.stats.jobs += 1;
            self.stats.urgent_jobs += u64::from(urgent);
            return Some(Job {
                neighbourhood,
                urgent,
            });
        }
    }

    /// Whether a job's result may still be shown: the level is what it was made from.
    /// A stale result's section is free for its next job at once, and dirty.
    pub fn accept(&mut self, scene: &Scene, stamp: &Stamp) -> bool {
        if stamp.epoch == self.epoch && scene.is_current(stamp) {
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
            self.mark(key, dirty::BLOCKS);
            // It was left out of the order of work while its job was out.
            self.unsorted += 1;
        }
    }

    /// Puts a job's result in place: `mesh` (none: the section draws nothing) is what
    /// is drawn from now on. Returns the mesh it replaces. A result for a section that
    /// is no longer tracked is handed back as it came.
    pub fn install(&mut self, stamp: Stamp, mesh: Option<M>) -> Option<M> {
        let Some(e) = self.sections.get_mut(&stamp.section) else {
            return mesh;
        };
        e.meshing = false;
        e.stamp = Some(stamp);
        if e.dirty != 0 {
            // It changed again meanwhile, and was left out of the order of work while
            // its job was out.
            self.unsorted += 1;
        }
        std::mem::replace(&mut e.mesh, mesh)
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

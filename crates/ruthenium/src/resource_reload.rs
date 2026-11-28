//! The game's resources made again with a server's packs over vanilla's
//! (`Minecraft.reloadResourcePacks` with `DownloadedPackSource`'s packs at the top), on a
//! thread: everything drawn or heard that a pack can change (the world's assets: block,
//! item and GUI atlases, models, block entities, the HUD's font, the sky's, clouds' and
//! weather's pictures; the language; the sound events and files), handed over whole once
//! it is built.
//!
//! Vanilla puts a loading overlay over the game until a reload is done; here the old
//! resources stay in use (and drawn) until the new ones are ready, and are swapped for
//! them at once. A reload that fails (a pack that can't be opened, an atlas that doesn't
//! fit, a builder that panics) changes nothing: the resources in use stay, and the
//! session is told, which recovers as vanilla does.
//!
//! What is never read from a server's pack: the menus' own fonts, Unifont and pictures
//! (they read vanilla's pack, `game_files::vanilla`), the players' default skins, and
//! shaders or post effects of any pack (PLAN D5: nothing reads them).

use std::collections::HashMap;
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use ruthenium_assets::{Id, Limits, Pack, Resources};
use ruthenium_audio::SoundFiles;
use ruthenium_render::WorldAssets;

/// Bytes of `lang/en_us.json` read from all server packs together: vanilla's whole file
/// is about 600 KiB, so this leaves room for a server's own strings while a pack's
/// language can't grow the map without end.
pub const MAX_LANG_BYTES: usize = 8 << 20;
/// Entries of the language the server packs may add or change.
pub const MAX_LANG_ENTRIES: usize = 100_000;
/// The largest side of an atlas (blocks, items, the GUI) when a server's packs are in
/// the stack, whatever the GPU takes: its top level is then at most 256 MiB (with mip
/// levels about 340 MiB) on the reload's thread and the GPU, room for 64x packs
/// (vanilla's block atlas is 2048 at 16x). Atlases that don't fit fail the reload, as
/// vanilla's do when the GPU's limit is passed, and the recovery puts back what was.
pub const MAX_SERVER_ATLAS: u32 = 8192;

/// The resources of one reload.
pub struct Reloaded {
    /// The world's drawing assets (when they were asked for).
    pub world: Option<WorldAssets>,
    /// `en_us` through the stack: server messages and the HUD's text.
    pub lang: Arc<HashMap<String, String>>,
    /// The sound events and files through the stack.
    pub sounds: SoundFiles,
    /// How many server packs it has.
    pub packs: usize,
}

impl std::fmt::Debug for Reloaded {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Reloaded")
            .field("packs", &self.packs)
            .finish_non_exhaustive()
    }
}

/// The largest side of an atlas a reload builds: the GPU's, and with server packs in
/// the stack at most [`MAX_SERVER_ATLAS`].
pub fn atlas_limit(max_texture: u32, server_packs: usize) -> u32 {
    if server_packs == 0 {
        max_texture
    } else {
        max_texture.min(MAX_SERVER_ATLAS)
    }
}

/// Opens a server's downloaded pack as one whose every byte is hostile (a reload opens
/// its packs together, in `open_server_packs`, to share one budget).
#[cfg(test)]
fn open_server_pack(path: &Path) -> Result<Pack, String> {
    Pack::open_untrusted(path, &Limits::untrusted()).map_err(|e| e.to_string())
}

/// Whether a downloaded file is a pack that can be loaded (`readPackMetadata`), for the
/// session's check when a reload starts.
pub fn is_loadable(path: &Path) -> bool {
    // Only pack.mcmeta is read here (no directory walk, no filter compiled); a pack that
    // passes and is still refused when the reload opens it fails as the reload.
    match Pack::check_untrusted(path, &Limits::untrusted()).map_err(|e| e.to_string()) {
        Ok(_) => true,
        Err(e) => {
            eprintln!("[server packs] {e}");
            false
        }
    }
}

/// Opens a reload's server packs, sharing one untrusted [`Budget`] (bytes, texels, and
/// `cancel`) across all of them so 16 server packs cannot multiply the per-load budgets.
fn open_server_packs(
    packs: &[(u128, PathBuf)],
    cancel: &Arc<AtomicBool>,
) -> Result<Vec<Pack>, String> {
    let budget = Arc::new(ruthenium_assets::limits::Budget::default());
    budget.cancel_with(cancel.clone());
    let mut opened = Vec::with_capacity(packs.len());
    for (id, path) in packs {
        if cancel.load(Ordering::Relaxed) {
            return Err("cancelled".to_owned());
        }
        let pack = Pack::open_untrusted_with(path, &Limits::untrusted(), budget.clone())
            .map_err(|e| format!("pack {id:032x}: {e}"))?;
        opened.push(pack);
    }
    Ok(opened)
}

/// Builds the resources of `game` with `packs` (lowest first) over vanilla's.
/// `max_texture`: the largest texture the GPU takes; `world`: whether to build the
/// world's assets too.
/// `cancel` is looked at between the stages, and given to every server pack, whose
/// builders look at it inside their loops (Agent U's `Pack::cancel_with`): once it is
/// set, the build gives up, and what a stopped stage returned (partly built) is dropped.
pub fn build(
    game: &Path,
    packs: &[(u128, PathBuf)],
    max_texture: u32,
    world: bool,
    cancel: &Arc<AtomicBool>,
) -> Result<Reloaded, String> {
    let go_on = || {
        if cancel.load(Ordering::Relaxed) {
            Err("cancelled".to_owned())
        } else {
            Ok(())
        }
    };
    let started = Instant::now();
    let opened = open_server_packs(packs, cancel)?;
    let resources = crate::game_files::with_server_packs(game, opened)?;
    let max_texture = atlas_limit(max_texture, packs.len());
    go_on()?;
    let lang = Arc::new(language(&resources, packs.len()));
    go_on()?;
    let sounds = SoundFiles::new(resources.clone());
    go_on()?;
    let world = if world {
        Some(crate::join::load_world_assets_from(
            game,
            &resources,
            max_texture,
            cancel,
        )?)
    } else {
        None
    };
    eprintln!(
        "[ruthenium] resources with {} server pack(s) built in {:.0?}",
        packs.len(),
        started.elapsed()
    );
    Ok(Reloaded {
        world,
        lang,
        sounds,
        packs: packs.len(),
    })
}

/// `ClientLanguage.loadFrom` for `en_us`: every namespace's `lang/en_us.json` of every
/// pack, lowest first, later ones over earlier ones. What the `server` packs at the top
/// add is bounded ([`MAX_LANG_BYTES`], [`MAX_LANG_ENTRIES`]).
fn language(resources: &Resources, server: usize) -> HashMap<String, String> {
    let mut namespaces: Vec<&str> = resources
        .packs()
        .iter()
        .flat_map(|p| p.namespaces().iter().map(String::as_str))
        .collect();
    namespaces.sort_unstable();
    namespaces.dedup();
    let first_server = resources.packs().len().saturating_sub(server);
    let mut out = HashMap::new();
    let (mut bytes, mut added) = (0usize, 0usize);
    let mut server_bytes_full = false;
    for ns in namespaces {
        if resources.cancelled() {
            break;
        }
        let Some(id) = Id::new(ns, "lang/en_us.json") else {
            continue;
        };
        for r in resources.stack(&id) {
            if resources.cancelled() {
                break;
            }
            let from_server = resources
                .packs()
                .iter()
                .position(|p| std::ptr::eq(p, r.pack()))
                .is_some_and(|i| i >= first_server);
            if from_server {
                if server_bytes_full {
                    continue;
                }
                if let Some(declared) = r.declared_size()
                    && (bytes as u64).saturating_add(declared) > MAX_LANG_BYTES as u64
                {
                    eprintln!(
                        "[server packs] {}: {ns}:lang/en_us.json is past {} MiB of language; left out",
                        r.pack().name(),
                        MAX_LANG_BYTES >> 20
                    );
                    continue;
                }
            }
            let max_bytes = from_server.then(|| MAX_LANG_BYTES.saturating_sub(bytes));
            let (max_added, mut dummy_added) = if from_server {
                (MAX_LANG_ENTRIES, 0)
            } else {
                (usize::MAX, 0)
            };
            let added_ref = if from_server {
                &mut added
            } else {
                &mut dummy_added
            };
            match ruthenium_assets::lang::merge_strings(
                &r, max_bytes, max_added, added_ref, &mut out,
            ) {
                Ok(file_bytes) => {
                    if from_server {
                        bytes = bytes.saturating_add(file_bytes);
                        if bytes >= MAX_LANG_BYTES {
                            server_bytes_full = true;
                        }
                    }
                }
                Err(e) => {
                    if from_server && max_bytes.is_some() && e.contains("past the language limit") {
                        eprintln!(
                            "[server packs] {}: {ns}:lang/en_us.json is past {} MiB of language; left out",
                            r.pack().name(),
                            MAX_LANG_BYTES >> 20
                        );
                    } else {
                        eprintln!("[ruthenium] {}: {ns}:lang/en_us.json: {e}", r.pack().name());
                    }
                }
            }
        }
    }
    out
}

/// A reload asked for, and who asked (the session's number, for its answer).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    pub packs: Vec<(u128, PathBuf)>,
    pub session: u64,
}

/// The least time between two reloads a server asks for (vanilla: none): pushes and
/// pops can ask for one again and again, and each builds every atlas and model anew.
/// Those asked for meanwhile wait, and only the newest is made.
pub const MIN_RELOAD_INTERVAL: Duration = Duration::from_secs(3);
/// A reload still building after this long is given up (as failed, which the session
/// recovers from); its thread stops at its next stage.
pub const RELOAD_DEADLINE: Duration = Duration::from_secs(60);

/// Why a reload that was overtaken by a newer one comes back.
pub const OVERTAKEN: &str = "overtaken by a newer reload";
/// Why a reload of a session that is over comes back.
pub const SESSION_OVER: &str = "its session is over";

/// Whether a reload's failure is only that it wasn't needed any more: overtaken by a
/// newer reload, or of a session that is over (its build didn't fail).
pub fn superseded(error: &str) -> bool {
    error == OVERTAKEN || error == SESSION_OVER
}

/// What builds a reload ([`build`], or a stand-in in tests).
type Build = Arc<
    dyn Fn(&Path, &[(u128, PathBuf)], u32, bool, &Arc<AtomicBool>) -> Result<Reloaded, String>
        + Send
        + Sync,
>;

/// The reload under way.
struct Running {
    request: Request,
    handle: JoinHandle<Result<Reloaded, String>>,
    cancel: Arc<AtomicBool>,
    started: Instant,
}

/// Makes one reload at a time, on one thread at a time (a reload given up runs on until
/// its next stage, and the next waits for it). One asked for meanwhile waits (the
/// newest only); one a
/// server asked for starts at least [`MIN_RELOAD_INTERVAL`] after the last such one; a
/// reload overtaken by a newer one, cancelled with its session, or past
/// [`RELOAD_DEADLINE`] is not applied, and its request comes back as failed, for an
/// answer. Requests of session 0 are the client's own (a first world, the way back to
/// vanilla's after a session): they don't wait for the interval.
pub struct Reloader {
    game: Option<PathBuf>,
    world: bool,
    build: Build,
    running: Option<Running>,
    /// A reload given up whose thread has yet to stop (told to, at its next stage): no
    /// other starts until it has, so given-up builds can't pile up.
    abandoned: Option<JoinHandle<Result<Reloaded, String>>>,
    next: Option<Request>,
    last_server_start: Option<Instant>,
    answers: VecDeque<(Request, Result<Reloaded, String>)>,
    min_interval: Duration,
    deadline: Duration,
}

impl std::fmt::Debug for Reloader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Reloader")
            .field("running", &self.running.as_ref().map(|r| &r.request))
            .field("next", &self.next)
            .finish_non_exhaustive()
    }
}

impl Reloader {
    /// A reloader that builds with `build` and these times (tests).
    #[cfg(test)]
    fn with_build(build: Build, min_interval: Duration, deadline: Duration) -> Reloader {
        Reloader {
            build,
            min_interval,
            deadline,
            ..Reloader::new(Some(PathBuf::from("/nowhere")), false)
        }
    }

    /// `world`: whether reloads build the world's assets (a window draws them).
    pub fn new(game: Option<PathBuf>, world: bool) -> Reloader {
        Reloader {
            game,
            world,
            build: Arc::new(build),
            running: None,
            abandoned: None,
            next: None,
            last_server_start: None,
            answers: VecDeque::new(),
            min_interval: MIN_RELOAD_INTERVAL,
            deadline: RELOAD_DEADLINE,
        }
    }

    /// Asks for the resources with `request`'s packs; `max_texture` as [`build`]'s.
    pub fn request(&mut self, request: Request, max_texture: u32) {
        if let Some(old) = self.next.replace(request) {
            self.answers.push_back((old, Err(OVERTAKEN.into())));
        }
        self.pump(max_texture);
    }

    /// Starts the waiting reload, if nothing runs and the interval allows.
    fn pump(&mut self, max_texture: u32) {
        let Some(next) = &self.next else { return };
        if self.running.is_some()
            || self.abandoned.is_some()
            || next.session != 0
                && self
                    .last_server_start
                    .is_some_and(|t| t.elapsed() < self.min_interval)
        {
            return;
        }
        let Some(request) = self.next.take() else {
            return;
        };
        let Some(game) = self.game.clone() else {
            eprintln!("[ruthenium] no game files to reload resources from");
            self.answers
                .push_back((request, Err("no game files".into())));
            return;
        };
        if request.session != 0 {
            self.last_server_start = Some(Instant::now());
        }
        let (packs, world, build) = (request.packs.clone(), self.world, self.build.clone());
        let cancel = Arc::new(AtomicBool::new(false));
        let flag = cancel.clone();
        match std::thread::Builder::new()
            .name("resource-reload".into())
            .spawn(move || build(&game, &packs, max_texture, world, &flag))
        {
            Ok(handle) => {
                self.running = Some(Running {
                    request,
                    handle,
                    cancel,
                    started: Instant::now(),
                })
            }
            Err(e) => {
                eprintln!("[ruthenium] no thread for the reload: {e}");
                self.answers
                    .push_back((request, Err(format!("no thread: {e}"))));
            }
        }
    }

    /// Whether vanilla's own resources (the client's request, no server packs) are being
    /// built or wait to be.
    pub fn vanilla_pending(&self) -> bool {
        let vanilla = |r: &Request| r.session == 0 && r.packs.is_empty();
        self.running.as_ref().is_some_and(|r| vanilla(&r.request))
            || self.next.as_ref().is_some_and(vanilla)
    }

    /// Whether a reload is under way or waiting.
    pub fn busy(&self) -> bool {
        self.running.is_some() || self.next.is_some()
    }

    /// The session `session` is over: its reload under way stops at its next stage and
    /// is not applied, any already-finished answer from it is dropped so it can never be
    /// applied to a later session, and one of its waiting is dropped (both come back as
    /// failed).
    pub fn cancel_session(&mut self, session: u64, max_texture: u32) {
        // Purge any completed results already queued for the ended session.
        self.answers.retain(|(r, _)| r.session != session);
        if session != 0 {
            // Don't let the previous server's reload interval delay the next server.
            self.last_server_start = None;
        }
        if self.next.as_ref().is_some_and(|r| r.session == session)
            && let Some(r) = self.next.take()
        {
            self.answers.push_back((r, Err(SESSION_OVER.into())));
        }
        if self
            .running
            .as_ref()
            .is_some_and(|r| r.request.session == session)
            && let Some(r) = self.running.take()
        {
            r.cancel.store(true, Ordering::Relaxed);
            self.abandoned = Some(r.handle);
            self.answers
                .push_back((r.request, Err(SESSION_OVER.into())));
        }
        self.pump(max_texture);
    }

    /// The packs of session `from` went on into session `to` (a transfer in play keeps
    /// them): its reloads, under way, waiting or ended and not yet taken, are `to`'s now,
    /// for `to` to hear how they went.
    pub fn move_session(&mut self, from: u64, to: u64) {
        let moved = |r: &mut Request| {
            if r.session == from {
                r.session = to;
            }
        };
        if let Some(r) = &mut self.running {
            moved(&mut r.request);
        }
        if let Some(r) = &mut self.next {
            moved(r);
        }
        for (r, _) in &mut self.answers {
            moved(r);
        }
    }

    /// A reload that has ended: its request and what it built (`Err`: it failed, and
    /// nothing changes).
    pub fn poll(&mut self, max_texture: u32) -> Option<(Request, Result<Reloaded, String>)> {
        if self.abandoned.as_ref().is_some_and(JoinHandle::is_finished)
            && let Some(h) = self.abandoned.take()
        {
            // What it built, if anything, is let go of with it.
            let _ = h.join();
        }
        if self
            .running
            .as_ref()
            .is_some_and(|r| r.started.elapsed() >= self.deadline)
            && let Some(r) = self.running.take()
        {
            r.cancel.store(true, Ordering::Relaxed);
            self.abandoned = Some(r.handle);
            eprintln!(
                "[ruthenium] the resource reload took longer than {:.0?}: given up",
                self.deadline
            );
            self.answers
                .push_back((r.request, Err("it took too long".into())));
        }
        if self
            .running
            .as_ref()
            .is_some_and(|r| r.handle.is_finished())
            && let Some(r) = self.running.take()
        {
            let result = r
                .handle
                .join()
                .unwrap_or_else(|_| Err("the reload's thread panicked".into()));
            let result = if self.next.is_some() {
                Err(OVERTAKEN.into())
            } else {
                result
            };
            self.answers.push_back((r.request, result));
        }
        self.pump(max_texture);
        self.answers.pop_front()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_zip::zip as zip_bytes;

    /// `name` inside a fresh temporary folder, deleted with the guard even when the
    /// test fails.
    fn temp(name: &str) -> (tempfile::TempDir, PathBuf) {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join(name);
        std::fs::create_dir_all(&dir).unwrap();
        (tmp, dir)
    }

    fn request(session: u64) -> Request {
        Request {
            packs: Vec::new(),
            session,
        }
    }

    /// Waits for the next answer (at most a few seconds).
    fn answer(r: &mut Reloader) -> (Request, Result<Reloaded, String>) {
        let until = Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(a) = r.poll(1024) {
                return a;
            }
            assert!(Instant::now() < until, "no answer");
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    fn err(a: (Request, Result<Reloaded, String>)) -> (u64, String) {
        (a.0.session, a.1.expect_err("a failure"))
    }

    /// A build that runs until it is cancelled, stage by stage.
    fn endless() -> Build {
        Arc::new(
            |_: &Path, _: &[(u128, PathBuf)], _, _, cancel: &Arc<AtomicBool>| {
                while !cancel.load(Ordering::Relaxed) {
                    std::thread::sleep(Duration::from_millis(1));
                }
                Err("cancelled".into())
            },
        )
    }

    /// A transfer in play keeps the packs: the reloads of the session left, under way and
    /// waiting, are the next session's (the end of the one left gives up none of them),
    /// and come back to it.
    #[test]
    fn reloads_go_on_with_the_packs_a_transfer_keeps() {
        let mut r = Reloader::with_build(endless(), Duration::ZERO, Duration::from_secs(60));
        r.request(request(4), 1024);
        r.request(request(4), 1024);
        assert!(r.running.is_some() && r.next.is_some());
        r.move_session(4, 5);
        r.cancel_session(4, 1024);
        assert!(r.running.is_some() && r.next.is_some());
        r.cancel_session(5, 1024);
        assert_eq!(err(answer(&mut r)), (5, SESSION_OVER.into()));
        assert_eq!(err(answer(&mut r)), (5, SESSION_OVER.into()));
    }

    /// A session's reload under way is given up when the session is over: it comes back
    /// at once, failed, and its thread is told to stop; the next starts.
    #[test]
    fn a_reload_of_a_session_that_is_over_is_dropped() {
        let flag = Arc::new(AtomicBool::new(false));
        let seen = flag.clone();
        let build: Build = Arc::new(
            move |_: &Path, _: &[(u128, PathBuf)], _, _, cancel: &Arc<AtomicBool>| {
                while !cancel.load(Ordering::Relaxed) {
                    std::thread::sleep(Duration::from_millis(1));
                }
                seen.store(true, Ordering::Relaxed);
                Err("cancelled".into())
            },
        );
        let mut r = Reloader::with_build(build, Duration::ZERO, Duration::from_secs(60));
        r.request(request(7), 1024);
        assert!(r.busy());
        r.cancel_session(7, 1024);
        assert_eq!(err(answer(&mut r)), (7, "its session is over".into()));
        assert!(!r.busy());
        let until = Instant::now() + Duration::from_secs(5);
        while !flag.load(Ordering::Relaxed) {
            assert!(Instant::now() < until, "the build was not told to stop");
            std::thread::sleep(Duration::from_millis(1));
        }
        // Another session's isn't touched.
        r.request(request(8), 1024);
        r.cancel_session(7, 1024);
        assert!(r.busy());
        r.cancel_session(8, 1024);
        assert_eq!(err(answer(&mut r)).0, 8);
    }

    /// A reload given up still runs until its next stage: no other starts until it has
    /// stopped (not even the client's own way back to vanilla's), so given-up builds
    /// can't pile up; then the waiting one starts.
    #[test]
    fn one_reload_thread_at_a_time() {
        use std::sync::atomic::AtomicUsize;
        let starts = Arc::new(AtomicUsize::new(0));
        let release = Arc::new(AtomicBool::new(false));
        let (s, rel) = (starts.clone(), release.clone());
        let build: Build = Arc::new(
            move |_: &Path, _: &[(u128, PathBuf)], _, _, cancel: &Arc<AtomicBool>| {
                s.fetch_add(1, Ordering::SeqCst);
                // A stage that takes its time to see the flag.
                while !(cancel.load(Ordering::Relaxed) && rel.load(Ordering::Relaxed)) {
                    std::thread::sleep(Duration::from_millis(1));
                }
                Err("stopped".into())
            },
        );
        let mut r = Reloader::with_build(build, Duration::ZERO, Duration::from_secs(60));
        r.request(request(7), 1024);
        r.cancel_session(7, 1024);
        assert_eq!(err(answer(&mut r)).0, 7);
        r.request(request(0), 1024);
        for _ in 0..50 {
            assert!(r.poll(1024).is_none());
            std::thread::sleep(Duration::from_millis(2));
        }
        assert_eq!(starts.load(Ordering::SeqCst), 1, "a second thread started");
        release.store(true, Ordering::Relaxed);
        let until = Instant::now() + Duration::from_secs(5);
        while starts.load(Ordering::SeqCst) < 2 {
            assert!(Instant::now() < until, "the waiting reload never started");
            r.poll(1024);
            std::thread::sleep(Duration::from_millis(1));
        }
        // The same past a deadline.
        let mut r = Reloader::with_build(
            endless_until(release.clone()),
            Duration::ZERO,
            Duration::from_millis(20),
        );
        release.store(false, Ordering::Relaxed);
        r.request(request(3), 1024);
        assert_eq!(err(answer(&mut r)), (3, "it took too long".into()));
        r.request(request(0), 1024);
        for _ in 0..30 {
            assert!(r.poll(1024).is_none());
            std::thread::sleep(Duration::from_millis(2));
        }
        assert!(
            r.running.is_none(),
            "a second thread started past the deadline"
        );
        release.store(true, Ordering::Relaxed);
    }

    /// Runs until cancelled and let go.
    fn endless_until(release: Arc<AtomicBool>) -> Build {
        Arc::new(
            move |_: &Path, _: &[(u128, PathBuf)], _, _, cancel: &Arc<AtomicBool>| {
                while !(cancel.load(Ordering::Relaxed) && release.load(Ordering::Relaxed)) {
                    std::thread::sleep(Duration::from_millis(1));
                }
                Err("stopped".into())
            },
        )
    }

    /// A reload still building at its deadline is given up.
    #[test]
    fn a_reload_past_its_deadline_is_given_up() {
        let mut r = Reloader::with_build(endless(), Duration::ZERO, Duration::from_millis(30));
        r.request(request(3), 1024);
        assert_eq!(err(answer(&mut r)), (3, "it took too long".into()));
        assert!(!r.busy());
    }

    /// Reloads a server asks for start at least the interval apart, only the newest of
    /// those asked for meanwhile; the client's own don't wait.
    #[test]
    fn reloads_a_server_asks_for_are_spaced_and_coalesced() {
        let quick: Build = Arc::new(
            |_: &Path, _: &[(u128, PathBuf)], _, _, _: &Arc<AtomicBool>| Err("built".into()),
        );
        let mut r = Reloader::with_build(
            quick.clone(),
            Duration::from_millis(300),
            Duration::from_secs(60),
        );
        let started = Instant::now();
        r.request(request(1), 1024);
        assert_eq!(err(answer(&mut r)), (1, "built".into()));
        r.request(request(1), 1024);
        r.request(request(1), 1024);
        // The first of the two is overtaken; the second waits for the interval.
        assert_eq!(
            err(answer(&mut r)),
            (1, "overtaken by a newer reload".into())
        );
        assert_eq!(err(answer(&mut r)), (1, "built".into()));
        assert!(started.elapsed() >= Duration::from_millis(300));
        // Within the interval the client's own starts at once and a server's waits (judged
        // by what is running, not by the clock, which a busy machine can't keep to).
        let mut r = Reloader::with_build(quick, Duration::from_secs(60), Duration::from_secs(60));
        r.request(request(1), 1024);
        assert_eq!(err(answer(&mut r)), (1, "built".into()));
        r.request(request(0), 1024);
        assert!(r.running.is_some(), "the client's own reload waited");
        assert_eq!(err(answer(&mut r)), (0, "built".into()));
        r.request(request(1), 1024);
        assert!(
            r.running.is_none() && r.next.is_some(),
            "a server's reload did not wait"
        );
    }

    /// Cancelled between stages, a real build stops (here before opening the packs).
    #[test]
    fn a_cancelled_build_stops() {
        let cancel = Arc::new(AtomicBool::new(true));
        let packs = [(1u128, PathBuf::from("/nowhere/p.zip"))];
        assert_eq!(
            build(Path::new("/nowhere"), &packs, 1024, true, &cancel).err(),
            Some("cancelled".to_owned())
        );
    }

    /// The server packs carry the reload's flag: once it is set, a stack of them is
    /// cancelled, and their builders stop inside their loops.
    #[test]
    fn server_packs_carry_the_reloads_flag() {
        let (_tmp, dir) = temp("flag");
        let p = dir.join("p.zip");
        std::fs::write(
            &p,
            zip_bytes(&[
                ("pack.mcmeta", MCMETA),
                ("assets/minecraft/lang/en_us.json", br#"{"a":"b"}"#),
            ]),
        )
        .unwrap();
        let cancel = Arc::new(AtomicBool::new(false));
        let opened = open_server_packs(&[(1, p)], &cancel).unwrap();
        let r = Resources::new(opened);
        assert!(!r.cancelled());
        assert_eq!(language(&r, 1).len(), 1);
        cancel.store(true, Ordering::Relaxed);
        assert!(r.cancelled());
        assert!(language(&r, 1).is_empty());
    }

    const MCMETA: &[u8] = br#"{"pack":{"description":"test","min_format":97,"max_format":97}}"#;

    /// Server packs can't make an atlas larger than [`MAX_SERVER_ATLAS`]; vanilla's
    /// alone gets what the GPU takes.
    #[test]
    fn server_packs_cap_the_atlas() {
        assert_eq!(atlas_limit(32768, 0), 32768);
        assert_eq!(atlas_limit(32768, 1), MAX_SERVER_ATLAS);
        assert_eq!(atlas_limit(4096, 3), 4096);
    }

    /// The session's check reads only the metadata: a zip with a valid pack.mcmeta is
    /// loadable, one without it (or with a broken one) is not.
    #[test]
    fn the_check_reads_the_packs_metadata() {
        let (_tmp, dir) = temp("check");
        let good = dir.join("good.zip");
        std::fs::write(&good, zip_bytes(&[("pack.mcmeta", MCMETA)])).unwrap();
        let none = dir.join("none.zip");
        std::fs::write(
            &none,
            zip_bytes(&[("assets/minecraft/lang/en_us.json", b"{}")]),
        )
        .unwrap();
        let broken = dir.join("broken.zip");
        std::fs::write(&broken, zip_bytes(&[("pack.mcmeta", b"{\"pack\":")])).unwrap();
        assert!(is_loadable(&good));
        assert!(!is_loadable(&none));
        assert!(!is_loadable(&broken));
        assert!(!is_loadable(&dir.join("missing.zip")));
    }

    /// A server pack's language goes over vanilla's, the newest pack over older ones.
    #[test]
    fn server_languages_stack_over_vanillas() {
        let (_tmp, dir) = temp("lang");
        let pack = |n: &str, lang: &str| {
            let p = dir.join(n);
            std::fs::write(
                &p,
                zip_bytes(&[
                    ("pack.mcmeta", MCMETA),
                    ("assets/minecraft/lang/en_us.json", lang.as_bytes()),
                ]),
            )
            .unwrap();
            open_server_pack(&p).unwrap()
        };
        let low = pack("low.zip", r#"{"a":"low","b":"low"}"#);
        let high = pack("high.zip", r#"{"b":"high","c":7}"#);
        let r = Resources::new(vec![low, high]);
        let lang = language(&r, 2);
        assert_eq!(lang.get("a").map(String::as_str), Some("low"));
        assert_eq!(lang.get("b").map(String::as_str), Some("high"));
        assert!(!lang.contains_key("c"), "only strings");
    }

    /// Past the cap, a server pack's language is left out.
    #[test]
    fn a_huge_server_language_is_left_out() {
        let (_tmp, dir) = temp("huge");
        let mut big = String::from("{");
        let n = MAX_LANG_BYTES / 20 + 10;
        for i in 0..n {
            big += &format!("\"k{i:08}\":\"vvvvv\",");
        }
        big += "\"end\":\"x\"}";
        let p = dir.join("big.zip");
        std::fs::write(
            &p,
            zip_bytes(&[
                ("pack.mcmeta", MCMETA),
                ("assets/minecraft/lang/en_us.json", big.as_bytes()),
            ]),
        )
        .unwrap();
        let r = Resources::new(vec![open_server_pack(&p).unwrap()]);
        assert!(language(&r, 1).is_empty());
    }

    /// Entries a server's packs add stop at the cap; those they change don't count.
    #[test]
    fn server_language_entries_are_capped() {
        let (_tmp, dir) = temp("entries");
        let mut s = String::from("{");
        for i in 0..MAX_LANG_ENTRIES + 5 {
            s += &format!("\"{i}\":\"v\",");
        }
        s += "\"last\":\"v\"}";
        let p = dir.join("many.zip");
        std::fs::write(
            &p,
            zip_bytes(&[
                ("pack.mcmeta", MCMETA),
                ("assets/minecraft/lang/en_us.json", s.as_bytes()),
            ]),
        )
        .unwrap();
        let r = Resources::new(vec![open_server_pack(&p).unwrap()]);
        assert_eq!(language(&r, 1).len(), MAX_LANG_ENTRIES);
    }

    /// A finished reload's answer still queued when its session ends is dropped by
    /// `cancel_session`, so it can never be applied afterwards; other sessions' stay.
    #[test]
    fn cancel_session_purges_queued_completed_answer() {
        let build: Build =
            Arc::new(|_: &Path, _: &[(u128, PathBuf)], _, _, _| Err("simulated result".into()));
        let mut r = Reloader::with_build(build, Duration::ZERO, Duration::from_secs(60));
        r.answers.push_back((request(9), Err("finished".into())));
        r.answers.push_back((request(5), Err("other".into())));
        r.cancel_session(9, 1024);
        let mut left = Vec::new();
        while let Some(a) = r.poll(1024) {
            left.push(err(a));
        }
        assert_eq!(left, [(5, "other".to_owned())]);
    }
}

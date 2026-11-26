//! Server resource packs in a session: the pushes and pops
//! (`ClientCommonPacketListenerImpl.handleResourcePackPush`, `handleResourcePackPop`),
//! the prompt that collects them (`addOrUpdatePackPrompt`, `PackConfirmScreen`'s
//! answer), and what [`crate::server_pack`] asks for: the statuses sent to the server,
//! the downloads (on a thread of their own, one batch after another as vanilla's
//! `DownloadQueue` makes them, with vanilla's headers and toast), and the reloads, which
//! the client makes ([`SessionEvent::ReloadResources`], answered with
//! [`Session::resources_reloaded`]).
//!
//! Every byte a server sends here is hostile: the URL only ever reaches the download
//! (which refuses what vanilla would not fetch), the prompt's text only the menus, and a
//! downloaded file only the client's check and its untrusted pack reader. Beyond
//! vanilla, the number of packs a server can have tracked, collected in one prompt, and
//! loaded at once is capped (see the constants).

use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, Instant};

use ruthenium_net::connection::{Reason, is_local};
use ruthenium_net::server_list::PackStatus;
use ruthenium_net::server_pack as download;
use ruthenium_protocol::nbt::{Compound, Tag};
use ruthenium_protocol::packets::{PackAction, PackPush, configuration as cfg, play};

use super::{Game, Phase, SessionEvent};
use crate::server_pack::{Batch, Effect, Feedback, PromptStatus, ServerPacks};

/// Packs a server can have the manager track at once (pushed and not yet removed):
/// vanilla has no cap. Each holds a URL of up to 32 KiB; without one, a server pushing
/// new ids faster than they are fetched grows the list without end. Far above what a
/// server uses (a handful).
pub const MAX_TRACKED: usize = 64;
/// Packs one prompt collects (vanilla: no cap), for the same reason.
pub const MAX_PROMPTED: usize = 64;
/// Server packs over vanilla's in one reload (vanilla: no cap). Each is a zip of up to
/// 250 MiB whose directory and files are read while the stack is open; a reload with
/// more fails as one with a broken pack does (FAILED_RELOAD), and the packs that were
/// active stay.
pub const MAX_ACTIVE: usize = 16;
/// How often the download toast is told of the bytes so far.
const TOAST_EVERY: Duration = Duration::from_millis(100);

/// What a session needs for server packs.
#[derive(Clone)]
pub struct ServerPackSetup {
    /// The server list entry's Server Resource Packs setting (`ServerData.packStatus`);
    /// `Prompt` for a server that isn't in the list.
    pub status: PackStatus,
    /// Where downloads are kept; none: every download fails.
    pub cache_dir: Option<PathBuf>,
    /// Whether a downloaded file is a pack that can be loaded (`Pack.readPackMetadata`);
    /// called on the game thread when a reload starts, as vanilla's `loadRequestedPacks`.
    pub check: Arc<dyn Fn(&Path) -> bool + Send + Sync>,
}

impl Default for ServerPackSetup {
    fn default() -> ServerPackSetup {
        ServerPackSetup {
            status: PackStatus::Prompt,
            cache_dir: None,
            check: Arc::new(|_| false),
        }
    }
}

impl std::fmt::Debug for ServerPackSetup {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ServerPackSetup")
            .field("status", &self.status)
            .field("cache_dir", &self.cache_dir)
            .finish_non_exhaustive()
    }
}

/// The prompt (`PackConfirmScreen`) as it stands: answer it with
/// [`super::Session::answer_pack_prompt`].
#[derive(Debug, Clone, PartialEq)]
pub struct PackPrompt {
    /// Required (Proceed and Disconnect) or not (Yes and No), as the last push said.
    pub required: bool,
    /// The server's own text, as the last push gave it.
    pub prompt: Option<Tag>,
    /// How many packs it is for.
    pub packs: usize,
}

/// The download toast (`createDownloadNotifier`'s `SystemToast`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PackToast {
    /// `download.pack.title` (count of total), with `download.pack.progress.*` under it
    /// once the pack's download began.
    Progress {
        count: usize,
        total: usize,
        progress: Option<DownloadProgress>,
    },
    /// `download.pack.failed`: this many of the batch failed.
    Failed { failed: usize, total: usize },
    /// It goes (`SystemToast.forceHide`).
    Hide,
}

/// `download.pack.progress.percent` or, without a length, `download.pack.progress.bytes`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DownloadProgress {
    Percent(u64),
    Bytes(u64),
}

/// A reload the client makes: the server packs (lowest first) over vanilla's.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourceReload {
    pub packs: Vec<(u128, PathBuf)>,
    /// The recovery after a failed reload (its failure ends the session).
    pub recovery: bool,
}

type BatchResult = (u64, Vec<(u128, Result<PathBuf, ()>)>);

/// The download thread, once there is one, and where its results come back.
pub(super) struct Downloads {
    jobs: Option<Sender<Batch>>,
    results_tx: Sender<BatchResult>,
    results: Receiver<BatchResult>,
    /// Set when the session ends: a download under way stops between reads, and the
    /// rest of its batch fails, instead of running on after the player has left.
    cancel: Arc<AtomicBool>,
}

impl Default for Downloads {
    fn default() -> Downloads {
        let (results_tx, results) = channel();
        Downloads {
            jobs: None,
            results_tx,
            results,
            cancel: Arc::default(),
        }
    }
}

impl Drop for Downloads {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

/// The pack state of a game thread.
pub(super) struct Packs {
    pub(super) manager: ServerPacks<(u128, PathBuf)>,
    pub(super) setup: ServerPackSetup,
    prompt: Option<Pending>,
    downloads: Downloads,
    /// The server is told nothing more (the session is closing).
    quiet: bool,
    cleaned: bool,
    /// The server's address, for whether pack hosts may be on the local network.
    pub(super) peer: Option<IpAddr>,
}

impl Default for Packs {
    fn default() -> Packs {
        Packs::new(ServerPackSetup::default())
    }
}

#[derive(Debug)]
struct Pending {
    requests: Vec<(u128, String, String)>,
    required: bool,
}

/// `ServerPackManager.PackPromptStatus` of a server list setting
/// (`ConnectScreen.convertPackStatus`).
fn prompt_status(s: PackStatus) -> PromptStatus {
    match s {
        PackStatus::Enabled => PromptStatus::Allowed,
        PackStatus::Disabled => PromptStatus::Declined,
        PackStatus::Prompt => PromptStatus::Pending,
    }
}

/// `loadRequestedPacks`: each pack checked, the newest first; none if one fails, or if
/// there are more than [`MAX_ACTIVE`].
fn open_checked(
    check: &(dyn Fn(&Path) -> bool + Send + Sync),
    list: &[(u128, PathBuf)],
) -> Option<Vec<(u128, PathBuf)>> {
    if list.len() > MAX_ACTIVE {
        eprintln!(
            "[server packs] {} packs to load, more than {MAX_ACTIVE}: the reload fails",
            list.len()
        );
        return None;
    }
    let mut out = Vec::with_capacity(list.len());
    for (id, path) in list.iter().rev() {
        if !check(path) {
            eprintln!(
                "[server packs] invalid pack metadata in {}, ignoring all",
                path.display()
            );
            return None;
        }
        out.push((*id, path.clone()));
    }
    out.reverse();
    Some(out)
}

impl Packs {
    pub(super) fn new(setup: ServerPackSetup) -> Packs {
        let check = setup.check.clone();
        let mut manager =
            ServerPacks::new(move |list: &[(u128, PathBuf)]| open_checked(&*check, list));
        manager.configure_for_server_control(prompt_status(setup.status));
        Packs {
            manager,
            setup,
            prompt: None,
            downloads: Downloads::default(),
            quiet: false,
            cleaned: false,
            peer: None,
        }
    }
}

/// A text component that translates `key`.
fn translatable(key: &str) -> Tag {
    let mut c = Compound::new();
    c.put("translate", Tag::String(key.into()));
    Tag::Compound(c)
}

fn action(f: Feedback) -> PackAction {
    match f {
        Feedback::Accepted => PackAction::Accepted,
        Feedback::Downloaded => PackAction::Downloaded,
        Feedback::Applied => PackAction::SuccessfullyLoaded,
        Feedback::Declined => PackAction::Declined,
        Feedback::Discarded => PackAction::Discarded,
        Feedback::DownloadFailed => PackAction::FailedDownload,
        Feedback::ActivationFailed => PackAction::FailedReload,
    }
}

impl Game {
    /// Sends a pack's status in the phase the session is in.
    fn send_pack_status(&self, id: u128, a: PackAction) {
        if self.packs.quiet {
            return;
        }
        match self.phase {
            Phase::Configuration => self.connection.send(cfg::resource_pack(id, a)),
            Phase::Play => self.connection.send(play::resource_pack(id, a)),
            Phase::Login => {}
        }
    }

    /// `handleResourcePackPush`.
    pub(super) fn handle_pack_push(&mut self, push: PackPush) {
        if !download::is_pack_url(&push.url) {
            eprintln!("[server packs] {:032x}: not an http(s) URL", push.id);
            self.send_pack_status(push.id, PackAction::InvalidUrl);
            return;
        }
        let status = self.packs.setup.status;
        if status != PackStatus::Prompt && (!push.required || status != PackStatus::Disabled) {
            self.push_pack(push.id, push.url, &push.hash);
        } else {
            // `addOrUpdatePackPrompt`: the prompt showing takes one more pack, and the
            // last push's texts.
            let mut requests = self
                .packs
                .prompt
                .take()
                .map_or_else(Vec::new, |p| p.requests);
            if requests.len() >= MAX_PROMPTED {
                eprintln!(
                    "[server packs] {:032x}: the prompt already has {MAX_PROMPTED} packs; discarded",
                    push.id
                );
                self.send_pack_status(push.id, PackAction::Discarded);
            } else {
                requests.push((push.id, push.url, push.hash));
            }
            let prompt = PackPrompt {
                required: push.required,
                prompt: push.prompt,
                packs: requests.len(),
            };
            eprintln!(
                "[server packs] asking about {} pack(s){}",
                prompt.packs,
                if prompt.required { " (required)" } else { "" }
            );
            self.packs.prompt = Some(Pending {
                requests,
                required: prompt.required,
            });
            self.report(SessionEvent::PackPrompt(prompt));
        }
        self.apply_pack_effects();
    }

    /// `DownloadedPackSource.pushPack`, within [`MAX_TRACKED`].
    fn push_pack(&mut self, id: u128, url: String, hash: &str) {
        if self.packs.manager.tracked() >= MAX_TRACKED {
            eprintln!(
                "[server packs] {id:032x}: {MAX_TRACKED} packs are tracked already; discarded"
            );
            self.send_pack_status(id, PackAction::Discarded);
            return;
        }
        eprintln!("[server packs] {id:032x}: pushed ({url})");
        self.packs.manager.push(id, url, download::parse_sha1(hash));
    }

    /// `handleResourcePackPop`.
    pub(super) fn handle_pack_pop(&mut self, id: Option<u128>) {
        match id {
            Some(id) => self.packs.manager.pop(id),
            None => self.packs.manager.pop_all(),
        }
        self.apply_pack_effects();
    }

    /// The prompt's answer (`PackConfirmScreen`'s callback); `None`: it was put away
    /// without one (another screen took its place), and its packs are forgotten, as
    /// vanilla's are.
    pub(super) fn answer_pack_prompt(&mut self, answer: Option<bool>) {
        let Some(p) = self.packs.prompt.take() else {
            return;
        };
        let Some(accept) = answer else {
            eprintln!(
                "[server packs] the prompt went unanswered; its {} pack(s) are forgotten",
                p.requests.len()
            );
            return;
        };
        if accept {
            self.packs.setup.status = PackStatus::Enabled;
            self.packs.manager.allow();
        } else {
            self.packs.manager.reject();
            if p.required {
                eprintln!("[server packs] a required pack was declined: disconnecting");
                self.close_with(Reason::Text(translatable(
                    "multiplayer.requiredTexturePrompt.disconnect",
                )));
            } else {
                self.packs.setup.status = PackStatus::Disabled;
            }
        }
        for (id, url, hash) in p.requests {
            self.push_pack(id, url, &hash);
        }
        self.apply_pack_effects();
        // `ServerList.saveSingleServer`.
        self.report(SessionEvent::PackStatus(self.packs.setup.status));
    }

    /// Closes the connection with `reason` shown for it; the server hears nothing
    /// more of its packs.
    fn close_with(&mut self, reason: Reason) {
        self.close_reason = Some(reason);
        self.packs.quiet = true;
        self.connection.close();
    }

    /// The frame's start: download results that came meanwhile, and a reload that
    /// waited for another.
    pub(super) fn pack_frame_start(&mut self) {
        self.take_download_results();
        self.packs.manager.frame_start();
        self.apply_pack_effects();
    }

    /// `runAllTasks`, after the frame's packets.
    pub(super) fn pack_tasks(&mut self) {
        self.take_download_results();
        self.packs.manager.run_tasks();
        self.apply_pack_effects();
    }

    fn take_download_results(&mut self) {
        while let Ok((batch, result)) = self.packs.downloads.results.try_recv() {
            self.packs.manager.downloaded(batch, result);
        }
    }

    /// The client's reload ended.
    pub(super) fn resources_reloaded(&mut self, ok: bool) {
        eprintln!(
            "[server packs] the reload {}",
            if ok { "is done" } else { "failed" }
        );
        self.packs.manager.reloaded(ok);
        self.apply_pack_effects();
    }

    /// `clearDownloadedResourcePacks`, once: every pack goes, quietly, and the client
    /// goes back to vanilla's resources.
    pub(super) fn pack_cleanup(&mut self) {
        if std::mem::replace(&mut self.packs.cleaned, true) {
            return;
        }
        self.packs.quiet = true;
        self.packs.prompt = None;
        self.packs.manager.cleanup_after_disconnect();
        self.apply_pack_effects();
        // The download thread ends after the batch it is on.
        self.packs.downloads.jobs = None;
    }

    fn apply_pack_effects(&mut self) {
        for e in self.packs.manager.take_effects() {
            match e {
                Effect::Feedback(id, f) => {
                    let quiet = if self.packs.quiet {
                        " (not sent: the session is closing)"
                    } else {
                        ""
                    };
                    eprintln!("[server packs] {id:032x}: {}{quiet}", f.name());
                    self.send_pack_status(id, action(f));
                }
                Effect::Download(batch) => self.start_download(batch),
                Effect::Reload(r) => {
                    eprintln!(
                        "[server packs] reloading with {} server pack(s){}",
                        r.packs.len(),
                        if r.recovery { " (recovery)" } else { "" }
                    );
                    self.report(SessionEvent::ReloadResources(ResourceReload {
                        packs: r.packs,
                        recovery: r.recovery,
                    }));
                }
                Effect::Abort => {
                    eprintln!("[server packs] the recovery reload failed too: disconnecting");
                    self.close_with(Reason::Text(translatable("resourcePack.load_fail")));
                }
            }
        }
    }

    /// Hands a batch to the download thread (started with the first one).
    fn start_download(&mut self, batch: Batch) {
        let failed = |batch: &Batch| {
            (
                batch.batch,
                batch.requests.iter().map(|r| (r.id, Err(()))).collect(),
            )
        };
        let Some(cache_dir) = self.packs.setup.cache_dir.clone() else {
            eprintln!("[server packs] no cache folder to download packs into");
            let _ = self.packs.downloads.results_tx.send(failed(&batch));
            return;
        };
        if self.packs.downloads.jobs.is_none() {
            let config = download::DownloadConfig {
                cache_dir,
                headers: download_headers(&self.name, self.uuid),
                // #12: Agent R's `pack_hosts_allowed(peer)` (adding 127.0.0.1 and ::1 for a
                // loopback peer) replaces this when integrated.
                private_allowed: self
                    .packs
                    .peer
                    .filter(|&p| is_local(p))
                    .map(download::AddressRange::host)
                    .into_iter()
                    .collect(),
            };
            let (jobs, rx) = channel::<Batch>();
            let results = self.packs.downloads.results_tx.clone();
            let events = self.events.clone();
            let cancel = self.packs.downloads.cancel.clone();
            let spawned = std::thread::Builder::new()
                .name("pack-download".into())
                .spawn(move || {
                    for batch in rx {
                        let r = run_batch(&config, &batch, &events, &cancel);
                        if results.send(r).is_err() {
                            return;
                        }
                    }
                });
            match spawned {
                Ok(_) => self.packs.downloads.jobs = Some(jobs),
                Err(e) => eprintln!("[server packs] no download thread: {e}"),
            }
        }
        let sent = self
            .packs
            .downloads
            .jobs
            .as_ref()
            .is_some_and(|j| j.send(batch.clone()).is_ok());
        if !sent {
            let _ = self.packs.downloads.results_tx.send(failed(&batch));
        }
    }
}

/// `createDownloadHeaders`: who and what asks, as vanilla says it.
pub fn download_headers(name: &str, uuid: u128) -> Vec<(String, String)> {
    use ruthenium_data::v26_3::version::{ID, RESOURCE_PACK_VERSION};
    let (major, minor) = RESOURCE_PACK_VERSION;
    [
        ("X-Minecraft-Username", name.to_owned()),
        ("X-Minecraft-UUID", format!("{uuid:032x}")),
        ("X-Minecraft-Version", ID.to_owned()),
        ("X-Minecraft-Version-ID", ID.to_owned()),
        // `PackFormat.toString`.
        ("X-Minecraft-Pack-Format", format!("{major}.{minor}")),
        ("User-Agent", format!("Minecraft Java/{ID}")),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_owned(), v))
    .collect()
}

/// One batch on the download thread, with the toast told how it goes.
fn run_batch(
    config: &download::DownloadConfig,
    batch: &Batch,
    events: &Sender<SessionEvent>,
    cancel: &Arc<AtomicBool>,
) -> BatchResult {
    let requests: Vec<download::PackRequest> = batch
        .requests
        .iter()
        .map(|r| download::PackRequest {
            id: r.id,
            url: r.url.clone(),
            sha1: r.hash,
        })
        .collect();
    let mut toast = Toast::new(requests.len(), events.clone(), cancel.clone());
    let results = download::download_batch(config, &requests, &mut toast);
    let out = results
        .into_iter()
        .map(|(id, r)| {
            (
                id,
                r.map_err(|e| {
                    let url = requests.iter().find(|q| q.id == id).map_or("", |q| &q.url);
                    eprintln!("[server packs] {id:032x}: failed to download {url}: {e}");
                })
                .inspect(|path| {
                    eprintln!("[server packs] {id:032x}: downloaded to {}", path.display());
                }),
            )
        })
        .collect();
    (batch.batch, out)
}

/// `createDownloadNotifier`.
struct Toast {
    total: usize,
    count: usize,
    failed: usize,
    size: Option<u64>,
    events: Sender<SessionEvent>,
    last: Option<Instant>,
    cancel: Arc<AtomicBool>,
}

impl Toast {
    fn new(total: usize, events: Sender<SessionEvent>, cancel: Arc<AtomicBool>) -> Toast {
        Toast {
            total,
            count: 0,
            failed: 0,
            size: None,
            events,
            last: None,
            cancel,
        }
    }

    fn send(&self, t: PackToast) {
        let _ = self.events.send(SessionEvent::PackToast(t));
    }

    fn progress(&mut self, bytes: u64, always: bool) {
        let now = Instant::now();
        if !always && self.last.is_some_and(|l| now - l < TOAST_EVERY) {
            return;
        }
        self.last = Some(now);
        let progress = match self.size {
            // (Vanilla divides by a length of 0 and throws.)
            Some(size) => DownloadProgress::Percent(
                bytes.saturating_mul(100).checked_div(size).unwrap_or(100),
            ),
            None => DownloadProgress::Bytes(bytes),
        };
        self.send(PackToast::Progress {
            count: self.count,
            total: self.total,
            progress: Some(progress),
        });
    }
}

impl download::Progress for Toast {
    fn request_start(&mut self) {
        self.count += 1;
        self.size = None;
        self.send(PackToast::Progress {
            count: self.count,
            total: self.total,
            progress: None,
        });
    }

    fn download_start(&mut self, size: Option<u64>) {
        self.size = size;
        self.progress(0, true);
    }

    fn downloaded(&mut self, bytes: u64) {
        self.progress(bytes, false);
    }

    fn cancelled(&mut self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }

    fn request_finished(&mut self, ok: bool) {
        if !ok {
            self.failed += 1;
        }
        if self.count == self.total {
            self.send(if self.failed > 0 {
                PackToast::Failed {
                    failed: self.failed,
                    total: self.total,
                }
            } else {
                PackToast::Hide
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::{DEFAULT_DIRTY_CAPACITY, Scene};
    use ruthenium_net::connection::{Connection, Event, Local, Remote, Sent};
    use ruthenium_protocol::packets::ClientInformation;

    const ID: u128 = 0x1111_2222_3333_4444_5555_6666_7777_8888;

    /// A session that ends (its downloads dropped) stops a download under way: the
    /// downloader asks the toast between reads.
    #[test]
    fn leaving_the_server_cancels_its_downloads() {
        use download::Progress as _;
        let downloads = Downloads::default();
        let (events, _rx) = channel();
        let mut toast = Toast::new(1, events, downloads.cancel.clone());
        assert!(!toast.cancelled());
        drop(downloads);
        assert!(toast.cancelled());
    }

    struct Rig {
        game: Game,
        remote: Remote,
        events: std::sync::mpsc::Receiver<SessionEvent>,
        _scene: Scene,
    }

    fn rig(setup: ServerPackSetup) -> Rig {
        let (connection, remote) = Connection::detached();
        let (publisher, scene) = Scene::new(DEFAULT_DIRTY_CAPACITY);
        let (tx, events) = channel();
        let mut game = Game::new(
            connection,
            Vec::new(),
            None,
            tx,
            publisher,
            ClientInformation::default(),
        );
        game.manual_ms = Some(0);
        game.phase = Phase::Configuration;
        game.packs = Packs::new(setup);
        Rig {
            game,
            remote,
            events,
            _scene: scene,
        }
    }

    fn push(url: &str, required: bool) -> PackPush {
        PackPush {
            id: ID,
            url: url.into(),
            hash: String::new(),
            required,
            prompt: None,
        }
    }

    impl Rig {
        fn sent(&mut self) -> Vec<Sent> {
            self.remote.sent()
        }

        fn events(&self) -> Vec<SessionEvent> {
            self.events.try_iter().collect()
        }
    }

    fn status(id: u128, a: PackAction) -> Sent {
        Sent::Packet(cfg::resource_pack(id, a))
    }

    /// Vanilla's answer to a URL it can't use, at once.
    #[test]
    fn a_url_that_isnt_http_is_answered_invalid() {
        let mut r = rig(ServerPackSetup::default());
        for url in [
            "ftp://example.com/p.zip",
            "file:///etc/passwd",
            "not a url",
            "",
        ] {
            r.game.handle_pack_push(push(url, false));
            assert_eq!(r.sent(), [status(ID, PackAction::InvalidUrl)], "{url}");
        }
        assert!(r.events().is_empty());
    }

    /// Prompt: the menus are asked; Yes accepts (ACCEPTED at once), the server list's
    /// setting becomes Enabled, and without a cache folder the download fails.
    #[test]
    fn yes_accepts_the_prompts_packs() {
        let mut r = rig(ServerPackSetup::default());
        r.game
            .handle_pack_push(push("http://127.0.0.1/p.zip", false));
        assert!(r.sent().is_empty());
        assert!(matches!(
            &r.events()[..],
            [SessionEvent::PackPrompt(PackPrompt {
                required: false,
                packs: 1,
                ..
            })]
        ));
        r.game.answer_pack_prompt(Some(true));
        assert_eq!(r.sent(), [status(ID, PackAction::Accepted)]);
        assert_eq!(r.events(), [SessionEvent::PackStatus(PackStatus::Enabled)]);
        r.game.pack_tasks();
        r.game.pack_tasks();
        assert_eq!(r.sent(), [status(ID, PackAction::FailedDownload)]);
    }

    /// No to an optional pack: DECLINED, and the setting becomes Disabled; the session
    /// goes on.
    #[test]
    fn no_declines_an_optional_pack() {
        let mut r = rig(ServerPackSetup::default());
        r.game
            .handle_pack_push(push("http://127.0.0.1/p.zip", false));
        r.game.answer_pack_prompt(Some(false));
        assert_eq!(r.sent(), [status(ID, PackAction::Declined)]);
        assert!(
            r.events()
                .contains(&SessionEvent::PackStatus(PackStatus::Disabled))
        );
    }

    /// No to a required pack: the session closes with vanilla's reason, and the server
    /// is told nothing more (vanilla disconnects before its DECLINED goes out).
    #[test]
    fn declining_a_required_pack_disconnects() {
        let mut r = rig(ServerPackSetup::default());
        r.game
            .handle_pack_push(push("http://127.0.0.1/p.zip", true));
        r.game.answer_pack_prompt(Some(false));
        assert_eq!(r.sent(), [Sent::Close]);
        assert!(
            r.events()
                .contains(&SessionEvent::PackStatus(PackStatus::Prompt))
        );
        r.remote
            .event(Event::Closed(ruthenium_net::connection::Closed {
                during_login: false,
                reason: Reason::Local(Local::Closed),
            }));
        assert!(!r.game.frame(super::super::FrameInput::default()));
        let closed = r.events().into_iter().find_map(|e| match e {
            SessionEvent::Closed(c) => Some(c),
            _ => None,
        });
        assert_eq!(
            closed.map(|c| c.reason),
            Some(Reason::Text(translatable(
                "multiplayer.requiredTexturePrompt.disconnect"
            )))
        );
    }

    /// Disabled and optional: DECLINED without asking. Disabled and required: asked.
    #[test]
    fn the_server_lists_setting_decides_whether_to_ask() {
        let setup = ServerPackSetup {
            status: PackStatus::Disabled,
            ..ServerPackSetup::default()
        };
        let mut r = rig(setup.clone());
        r.game
            .handle_pack_push(push("http://127.0.0.1/p.zip", false));
        assert_eq!(r.sent(), [status(ID, PackAction::Declined)]);
        assert!(r.events().is_empty());
        let mut r = rig(setup);
        r.game
            .handle_pack_push(push("http://127.0.0.1/p.zip", true));
        assert!(r.sent().is_empty());
        assert!(matches!(&r.events()[..], [SessionEvent::PackPrompt(_)]));
        let mut r = rig(ServerPackSetup {
            status: PackStatus::Enabled,
            ..ServerPackSetup::default()
        });
        r.game
            .handle_pack_push(push("http://127.0.0.1/p.zip", true));
        assert_eq!(r.sent(), [status(ID, PackAction::Accepted)]);
    }

    /// Past [`MAX_TRACKED`] packs, a push is discarded.
    #[test]
    fn tracked_packs_are_capped() {
        let mut r = rig(ServerPackSetup {
            status: PackStatus::Enabled,
            ..ServerPackSetup::default()
        });
        for i in 0..MAX_TRACKED as u128 + 1 {
            r.game.handle_pack_push(PackPush {
                id: i,
                ..push("http://127.0.0.1/p.zip", false)
            });
        }
        let sent = r.sent();
        assert_eq!(sent.len(), MAX_TRACKED + 1);
        assert_eq!(
            sent.last(),
            Some(&status(MAX_TRACKED as u128, PackAction::Discarded))
        );
        assert!(
            sent[..MAX_TRACKED]
                .iter()
                .all(|s| matches!(s, Sent::Packet(_)))
        );
        assert_eq!(sent[0], status(0, PackAction::Accepted));
    }

    /// Past [`MAX_PROMPTED`] packs, the prompt takes no more.
    #[test]
    fn prompted_packs_are_capped() {
        let mut r = rig(ServerPackSetup::default());
        for i in 0..MAX_PROMPTED as u128 + 1 {
            r.game.handle_pack_push(PackPush {
                id: i,
                ..push("http://127.0.0.1/p.zip", false)
            });
        }
        assert_eq!(
            r.sent(),
            [status(MAX_PROMPTED as u128, PackAction::Discarded)]
        );
        let last = r.events().into_iter().last();
        assert!(matches!(last, Some(SessionEvent::PackPrompt(p)) if p.packs == MAX_PROMPTED));
    }

    /// More than [`MAX_ACTIVE`] packs to load fail the reload; one that the check
    /// refuses does too.
    #[test]
    fn a_reload_of_too_many_or_unloadable_packs_fails() {
        let ok = |_: &Path| true;
        let list = |n: usize| -> Vec<(u128, PathBuf)> {
            (0..n)
                .map(|i| (i as u128, PathBuf::from(format!("/p/{i}"))))
                .collect()
        };
        assert_eq!(open_checked(&ok, &list(MAX_ACTIVE)), Some(list(MAX_ACTIVE)));
        assert_eq!(open_checked(&ok, &list(MAX_ACTIVE + 1)), None);
        let not_3 = |p: &Path| !p.ends_with("3");
        assert_eq!(open_checked(&not_3, &list(5)), None);
    }

    /// `createDownloadHeaders`.
    #[test]
    fn downloads_say_who_asks_as_vanilla_does() {
        let h = download_headers("Dev", 0xa01a21f5_c117_3c1f_b5d0_e055b927f733);
        let get = |k: &str| h.iter().find(|(n, _)| n == k).map(|(_, v)| v.as_str());
        assert_eq!(get("X-Minecraft-Username"), Some("Dev"));
        assert_eq!(
            get("X-Minecraft-UUID"),
            Some("a01a21f5c1173c1fb5d0e055b927f733")
        );
        assert_eq!(get("X-Minecraft-Version"), Some("26.3"));
        assert_eq!(get("X-Minecraft-Version-ID"), Some("26.3"));
        assert_eq!(get("X-Minecraft-Pack-Format"), Some("97.1"));
        assert_eq!(get("User-Agent"), Some("Minecraft Java/26.3"));
        assert_eq!(h.len(), 6);
    }

    /// The prompt put away unanswered forgets its packs; nothing is sent.
    #[test]
    fn a_dismissed_prompt_forgets_its_packs() {
        let mut r = rig(ServerPackSetup::default());
        r.game
            .handle_pack_push(push("http://127.0.0.1/p.zip", false));
        r.game.answer_pack_prompt(None);
        r.game.answer_pack_prompt(Some(true));
        r.game.pack_tasks();
        assert!(r.sent().is_empty());
    }
}

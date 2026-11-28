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

use std::collections::VecDeque;
use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
#[cfg(test)]
use std::sync::atomic::Ordering;
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
    /// The packs the session before handed on (a transfer in play: [`super::Transfer`]'s
    /// `packs`); none: the session starts with none.
    pub kept: KeptPacks,
}

impl Default for ServerPackSetup {
    fn default() -> ServerPackSetup {
        ServerPackSetup {
            status: PackStatus::Prompt,
            cache_dir: None,
            check: Arc::new(|_| false),
            kept: KeptPacks::default(),
        }
    }
}

impl std::fmt::Debug for ServerPackSetup {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ServerPackSetup")
            .field("status", &self.status)
            .field("cache_dir", &self.cache_dir)
            .field("kept", &self.kept)
            .finish_non_exhaustive()
    }
}

/// A session's server packs, handed on to the next session by a transfer in play, as
/// vanilla keeps them (`ClientCommonPacketListenerImpl.onDisconnect` calls
/// `Minecraft.disconnect(screen, isTransferring)`, whose `keepResourcePacks` skips
/// `clearDownloadedResourcePacks`, Minecraft.java:2116; a transfer in configuration drops
/// them, `ClientConfigurationPacketListenerImpl.java:210`). Vanilla's `ServerPackManager`
/// simply lives on; here the session's pack state (the manager, the downloads under way,
/// the checks) moves from the game thread of the session left to the next one's.
///
/// Shared by the session that hands them on (its game thread puts them in, its
/// [`super::Session`] stays), the [`super::Transfer`] that names them and the next
/// session, which takes them ([`ServerPackSetup::kept`]). A reload's answer that reaches
/// the session left after it stopped taking them waits here for the next session, whose
/// packs are waiting for it.
#[derive(Clone, Default)]
pub struct KeptPacks(Arc<std::sync::Mutex<Kept>>);

#[derive(Default)]
struct Kept {
    packs: Option<Box<Packs>>,
    /// Packs were handed on (they may have been taken since).
    handed: bool,
    /// Answers to the reload under way that came too late for the session left.
    answers: Vec<bool>,
}

impl KeptPacks {
    fn lock(&self) -> std::sync::MutexGuard<'_, Kept> {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Whether the session handed its packs on.
    pub fn kept(&self) -> bool {
        self.lock().handed
    }

    /// Packs handed on (none in fact), for tests of what follows such a transfer.
    #[doc(hidden)]
    pub fn handed_for_tests() -> KeptPacks {
        let k = KeptPacks::default();
        k.lock().handed = true;
        k
    }

    pub(super) fn put(&self, packs: Packs) {
        let mut k = self.lock();
        k.packs = Some(Box::new(packs));
        k.handed = true;
    }

    fn take(&self) -> Option<Box<Packs>> {
        self.lock().packs.take()
    }

    /// The reload under way ended, and the session it was for takes no more: if that
    /// session handed its packs on, they take the answer (else it is of no use).
    pub(super) fn late_answer(&self, ok: bool) {
        let mut k = self.lock();
        if k.handed {
            k.answers.push(ok);
        }
    }

    fn take_answers(&self) -> Vec<bool> {
        std::mem::take(&mut self.lock().answers)
    }
}

impl std::fmt::Debug for KeptPacks {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let k = self.lock();
        f.debug_struct("KeptPacks")
            .field("handed", &k.handed)
            .field("waiting", &k.packs.is_some())
            .finish_non_exhaustive()
    }
}

/// Two are the same if both handed packs on or neither did.
impl PartialEq for KeptPacks {
    fn eq(&self, other: &KeptPacks) -> bool {
        self.kept() == other.kept()
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
    jobs: Option<std::sync::mpsc::SyncSender<Batch>>,
    results_tx: Sender<BatchResult>,
    results: Receiver<BatchResult>,
    /// Cancelled when the session ends: stops downloads between reads/redirects and shuts
    /// down active sockets via `ureq::Agent::shutdown` immediately.
    cancel: download::CancelToken,
}

impl Default for Downloads {
    fn default() -> Downloads {
        let (results_tx, results) = channel();
        Downloads {
            jobs: None,
            results_tx,
            results,
            cancel: download::CancelToken::new(),
        }
    }
}

impl Drop for Downloads {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}

/// A downloaded file, as its check knows it: its path, length and modification time.
type FileKey = (PathBuf, u64, Option<std::time::SystemTime>);

/// How many checks [`Checks`] keeps: two for every pack that can be tracked.
const CHECKS_KEPT: usize = 2 * MAX_TRACKED;

/// The checks of downloaded packs (`readPackMetadata`), by file: made on the download
/// thread as each download ends, so that the check when a reload starts, on the game
/// thread, only looks its files up (vanilla reads every pack's metadata on its main
/// thread then). A file that changed since is checked again; one that is gone fails.
#[derive(Clone, Default)]
pub(super) struct Checks(Arc<std::sync::Mutex<VecDeque<(FileKey, bool)>>>);

impl Checks {
    fn key(path: &Path) -> Option<FileKey> {
        let m = std::fs::metadata(path).ok()?;
        Some((path.to_owned(), m.len(), m.modified().ok()))
    }

    /// Whether `path` passes `check`: the kept answer for the file as it is now, else
    /// `check`'s, kept.
    fn check(&self, check: &(dyn Fn(&Path) -> bool + Send + Sync), path: &Path) -> bool {
        let Some(key) = Checks::key(path) else {
            return false;
        };
        let kept = || {
            self.0
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
        };
        if let Some(&(_, ok)) = kept().iter().find(|(k, _)| *k == key) {
            return ok;
        }
        let ok = check(path);
        let mut kept = kept();
        kept.retain(|(k, _)| k.0 != key.0);
        kept.push_back((key, ok));
        while kept.len() > CHECKS_KEPT {
            kept.pop_front();
        }
        ok
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
    /// The checks of the packs downloaded.
    checks: Checks,
    /// Where these packs came from, if the session before handed them on: a reload's
    /// answer that came too late for it is taken from there.
    from: Option<KeptPacks>,
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
        let checks = Checks::default();
        let kept = checks.clone();
        let mut manager = ServerPacks::new(move |list: &[(u128, PathBuf)]| {
            open_checked(&|p: &Path| kept.check(&*check, p), list)
        });
        manager.configure_for_server_control(prompt_status(setup.status));
        Packs {
            manager,
            setup,
            prompt: None,
            downloads: Downloads::default(),
            quiet: false,
            cleaned: false,
            peer: None,
            checks,
            from: None,
        }
    }

    /// A session's packs: those the session before handed on (`setup.kept`), or none.
    /// Kept packs stay loaded, and the new server is told of them from now on
    /// (`ConnectScreen.java:138`: `configureForServerControl` with the server list
    /// entry's setting, which the transfer keeps). The prompt that was showing is gone
    /// (vanilla's screen made way for the `ConnectScreen`), and downloads from now on go
    /// as this server's (its address decides which hosts may serve them); a download
    /// under way finishes as the last server's.
    pub(super) fn for_session(mut setup: ServerPackSetup) -> Packs {
        let kept = std::mem::take(&mut setup.kept);
        let Some(mut p) = kept.take() else {
            return Packs::new(setup);
        };
        eprintln!(
            "[server packs] {} pack(s) kept from the server left",
            p.manager.tracked()
        );
        p.manager
            .configure_for_server_control(prompt_status(setup.status));
        p.setup = setup;
        p.prompt = None;
        p.quiet = false;
        p.cleaned = false;
        p.peer = None;
        p.downloads.jobs = None;
        // (An answer still on its way to the packs' last home follows them.)
        let late = p.from.replace(kept.clone()).map(|k| k.take_answers());
        kept.lock().answers.extend(late.into_iter().flatten());
        *p
    }

    /// What stays of a session's packs once it handed them on: nothing to clean up.
    pub(super) fn handed_on() -> Packs {
        Packs {
            quiet: true,
            cleaned: true,
            ..Packs::default()
        }
    }
}

/// The private addresses a server's pack hosts may have: none unless the server itself
/// is on this machine or the local network, and then only the server's own address (and
/// for a server on this machine, this machine's under either family), not the whole
/// private range: a server on the local network can't send the client to the router or
/// another machine there.
pub fn pack_hosts_allowed(peer: Option<IpAddr>) -> Vec<IpAddr> {
    let Some(peer) = peer.map(|p| match p {
        IpAddr::V6(v6) => v6.to_ipv4_mapped().map_or(p, IpAddr::V4),
        v4 => v4,
    }) else {
        return Vec::new();
    };
    if !is_local(peer) {
        return Vec::new();
    }
    let mut out = vec![peer];
    if peer.is_loopback() {
        for ip in [
            IpAddr::V4(std::net::Ipv4Addr::LOCALHOST),
            IpAddr::V6(std::net::Ipv6Addr::LOCALHOST),
        ] {
            if !out.contains(&ip) {
                out.push(ip);
            }
        }
    }
    out
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
            if self
                .packs
                .prompt
                .as_ref()
                .is_some_and(|p| p.requests.len() >= MAX_PROMPTED)
            {
                eprintln!(
                    "[server packs] {:032x}: the prompt already has {MAX_PROMPTED} packs; discarded",
                    push.id
                );
                self.send_pack_status(push.id, PackAction::Discarded);
                return;
            }
            let mut requests = self
                .packs
                .prompt
                .take()
                .map_or_else(Vec::new, |p| p.requests);
            requests.push((push.id, push.url, push.hash));
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
        eprintln!("[server packs] {id:032x}: pushed ({url:?})");
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
        let late = self.packs.from.as_ref().map(KeptPacks::take_answers);
        for ok in late.into_iter().flatten() {
            eprintln!("[server packs] the reload's answer came after the transfer");
            self.resources_reloaded(ok);
        }
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
        self.packs.downloads.cancel.cancel();
        self.packs.manager.cleanup_after_disconnect();
        self.apply_pack_effects();
        // The download thread stops right away via `cancel.cancel()`.
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
            let (hdr_name, hdr_uuid) = if self.local_name.is_empty() {
                (self.name.as_str(), self.uuid)
            } else {
                (self.local_name.as_str(), self.local_uuid)
            };
            let config = download::DownloadConfig {
                cache_dir,
                headers: download_headers(hdr_name, hdr_uuid),
                // Only the server's own address (and, for a server on this machine, the
                // loopback addresses) may serve its packs when it isn't public.
                private_allowed: pack_hosts_allowed(self.packs.peer)
                    .into_iter()
                    .map(download::AddressRange::host)
                    .collect(),
            };
            let (jobs, rx) = std::sync::mpsc::sync_channel::<Batch>(MAX_TRACKED);
            let results = self.packs.downloads.results_tx.clone();
            let events = self.events.clone();
            let cancel = self.packs.downloads.cancel.clone();
            let (checks, check) = (self.packs.checks.clone(), self.packs.setup.check.clone());
            let spawned = std::thread::Builder::new()
                .name("pack-download".into())
                .spawn(move || {
                    for batch in rx {
                        let r = run_batch(&config, &batch, &events, &cancel);
                        // Checked here, while the session goes on.
                        for path in r.1.iter().filter_map(|(_, p)| p.as_ref().ok()) {
                            checks.check(&*check, path);
                        }
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
            .is_some_and(|j| j.try_send(batch.clone()).is_ok());
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
    cancel: &download::CancelToken,
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
                    eprintln!("[server packs] {id:032x}: failed to download {url:?}: {e}");
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
    cancel: download::CancelToken,
}

impl Toast {
    fn new(total: usize, events: Sender<SessionEvent>, cancel: download::CancelToken) -> Toast {
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
        self.cancel.is_cancelled()
    }

    fn cancel_token(&self) -> Option<download::CancelToken> {
        Some(self.cancel.clone())
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
        game.packs = Packs::for_session(setup);
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

    /// Downloads go to the test, which hands back their results ([`deliver`]).
    fn fake_downloads(r: &mut Rig) -> Receiver<Batch> {
        let (tx, rx) = std::sync::mpsc::sync_channel(MAX_TRACKED);
        r.game.packs.downloads.jobs = Some(tx);
        rx
    }

    /// The batch asked for, downloaded: each pack to its file in `files` (else failed).
    fn deliver(r: &mut Rig, jobs: &Receiver<Batch>, files: &[(u128, &Path)]) {
        let batch = jobs.try_recv().expect("a download");
        let result = batch
            .requests
            .iter()
            .map(|q| {
                let file = files.iter().find(|(id, _)| *id == q.id);
                (q.id, file.map(|(_, p)| p.to_path_buf()).ok_or(()))
            })
            .collect();
        r.game
            .packs
            .downloads
            .results_tx
            .send((batch.batch, result))
            .unwrap();
        r.game.pack_tasks();
    }

    /// The reloads the session asked for since the last call.
    fn reloads(r: &Rig) -> Vec<Vec<(u128, PathBuf)>> {
        r.events()
            .into_iter()
            .filter_map(|e| match e {
                SessionEvent::ReloadResources(r) => Some(r.packs),
                _ => None,
            })
            .collect()
    }

    const OTHER: u128 = 0x9999_aaaa_bbbb_cccc_dddd_eeee_ffff_0000;

    /// Two packs on disk, and a server list setting that takes packs.
    fn two_packs() -> (tempfile::TempDir, PathBuf, PathBuf, ServerPackSetup) {
        let tmp = tempfile::tempdir().unwrap();
        let (a, b) = (tmp.path().join("a.zip"), tmp.path().join("b.zip"));
        std::fs::write(&a, b"one").unwrap();
        std::fs::write(&b, b"two").unwrap();
        let setup = ServerPackSetup {
            status: PackStatus::Enabled,
            cache_dir: Some(tmp.path().to_owned()),
            check: Arc::new(|_| true),
            kept: KeptPacks::default(),
        };
        (tmp, a, b, setup)
    }

    fn transfer_packet(phase: Phase) -> Event {
        let t = ruthenium_protocol::packets::Common::Transfer {
            host: "127.0.0.1".into(),
            port: 25612,
        };
        match phase {
            Phase::Play => Event::Play(play::Clientbound::Common(t)),
            _ => Event::Configuration(cfg::Clientbound::Common(t)),
        }
    }

    /// The first server, in `phase`, has the pack `ID` (`a`) applied and transfers
    /// the player: what its session reported at its end, and the transfer.
    fn applied_then_transferred(
        phase: Phase,
        a: &Path,
        setup: &ServerPackSetup,
    ) -> (Vec<SessionEvent>, super::super::Transfer) {
        let mut r = rig(setup.clone());
        r.game.phase = phase;
        let jobs = fake_downloads(&mut r);
        r.game
            .handle_pack_push(push("http://127.0.0.1/a.zip", false));
        r.game.pack_tasks();
        deliver(&mut r, &jobs, &[(ID, a)]);
        assert_eq!(reloads(&r), [vec![(ID, a.to_owned())]]);
        r.game.resources_reloaded(true);
        let s = |a| match phase {
            Phase::Play => Sent::Packet(play::resource_pack(ID, a)),
            _ => status(ID, a),
        };
        assert_eq!(
            r.sent(),
            [
                s(PackAction::Accepted),
                s(PackAction::Downloaded),
                s(PackAction::SuccessfullyLoaded)
            ]
        );
        r.remote.event(transfer_packet(phase));
        assert!(r.game.frame(super::super::FrameInput::default()));
        r.remote
            .event(Event::Closed(ruthenium_net::connection::Closed {
                during_login: false,
                reason: Reason::Local(Local::Closed),
            }));
        assert!(!r.game.frame(super::super::FrameInput::default()));
        let events = r.events();
        let t = events
            .iter()
            .find_map(|e| match e {
                SessionEvent::Transfer(t) => Some(t.clone()),
                _ => None,
            })
            .expect("a transfer");
        (events, t)
    }

    /// The next server's session, with what the transfer handed on; its downloads are
    /// its own (none under way as the last server's: the next goes as this server's,
    /// whose address decides which hosts may serve it).
    fn next_session(setup: &ServerPackSetup, t: &super::super::Transfer) -> (Rig, Receiver<Batch>) {
        let mut r = rig(ServerPackSetup {
            kept: t.packs.clone(),
            ..setup.clone()
        });
        assert!(r.game.packs.downloads.jobs.is_none());
        let jobs = fake_downloads(&mut r);
        (r, jobs)
    }

    /// Ends a session the way a server's disconnect does.
    fn end_session(r: &mut Rig) {
        r.remote
            .event(Event::Closed(ruthenium_net::connection::Closed {
                during_login: false,
                reason: Reason::Text(Tag::String("Bye".into())),
            }));
        assert!(!r.game.frame(super::super::FrameInput::default()));
    }

    /// A transfer in play keeps the packs, as vanilla's `disconnect(screen, true)`: the
    /// session left asks for no reload, and the next one starts with them active and
    /// still says nothing of them. Its push of the same pack is that pack again:
    /// accepted, its file downloaded (vanilla's cache has it), and the same packs
    /// reloaded (the client already has them in use); the chain's end drops them.
    #[test]
    fn a_transfer_in_play_keeps_the_packs_until_the_chain_ends() {
        let (_tmp, a, _, setup) = two_packs();
        let (events, t) = applied_then_transferred(Phase::Play, &a, &setup);
        assert!(t.packs.kept());
        assert!(
            !events
                .iter()
                .any(|e| matches!(e, SessionEvent::ReloadResources(_))),
            "{events:?}"
        );
        let (mut b, jobs) = next_session(&setup, &t);
        assert_eq!(b.game.packs.manager.tracked(), 1);
        b.game.pack_frame_start();
        b.game.pack_tasks();
        assert!(b.sent().is_empty() && b.events().is_empty());
        assert!(jobs.try_recv().is_err());
        b.game
            .handle_pack_push(push("http://127.0.0.1/a.zip", false));
        b.game.pack_tasks();
        deliver(&mut b, &jobs, &[(ID, &a)]);
        assert_eq!(reloads(&b), [vec![(ID, a.clone())]]);
        b.game.resources_reloaded(true);
        assert_eq!(
            b.sent(),
            [
                status(ID, PackAction::Accepted),
                status(ID, PackAction::Downloaded),
                status(ID, PackAction::SuccessfullyLoaded)
            ]
        );
        end_session(&mut b);
        assert_eq!(reloads(&b), [Vec::new()]);
    }

    /// The next server leaves the kept packs alone, pops one, pops all, or pushes
    /// another: as vanilla's manager, which lives on, does with them.
    #[test]
    fn the_next_server_has_the_kept_packs_as_its_own() {
        let (_tmp, a, b_file, setup) = two_packs();
        // Nothing pushed: they stay for as long as the session does.
        let (_, t) = applied_then_transferred(Phase::Play, &a, &setup);
        let (mut b, _jobs) = next_session(&setup, &t);
        for _ in 0..3 {
            b.game.pack_frame_start();
            b.game.pack_tasks();
        }
        assert!(reloads(&b).is_empty());
        end_session(&mut b);
        assert_eq!(reloads(&b), [Vec::new()]);
        // A pop of the kept pack, or of all: gone at once.
        for id in [Some(ID), None] {
            let (_, t) = applied_then_transferred(Phase::Play, &a, &setup);
            let (mut b, _jobs) = next_session(&setup, &t);
            b.game.handle_pack_pop(id);
            b.game.pack_tasks();
            assert_eq!(reloads(&b), [Vec::new()], "{id:?}");
            // (Vanilla says nothing of a pack the server took away.)
            assert!(b.sent().is_empty());
            b.game.resources_reloaded(true);
            end_session(&mut b);
            assert!(reloads(&b).is_empty(), "nothing left to drop");
        }
        // Another pack: on top of the kept one.
        let (_, t) = applied_then_transferred(Phase::Play, &a, &setup);
        let (mut b, jobs) = next_session(&setup, &t);
        b.game.handle_pack_push(PackPush {
            id: OTHER,
            ..push("http://127.0.0.1/b.zip", false)
        });
        b.game.pack_tasks();
        deliver(&mut b, &jobs, &[(OTHER, &b_file)]);
        let got = reloads(&b);
        assert_eq!(got.len(), 1);
        let mut ids: Vec<u128> = got[0].iter().map(|(id, _)| *id).collect();
        ids.sort_unstable();
        assert_eq!(ids, [ID, OTHER]);
        b.game.resources_reloaded(true);
        end_session(&mut b);
        assert_eq!(reloads(&b), [Vec::new()]);
    }

    /// A server list entry set to Disabled (the transfer keeps the entry) declines what
    /// the next server offers, and the kept pack, accepted before, stays
    /// (`rejectServerPacks` removes only packs not accepted).
    #[test]
    fn a_declining_entry_keeps_what_was_accepted() {
        let (_tmp, a, _, setup) = two_packs();
        let (_, t) = applied_then_transferred(Phase::Play, &a, &setup);
        let declined = ServerPackSetup {
            status: PackStatus::Disabled,
            ..setup.clone()
        };
        let (mut b, _jobs) = next_session(&declined, &t);
        b.game.handle_pack_push(PackPush {
            id: OTHER,
            ..push("http://127.0.0.1/b.zip", false)
        });
        b.game.pack_tasks();
        assert_eq!(b.sent(), [status(OTHER, PackAction::Declined)]);
        assert!(reloads(&b).is_empty());
        end_session(&mut b);
        assert_eq!(reloads(&b), [Vec::new()]);
    }

    /// A transfer in configuration drops the packs
    /// (`ClientConfigurationPacketListenerImpl.onDisconnect`): the session left goes
    /// back to vanilla's, and the next one starts with none.
    #[test]
    fn a_transfer_in_configuration_drops_the_packs() {
        let (_tmp, a, _, setup) = two_packs();
        let (events, t) = applied_then_transferred(Phase::Configuration, &a, &setup);
        assert!(!t.packs.kept());
        let at = |f: fn(&SessionEvent) -> bool| events.iter().position(f).unwrap();
        assert!(
            at(|e| matches!(e, SessionEvent::ReloadResources(r) if r.packs.is_empty()))
                < at(|e| matches!(e, SessionEvent::Transfer(_)))
        );
        let (mut b, _jobs) = next_session(&setup, &t);
        assert_eq!(b.game.packs.manager.tracked(), 0);
        end_session(&mut b);
        assert!(reloads(&b).is_empty());
    }

    /// A reload under way at a transfer in play goes on: its answer, whether it comes to
    /// the next session or too late to the one left ([`KeptPacks::late_answer`]), is
    /// the kept packs'.
    #[test]
    fn a_reload_under_way_at_a_transfer_is_answered_to_the_next_session() {
        let (_tmp, a, _, setup) = two_packs();
        for late in [false, true] {
            let mut r = rig(ServerPackSetup {
                status: PackStatus::Enabled,
                ..setup.clone()
            });
            r.game.phase = Phase::Play;
            let jobs = fake_downloads(&mut r);
            r.game
                .handle_pack_push(push("http://127.0.0.1/a.zip", false));
            r.game.pack_tasks();
            deliver(&mut r, &jobs, &[(ID, &a)]);
            assert_eq!(reloads(&r).len(), 1);
            r.remote.event(transfer_packet(Phase::Play));
            assert!(r.game.frame(super::super::FrameInput::default()));
            r.remote
                .event(Event::Closed(ruthenium_net::connection::Closed {
                    during_login: false,
                    reason: Reason::Local(Local::Closed),
                }));
            assert!(!r.game.frame(super::super::FrameInput::default()));
            let t = r
                .events()
                .into_iter()
                .find_map(|e| match e {
                    SessionEvent::Transfer(t) => Some(t),
                    _ => None,
                })
                .unwrap();
            let (mut b, _jobs) = next_session(&setup, &t);
            if late {
                t.packs.late_answer(true);
                b.game.pack_frame_start();
            } else {
                b.game.resources_reloaded(true);
            }
            assert_eq!(
                b.sent(),
                [status(ID, PackAction::SuccessfullyLoaded)],
                "late: {late}"
            );
            // Not twice.
            b.game.pack_frame_start();
            assert!(b.sent().is_empty());
        }
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

    /// A pack is checked once for the file it is: again once it changed; a file that is
    /// gone fails without a check; at most [`CHECKS_KEPT`] are kept.
    #[test]
    fn pack_checks_are_kept_by_file() {
        use std::sync::atomic::AtomicUsize;
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        let file = dir.join("p.zip");
        std::fs::write(&file, b"one").unwrap();
        let calls = Arc::new(AtomicUsize::new(0));
        let counted = calls.clone();
        let check = move |_: &Path| {
            counted.fetch_add(1, Ordering::Relaxed);
            true
        };
        let checks = Checks::default();
        assert!(checks.check(&check, &file));
        assert!(checks.check(&check, &file));
        assert_eq!(calls.load(Ordering::Relaxed), 1);
        std::fs::write(&file, b"longer").unwrap();
        assert!(checks.check(&check, &file));
        assert_eq!(calls.load(Ordering::Relaxed), 2);
        assert!(!checks.check(&check, &dir.join("gone.zip")));
        assert_eq!(calls.load(Ordering::Relaxed), 2);
        for i in 0..CHECKS_KEPT + 5 {
            let f = dir.join(format!("{i}.zip"));
            std::fs::write(&f, b"x").unwrap();
            checks.check(&check, &f);
        }
        assert_eq!(checks.0.lock().unwrap().len(), CHECKS_KEPT);
    }

    /// Only the server's own address is allowed for pack hosts, and only when it is
    /// local; a server on this machine allows this machine under either family.
    #[test]
    fn pack_hosts_may_be_the_local_server_only() {
        let ip = |s: &str| s.parse::<IpAddr>().unwrap();
        assert!(pack_hosts_allowed(None).is_empty());
        assert!(pack_hosts_allowed(Some(ip("203.0.113.9"))).is_empty());
        assert_eq!(
            pack_hosts_allowed(Some(ip("192.168.1.20"))),
            [ip("192.168.1.20")]
        );
        assert_eq!(
            pack_hosts_allowed(Some(ip("::ffff:10.0.0.7"))),
            [ip("10.0.0.7")]
        );
        assert_eq!(
            pack_hosts_allowed(Some(ip("127.0.0.1"))),
            [ip("127.0.0.1"), ip("::1")]
        );
        assert_eq!(
            pack_hosts_allowed(Some(ip("::1"))),
            [ip("::1"), ip("127.0.0.1")]
        );
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

    /// Pushing a non-required pack past [`MAX_PROMPTED`] discards it without altering
    /// `required` or emitting another UI prompt event.
    #[test]
    fn prompt_at_cap_preserves_required_and_emits_no_extra_event() {
        let mut r = rig(ServerPackSetup::default());
        for i in 0..MAX_PROMPTED as u128 {
            r.game.handle_pack_push(PackPush {
                id: i,
                ..push("http://127.0.0.1/p.zip", true)
            });
        }
        let events_before = r.events().len();
        assert_eq!(events_before, MAX_PROMPTED);
        r.game.handle_pack_push(PackPush {
            id: MAX_PROMPTED as u128,
            ..push("http://127.0.0.1/extra.zip", false)
        });
        assert_eq!(
            r.sent(),
            [status(MAX_PROMPTED as u128, PackAction::Discarded)]
        );
        assert!(r.events().is_empty(), "no extra prompt event past cap");
        // Declining must still disconnect because the original prompt was required.
        r.game.answer_pack_prompt(Some(false));
        assert!(r.game.close_reason.is_some());
    }
}

//! Server resource packs on the game thread: vanilla's `ServerPackManager` (what is
//! pushed, accepted, downloaded and active), and the parts of `DownloadedPackSource` and
//! `Minecraft` that drive it (the update scheduler, starting a reload with the packs to
//! load, recovering from a failed one).
//!
//! Plain logic, as vanilla's is: what it asks for (a download, a reload, a status for the
//! server) comes out of [`ServerPacks`] to be done by the session, and the answers (a
//! download's result, a reload's) go back in. A pack is only ever named by the server's
//! id and the file the download left in the cache; nothing here reads a pack.
//!
//! The order of everything follows vanilla's single main thread: the manager's
//! `registerForUpdate` schedules one task on the main executor (`Minecraft.schedule`),
//! which runs in the frame's `runAllTasks`, after the frame's packets; a download's
//! result is such a task too. [`ServerPacks::run_tasks`] is that step.
//! `golden_tests` compares the statuses and requests with vanilla's own classes driven
//! the same way.

use std::collections::VecDeque;
use std::path::PathBuf;

/// `PackLoadFeedback.Update` and `FinalResult`, as the server is told them
/// (`DownloadedPackSource.createPackResponseSender`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Feedback {
    Accepted,
    Downloaded,
    /// `APPLIED`: `SUCCESSFULLY_LOADED`.
    Applied,
    Declined,
    Discarded,
    /// `DOWNLOAD_FAILED`: `FAILED_DOWNLOAD`.
    DownloadFailed,
    /// `ACTIVATION_FAILED`: `FAILED_RELOAD`.
    ActivationFailed,
}

impl Feedback {
    /// Vanilla's name of it in its enum (for logs and the golden cases).
    pub fn name(self) -> &'static str {
        match self {
            Feedback::Accepted => "ACCEPTED",
            Feedback::Downloaded => "DOWNLOADED",
            Feedback::Applied => "APPLIED",
            Feedback::Declined => "DECLINED",
            Feedback::Discarded => "DISCARDED",
            Feedback::DownloadFailed => "DOWNLOAD_FAILED",
            Feedback::ActivationFailed => "ACTIVATION_FAILED",
        }
    }
}

/// `ServerPackManager.PackPromptStatus`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromptStatus {
    Pending,
    Allowed,
    Declined,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Activation {
    Inactive,
    Pending,
    Active,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Download {
    Requested,
    Pending,
    Done,
}

/// `ServerPackManager.RemovalReason`, with the final result each one reports.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Removal {
    DownloadFailed,
    ActivationFailed,
    Declined,
    Discarded,
    ServerRemoved,
    ServerReplaced,
}

impl Removal {
    fn server_response(self) -> Option<Feedback> {
        match self {
            Removal::DownloadFailed => Some(Feedback::DownloadFailed),
            Removal::ActivationFailed => Some(Feedback::ActivationFailed),
            Removal::Declined => Some(Feedback::Declined),
            Removal::Discarded => Some(Feedback::Discarded),
            Removal::ServerRemoved | Removal::ServerReplaced => None,
        }
    }
}

/// `ServerPackData`. `serial` stands for the object's identity: the same id can be
/// pushed again while the old entry is still in the list.
#[derive(Debug, Clone)]
struct PackData {
    serial: u64,
    id: u128,
    url: String,
    hash: Option<[u8; 20]>,
    path: Option<PathBuf>,
    removal: Option<Removal>,
    download: Download,
    activation: Activation,
    prompt_accepted: bool,
}

impl PackData {
    fn set_removal_if_not_set(&mut self, reason: Removal) {
        if self.removal.is_none() {
            self.removal = Some(reason);
        }
    }

    fn is_removed(&self) -> bool {
        self.removal.is_some()
    }
}

/// One pack of a download batch (`DownloadQueue.DownloadRequest` with its id).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    pub id: u128,
    pub url: String,
    pub hash: Option<[u8; 20]>,
}

/// A batch the manager asked for (`PackDownloader.download`): its requests in the order
/// vanilla's `HashMap` of them is gone through. Its result goes back with
/// [`ServerPacks::downloaded`] and the same `batch`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Batch {
    pub batch: u64,
    pub requests: Vec<Request>,
}

/// The callbacks of a scheduled reload (`PackReloadConfig.Callbacks`): the entries it
/// loads and unloads.
#[derive(Debug, Clone)]
struct Callbacks {
    to_load: Vec<u64>,
    to_unload: Vec<u64>,
}

/// See the module docs.
#[derive(Debug)]
struct Manager {
    prompt: PromptStatus,
    packs: Vec<PackData>,
    next_serial: u64,
    next_batch: u64,
    /// Batches out: their entries, by batch.
    out: Vec<(u64, Vec<u64>)>,
}

/// What the manager asks of the rest: `PackDownloader`, `PackLoadFeedback`,
/// `PackReloadConfig` and the update request. The source side of [`ServerPacks`].
trait Env {
    fn feedback(&mut self, id: u128, f: Feedback);
    fn download(&mut self, batch: Batch);
    fn schedule_reload(&mut self, manager: &mut Manager, callbacks: Callbacks);
    fn register_for_update(&mut self);
}

impl Manager {
    fn new(prompt: PromptStatus) -> Manager {
        Manager {
            prompt,
            packs: Vec::new(),
            next_serial: 0,
            next_batch: 0,
            out: Vec::new(),
        }
    }

    fn by_serial(&mut self, serial: u64) -> Option<&mut PackData> {
        self.packs.iter_mut().find(|p| p.serial == serial)
    }

    fn mark_existing_packs_as_removed(&mut self, id: u128) {
        for p in &mut self.packs {
            if p.id == id {
                p.set_removal_if_not_set(Removal::ServerReplaced);
            }
        }
    }

    fn push_pack(&mut self, env: &mut dyn Env, id: u128, url: String, hash: Option<[u8; 20]>) {
        if self.prompt == PromptStatus::Declined {
            env.feedback(id, Feedback::Declined);
            return;
        }
        let serial = self.next_serial;
        self.next_serial += 1;
        let pack = PackData {
            serial,
            id,
            url,
            hash,
            path: None,
            removal: None,
            download: Download::Requested,
            activation: Activation::Inactive,
            prompt_accepted: false,
        };
        self.mark_existing_packs_as_removed(id);
        self.packs.push(pack);
        if self.prompt == PromptStatus::Allowed {
            let last = self.packs.len() - 1;
            Self::accept_pack(env, &mut self.packs[last]);
        }
        env.register_for_update();
    }

    fn accept_pack(env: &mut dyn Env, pack: &mut PackData) {
        env.feedback(pack.id, Feedback::Accepted);
        pack.prompt_accepted = true;
    }

    fn pop_pack(&mut self, env: &mut dyn Env, id: u128) {
        if let Some(p) = self
            .packs
            .iter_mut()
            .find(|p| !p.is_removed() && p.id == id)
        {
            p.set_removal_if_not_set(Removal::ServerRemoved);
            env.register_for_update();
        }
    }

    fn pop_all(&mut self, env: &mut dyn Env) {
        for p in &mut self.packs {
            p.set_removal_if_not_set(Removal::ServerRemoved);
        }
        env.register_for_update();
    }

    fn allow_server_packs(&mut self, env: &mut dyn Env) {
        self.prompt = PromptStatus::Allowed;
        for p in &mut self.packs {
            if !p.prompt_accepted && !p.is_removed() {
                Self::accept_pack(env, p);
            }
        }
        env.register_for_update();
    }

    fn reject_server_packs(&mut self, env: &mut dyn Env) {
        self.prompt = PromptStatus::Declined;
        for p in &mut self.packs {
            if !p.prompt_accepted {
                p.set_removal_if_not_set(Removal::Declined);
            }
        }
        env.register_for_update();
    }

    fn reset_prompt_status(&mut self) {
        self.prompt = PromptStatus::Pending;
    }

    fn tick(&mut self, env: &mut dyn Env) {
        let downloads_pending = self.update_downloads(env);
        if !downloads_pending {
            self.trigger_reload_if_needed(env);
        }
        self.cleanup_removed_packs(env);
    }

    fn cleanup_removed_packs(&mut self, env: &mut dyn Env) {
        self.packs.retain(|p| {
            if p.activation != Activation::Inactive {
                return true;
            }
            match p.removal {
                Some(reason) => {
                    if let Some(response) = reason.server_response() {
                        env.feedback(p.id, response);
                    }
                    false
                }
                None => true,
            }
        });
    }

    /// `onDownload`: the batch's result, by id (`Ok` with the file, `Err` failed).
    fn on_download(
        &mut self,
        env: &mut dyn Env,
        data: &[u64],
        result: &[(u128, Result<PathBuf, ()>)],
    ) {
        let failed = |id: u128| result.iter().any(|(i, r)| *i == id && r.is_err());
        if result.iter().any(|(_, r)| r.is_err()) {
            for p in &mut self.packs {
                if p.activation != Activation::Active {
                    if failed(p.id) {
                        p.set_removal_if_not_set(Removal::DownloadFailed);
                    } else {
                        p.set_removal_if_not_set(Removal::Discarded);
                    }
                }
            }
        }
        for &serial in data {
            let Some(p) = self.by_serial(serial) else {
                continue;
            };
            let id = p.id;
            let file = result.iter().find_map(|(i, r)| match r {
                Ok(path) if *i == id => Some(path.clone()),
                _ => None,
            });
            if let Some(file) = file {
                p.download = Download::Done;
                p.path = Some(file);
                if !p.is_removed() {
                    env.feedback(id, Feedback::Downloaded);
                }
            }
        }
        env.register_for_update();
    }

    fn update_downloads(&mut self, env: &mut dyn Env) -> bool {
        let mut download_packs = Vec::new();
        let mut in_progress = false;
        for p in &mut self.packs {
            if !p.is_removed() && p.prompt_accepted {
                if p.download != Download::Done {
                    in_progress = true;
                }
                if p.download == Download::Requested {
                    p.download = Download::Pending;
                    download_packs.push(p.serial);
                }
            }
        }
        if !download_packs.is_empty() {
            let requests: Vec<Request> = download_packs
                .iter()
                .filter_map(|&s| self.packs.iter().find(|p| p.serial == s))
                .map(|p| Request {
                    id: p.id,
                    url: p.url.clone(),
                    hash: p.hash,
                })
                .collect();
            let batch = self.next_batch;
            self.next_batch += 1;
            self.out.push((batch, download_packs));
            env.download(Batch {
                batch,
                requests: java_hash_map_order(requests),
            });
        }
        in_progress
    }

    fn trigger_reload_if_needed(&mut self, env: &mut dyn Env) {
        let mut needs_reload = false;
        let mut to_load = Vec::new();
        let mut to_unload = Vec::new();
        for p in &self.packs {
            if p.activation == Activation::Pending {
                return;
            }
            let should_be_active =
                p.prompt_accepted && p.download == Download::Done && !p.is_removed();
            if should_be_active && p.activation == Activation::Inactive {
                to_load.push(p.serial);
                needs_reload = true;
            }
            if p.activation == Activation::Active {
                if !should_be_active {
                    needs_reload = true;
                    to_unload.push(p.serial);
                } else {
                    to_load.push(p.serial);
                }
            }
        }
        if needs_reload {
            for &s in &to_load {
                if let Some(p) = self.by_serial(s)
                    && p.activation != Activation::Active
                {
                    p.activation = Activation::Pending;
                }
            }
            for &s in &to_unload {
                if let Some(p) = self.by_serial(s) {
                    p.activation = Activation::Pending;
                }
            }
            env.schedule_reload(self, Callbacks { to_load, to_unload });
        }
    }

    fn on_success(&mut self, env: &mut dyn Env, cb: &Callbacks) {
        for &s in &cb.to_load {
            if let Some(p) = self.by_serial(s) {
                p.activation = Activation::Active;
                if p.removal.is_none() {
                    env.feedback(p.id, Feedback::Applied);
                }
            }
        }
        for &s in &cb.to_unload {
            if let Some(p) = self.by_serial(s) {
                p.activation = Activation::Inactive;
            }
        }
        env.register_for_update();
    }

    fn on_failure(&mut self, env: &mut dyn Env, cb: &mut Callbacks, is_recovery: bool) {
        if !is_recovery {
            cb.to_load.clear();
            for p in &mut self.packs {
                match p.activation {
                    Activation::Inactive => p.set_removal_if_not_set(Removal::Discarded),
                    Activation::Pending => {
                        p.activation = Activation::Inactive;
                        p.set_removal_if_not_set(Removal::ActivationFailed);
                    }
                    Activation::Active => cb.to_load.push(p.serial),
                }
            }
            env.register_for_update();
        } else {
            for p in &mut self.packs {
                if p.activation == Activation::Pending {
                    p.activation = Activation::Inactive;
                }
            }
        }
    }

    /// `packsToLoad`: the entries' ids and files, in the callbacks' order.
    fn packs_to_load(&self, cb: &Callbacks) -> Vec<(u128, PathBuf)> {
        cb.to_load
            .iter()
            .filter_map(|&s| self.packs.iter().find(|p| p.serial == s))
            .filter_map(|p| Some((p.id, p.path.clone()?)))
            .collect()
    }

    /// The entries a batch was for, once its result is in.
    fn take_batch(&mut self, batch: u64) -> Option<Vec<u64>> {
        let at = self.out.iter().position(|(b, _)| *b == batch)?;
        Some(self.out.remove(at).1)
    }
}

/// `UUID.hashCode`.
fn java_uuid_hash(id: u128) -> i32 {
    let (msb, lsb) = ((id >> 64) as u64, id as u64);
    let hilo = msb ^ lsb;
    ((hilo >> 32) as i32) ^ (hilo as i32)
}

/// The order a `java.util.HashMap` (put in this order, default capacity) goes through
/// its keys: by bucket of the spread hash, and in a bucket by insertion. Treeified
/// buckets need eight keys in one bucket; batches are far smaller in practice, and the
/// order is only the download order.
fn java_hash_map_order(requests: Vec<Request>) -> Vec<Request> {
    let mut capacity = 16usize;
    while requests.len() > capacity * 3 / 4 {
        capacity *= 2;
    }
    let bucket = |id: u128| {
        let h = java_uuid_hash(id);
        let spread = h ^ ((h as u32) >> 16) as i32;
        (spread as u32 as usize) & (capacity - 1)
    };
    let mut keyed: Vec<(usize, usize, Request)> = requests
        .into_iter()
        .enumerate()
        .map(|(i, r)| (bucket(r.id), i, r))
        .collect();
    keyed.sort_by_key(|(b, i, _)| (*b, *i));
    keyed.into_iter().map(|(_, _, r)| r).collect()
}

/// A reload to make: the packs over vanilla's, lowest first, and whether it is the
/// recovery after a failed one (`reloadResourcePacks(isRecovery)`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reload<P> {
    pub packs: Vec<P>,
    pub recovery: bool,
}

/// What the session is asked to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Effect<P> {
    /// Tell the server (`ServerboundResourcePackPacket`).
    Feedback(u128, Feedback),
    /// Download a batch, off the game thread; its result goes to
    /// [`ServerPacks::downloaded`].
    Download(Batch),
    /// Load the game's resources anew with these packs; the result goes to
    /// [`ServerPacks::reloaded`].
    Reload(Reload<P>),
    /// Even the packs that were active could not be loaded again
    /// (`abortResourcePackRecovery`): the session ends.
    Abort,
}

#[derive(Debug)]
enum Task {
    /// The update scheduler's `runAllUpdates`.
    Updates,
    /// A download's result (`thenAcceptAsync(output, mainThreadExecutor)`).
    Downloaded(u64, Vec<(u128, Result<PathBuf, ()>)>),
}

/// `loadRequestedPacks` (see [`Source::open`]).
type Open<P> = Box<dyn FnMut(&[(u128, PathBuf)]) -> Option<Vec<P>> + Send>;

/// `DownloadedPackSource`'s side, and `Minecraft`'s reloads.
struct Source<P> {
    /// `loadRequestedPacks`: the packs to load, as `packsToLoad` gives them, checked
    /// and in the stack's order (lowest first); `None` when one of them has no valid
    /// metadata.
    open: Open<P>,
    /// `packFeedback`: the server's (`createPackResponseSender`), or only the log
    /// (`LOG_ONLY_FEEDBACK`).
    to_server: bool,
    effects: Vec<Effect<P>>,
    tasks: VecDeque<Task>,
    scheduled: bool,
    has_updates: bool,
    pending_reload: Option<Callbacks>,
    /// `packSource`: what the next reload loads.
    pack_source: Vec<P>,
    /// `Minecraft`: the reload under way (whether it is a recovery), and one asked
    /// for meanwhile (`pendingReload`).
    running: Option<bool>,
    queued: bool,
}

impl<P: Clone> Env for Source<P> {
    fn feedback(&mut self, id: u128, f: Feedback) {
        if self.to_server {
            self.effects.push(Effect::Feedback(id, f));
        }
    }

    fn download(&mut self, batch: Batch) {
        self.effects.push(Effect::Download(batch));
    }

    /// `startReload`.
    fn schedule_reload(&mut self, manager: &mut Manager, mut callbacks: Callbacks) {
        let normal = manager.packs_to_load(&callbacks);
        let packs = match (self.open)(&normal) {
            Some(p) => p,
            None => {
                manager.on_failure(self, &mut callbacks, false);
                let recovery = manager.packs_to_load(&callbacks);
                (self.open)(&recovery).unwrap_or_else(|| {
                    eprintln!("[server packs] double failure in loading server packs");
                    Vec::new()
                })
            }
        };
        self.pending_reload = Some(callbacks);
        self.pack_source = packs;
        self.reload_resource_packs(false);
    }

    fn register_for_update(&mut self) {
        self.has_updates = true;
        if !self.scheduled {
            self.scheduled = true;
            self.tasks.push_back(Task::Updates);
        }
    }
}

impl<P: Clone> Source<P> {
    /// `Minecraft.reloadResourcePacks(isRecovery)`.
    fn reload_resource_packs(&mut self, recovery: bool) {
        if self.queued {
            return;
        }
        if !recovery && self.running.is_some() {
            self.queued = true;
            return;
        }
        self.running = Some(recovery);
        self.effects.push(Effect::Reload(Reload {
            packs: self.pack_source.clone(),
            recovery,
        }));
    }
}

/// See the module docs. `P` is what a checked pack is to the caller.
pub struct ServerPacks<P> {
    manager: Manager,
    source: Source<P>,
}

impl<P> std::fmt::Debug for ServerPacks<P> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ServerPacks")
            .field("manager", &self.manager)
            .finish_non_exhaustive()
    }
}

impl<P: Clone> ServerPacks<P> {
    /// `open` checks the packs of a reload (`loadRequestedPacks`, see [`Source`]).
    pub fn new(
        open: impl FnMut(&[(u128, PathBuf)]) -> Option<Vec<P>> + Send + 'static,
    ) -> ServerPacks<P> {
        ServerPacks {
            manager: Manager::new(PromptStatus::Pending),
            source: Source {
                open: Box::new(open),
                to_server: false,
                effects: Vec::new(),
                tasks: VecDeque::new(),
                scheduled: false,
                has_updates: false,
                pending_reload: None,
                pack_source: Vec::new(),
                running: None,
                queued: false,
            },
        }
    }

    /// What to do, oldest first.
    pub fn take_effects(&mut self) -> Vec<Effect<P>> {
        std::mem::take(&mut self.source.effects)
    }

    /// `configureForServerControl`: statuses go to the server from now on, and the
    /// prompt starts as the server list's entry says.
    pub fn configure_for_server_control(&mut self, status: PromptStatus) {
        self.source.to_server = true;
        match status {
            PromptStatus::Allowed => self.manager.allow_server_packs(&mut self.source),
            PromptStatus::Declined => self.manager.reject_server_packs(&mut self.source),
            PromptStatus::Pending => self.manager.reset_prompt_status(),
        }
    }

    /// `pushPack`, with the hash parsed (`tryParseSha1Hash`).
    pub fn push(&mut self, id: u128, url: String, hash: Option<[u8; 20]>) {
        self.manager.push_pack(&mut self.source, id, url, hash);
    }

    pub fn pop(&mut self, id: u128) {
        self.manager.pop_pack(&mut self.source, id);
    }

    pub fn pop_all(&mut self) {
        self.manager.pop_all(&mut self.source);
    }

    pub fn allow(&mut self) {
        self.manager.allow_server_packs(&mut self.source);
    }

    pub fn reject(&mut self) {
        self.manager.reject_server_packs(&mut self.source);
    }

    pub fn prompt_status(&self) -> PromptStatus {
        self.manager.prompt
    }

    /// A batch's result (on its way to the game thread: it runs with the tasks).
    pub fn downloaded(&mut self, batch: u64, result: Vec<(u128, Result<PathBuf, ()>)>) {
        self.source.tasks.push_back(Task::Downloaded(batch, result));
    }

    /// `runAllTasks`: the tasks waiting, and those they add, in order.
    pub fn run_tasks(&mut self) {
        while let Some(task) = self.source.tasks.pop_front() {
            match task {
                Task::Updates => {
                    while self.source.has_updates {
                        self.source.has_updates = false;
                        self.manager.tick(&mut self.source);
                    }
                    self.source.scheduled = false;
                }
                Task::Downloaded(batch, result) => {
                    if let Some(data) = self.manager.take_batch(batch) {
                        self.manager.on_download(&mut self.source, &data, &result);
                    }
                }
            }
        }
    }

    /// Whether a reload is under way.
    pub fn reloading(&self) -> bool {
        self.source.running.is_some()
    }

    /// The frame's start (`runTick`'s `pendingReload`): a reload asked for while
    /// another was under way starts once that one is over.
    pub fn frame_start(&mut self) {
        if self.source.queued && self.source.running.is_none() {
            self.source.queued = false;
            self.source.reload_resource_packs(false);
        }
    }

    /// The reload under way ended (the `LoadingOverlay`'s callback): `onReloadSuccess`;
    /// a failed one is recovered from with the packs that were active
    /// (`clearResourcePacksOnError`, `onRecovery`), and a failed recovery gives up
    /// (`onRecoveryFailure`, `abortResourcePackRecovery`).
    pub fn reloaded(&mut self, ok: bool) {
        let Some(recovery) = self.source.running.take() else {
            return;
        };
        if ok {
            if let Some(cb) = self.source.pending_reload.take() {
                self.manager.on_success(&mut self.source, &cb);
            }
        } else if !recovery {
            if let Some(mut cb) = self.source.pending_reload.take() {
                self.manager.on_failure(&mut self.source, &mut cb, false);
                let recovery = self.manager.packs_to_load(&cb);
                let packs = (self.source.open)(&recovery).unwrap_or_else(|| {
                    eprintln!("[server packs] double failure in loading server packs");
                    Vec::new()
                });
                self.source.pack_source = packs;
                self.source.pending_reload = Some(cb);
            }
            self.source.reload_resource_packs(true);
        } else {
            if let Some(mut cb) = self.source.pending_reload.take() {
                self.manager.on_failure(&mut self.source, &mut cb, true);
                self.source.pack_source = Vec::new();
            }
            self.source.effects.push(Effect::Abort);
        }
    }

    /// `cleanupAfterDisconnect` and the `runAllTasks` after it
    /// (`clearDownloadedResourcePacks`): every pack goes, the statuses go only to the
    /// log, the prompt starts again; what that unloads is reloaded.
    pub fn cleanup_after_disconnect(&mut self) {
        self.manager.pop_all(&mut self.source);
        self.source.to_server = false;
        self.manager.reset_prompt_status();
        self.run_tasks();
    }
}

#[cfg(test)]
mod golden;
#[cfg(test)]
mod golden_tests;
#[cfg(test)]
mod tests;

//! The server's resource packs in the menus: the prompt (vanilla's `PackConfirmScreen`,
//! as Ruthenium's own form), the server list entry's setting as the answer leaves it,
//! the download toast, and the reloads the session asks for, made by a [`Reloader`] and
//! handed to the renderer, the sound and the language once built.
//!
//! The prompt's words are the menus' own (vanilla's English, never a server pack's: a
//! pack could otherwise turn Yes into No); the server's text under them is shown as
//! plain text, at most [`MAX_PROMPT_CHARS`] characters in at most
//! [`MAX_PROMPT_LINES`] lines, wrapped and scrolled, with nothing in it that can be
//! clicked.

use std::path::PathBuf;
use std::time::Instant;

use ruthenium_net::server_list::PackStatus;
use ruthenium_protocol::nbt::Tag;
use ruthenium_render::Renderer;
use ruthenium_sim::session::{DownloadProgress, PackPrompt, PackToast, ResourceReload};
use ruthenium_ui::modern::{Action, Rect};
use ruthenium_ui::text::{TextStyle, Weight};

use super::{FormKind, Menu, Screen, button};
use crate::resource_reload::{Reloaded, Reloader, Request};

/// Characters of the server's own prompt shown (the rest is cut): a server's text
/// component can be as long as a packet, and the form lays out every line.
pub const MAX_PROMPT_CHARS: usize = 2000;
/// Lines of it shown.
pub const MAX_PROMPT_LINES: usize = 40;
/// Message lines the prompt shows at once; the wheel scrolls the rest.
const PROMPT_WINDOW: usize = 9;
/// How long the toast stays after it last changed (`SystemToast`'s 5000 ms).
const TOAST_MS: u128 = 5000;

/// The menus' part of the server's packs.
#[derive(Debug, Default)]
pub(super) struct PackState {
    pub reloader: Option<Reloader>,
    /// Reloads asked for, started once the renderer is at hand.
    pub wanted: Vec<Request>,
    /// The first world's (vanilla) assets were asked for.
    pub world_requested: bool,
    /// The session's number (the reloads' answers go to the session that asked).
    pub session_number: u64,
    /// The prompt shows: whether it is over the world, and whether it is required.
    pub prompt: Option<(bool, bool)>,
    pub toast: Option<(PackToast, Instant)>,
    /// The packs the sound and the language were last given (none: vanilla's).
    pub given: Vec<(u128, PathBuf)>,
    /// Development: every server's setting is this (`--server-packs`).
    pub status_override: Option<PackStatus>,
    /// The packs the sound engine's files are of (none: vanilla's).
    pub sounds_of: Vec<(u128, PathBuf)>,
    /// A session whose packs are what the renderer has ended: no world is drawn until
    /// vanilla's assets are back.
    pub stale: bool,
    /// The largest texture the GPU takes, once known.
    pub max_texture: Option<u32>,
    /// Orders the sound engine's file swaps, so a late one can't undo a newer one.
    pub sound_swaps: SoundSwaps,
    /// The session (its number) in which a build of vanilla's own resources last
    /// failed: not tried again until the next session starts (see `reload_failed`).
    pub vanilla_failed: Option<u64>,
    /// When a reload last failed for good: vanilla's `resourcePack.load_fail` toast.
    pub load_failed: Option<Instant>,
}

/// Swaps of the sound engine's files, numbered: the swap back to vanilla's at a
/// session's end is made on a thread, and a reload's own swap (the next server's sounds)
/// may come first; the late one is then dropped instead of putting vanilla's over it.
#[derive(Debug, Clone, Default)]
pub struct SoundSwaps(std::sync::Arc<std::sync::Mutex<u64>>);

impl SoundSwaps {
    fn lock(&self) -> std::sync::MutexGuard<'_, u64> {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// A swap to be made later (on a thread): its number.
    pub fn begin(&self) -> u64 {
        let mut n = self.lock();
        *n += 1;
        *n
    }

    /// Makes swap `n` with `swap`, unless a newer one has begun or been made since.
    pub fn finish(&self, n: u64, swap: impl FnOnce()) -> bool {
        let current = self.lock();
        if *current != n {
            return false;
        }
        swap();
        true
    }

    /// Makes a swap now; any begun before it is dropped.
    pub fn now(&self, swap: impl FnOnce()) {
        let mut n = self.lock();
        *n += 1;
        swap();
    }
}

/// The server's prompt as plain text, cut to size: control characters (but line breaks)
/// as spaces, format characters stripped, consecutive combining marks capped to prevent
/// vertical Zalgo overflow, at most [`MAX_PROMPT_CHARS`] characters and
/// [`MAX_PROMPT_LINES`] lines.
pub fn prompt_lines(text: &str) -> Vec<String> {
    let mut combining_run = 0usize;
    let cleaned: String = text
        .chars()
        .filter(|&c| {
            if is_format_char(c) {
                return false;
            }
            if is_combining_mark(c) {
                combining_run += 1;
                return combining_run <= 2;
            }
            combining_run = 0;
            true
        })
        .take(MAX_PROMPT_CHARS)
        .map(|c| if c != '\n' && c.is_control() { ' ' } else { c })
        .collect();
    cleaned
        .split('\n')
        .map(|l| l.trim_end().to_owned())
        .take(MAX_PROMPT_LINES)
        .collect()
}

/// Non-spacing / enclosing combining marks (common Unicode blocks used in Zalgo text).
fn is_combining_mark(c: char) -> bool {
    matches!(
        c,
        '\u{0300}'..='\u{036F}'
            | '\u{0483}'..='\u{0489}'
            | '\u{1AB0}'..='\u{1AFF}'
            | '\u{1DC0}'..='\u{1DFF}'
            | '\u{20D0}'..='\u{20FF}'
            | '\u{FE20}'..='\u{FE2F}'
    )
}

/// Unicode's format characters (general category Cf): bidirectional overrides and
/// isolates, which can make the server's text read in another order than it is
/// (`Yes` shown where `No` is meant), zero-width ones that hide what is there, the byte
/// order mark, interlinear annotation marks and tags. They are left out of the prompt.
pub fn is_format_char(c: char) -> bool {
    matches!(
        c,
        '\u{AD}'
            | '\u{600}'..='\u{605}'
            | '\u{61C}'
            | '\u{6DD}'
            | '\u{70F}'
            | '\u{890}'..='\u{891}'
            | '\u{8E2}'
            | '\u{180E}'
            | '\u{200B}'..='\u{200F}'
            | '\u{202A}'..='\u{202E}'
            | '\u{2060}'..='\u{2064}'
            | '\u{2066}'..='\u{206F}'
            | '\u{FEFF}'
            | '\u{FFF9}'..='\u{FFFB}'
            | '\u{110BD}'
            | '\u{110CD}'
            | '\u{13430}'..='\u{1343F}'
            | '\u{1BCA0}'..='\u{1BCA3}'
            | '\u{1D173}'..='\u{1D17A}'
            | '\u{E0001}'
            | '\u{E0020}'..='\u{E007F}'
    )
}

/// The toast's two lines (`download.pack.*` in vanilla's English).
pub fn toast_lines(t: &PackToast) -> Option<(String, Option<String>)> {
    match *t {
        PackToast::Progress {
            count,
            total,
            progress,
        } => Some((
            format!("Downloading resource pack {count}/{total}"),
            progress.map(|p| match p {
                DownloadProgress::Percent(p) => format!("Progress: {p}%"),
                DownloadProgress::Bytes(b) => {
                    format!("Progress: {} (total size unknown)", human_bytes(b))
                }
            }),
        )),
        PackToast::Failed { failed, total } => Some((
            format!("{failed} out of {total} pack(s) failed to download"),
            None,
        )),
        PackToast::Hide => None,
    }
}

/// `Unit.humanReadable`.
pub fn human_bytes(bytes: u64) -> String {
    if bytes < 1024 {
        return format!("{bytes} B");
    }
    let exp = ((bytes as f64).ln() / 1024f64.ln()) as i32;
    let pre = "KMGTPE".as_bytes()[(exp - 1).clamp(0, 5) as usize] as char;
    format!("{:.1} {pre}B", bytes as f64 / 1024f64.powi(exp))
}

impl Menu {
    /// Development: joins with this Server Resource Packs setting whatever the server
    /// list says (`--server-packs`).
    pub fn set_server_packs(&mut self, status: PackStatus) {
        self.packs.status_override = Some(status);
    }

    /// The session asks about its packs (`addOrUpdatePackPrompt`): the prompt shows, or
    /// the one showing takes the newest push's words.
    pub(super) fn pack_prompt_arrived(&mut self, p: PackPrompt) {
        let in_world = match self.packs.prompt {
            Some((in_world, _)) => in_world,
            None => {
                matches!(
                    self.screen,
                    Screen::Game | Screen::Chat(_) | Screen::Container | Screen::Paused
                ) || self.game_menu_open()
            }
        };
        if in_world {
            self.game.release(self.session.as_ref());
        }
        let server = p.prompt.as_ref().map(|t| self.prompt_text(t));
        let (title, line1, line2) = if p.required {
            (
                "Required Resource Pack",
                "This server requires the use of a custom resource pack.",
                "Rejecting this custom resource pack will disconnect you from this server.",
            )
        } else {
            (
                "Server Resource Pack",
                "This server recommends the use of a custom resource pack.",
                "Would you like to download and install it automagically?",
            )
        };
        let mut lines = vec![line1.to_owned(), line2.to_owned()];
        if let Some(server) = server {
            lines.push(String::new());
            lines.push("Message from server:".into());
            lines.extend(server);
        }
        let buttons = if p.required {
            vec![
                button("Proceed", Action::Confirm),
                button("Disconnect", Action::Cancel),
            ]
        } else {
            vec![button("Yes", Action::Confirm), button("No", Action::Cancel)]
        };
        if let Screen::Form(FormKind::PackPrompt, form, _) = &mut self.screen {
            form.update_prompt(title.to_owned(), lines, buttons, PROMPT_WINDOW, &self.fonts);
        } else {
            let refs: Vec<&str> = lines.iter().map(String::as_str).collect();
            self.session_form(FormKind::PackPrompt, title, &refs, buttons);
            if let Screen::Form(FormKind::PackPrompt, form, _) = &mut self.screen {
                form.set_message_window(PROMPT_WINDOW, &self.fonts);
            }
        }
        self.packs.prompt = Some((in_world, p.required));
    }

    /// The server's text component as the prompt shows it: in vanilla's language only.
    fn prompt_text(&mut self, t: &Tag) -> Vec<String> {
        let lang = self.joiner.as_mut().map(|j| j.data());
        let empty = std::collections::HashMap::new();
        let lang = lang.as_ref().map_or(&empty, |d| &d.lang);
        prompt_lines(&crate::join::plain_text(t, lang))
    }

    /// The prompt's Yes (Proceed) or No (Disconnect); Escape is No, as vanilla's
    /// `ConfirmScreen` has it.
    pub(super) fn answer_pack_prompt(&mut self, accept: bool) {
        let Some((in_world, _)) = self.packs.prompt.take() else {
            self.screen = Screen::Servers;
            return;
        };
        let Some(s) = &self.session else {
            self.screen = Screen::Servers;
            return;
        };
        s.answer_pack_prompt(accept);
        // (Declining a required pack: the session closes, with vanilla's reason.)
        if self.death.is_some() {
            self.death_form();
        } else if in_world {
            self.enter_game();
        } else {
            self.connecting_form(ruthenium_net::connection::Stage::Joining);
        }
    }

    /// The prompt was put away without an answer (another screen took its place): the
    /// session forgets its packs, as vanilla does.
    pub(super) fn check_pack_prompt(&mut self) {
        if self.packs.prompt.is_some()
            && !matches!(self.screen, Screen::Form(FormKind::PackPrompt, ..))
        {
            self.packs.prompt = None;
            if let Some(s) = &self.session {
                s.dismiss_pack_prompt();
            }
        }
    }

    /// The server list's setting for the server joined, as the answer left it
    /// (`ServerList.saveSingleServer`).
    pub(super) fn pack_status_changed(&mut self, status: PackStatus) {
        let (name, ip) = (self.server_name.clone(), self.server_address.clone());
        if let Some(s) = &mut self.servers {
            if let Some(e) = crate::servers::entry_named(&mut s.list, &name, &ip) {
                e.pack_status = status;
                s.save();
            }
        } else if let Some(p) = &self.servers_path {
            let mut list = ruthenium_net::server_list::ServerList::load(p);
            if let Some(e) = crate::servers::entry_named(&mut list, &name, &ip) {
                e.pack_status = status;
                if let Err(e) = list.save(p) {
                    eprintln!("[ruthenium] couldn't save {}: {e}", p.display());
                }
            }
        }
    }

    /// The server list's setting for the server about to be joined (`Prompt` for one
    /// that isn't in the list).
    pub(super) fn entry_pack_status(&self, name: &str, ip: &str) -> PackStatus {
        if let Some(s) = self.packs.status_override {
            return s;
        }
        let mut list = match (&self.servers, &self.servers_path) {
            (Some(s), _) => s.list.clone(),
            (None, Some(p)) => ruthenium_net::server_list::ServerList::load(p),
            (None, None) => return PackStatus::Prompt,
        };
        crate::servers::entry_named(&mut list, name, ip)
            .map_or(PackStatus::Prompt, |e| e.pack_status)
    }

    /// The session is over (it closed, or another took its place): its reloads are
    /// given up, and its server's packs go (`clearDownloadedResourcePacks`). The
    /// language and the sounds are vanilla's again at once; the world's assets come back
    /// with a reload, and until then no world is drawn, so the next server's never shows
    /// with this one's pictures, font or models.
    pub(super) fn packs_session_over(&mut self) {
        let number = self.packs.session_number;
        self.packs.wanted.retain(|r| r.session != number);
        self.packs.prompt = None;
        self.packs.toast = None;
        let max = self.packs.max_texture.unwrap_or(16384);
        if let Some(reloader) = &mut self.packs.reloader {
            reloader.cancel_session(number, max);
        }
        if let Some(j) = &mut self.joiner {
            j.set_server_lang(None);
            if !self.packs.sounds_of.is_empty() {
                self.packs.sounds_of.clear();
                let (sound, game) = (j.sound().clone(), j.game());
                let swaps = self.packs.sound_swaps.clone();
                let n = swaps.begin();
                let spawned = std::thread::Builder::new()
                    .name("vanilla-sounds".into())
                    .spawn(move || {
                        let Some(game) = game else { return };
                        match crate::game_files::vanilla(&game) {
                            Ok(r) => {
                                let files = ruthenium_audio::SoundFiles::new(r);
                                if !swaps.finish(n, || sound.replace_files(files)) {
                                    eprintln!(
                                        "[sound] vanilla's sounds came after newer ones: dropped"
                                    );
                                }
                            }
                            Err(e) => eprintln!("[sound] no vanilla sounds: {e}"),
                        }
                    });
                if let Err(e) = spawned {
                    eprintln!("[sound] no thread for vanilla's sounds: {e}");
                }
            }
        }
        // Once only: leaving ends a session here and again when its Closed event comes,
        // and a second request would overtake the first and build vanilla's again.
        let vanilla_pending = self
            .packs
            .wanted
            .iter()
            .any(|r| r.session == 0 && r.packs.is_empty())
            || self
                .packs
                .reloader
                .as_ref()
                .is_some_and(Reloader::vanilla_pending);
        if (!self.packs.given.is_empty() || self.packs.stale) && !vanilla_pending {
            self.packs.stale = true;
            self.packs.wanted.push(Request {
                packs: Vec::new(),
                session: 0,
            });
        }
    }

    /// A transfer in play kept the server's packs (`Minecraft.disconnect(screen, true)`):
    /// they go on into the session that now has the number `session_number`, which was
    /// `from`'s. Its reloads, asked for, under way or ended and not yet answered, are the
    /// next session's to hear of; the resources, the language and the sounds in use
    /// stay.
    pub(super) fn packs_carried_over(&mut self, from: u64) {
        let to = self.packs.session_number;
        for r in &mut self.packs.wanted {
            if r.session == from {
                r.session = to;
            }
        }
        if let Some(reloader) = &mut self.packs.reloader {
            reloader.move_session(from, to);
        }
    }

    /// The session asks for its resources again.
    pub(super) fn reload_asked(&mut self, r: ResourceReload) {
        self.packs.wanted.push(Request {
            packs: r.packs,
            session: self.packs.session_number,
        });
    }

    /// Starts the reloads asked for, and hands what a finished one built to the
    /// renderer, the sound and the language; the session that asked hears how it went.
    pub(super) fn sync_reloads(&mut self, renderer: &mut Renderer) {
        let max = renderer.gpu().caps().max_image_dimension_2d;
        self.packs.max_texture = Some(max);
        if self.packs.reloader.is_none() {
            let game = self.joiner.as_ref().and_then(|j| j.game());
            self.packs.reloader = Some(Reloader::new(game, true));
        }
        let wanted = std::mem::take(&mut self.packs.wanted);
        let Some(reloader) = &mut self.packs.reloader else {
            return;
        };
        for r in wanted {
            // The resources in use are already of these packs (vanilla makes them again;
            // here they stay, and the world isn't meshed again for nothing). Never take
            // this fast path while assets are stale or sounds differ.
            if !self.packs.stale
                && !reloader.busy()
                && renderer.has_world()
                && r.packs == self.packs.given
                && self.packs.sounds_of == self.packs.given
            {
                eprintln!("[ruthenium] the resources in use are of these packs already");
                if r.session == self.packs.session_number
                    && let Some(s) = &self.session
                {
                    s.resources_reloaded(true);
                }
                continue;
            }
            reloader.request(r, max);
        }
        // The first world's assets (or recovering from a failed vanilla cleanup reload):
        // vanilla's, unless a reload is already building them, or one failed in this
        // session (tried again only once the next session starts).
        let busy = reloader.busy();
        let want = self.vanilla_wanted(self.session.is_some(), renderer.has_world(), busy);
        let Some(reloader) = &mut self.packs.reloader else {
            return;
        };
        if want {
            self.packs.world_requested = true;
            if !reloader.busy() {
                reloader.request(
                    Request {
                        packs: Vec::new(),
                        session: 0,
                    },
                    max,
                );
            }
        }
        let mut completed = Vec::new();
        while let Some(item) = reloader.poll(max) {
            completed.push(item);
        }
        for (request, result) in completed {
            let active = request.session == 0
                || (self.session.is_some() && request.session == self.packs.session_number);
            if !active {
                eprintln!(
                    "[ruthenium] dropping reload from ended session {}",
                    request.session
                );
                continue;
            }
            let (ok, superseded) = match result {
                Ok(r) => (self.apply_reload(r, &request, renderer), false),
                Err(e) => {
                    eprintln!("[ruthenium] the resource reload failed: {e}");
                    (false, crate::resource_reload::superseded(&e))
                }
            };
            if !ok && request.session == 0 {
                // The first world's assets didn't build: ask again with the next session.
                self.packs.world_requested = false;
                if !superseded {
                    self.vanilla_reload_failed();
                }
            }
            if request.session == self.packs.session_number
                && request.session != 0
                && let Some(s) = &self.session
            {
                s.resources_reloaded(ok);
            }
        }
    }

    /// Whether this frame asks for vanilla's own resources: in a session, the first
    /// world's (none drawn yet, none asked for) or the way back from a session's packs
    /// (`stale`, nothing building), unless they failed to build in this session.
    fn vanilla_wanted(&self, in_session: bool, has_world: bool, busy: bool) -> bool {
        in_session
            && self.packs.vanilla_failed != Some(self.packs.session_number)
            && ((!has_world && !self.packs.world_requested) || (self.packs.stale && !busy))
    }

    /// Puts a reload's resources in use: false if the renderer couldn't take them (then
    /// `stale` is set so a clean vanilla restoration reload is never skipped).
    fn apply_reload(&mut self, r: Reloaded, request: &Request, renderer: &mut Renderer) -> bool {
        if let Some(world) = r.world {
            let had = renderer.has_world();
            let done = if had {
                renderer.replace_world(world, self.mesh_workers)
            } else {
                renderer.load_world(world, self.mesh_workers)
            };
            if let Err(e) = done {
                eprintln!("[ruthenium] the world's new assets didn't load: {e}");
                // `replace_world` may have replaced sky/HUD/block entities before failing,
                // so mark stale to guarantee a clean vanilla reload runs on recovery/leave.
                if had {
                    self.packs.stale = true;
                }
                return false;
            }
        }
        if let Some(j) = &mut self.joiner {
            j.set_server_lang((r.packs > 0).then(|| r.lang.clone()));
            if self.packs.sounds_of != request.packs {
                let sound = j.sound();
                self.packs.sound_swaps.now(|| sound.replace_files(r.sounds));
                self.packs.sounds_of = request.packs.clone();
            }
        }
        self.packs.stale = false;
        self.packs.given = request.packs.clone();
        eprintln!(
            "[ruthenium] resources in use: vanilla's{}",
            if r.packs > 0 {
                format!(" with {} server pack(s)", r.packs)
            } else {
                String::new()
            }
        );
        true
    }

    /// Vanilla's own resources didn't build (a cleanup after a session, or the first
    /// world's). Vanilla retries nothing: a reload of the vanilla pack alone that fails
    /// crashes the game (`rollbackResourcePacks` with one pack selected, Minecraft.java:834),
    /// and a recovery reload that fails ends in `abortResourcePackRecovery`
    /// (Minecraft.java:853): it leaves the world for the title screen with the
    /// `resourcePack.load_fail` toast. Here that is what happens (without the crash): the
    /// session, which has nothing to be drawn with, is left for the title screen, the
    /// toast shows, and vanilla's resources are tried again only once another session
    /// starts, at most once per session, instead of every frame while they are missing.
    fn vanilla_reload_failed(&mut self) {
        self.packs.vanilla_failed = Some(self.packs.session_number);
        self.packs.load_failed = Some(Instant::now());
        if self.session.is_some() {
            eprintln!("[ruthenium] vanilla's resources didn't build: leaving the server");
            self.leave();
            self.screen = Screen::Title;
        }
        // (Leaving asks for vanilla's again: not until the next session.)
        self.packs
            .wanted
            .retain(|r| !(r.session == 0 && r.packs.is_empty()));
    }

    /// What the toast says now: a failed reload's, else the download's.
    pub(super) fn toast_text(&self) -> Option<(String, Option<String>)> {
        if self.packs.load_failed.is_some() {
            return Some(("Resource reload failed".into(), None));
        }
        self.packs.toast.as_ref().and_then(|(t, _)| toast_lines(t))
    }

    /// Drops the toast once it has been up long enough.
    pub(super) fn expire_pack_toast(&mut self) {
        if self
            .packs
            .toast
            .as_ref()
            .is_some_and(|(_, at)| at.elapsed().as_millis() > TOAST_MS)
        {
            self.packs.toast = None;
        }
        if self
            .packs
            .load_failed
            .is_some_and(|at| at.elapsed().as_millis() > TOAST_MS)
        {
            self.packs.load_failed = None;
        }
    }
}

/// The packs' toast (its lines, from [`Menu::toast_text`]), top right, while it is up.
pub(super) fn draw_pack_toast(
    lines: Option<(String, Option<String>)>,
    scale: &ruthenium_ui::modern::UiScale,
    fonts: &ruthenium_ui::text::Fonts,
    atlas: Option<&ruthenium_ui::text::GlyphAtlas>,
    shapes: &mut ruthenium_ui::modern::ShapeList,
    text: &mut ruthenium_ui::text::TextList,
) {
    let (Some((title, message)), Some(atlas)) = (lines, atlas) else {
        return;
    };
    {
        let style = |size: f32, weight| TextStyle {
            size: size * scale.px,
            weight,
            tracking: 0.0,
        };
        let title = fonts.shape(&title, style(15.0, Weight::SEMIBOLD));
        let message = message.map(|m| fonts.shape(&m, style(14.0, Weight::REGULAR)));
        let width = title.width.max(message.as_ref().map_or(0.0, |m| m.width)) / scale.px;
        let h = if message.is_some() { 56.0 } else { 36.0 };
        let r = Rect {
            x: scale.width - width - 48.0,
            y: 16.0,
            w: width + 32.0,
            h,
        };
        shapes.rounded(scale, r, 14.0, [0.06, 0.07, 0.12, 0.78]);
        let x = (r.x + 16.0) * scale.px;
        text.line(
            atlas,
            &title,
            x,
            (r.y + 23.0) * scale.px,
            [1.0, 1.0, 1.0, 0.95],
        );
        if let Some(m) = &message {
            text.line(atlas, m, x, (r.y + 44.0) * scale.px, [0.85, 0.87, 1.0, 0.8]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A hostile prompt is cut to size, its control characters blanked.
    #[test]
    fn the_server_prompt_is_bounded() {
        let long = "x".repeat(MAX_PROMPT_CHARS * 3);
        let lines = prompt_lines(&long);
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].chars().count(), MAX_PROMPT_CHARS);
        let many = "a\n".repeat(MAX_PROMPT_LINES * 5);
        assert_eq!(prompt_lines(&many).len(), MAX_PROMPT_LINES);
        assert_eq!(prompt_lines("a\u{7}b\u{1b}[2Jc\r\nd"), ["a b [2Jc", "d"]);
        // Format characters go: a right-to-left override, isolates, zero widths, the BOM.
        assert_eq!(
            prompt_lines("Click \u{202E}oN\u{202C} \u{2066}x\u{2069}\u{200B}y\u{FEFF}\u{E0041}"),
            ["Click oN xy"]
        );
        // They don't count against the cap.
        let hidden = format!("{}a", "\u{200B}".repeat(MAX_PROMPT_CHARS));
        assert_eq!(prompt_lines(&hidden), ["a"]);
    }

    /// A session that ends gives up its reloads, puts the language back to vanilla's at
    /// once, and keeps its packs' world from being drawn until vanilla's assets are back.
    #[test]
    fn the_end_of_a_session_goes_back_to_vanilla_at_once() {
        use crate::account::Accounts;
        use crate::settings::SettingsFile;
        let mut m = Menu::new(None, Accounts::disabled(), SettingsFile::transient());
        m.set_joiner(crate::join::Joiner::new(None, None));
        let mut lang = std::collections::HashMap::new();
        lang.insert("death.attack.generic".to_owned(), "PACK %s".to_owned());
        if let Some(j) = &mut m.joiner {
            j.set_server_lang(Some(std::sync::Arc::new(lang)));
        }
        m.packs.session_number = 4;
        m.packs.given = vec![(1, PathBuf::from("/p.zip"))];
        m.packs.wanted.push(Request {
            packs: vec![(1, PathBuf::from("/p.zip"))],
            session: 4,
        });
        m.packs_session_over();
        // Leaving ends it here and again when its Closed event comes: one request.
        m.packs_session_over();
        let lang = m.joiner.as_mut().unwrap().server_lang();
        assert!(!lang.contains_key("death.attack.generic"));
        assert!(m.packs.stale);
        assert!(m.world_view().is_none());
        assert_eq!(
            m.packs.wanted,
            [Request {
                packs: Vec::new(),
                session: 0,
            }]
        );
        // Without packs in use there is nothing to go back from.
        let mut m = Menu::new(None, Accounts::disabled(), SettingsFile::transient());
        m.packs_session_over();
        assert!(!m.packs.stale && m.packs.wanted.is_empty());
    }

    /// A transfer in play keeps the packs in use: the language stays the packs', no
    /// vanilla reload is asked for, and what the session left asked for is the next
    /// one's. One in configuration goes back to vanilla's as any session's end does; so
    /// does a kept transfer whose session can't start.
    #[cfg(feature = "dev-offline")]
    #[test]
    fn a_transfer_in_play_keeps_the_packs_in_use() {
        use crate::account::Accounts;
        use crate::settings::SettingsFile;
        use ruthenium_sim::session::{KeptPacks, Transfer};
        let mut m = Menu::new(None, Accounts::disabled(), SettingsFile::transient());
        m.set_joiner(crate::join::Joiner::new(None, None));
        let pack_lang = |m: &mut Menu| {
            let mut lang = std::collections::HashMap::new();
            lang.insert("death.attack.generic".to_owned(), "PACK %s".to_owned());
            if let Some(j) = &mut m.joiner {
                j.set_server_lang(Some(std::sync::Arc::new(lang)));
            }
        };
        let has_pack_lang = |m: &mut Menu| {
            let lang = m.joiner.as_mut().unwrap().server_lang();
            lang.contains_key("death.attack.generic")
        };
        let to = |packs| Transfer {
            to: ruthenium_net::ServerAddress::new("127.0.0.1", 1),
            state: Default::default(),
            packs,
        };
        m.start_join("Lobby", "127.0.0.1:1");
        assert!(m.session.is_some());
        pack_lang(&mut m);
        let given = vec![(1, PathBuf::from("/p.zip"))];
        let number = m.packs.session_number;
        m.packs.given = given.clone();
        m.packs.wanted.push(Request {
            packs: given.clone(),
            session: number,
        });
        m.start_transfer(to(KeptPacks::handed_for_tests()));
        assert!(m.session.is_some());
        assert_eq!(
            m.packs.wanted,
            [Request {
                packs: given.clone(),
                session: number + 1,
            }]
        );
        assert!(!m.packs.stale);
        assert!(has_pack_lang(&mut m));
        // In configuration: dropped.
        m.start_transfer(to(KeptPacks::default()));
        assert!(m.packs.stale);
        assert!(!has_pack_lang(&mut m));
        assert_eq!(
            m.packs.wanted,
            [Request {
                packs: Vec::new(),
                session: 0,
            }]
        );
        // Kept, but the next session can't start: dropped too.
        let mut m = Menu::new(None, Accounts::disabled(), SettingsFile::transient());
        m.start_join("Lobby", "127.0.0.1:1");
        m.packs.given = given;
        m.start_transfer(to(KeptPacks::handed_for_tests()));
        assert!(m.session.is_none());
        assert!(m.packs.stale);
    }

    /// Vanilla's own resources that fail to build are not asked for again every frame
    /// (they were, while the assets were stale): once a session, as vanilla retries
    /// nothing; the failure toast shows for its five seconds. Being overtaken by a newer
    /// reload is no failure.
    #[test]
    fn a_failed_vanilla_reload_is_tried_again_only_in_the_next_session() {
        use crate::account::Accounts;
        use crate::settings::SettingsFile;
        let mut m = Menu::new(None, Accounts::disabled(), SettingsFile::transient());
        m.packs.session_number = 3;
        m.packs.stale = true;
        m.packs.world_requested = true;
        assert!(m.vanilla_wanted(true, true, false));
        assert!(!m.vanilla_wanted(true, true, true), "not while one builds");
        assert!(
            !m.vanilla_wanted(false, true, false),
            "not outside a session"
        );
        m.vanilla_reload_failed();
        let asked = (0..1000)
            .filter(|_| m.vanilla_wanted(true, true, false) || m.vanilla_wanted(true, false, false))
            .count();
        assert_eq!(asked, 0, "asked again in the session it failed in");
        assert_eq!(
            m.toast_text(),
            Some(("Resource reload failed".to_owned(), None))
        );
        m.packs.load_failed = Some(Instant::now() - std::time::Duration::from_millis(6000));
        m.expire_pack_toast();
        assert_eq!(m.toast_text(), None);
        // The next session tries once more.
        m.packs.session_number = 4;
        assert!(m.vanilla_wanted(true, true, false));
        m.packs.world_requested = false;
        m.packs.stale = false;
        assert!(
            m.vanilla_wanted(true, false, false),
            "the first world's assets"
        );
        // Overtaken or of a session that is over: no failure.
        assert!(crate::resource_reload::superseded(
            crate::resource_reload::OVERTAKEN
        ));
        assert!(crate::resource_reload::superseded(
            crate::resource_reload::SESSION_OVER
        ));
        assert!(!crate::resource_reload::superseded("it took too long"));
        assert!(!crate::resource_reload::superseded("no game files"));
    }

    /// The swap back to vanilla's sounds, made late, doesn't undo the next server's.
    #[test]
    fn a_late_sound_swap_is_dropped() {
        let swaps = SoundSwaps::default();
        let mut made = Vec::new();
        let vanilla = swaps.begin();
        swaps.now(|| made.push("B"));
        assert!(!swaps.finish(vanilla, || made.push("vanilla")));
        assert_eq!(made, ["B"]);
        // On time, it is made.
        let vanilla = swaps.begin();
        assert!(swaps.finish(vanilla, || made.push("vanilla")));
        assert_eq!(made, ["B", "vanilla"]);
    }

    /// Vanilla's toast texts, and `Unit.humanReadable`.
    #[test]
    fn the_toast_reads_as_vanillas() {
        assert_eq!(human_bytes(1023), "1023 B");
        assert_eq!(human_bytes(1536), "1.5 KB");
        assert_eq!(human_bytes(5 << 20), "5.0 MB");
        let t = PackToast::Progress {
            count: 1,
            total: 2,
            progress: Some(DownloadProgress::Percent(40)),
        };
        assert_eq!(
            toast_lines(&t),
            Some((
                "Downloading resource pack 1/2".into(),
                Some("Progress: 40%".into())
            ))
        );
        assert_eq!(
            toast_lines(&PackToast::Failed {
                failed: 1,
                total: 2
            }),
            Some(("1 out of 2 pack(s) failed to download".into(), None))
        );
        assert_eq!(toast_lines(&PackToast::Hide), None);
    }
}

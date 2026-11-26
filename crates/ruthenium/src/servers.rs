//! The Multiplayer screen's state: the saved servers (`servers.dat` in Ruthenium's data
//! folder), their pings, favicons and the list screen showing them, and the games found
//! on the local network while it shows.
//!
//! The first time the list is opened and Ruthenium has no `servers.dat`, the official
//! launcher's list is copied in (read only; Ruthenium never writes to the launcher's
//! folder).

use std::path::PathBuf;

use ruthenium_assets::image::{Image, alpha, blue, green, red};
use ruthenium_gpu::{Arrival, TextureDesc, TextureId};
use ruthenium_net::lan::{LanDetector, LanServer};
use ruthenium_net::ping::{PingEvent, Pinger};
use ruthenium_net::server_list::{ServerEntry, ServerList};
use ruthenium_net::{PROTOCOL_VERSION, ServerAddress};
use ruthenium_protocol::status::{Players, png_size};
use ruthenium_protocol::text::{Style, StyledText};
use ruthenium_render::Renderer;
use ruthenium_ui::modern::servers::{Entry as Row, LanView};
use ruthenium_ui::modern::{Ping, RowView, ServerListScreen, UiScale};
use ruthenium_ui::text::Fonts;

/// Favicons are 64 x 64 (vanilla's `FaviconTexture` refuses other sizes).
const ICON_SIZE: u32 = 64;
/// The largest icon vanilla keeps (`ServerData.validateIcon`).
const MAX_ICON: u32 = 1024;
/// `ChatFormatting.RED`, and the colour of "Can't connect to server" (`-65536`).
const RED: u32 = 0xFF5555;
const PURE_RED: u32 = 0xFF0000;

#[derive(Debug)]
enum Icon {
    None,
    /// Premultiplied RGBA waiting for room in the upload ring.
    Pending(Vec<u8>),
    Ready(TextureId),
}

#[derive(Debug)]
struct Entry {
    /// The ping's id: a new one per ping, so late answers to an old ping are ignored.
    id: u64,
    ping: Ping,
    motd: StyledText,
    status: StyledText,
    /// The tooltip over the player count: the sample's names (`ServerStatusPinger`).
    players: Vec<StyledText>,
    icon: Icon,
}

/// See the module docs.
pub struct Servers {
    path: Option<PathBuf>,
    pub list: ServerList,
    pinger: Option<Pinger>,
    entries: Vec<Entry>,
    pub screen: ServerListScreen,
    /// Listens for games on the local network (none if it couldn't start).
    detector: Option<LanDetector>,
    lan: Vec<Lan>,
    next_id: u64,
    /// The rows changed: shape them again before drawing.
    dirty: bool,
    /// Textures to destroy (replaced icons).
    retired: Vec<TextureId>,
}

impl std::fmt::Debug for Servers {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Servers").finish_non_exhaustive()
    }
}

fn red_text(s: &str) -> StyledText {
    coloured(s, RED)
}

fn coloured(s: &str, color: u32) -> StyledText {
    StyledText::literal(
        s,
        Style {
            color: Some(color),
            ..Style::default()
        },
    )
}

/// What a row of the list is, with the server it stands for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Selected<'a> {
    Saved(usize, &'a ServerEntry),
    Scanning,
    Lan(&'a LanServer),
}

/// A game on the local network and its message as text.
#[derive(Debug)]
struct Lan {
    server: LanServer,
    motd: StyledText,
}

/// A favicon as premultiplied RGBA, if it is a 64 x 64 PNG.
fn decode_icon(png: &[u8]) -> Option<Vec<u8>> {
    // The server's own bytes: nothing larger than an icon is decoded.
    let img = Image::decode_png_with(
        png,
        ICON_SIZE,
        u64::from(ICON_SIZE) * u64::from(ICON_SIZE),
        &mut |_| true,
    )
    .ok()?;
    if (img.width, img.height) != (ICON_SIZE, ICON_SIZE) {
        return None;
    }
    let mut out = Vec::with_capacity(img.pixels.len() * 4);
    for &p in &img.pixels {
        let a = alpha(p);
        let pm = |c: u32| ((c * a + 127) / 255) as u8;
        out.extend_from_slice(&[pm(red(p)), pm(green(p)), pm(blue(p)), a as u8]);
    }
    Some(out)
}

/// The player count's tooltip: the sample's names as literal text (their `§` codes apply),
/// and how many more are online, if the sample has any names at all.
fn player_lines(p: &Players) -> Vec<StyledText> {
    let mut lines: Vec<StyledText> = p
        .sample
        .iter()
        .map(|s| StyledText::literal(&s.name, Style::default()))
        .collect();
    let shown = p.sample.len() as i32;
    if shown > 0 && shown < p.online {
        lines.push(StyledText::literal(
            &format!("... and {} more ...", p.online - shown),
            Style::default(),
        ));
    }
    lines
}

/// The official launcher's `servers.dat`, if there is one.
fn vanilla_list() -> Option<(PathBuf, ServerList)> {
    use ruthenium_prepare::discover::{Env, Launcher, default_roots};
    default_roots(&Env::from_process())
        .into_iter()
        .filter(|(l, _)| *l == Launcher::Official)
        .map(|(_, root)| root.join("servers.dat"))
        .find(|p| p.is_file())
        .map(|p| {
            let list = ServerList::load(&p);
            (p, list)
        })
}

impl Servers {
    /// Loads the list and pings every server.
    pub fn open(path: Option<PathBuf>, scale: UiScale, fonts: &Fonts) -> Servers {
        let list = match &path {
            Some(p) if p.exists() => ServerList::load(p),
            Some(p) => match vanilla_list() {
                Some((from, list)) if !list.servers.is_empty() => {
                    eprintln!(
                        "[ruthenium] copied {} servers from {} to {}",
                        list.servers.len(),
                        from.display(),
                        p.display()
                    );
                    if let Err(e) = list.save(p) {
                        eprintln!("[ruthenium] couldn't save {}: {e}", p.display());
                    }
                    list
                }
                _ => ServerList::default(),
            },
            None => ServerList::default(),
        };
        let pinger = Pinger::new()
            .inspect_err(|e| eprintln!("[ruthenium] no server pings: {e}"))
            .ok();
        let detector = LanDetector::start()
            .inspect_err(|e| eprintln!("[ruthenium] unable to start LAN server detection: {e}"))
            .ok();
        let mut servers = Servers {
            path,
            list,
            pinger,
            entries: Vec::new(),
            screen: ServerListScreen::new(scale, fonts),
            detector,
            lan: Vec::new(),
            next_id: 0,
            dirty: true,
            retired: Vec::new(),
        };
        servers.rebuild();
        servers.update_rows(fonts, |_| false);
        servers.dirty = true;
        servers
    }

    /// New entries for the list as it is now, each pinged afresh; icons come from the
    /// saved favicons.
    fn rebuild(&mut self) {
        for e in self.entries.drain(..) {
            if let Icon::Ready(t) = e.icon {
                self.retired.push(t);
            }
        }
        if let Some(p) = &self.pinger {
            p.cancel_all();
        }
        let servers = self.list.servers.clone();
        for s in &servers {
            let e = self.entry_for(s);
            self.entries.push(e);
        }
        self.dirty = true;
    }

    fn entry_for(&mut self, s: &ServerEntry) -> Entry {
        let id = self.next_id;
        self.next_id += 1;
        if let Some(p) = &self.pinger {
            p.ping(id, &s.ip);
        }
        Entry {
            id,
            ping: Ping::Pinging,
            motd: StyledText::literal("Pinging...", Style::default()),
            status: StyledText::default(),
            players: Vec::new(),
            icon: s
                .icon
                .as_deref()
                .and_then(decode_icon)
                .map_or(Icon::None, Icon::Pending),
        }
    }

    pub fn save(&self) {
        if let Some(p) = &self.path
            && let Err(e) = self.list.save(p)
        {
            eprintln!("[ruthenium] couldn't save {}: {e}", p.display());
        }
    }

    /// Pings every server again (the Refresh button), reloading the file as vanilla does.
    pub fn refresh(&mut self) {
        if let Some(p) = &self.path {
            self.list = ServerList::load(p);
        }
        let selected = self.screen.selected();
        self.rebuild();
        self.screen.select(selected);
    }

    /// Shapes the rows now (without icons), so the list can be used before a frame.
    pub fn sync_rows(&mut self, fonts: &Fonts) {
        self.update_rows(fonts, |_| false);
        self.dirty = true;
    }

    /// Takes in ping results and games found on the local network.
    pub fn poll(&mut self) {
        if let Some(found) = self.detector.as_ref().and_then(LanDetector::take_changed) {
            self.lan = found
                .into_iter()
                .map(|server| Lan {
                    motd: StyledText::literal(&server.motd, Style::default()),
                    server,
                })
                .collect();
            self.dirty = true;
        }
        let Some(pinger) = &self.pinger else { return };
        let events: Vec<(u64, PingEvent)> = pinger.poll().collect();
        let mut save = false;
        for (id, event) in events {
            let Some(i) = self.entries.iter().position(|e| e.id == id) else {
                continue;
            };
            let e = &mut self.entries[i];
            self.dirty = true;
            match event {
                PingEvent::Status(status) => {
                    e.motd = status.description;
                    let compatible = status
                        .version
                        .as_ref()
                        .is_some_and(|v| v.protocol == PROTOCOL_VERSION);
                    e.status = match (&status.players, &status.version) {
                        (_, Some(v)) if !compatible => red_text(&v.name),
                        (_, None) => red_text("Old"),
                        (Some(p), _) => StyledText::literal(
                            &format!("§7{}§8 / §7{}", p.online, p.max),
                            Style::default(),
                        ),
                        (None, _) => StyledText::literal("§8???", Style::default()),
                    };
                    e.players = status.players.as_ref().map_or_else(Vec::new, player_lines);
                    if !compatible {
                        e.ping = Ping::Incompatible;
                    }
                    // A new favicon is kept with the list, as vanilla does.
                    if let Some(icon) = status.favicon
                        && self.list.servers[i].icon.as_ref() != Some(&icon)
                    {
                        let fits =
                            png_size(&icon).is_some_and(|(w, h)| w <= MAX_ICON && h <= MAX_ICON);
                        let decoded = if fits { decode_icon(&icon) } else { None };
                        self.list.servers[i].icon = decoded.is_some().then_some(icon);
                        if let Icon::Ready(t) = std::mem::replace(&mut e.icon, Icon::None) {
                            self.retired.push(t);
                        }
                        e.icon = decoded.map_or(Icon::None, Icon::Pending);
                        save = true;
                    }
                }
                PingEvent::Pong(ms) => {
                    if e.ping != Ping::Incompatible {
                        e.ping = Ping::Latency(ms);
                    }
                }
                // Whatever the cause, vanilla says this (`ServerStatusPinger.onPingFailed`).
                PingEvent::Failed(err) => {
                    eprintln!("[ruthenium] can't ping {}: {err}", self.list.servers[i].ip);
                    e.ping = Ping::Failed;
                    e.status = StyledText::default();
                    e.players.clear();
                    e.motd = coloured("Can't connect to server", PURE_RED);
                }
                // An older server answered the legacy ping: shown as incompatible.
                PingEvent::Legacy(l) => {
                    e.ping = Ping::Incompatible;
                    e.motd = StyledText::literal(&l.motd, Style::default());
                    e.status = red_text(&l.version);
                }
            }
        }
        if save {
            self.save();
        }
    }

    /// Uploads waiting icons and destroys replaced ones, then reshapes the rows if they
    /// changed.
    pub fn prepare(&mut self, renderer: &mut Renderer, fonts: &Fonts) {
        for t in self.retired.drain(..) {
            renderer.destroy_texture(t);
        }
        for e in &mut self.entries {
            if let Icon::Pending(rgba) = &e.icon {
                match renderer.create_texture(
                    TextureDesc::rgba8(ICON_SIZE, ICON_SIZE),
                    &[rgba],
                    Arrival::NextFrame,
                ) {
                    Ok(t) => {
                        e.icon = Icon::Ready(t);
                        self.dirty = true;
                    }
                    Err(e) if e.try_again() => {}
                    Err(err) => {
                        eprintln!("[ruthenium] server icon upload: {err}");
                        e.icon = Icon::None;
                    }
                }
            }
        }
        if std::mem::take(&mut self.dirty) {
            self.update_rows(fonts, |t| renderer.texture_ready(t));
        }
    }

    /// Shapes the rows again; `ready` says which icon textures may be drawn.
    fn update_rows(&mut self, fonts: &Fonts, ready: impl Fn(TextureId) -> bool) {
        let views: Vec<RowView> = self
            .list
            .servers
            .iter()
            .zip(&self.entries)
            .map(|(s, e)| RowView {
                name: &s.name,
                motd: &e.motd,
                status: &e.status,
                players: &e.players,
                ping: e.ping,
                icon: match e.icon {
                    Icon::Ready(t) if ready(t) => Some(t.index()),
                    _ => None,
                },
            })
            .collect();
        // A texture made this frame is ready next frame: look again then.
        self.dirty = self
            .entries
            .iter()
            .any(|e| matches!(e.icon, Icon::Ready(t) if !ready(t)));
        self.screen.set_rows(fonts, &views);
        let lan: Vec<LanView> = self
            .lan
            .iter()
            .map(|l| LanView {
                motd: &l.motd,
                address: &l.server.address,
            })
            .collect();
        self.screen.set_lan(fonts, &lan);
    }

    /// Development and tests: shows `server` as found on the local network.
    #[cfg(test)]
    pub fn found_on_lan(&mut self, server: LanServer) {
        self.lan.push(Lan {
            motd: StyledText::literal(&server.motd, Style::default()),
            server,
        });
        self.dirty = true;
    }

    /// The selected saved server.
    pub fn selected(&self) -> Option<(usize, &ServerEntry)> {
        match self.selection()? {
            Selected::Saved(i, s) => Some((i, s)),
            _ => None,
        }
    }

    /// What the selected row is.
    pub fn selection(&self) -> Option<Selected<'_>> {
        Some(match self.screen.selected_entry()? {
            Row::Saved(i) => Selected::Saved(i, self.list.servers.get(i)?),
            Row::Scanning => Selected::Scanning,
            Row::Lan(i) => Selected::Lan(&self.lan.get(i)?.server),
        })
    }

    /// Adds a server from Add Server and pings it.
    pub fn add(&mut self, server: ServerEntry) {
        self.list.add_shown(server);
        self.save();
        // Shown or unhidden, it is now the last server.
        let s = self.list.servers.last().cloned().expect("just added");
        let e = self.entry_for(&s);
        self.entries.push(e);
        self.screen.select(None);
        self.dirty = true;
    }

    /// Applies Edit Server to server `i` and pings it again.
    pub fn edit(&mut self, i: usize, edited: ServerEntry) {
        let Some(s) = self.list.servers.get_mut(i) else {
            return;
        };
        s.name = edited.name;
        s.ip = edited.ip;
        s.pack_status = edited.pack_status;
        self.save();
        let s = self.list.servers[i].clone();
        let e = self.entry_for(&s);
        if let Icon::Ready(t) = std::mem::replace(&mut self.entries[i], e).icon {
            self.retired.push(t);
        }
        self.dirty = true;
    }

    pub fn delete(&mut self, i: usize) {
        if i >= self.list.servers.len() {
            return;
        }
        self.list.servers.remove(i);
        if let Icon::Ready(t) = self.entries.remove(i).icon {
            self.retired.push(t);
        }
        self.save();
        self.screen.select(None);
        self.dirty = true;
    }

    /// Direct Connect: remembers an address not in the list as a hidden server (as
    /// vanilla does) and returns the entry to join.
    pub fn direct(&mut self, ip: &str) -> ServerEntry {
        if let Some(s) = self.list.find(ip) {
            return s.clone();
        }
        let s = ServerEntry::new("Minecraft Server", ip);
        self.list.add(s.clone(), true);
        self.save();
        s
    }

    /// Frees the icons (leaving the screen).
    pub fn close(mut self, renderer: &mut Renderer) {
        for t in self.retired.drain(..) {
            renderer.destroy_texture(t);
        }
        for e in self.entries.drain(..) {
            if let Icon::Ready(t) = e.icon {
                renderer.destroy_texture(t);
            }
        }
    }
}

/// Java's `String.hashCode` (over UTF-16 units): what `servers.dat` keeps of an accepted
/// code of conduct (`ServerData.acceptCodeOfConduct`).
pub fn java_hash(s: &str) -> i32 {
    s.encode_utf16()
        .fold(0i32, |h, u| h.wrapping_mul(31).wrapping_add(i32::from(u)))
}

/// The entry with this name and address, shown or else hidden: the one vanilla's
/// `ServerList.saveSingleServer` writes a joined server's changes to.
pub fn entry_named<'a>(
    list: &'a mut ServerList,
    name: &str,
    ip: &str,
) -> Option<&'a mut ServerEntry> {
    let named = |s: &&mut ServerEntry| s.name == name && s.ip == ip;
    match list.servers.iter_mut().find(named) {
        Some(s) => Some(s),
        None => list.hidden.iter_mut().find(named),
    }
}

/// Whether `ip` would parse to a real address (the Done and Join buttons).
pub fn valid_address(ip: &str) -> bool {
    ServerAddress::is_valid(ip)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What Java's `String.hashCode` gives for these strings.
    #[test]
    fn hashes_strings_as_java_does() {
        for (s, java) in [
            ("Be kind. ʙᴇ ᴋɪɴᴅ. 日本語", -1240064346),
            ("", 0),
            ("a", 97),
            ("😀 emoji", -1572615351),
            ("Be kinder.", 1270433360),
        ] {
            assert_eq!(java_hash(s), java, "{s:?}");
        }
    }

    #[test]
    fn the_player_count_tooltip_names_the_sample() {
        let sample = |names: &[&str]| {
            names
                .iter()
                .map(|n| ruthenium_protocol::status::SamplePlayer {
                    name: (*n).to_owned(),
                    id: "00000000-0000-0000-0000-000000000001".into(),
                })
                .collect()
        };
        let p = Players {
            max: 100,
            online: 5,
            sample: sample(&["§aᴀʟᴇx", "Steve"]),
        };
        let lines: Vec<String> = player_lines(&p).iter().map(StyledText::plain).collect();
        assert_eq!(lines, ["ᴀʟᴇx", "Steve", "... and 3 more ..."]);
        assert_eq!(player_lines(&p)[0].spans[0].style.color, Some(0x55FF55));
        // All of them named, or none: no "more" line.
        let p = Players { online: 2, ..p };
        assert_eq!(player_lines(&p).len(), 2);
        let p = Players {
            online: 9,
            sample: Vec::new(),
            ..p
        };
        assert!(player_lines(&p).is_empty());
    }

    #[test]
    fn joined_servers_are_found_by_name_and_address() {
        let mut list = ServerList::default();
        list.add(ServerEntry::new("A", "a.example"), false);
        list.add(ServerEntry::new("Minecraft Server", "a.example"), true);
        assert_eq!(
            entry_named(&mut list, "Minecraft Server", "a.example").map(|e| e.name.clone()),
            Some("Minecraft Server".into())
        );
        entry_named(&mut list, "A", "a.example")
            .unwrap()
            .accepted_code_of_conduct = 7;
        assert_eq!(list.servers[0].accepted_code_of_conduct, 7);
        assert!(entry_named(&mut list, "B", "a.example").is_none());
    }
}

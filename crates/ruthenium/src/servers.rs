//! The Multiplayer screen's state: the saved servers (`servers.dat` in Ruthenium's data
//! folder), their pings, favicons and the list screen showing them.
//!
//! The first time the list is opened and Ruthenium has no `servers.dat`, the official
//! launcher's list is copied in (read only; Ruthenium never writes to the launcher's
//! folder).

use std::path::PathBuf;

use ruthenium_assets::image::{Image, alpha, blue, green, red};
use ruthenium_gpu::{Arrival, TextureDesc, TextureId};
use ruthenium_net::ping::{PingError, PingEvent, Pinger};
use ruthenium_net::server_list::{ServerEntry, ServerList};
use ruthenium_net::{PROTOCOL_VERSION, ServerAddress};
use ruthenium_protocol::status::png_size;
use ruthenium_protocol::text::{Style, StyledText};
use ruthenium_render::Renderer;
use ruthenium_ui::modern::{Ping, RowView, ServerListScreen, UiScale};
use ruthenium_ui::text::Fonts;

/// Favicons are 64 x 64 (vanilla's `FaviconTexture` refuses other sizes).
const ICON_SIZE: u32 = 64;
/// The largest icon vanilla keeps (`ServerData.validateIcon`).
const MAX_ICON: u32 = 1024;
const RED: u32 = 0xFF5555;

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
    icon: Icon,
}

/// See the module docs.
pub struct Servers {
    path: Option<PathBuf>,
    pub list: ServerList,
    pinger: Option<Pinger>,
    entries: Vec<Entry>,
    pub screen: ServerListScreen,
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
    StyledText::literal(
        s,
        Style {
            color: Some(RED),
            ..Style::default()
        },
    )
}

/// A favicon as premultiplied RGBA, if it is a 64 x 64 PNG.
fn decode_icon(png: &[u8]) -> Option<Vec<u8>> {
    let img = Image::decode_png(png).ok()?;
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
        let mut servers = Servers {
            path,
            list,
            pinger,
            entries: Vec::new(),
            screen: ServerListScreen::new(scale, fonts),
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
            icon: s
                .icon
                .as_deref()
                .and_then(decode_icon)
                .map_or(Icon::None, Icon::Pending),
        }
    }

    fn save(&self) {
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

    /// Takes in ping results.
    pub fn poll(&mut self) {
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
                        (_, None) => red_text("(old version)"),
                        (Some(p), _) => StyledText::literal(
                            &format!("§7{}§8 / §7{}", p.online, p.max),
                            Style::default(),
                        ),
                        (None, _) => StyledText::literal("§8???", Style::default()),
                    };
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
                PingEvent::Failed(err) => {
                    eprintln!("[ruthenium] can't ping {}: {err}", self.list.servers[i].ip);
                    e.ping = Ping::Failed;
                    e.status = StyledText::default();
                    e.motd = red_text(match err {
                        PingError::UnknownHost => "Can't resolve hostname",
                        PingError::Connection(_) => "Can't connect to server",
                    });
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
    }

    /// The selected server.
    pub fn selected(&self) -> Option<(usize, &ServerEntry)> {
        let i = self.screen.selected()?;
        Some((i, self.list.servers.get(i)?))
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

/// Whether `ip` would parse to a real address (the Done and Join buttons).
pub fn valid_address(ip: &str) -> bool {
    ServerAddress::is_valid(ip)
}

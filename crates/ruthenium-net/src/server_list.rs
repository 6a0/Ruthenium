//! `servers.dat`: the saved server list (`multiplayer/ServerList.java`,
//! `ServerData.java`), the same file vanilla writes, so a player's list can be copied
//! either way.
//!
//! An uncompressed NBT file whose root holds `servers`, a list of compounds: `name`,
//! `ip`, optional `icon` (base64 PNG), optional `acceptTextures` (absent: ask),
//! `acceptedCodeOfConduct` (a Java string hash, written when not 0) and `hidden`.
//! Hidden servers are ones joined with Direct Connect: kept (16 at most, newest first) so
//! their settings stick, but not shown. A missing or unreadable file is an empty list.
//!
//! Saving writes a new file beside it, moves the old one to `servers.dat_old` and the
//! new one into place, as vanilla's `Util.safeReplaceFile` does.

use std::io;
use std::path::Path;

use base64::Engine;
use ruthenium_protocol::nbt::{self, Compound, Tag};

/// The most hidden servers kept (`ServerList.MAX_HIDDEN_SERVERS`).
pub const MAX_HIDDEN: usize = 16;
/// The longest server name and address the edit fields take (`EditBox` default, and
/// `ManageServerScreen`'s address field).
pub const MAX_NAME: usize = 32;
pub const MAX_ADDRESS: usize = 128;

/// Whether to use the server's resource pack.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum PackStatus {
    Enabled,
    Disabled,
    /// Ask when the server offers one.
    #[default]
    Prompt,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ServerEntry {
    pub name: String,
    /// The address as typed.
    pub ip: String,
    /// The last favicon seen (PNG), saved with the list.
    pub icon: Option<Vec<u8>>,
    pub pack_status: PackStatus,
    pub accepted_code_of_conduct: i32,
}

impl ServerEntry {
    pub fn new(name: &str, ip: &str) -> ServerEntry {
        ServerEntry {
            name: name.to_owned(),
            ip: ip.to_owned(),
            ..ServerEntry::default()
        }
    }

    fn read(c: &Compound) -> ServerEntry {
        ServerEntry {
            name: c.string("name").unwrap_or("").to_owned(),
            ip: c.string("ip").unwrap_or("").to_owned(),
            icon: c
                .string("icon")
                .and_then(|s| base64::engine::general_purpose::STANDARD.decode(s).ok()),
            pack_status: match c.boolean("acceptTextures") {
                Some(true) => PackStatus::Enabled,
                Some(false) => PackStatus::Disabled,
                None => PackStatus::Prompt,
            },
            accepted_code_of_conduct: c.int("acceptedCodeOfConduct").unwrap_or(0),
        }
    }

    fn write(&self, hidden: bool) -> Compound {
        let mut c = Compound::new();
        c.put("name", Tag::String(self.name.clone()));
        c.put("ip", Tag::String(self.ip.clone()));
        if let Some(icon) = &self.icon {
            let b64 = base64::engine::general_purpose::STANDARD.encode(icon);
            c.put("icon", Tag::String(b64));
        }
        match self.pack_status {
            PackStatus::Enabled => c.put("acceptTextures", Tag::Byte(1)),
            PackStatus::Disabled => c.put("acceptTextures", Tag::Byte(0)),
            PackStatus::Prompt => {}
        }
        if self.accepted_code_of_conduct != 0 {
            c.put(
                "acceptedCodeOfConduct",
                Tag::Int(self.accepted_code_of_conduct),
            );
        }
        c.put("hidden", Tag::Byte(i8::from(hidden)));
        c
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ServerList {
    /// Shown, in the player's order.
    pub servers: Vec<ServerEntry>,
    /// Joined directly, newest first.
    pub hidden: Vec<ServerEntry>,
}

impl ServerList {
    /// Parses a file's bytes.
    pub fn from_bytes(bytes: &[u8]) -> Result<ServerList, nbt::NbtError> {
        let root = nbt::read(bytes)?;
        let mut list = ServerList::default();
        for tag in root.list("servers") {
            let Some(c) = tag.as_compound() else { continue };
            let entry = ServerEntry::read(c);
            if c.boolean("hidden").unwrap_or(false) {
                list.hidden.push(entry);
            } else {
                list.servers.push(entry);
            }
        }
        Ok(list)
    }

    pub fn to_bytes(&self) -> Result<Vec<u8>, nbt::NbtError> {
        let servers = self
            .servers
            .iter()
            .map(|s| Tag::Compound(s.write(false)))
            .chain(self.hidden.iter().map(|s| Tag::Compound(s.write(true))))
            .collect();
        let mut root = Compound::new();
        root.put("servers", Tag::List(servers));
        nbt::write(&root)
    }

    /// Loads `path`; a missing or unreadable file is an empty list (logged), as in vanilla.
    pub fn load(path: &Path) -> ServerList {
        match std::fs::read(path) {
            Ok(bytes) => ServerList::from_bytes(&bytes).unwrap_or_else(|e| {
                eprintln!("[net] couldn't read {}: {e}", path.display());
                ServerList::default()
            }),
            Err(e) if e.kind() == io::ErrorKind::NotFound => ServerList::default(),
            Err(e) => {
                eprintln!("[net] couldn't read {}: {e}", path.display());
                ServerList::default()
            }
        }
    }

    /// Saves to `path`, keeping the previous file as `<name>_old`.
    pub fn save(&self, path: &Path) -> io::Result<()> {
        let bytes = self.to_bytes().map_err(io::Error::other)?;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let new = path.with_file_name(format!("{name}.new"));
        let old = path.with_file_name(format!("{name}_old"));
        {
            use std::io::Write;
            let mut f = std::fs::File::create(&new)?;
            f.write_all(&bytes)?;
            f.sync_all()?;
        }
        if path.exists() {
            let _ = std::fs::remove_file(&old);
            std::fs::rename(path, &old)?;
        }
        if let Err(e) = std::fs::rename(&new, path) {
            // Put the previous file back.
            let _ = std::fs::rename(&old, path);
            return Err(e);
        }
        Ok(())
    }

    /// The shown or hidden server with this address.
    pub fn find(&self, ip: &str) -> Option<&ServerEntry> {
        self.servers.iter().chain(&self.hidden).find(|s| s.ip == ip)
    }

    /// Moves the hidden server with this address to the end of the shown list.
    pub fn unhide(&mut self, ip: &str) -> Option<&mut ServerEntry> {
        let i = self.hidden.iter().position(|s| s.ip == ip)?;
        let s = self.hidden.remove(i);
        self.servers.push(s);
        self.servers.last_mut()
    }

    /// Adds a server: shown at the end, or hidden at the front (dropping the oldest past
    /// [`MAX_HIDDEN`]).
    pub fn add(&mut self, server: ServerEntry, hidden: bool) {
        if hidden {
            self.hidden.insert(0, server);
            self.hidden.truncate(MAX_HIDDEN);
        } else {
            self.servers.push(server);
        }
    }

    /// Adds a server from the Add Server screen: an address already hidden is shown again
    /// with the new name and icon (`JoinMultiplayerScreen.addServerCallback`, which
    /// takes `ServerData.copyNameIconFrom`, ServerData.java:104: address, name and icon).
    /// Everything else stays the hidden entry's, as in vanilla: its pack status (set when
    /// it was joined from Direct Connect, ServerList.java's `saveSingleServer`) and its
    /// accepted code of conduct; the form's own pack status is dropped.
    pub fn add_shown(&mut self, server: ServerEntry) {
        match self.unhide(&server.ip) {
            Some(existing) => {
                existing.name = server.name;
                existing.icon = server.icon;
            }
            None => self.add(server, false),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_with_hidden_servers() {
        let mut list = ServerList::default();
        let mut a = ServerEntry::new("A", "a.example");
        a.icon = Some(vec![1, 2, 3]);
        a.pack_status = PackStatus::Enabled;
        a.accepted_code_of_conduct = -42;
        list.add(a, false);
        list.add(ServerEntry::new("B", "b.example:25570"), false);
        list.add(ServerEntry::new("Minecraft Server", "direct.example"), true);
        let back = ServerList::from_bytes(&list.to_bytes().unwrap()).unwrap();
        assert_eq!(back, list);
        assert_eq!(
            back.find("direct.example").unwrap().name,
            "Minecraft Server"
        );
    }

    #[test]
    fn hidden_servers_are_capped_and_unhidden() {
        let mut list = ServerList::default();
        for i in 0..20 {
            list.add(ServerEntry::new("x", &format!("h{i}")), true);
        }
        assert_eq!(list.hidden.len(), MAX_HIDDEN);
        assert_eq!(list.hidden[0].ip, "h19");
        list.add_shown(ServerEntry::new("Named", "h19"));
        assert_eq!(list.servers.len(), 1);
        assert_eq!(list.servers[0].name, "Named");
        assert_eq!(list.hidden.len(), MAX_HIDDEN - 1);
    }

    #[test]
    fn saves_atomically() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("servers");
        let path = dir.join("servers.dat");
        assert_eq!(ServerList::load(&path), ServerList::default());
        let mut list = ServerList::default();
        list.add(ServerEntry::new("One", "one"), false);
        list.save(&path).unwrap();
        list.add(ServerEntry::new("Two", "two"), false);
        list.save(&path).unwrap();
        assert_eq!(ServerList::load(&path), list);
        let old = ServerList::load(&dir.join("servers.dat_old"));
        assert_eq!(old.servers.len(), 1);
        std::fs::write(&path, b"junk").unwrap();
        assert_eq!(ServerList::load(&path), ServerList::default());
    }

    /// Vanilla's `addServerCallback`: an address already hidden is unhidden with the
    /// form's name and icon (`copyNameIconFrom`), and keeps its own pack status and
    /// accepted code of conduct.
    #[test]
    fn add_shown_keeps_the_hidden_entrys_pack_status() {
        let mut list = ServerList::default();
        let mut hidden = ServerEntry::new("Direct", "play.example.com");
        hidden.pack_status = PackStatus::Enabled;
        hidden.accepted_code_of_conduct = 7;
        hidden.icon = Some(vec![1, 2, 3]);
        list.add(hidden, true);
        let mut explicit = ServerEntry::new("My Server", "play.example.com");
        explicit.pack_status = PackStatus::Disabled;
        list.add_shown(explicit);
        assert_eq!(list.servers.len(), 1);
        let s = &list.servers[0];
        assert_eq!(s.name, "My Server");
        assert_eq!(
            s.icon, None,
            "the form's icon (none) replaces the hidden one's"
        );
        assert_eq!(s.pack_status, PackStatus::Enabled);
        assert_eq!(s.accepted_code_of_conduct, 7);
        assert!(list.hidden.is_empty());
        // A new address takes the form's pack status.
        let mut new = ServerEntry::new("Other", "other.example");
        new.pack_status = PackStatus::Disabled;
        list.add_shown(new);
        assert_eq!(list.servers[1].pack_status, PackStatus::Disabled);
    }
}

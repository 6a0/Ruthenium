//! Finding games opened to the local network (`client/server/LanServerDetection.java`,
//! `LanServerPinger.java`).
//!
//! A game open to LAN sends `[MOTD]<message>[/MOTD][AD]<port>[/AD]` (UTF-8) to the
//! multicast group 224.0.2.60, port 4445, every 1.5 seconds. The detector listens there
//! on a thread while the server list shows. Each new address (the sender's IP, a colon,
//! and what the `AD` part says) is a server, kept with the first message it came with
//! until the list closes, as vanilla does.

use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4, UdpSocket};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use socket2::{Domain, Protocol, Socket, Type};

/// The multicast group and port LAN games announce themselves on.
pub const GROUP: Ipv4Addr = Ipv4Addr::new(224, 0, 2, 60);
pub const PORT: u16 = 4445;
/// How often a game announces itself (`LanServerPinger.PING_INTERVAL`).
pub const INTERVAL: Duration = Duration::from_millis(1500);
/// How long the detector waits for a packet before it looks whether it should stop.
const POLL: Duration = Duration::from_millis(250);

/// A game found on the local network.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LanServer {
    /// Its message, as sent (legacy `§` codes and all).
    pub motd: String,
    /// Where to join it: `<ip>:<port>`.
    pub address: String,
}

/// What a game open to LAN sends.
pub fn ping_string(motd: &str, address: &str) -> String {
    format!("[MOTD]{motd}[/MOTD][AD]{address}[/AD]")
}

/// The message in an announcement: what lies between the first `[MOTD]` and the
/// `[/MOTD]` after it, or "missing no" without them (`LanServerPinger.parseMotd`).
pub fn parse_motd(ping: &str) -> String {
    const OPEN: &str = "[MOTD]";
    let Some(start) = ping.find(OPEN) else {
        return "missing no".into();
    };
    let from = start + OPEN.len();
    match ping[from..].find("[/MOTD]") {
        Some(end) => ping[from..from + end].to_owned(),
        None => "missing no".into(),
    }
}

/// The address part of an announcement (`LanServerPinger.parseAddress`): none without
/// a `[/MOTD]`, with a second one, or without `[AD]...[/AD]` after it.
pub fn parse_address(ping: &str) -> Option<String> {
    const END_MOTD: &str = "[/MOTD]";
    let end_motd = ping.find(END_MOTD)? + END_MOTD.len();
    if ping[end_motd..].contains(END_MOTD) {
        return None;
    }
    let start = end_motd + ping[end_motd..].find("[AD]")? + "[AD]".len();
    let end = ping[start..].find("[/AD]")?;
    Some(ping[start..start + end].to_owned())
}

#[derive(Debug, Default)]
struct Found {
    servers: Vec<LanServer>,
    changed: bool,
}

impl Found {
    /// Takes in an announcement from `from` (`LanServerList.addServer`).
    fn add(&mut self, ping: &str, from: Ipv4Addr) {
        let Some(port) = parse_address(ping) else {
            return;
        };
        let address = format!("{from}:{port}");
        if self.servers.iter().all(|s| s.address != address) {
            self.servers.push(LanServer {
                motd: parse_motd(ping),
                address,
            });
            self.changed = true;
        }
    }
}

/// Listens for LAN games on a thread until dropped.
#[derive(Debug)]
pub struct LanDetector {
    found: Arc<Mutex<Found>>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl LanDetector {
    /// Joins the group and starts listening. Fails when the port can't be bound (another
    /// program holds it without sharing) or the group can't be joined.
    pub fn start() -> std::io::Result<LanDetector> {
        let socket = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))?;
        // Other games on this machine listen on the same port.
        socket.set_reuse_address(true)?;
        socket.bind(&SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, PORT)).into())?;
        socket.join_multicast_v4(&GROUP, &Ipv4Addr::UNSPECIFIED)?;
        socket.set_read_timeout(Some(POLL))?;
        let socket: UdpSocket = socket.into();
        let found = Arc::new(Mutex::new(Found::default()));
        let stop = Arc::new(AtomicBool::new(false));
        let thread = std::thread::Builder::new()
            .name("lan-detector".into())
            .spawn({
                let (found, stop) = (found.clone(), stop.clone());
                move || listen(&socket, &found, &stop)
            })?;
        Ok(LanDetector {
            found,
            stop,
            thread: Some(thread),
        })
    }

    /// Every server found so far, if one was found since the last call
    /// (`takeDirtyServers`).
    pub fn take_changed(&self) -> Option<Vec<LanServer>> {
        let mut f = self.found.lock().ok()?;
        std::mem::take(&mut f.changed).then(|| f.servers.clone())
    }
}

impl Drop for LanDetector {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        // It notices within `POLL`; don't wait for it.
        drop(self.thread.take());
    }
}

fn listen(socket: &UdpSocket, found: &Mutex<Found>, stop: &AtomicBool) {
    let mut buf = [0u8; 1024];
    while !stop.load(Ordering::Relaxed) {
        match socket.recv_from(&mut buf) {
            Ok((n, SocketAddr::V4(from))) => {
                let ping = String::from_utf8_lossy(&buf[..n]);
                if let Ok(mut f) = found.lock() {
                    f.add(&ping, *from.ip());
                }
            }
            Ok(_) => {}
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock
                        | std::io::ErrorKind::TimedOut
                        | std::io::ErrorKind::Interrupted
                ) => {}
            Err(e) => {
                eprintln!("[net] LAN detection stopped: {e}");
                break;
            }
        }
    }
    let _ = socket.leave_multicast_v4(&GROUP, &Ipv4Addr::UNSPECIFIED);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn announcements_parse_as_vanillas_do() {
        let p = ping_string("§aMy World", "25565");
        assert_eq!(p, "[MOTD]§aMy World[/MOTD][AD]25565[/AD]");
        assert_eq!(parse_motd(&p), "§aMy World");
        assert_eq!(parse_address(&p).as_deref(), Some("25565"));
        assert_eq!(parse_motd("no tags"), "missing no");
        assert_eq!(parse_motd("[MOTD]open"), "missing no");
        assert_eq!(parse_address("[MOTD]x[/MOTD]"), None);
        assert_eq!(parse_address("[AD]1[/AD][MOTD]x[/MOTD]"), None);
        assert_eq!(parse_address("[MOTD]x[/MOTD][AD]1[/AD][/MOTD]"), None);
        assert_eq!(parse_address("[MOTD]x[/MOTD][AD]1"), None);
        // The address part may come without a motd part's opening tag.
        assert_eq!(parse_address("x[/MOTD][AD]7[/AD]").as_deref(), Some("7"));
        assert_eq!(parse_motd("[MOTD][/MOTD][AD]7[/AD]"), "");
    }

    #[test]
    fn each_address_is_one_server() {
        let mut f = Found::default();
        let ip = Ipv4Addr::new(192, 168, 1, 5);
        f.add(&ping_string("A", "25565"), ip);
        f.add(&ping_string("A renamed", "25565"), ip);
        f.add(&ping_string("B", "25566"), ip);
        f.add("junk", ip);
        assert_eq!(
            f.servers,
            [
                LanServer {
                    motd: "A".into(),
                    address: "192.168.1.5:25565".into()
                },
                LanServer {
                    motd: "B".into(),
                    address: "192.168.1.5:25566".into()
                },
            ]
        );
    }

    /// Two detectors share the port, and both hear an announcement sent to the group
    /// (where the machine has a route for multicast).
    #[test]
    fn hears_announcements() {
        let (Ok(a), Ok(b)) = (LanDetector::start(), LanDetector::start()) else {
            eprintln!("no multicast here; skipped");
            return;
        };
        let sender = UdpSocket::bind("0.0.0.0:0").unwrap();
        let _ = sender.set_multicast_loop_v4(true);
        let ping = ping_string("Test world", "25631");
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        let mut got = (None, None);
        while (got.0.is_none() || got.1.is_none()) && std::time::Instant::now() < deadline {
            if sender.send_to(ping.as_bytes(), (GROUP, PORT)).is_err() {
                eprintln!("can't send to the group here; skipped");
                return;
            }
            std::thread::sleep(Duration::from_millis(50));
            got.0 = got.0.or_else(|| a.take_changed());
            got.1 = got.1.or_else(|| b.take_changed());
        }
        for servers in [got.0, got.1] {
            let servers = servers.expect("an announcement arrived");
            assert_eq!(servers[0].motd, "Test world");
            assert!(servers[0].address.ends_with(":25631"), "{:?}", servers[0]);
        }
    }
}

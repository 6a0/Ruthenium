//! The server list's status ping (`multiplayer/ServerStatusPinger.java`), run on a
//! background thread so the menus never wait on the network.
//!
//! Per server: resolve the address ([`crate::resolve`]); connect; send the handshake
//! (protocol version, the host and port **as typed**, before any SRV redirect, and the
//! status intent) and a status request; read the status response; send a ping request
//! and time the pong. Each step reports as it happens, so the list can show the status
//! before the latency arrives. A second status response is refused, as vanilla does.
//!
//! When the status ping fails after the address resolved, the legacy ping (`0xFE`, the
//! server list ping of 1.4 to 1.6, `LegacyServerPinger.java`) is tried on the same
//! address, as vanilla does; servers that still answer it report their version, message
//! and player count.

use std::sync::Arc;
use std::sync::mpsc as std_mpsc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use ruthenium_data::v26_3::packets::{handshake, status};
use ruthenium_data::v26_3::version::PROTOCOL_VERSION;
use ruthenium_protocol::status::{MAX_RESPONSE, ServerStatus};
use ruthenium_protocol::wire::{self, Reader, WireError};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::mpsc;
use tokio::task::JoinSet;

use crate::address::ServerAddress;
use crate::resolve::Resolver;

/// Netty's default connect timeout and vanilla's read timeout (`Connection.java`).
const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);
const READ_TIMEOUT: Duration = Duration::from_secs(30);
/// The handshake's intent for a status query (`ClientIntent.STATUS`).
const INTENT_STATUS: i32 = 1;
/// The handshake's host field limit (`ClientIntentionPacket`).
const MAX_HOST: usize = 255;

/// What a ping reports, in order: `Status` then `Pong`, or `Failed` at any point, and
/// after a failure maybe `Legacy`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PingEvent {
    Status(ServerStatus),
    /// Milliseconds from the ping request to the pong.
    Pong(u64),
    Failed(PingError),
    /// The server answered the legacy ping.
    Legacy(LegacyStatus),
}

/// A legacy ping's answer (`LegacyServerPinger.Output`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LegacyStatus {
    pub protocol: i32,
    pub version: String,
    /// The message, with legacy `§` codes.
    pub motd: String,
    pub players: i32,
    pub max_players: i32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PingError {
    /// The host does not resolve.
    UnknownHost,
    /// Connecting, or the exchange, failed.
    Connection(String),
}

impl std::fmt::Display for PingError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PingError::UnknownHost => f.write_str("unknown host"),
            PingError::Connection(e) => f.write_str(e),
        }
    }
}

enum Command {
    Ping { id: u64, address: String },
    CancelAll,
}

/// Pings on a background thread. Dropping it stops every ping in flight.
pub struct Pinger {
    commands: mpsc::UnboundedSender<Command>,
    events: std_mpsc::Receiver<(u64, PingEvent)>,
    thread: Option<JoinHandle<()>>,
}

impl std::fmt::Debug for Pinger {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Pinger").finish_non_exhaustive()
    }
}

impl Pinger {
    pub fn new() -> std::io::Result<Pinger> {
        let (commands, mut rx) = mpsc::unbounded_channel::<Command>();
        let (tx, events) = std_mpsc::channel();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        let thread = std::thread::Builder::new()
            .name("server pinger".into())
            .spawn(move || {
                runtime.block_on(async move {
                    let resolver = Arc::new(Resolver::new());
                    let mut tasks = JoinSet::new();
                    loop {
                        tokio::select! {
                            command = rx.recv() => match command {
                                Some(Command::Ping { id, address }) => {
                                    let (resolver, tx) = (resolver.clone(), tx.clone());
                                    tasks.spawn(async move {
                                        ping(&resolver, &address, |e| {
                                            let _ = tx.send((id, e));
                                        })
                                        .await;
                                    });
                                }
                                Some(Command::CancelAll) => tasks.abort_all(),
                                None => break,
                            },
                            Some(_) = tasks.join_next(), if !tasks.is_empty() => {}
                        }
                    }
                });
                // Blocking lookups still running are left to finish on their own.
                runtime.shutdown_background();
            })?;
        Ok(Pinger {
            commands,
            events,
            thread: Some(thread),
        })
    }

    /// Starts pinging `address` (as typed); its events come back under `id`.
    pub fn ping(&self, id: u64, address: &str) {
        let _ = self.commands.send(Command::Ping {
            id,
            address: address.to_owned(),
        });
    }

    /// Stops every ping in flight; their remaining events never arrive.
    pub fn cancel_all(&self) {
        let _ = self.commands.send(Command::CancelAll);
        while self.events.try_recv().is_ok() {}
    }

    /// The events that arrived since the last call, oldest first.
    pub fn poll(&self) -> impl Iterator<Item = (u64, PingEvent)> + '_ {
        self.events.try_iter()
    }
}

impl Drop for Pinger {
    fn drop(&mut self) {
        let (closed, _) = mpsc::unbounded_channel();
        drop(std::mem::replace(&mut self.commands, closed));
        // The thread ends as soon as it sees the channel close; don't wait for it.
        drop(self.thread.take());
    }
}

async fn ping(resolver: &Resolver, input: &str, mut report: impl FnMut(PingEvent)) {
    let address = ServerAddress::parse(input);
    let Some(socket) = resolver.resolve(&address).await else {
        report(PingEvent::Failed(PingError::UnknownHost));
        return;
    };
    if let Err(e) = exchange(socket, &address, &mut report).await {
        report(PingEvent::Failed(PingError::Connection(e)));
        if let Ok(Some(legacy)) = legacy_ping(socket, &address).await {
            report(PingEvent::Legacy(legacy));
        }
    }
}

/// The legacy ping's request: `0xFE 0x01`, then a plugin message (`0xFA`) on
/// `MC|PingHost` with protocol 127, the host and the port.
fn legacy_request(host: &str, port: u16) -> Vec<u8> {
    fn string(out: &mut Vec<u8>, s: &str) {
        let units: Vec<u16> = s.encode_utf16().collect();
        out.extend_from_slice(&(units.len() as u16).to_be_bytes());
        out.extend(units.iter().flat_map(|u| u.to_be_bytes()));
    }
    let mut payload = vec![127];
    string(&mut payload, host);
    payload.extend_from_slice(&i32::from(port).to_be_bytes());
    let mut out = vec![0xFE, 0x01, 0xFA];
    string(&mut out, "MC|PingHost");
    out.extend_from_slice(&(payload.len() as u16).to_be_bytes());
    out.extend(payload);
    out
}

/// Reads a legacy answer: `0xFF`, then a string (UTF-16 units, counted) of six fields
/// split by NUL, the first `§1`. `None` until `bytes` holds all of it; `Some(None)` for
/// an answer of another kind.
fn legacy_answer(bytes: &[u8]) -> Option<Option<LegacyStatus>> {
    let (&first, rest) = bytes.split_first()?;
    if first != 0xFF {
        return Some(None);
    }
    let count = i16::from_be_bytes([*rest.first()?, *rest.get(1)?]);
    let Ok(count) = usize::try_from(count) else {
        return Some(None);
    };
    let body = rest.get(2..2 + 2 * count)?;
    let units: Vec<u16> = body
        .as_chunks::<2>()
        .0
        .iter()
        .map(|c| u16::from_be_bytes(*c))
        .collect();
    let text = String::from_utf16_lossy(&units);
    let parts: Vec<&str> = text.splitn(6, '\0').collect();
    if parts.len() < 6 || parts[0] != "§1" {
        return Some(None);
    }
    let int = |s: &str, default| s.parse().unwrap_or(default);
    Some(Some(LegacyStatus {
        protocol: int(parts[1], 0),
        version: parts[2].to_owned(),
        motd: parts[3].to_owned(),
        players: int(parts[4], -1),
        max_players: int(parts[5], -1),
    }))
}

/// The legacy ping: what the server answers, if it answers in kind.
async fn legacy_ping(
    socket: std::net::SocketAddr,
    address: &ServerAddress,
) -> Result<Option<LegacyStatus>, String> {
    let mut stream = tokio::time::timeout(CONNECT_TIMEOUT, TcpStream::connect(socket))
        .await
        .map_err(|_| "connection timed out".to_owned())?
        .map_err(|e| e.to_string())?;
    let _ = stream.set_nodelay(true);
    stream
        .write_all(&legacy_request(&address.host_ascii(), address.port()))
        .await
        .map_err(|e| e.to_string())?;
    let mut buf = Vec::new();
    loop {
        if let Some(answer) = legacy_answer(&buf) {
            return Ok(answer);
        }
        let mut chunk = [0u8; 4096];
        let n = tokio::time::timeout(READ_TIMEOUT, stream.read(&mut chunk))
            .await
            .map_err(|_| "timed out".to_owned())?
            .map_err(|e| e.to_string())?;
        if n == 0 {
            return Ok(None);
        }
        buf.extend_from_slice(&chunk[..n]);
    }
}

async fn exchange(
    socket: std::net::SocketAddr,
    address: &ServerAddress,
    report: &mut impl FnMut(PingEvent),
) -> Result<(), String> {
    let mut stream = tokio::time::timeout(CONNECT_TIMEOUT, TcpStream::connect(socket))
        .await
        .map_err(|_| "connection timed out".to_owned())?
        .map_err(|e| e.to_string())?;
    let _ = stream.set_nodelay(true);
    let mut host = address.host_ascii();
    if host.encode_utf16().count() > MAX_HOST {
        host = host.chars().take(MAX_HOST).collect();
    }
    let mut out = handshake_packet(&host, address.port());
    out.extend(wire::frame(status::Serverbound::StatusRequest.id(), &[]));
    stream.write_all(&out).await.map_err(|e| e.to_string())?;

    let mut buf = Vec::with_capacity(4096);
    let mut started: Option<Instant> = None;
    loop {
        let frame = next_frame(&mut stream, &mut buf).await?;
        let mut r = Reader::new(&frame);
        let id = r.varint().map_err(wire_error)?;
        match status::Clientbound::from_id(id as u32) {
            Some(status::Clientbound::StatusResponse) if started.is_none() => {
                let json = r.string(MAX_RESPONSE).map_err(wire_error)?;
                report(PingEvent::Status(ServerStatus::parse(json)?));
                let now = Instant::now();
                started = Some(now);
                let millis = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_or(0, |d| d.as_millis() as i64);
                let request =
                    wire::frame(status::Serverbound::PingRequest.id(), &millis.to_be_bytes());
                stream
                    .write_all(&request)
                    .await
                    .map_err(|e| e.to_string())?;
            }
            Some(status::Clientbound::StatusResponse) => {
                return Err("unrequested status response".into());
            }
            Some(status::Clientbound::PongResponse) => {
                let Some(t) = started else {
                    return Err("pong before the status".into());
                };
                report(PingEvent::Pong(t.elapsed().as_millis() as u64));
                return Ok(());
            }
            None => return Err(format!("unknown status packet {id}")),
        }
    }
}

fn wire_error(e: WireError) -> String {
    format!("bad packet: {e}")
}

fn handshake_packet(host: &str, port: u16) -> Vec<u8> {
    let mut body = Vec::with_capacity(host.len() + 10);
    wire::write_varint(&mut body, PROTOCOL_VERSION);
    wire::write_string(&mut body, host);
    body.extend_from_slice(&port.to_be_bytes());
    wire::write_varint(&mut body, INTENT_STATUS);
    wire::frame(handshake::Serverbound::Intention.id(), &body)
}

/// Reads until `buf` starts with a whole frame, then takes it off.
async fn next_frame(stream: &mut TcpStream, buf: &mut Vec<u8>) -> Result<Vec<u8>, String> {
    loop {
        if let Some((frame, used)) = wire::split_frame(buf).map_err(wire_error)? {
            let frame = frame.to_vec();
            buf.drain(..used);
            return Ok(frame);
        }
        let mut chunk = [0u8; 8192];
        let n = tokio::time::timeout(READ_TIMEOUT, stream.read(&mut chunk))
            .await
            .map_err(|_| "timed out".to_owned())?
            .map_err(|e| e.to_string())?;
        if n == 0 {
            return Err("disconnected".into());
        }
        buf.extend_from_slice(&chunk[..n]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::net::TcpListener;

    #[test]
    fn handshake_layout() {
        let p = handshake_packet("localhost", 25565);
        // length, id 0, protocol 777 (0x89 0x06), "localhost", 0x63DD, intent 1.
        let mut want = vec![0x00, 0x89, 0x06, 9];
        want.extend_from_slice(b"localhost");
        want.extend_from_slice(&[0x63, 0xDD, 0x01]);
        assert_eq!(p[0] as usize, want.len());
        assert_eq!(&p[1..], want.as_slice());
    }

    /// A fake server: checks the handshake, answers status and ping.
    async fn serve(listener: TcpListener, json: &'static str) {
        let (mut s, _) = listener.accept().await.unwrap();
        let mut buf = Vec::new();
        let hs = next_frame(&mut s, &mut buf).await.unwrap();
        let mut r = Reader::new(&hs);
        assert_eq!(r.varint().unwrap(), 0);
        assert_eq!(r.varint().unwrap(), PROTOCOL_VERSION);
        assert_eq!(r.string(255).unwrap(), "127.0.0.1");
        let req = next_frame(&mut s, &mut buf).await.unwrap();
        assert_eq!(req, [0]);
        let mut body = Vec::new();
        wire::write_string(&mut body, json);
        s.write_all(&wire::frame(0, &body)).await.unwrap();
        let ping = next_frame(&mut s, &mut buf).await.unwrap();
        assert_eq!((ping[0], ping.len()), (1, 9));
        s.write_all(&wire::frame(1, &ping[1..])).await.unwrap();
    }

    #[test]
    fn pings_a_local_server() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let listener = rt.block_on(TcpListener::bind("127.0.0.1:0")).unwrap();
        let port = listener.local_addr().unwrap().port();
        rt.spawn(serve(
            listener,
            r#"{"description":"Hi","players":{"max":5,"online":1}}"#,
        ));
        let pinger = Pinger::new().unwrap();
        pinger.ping(7, &format!("127.0.0.1:{port}"));
        let mut got = Vec::new();
        let deadline = Instant::now() + Duration::from_secs(10);
        while got.len() < 2 && Instant::now() < deadline {
            rt.block_on(async { tokio::time::sleep(Duration::from_millis(5)).await });
            got.extend(pinger.poll());
        }
        assert_eq!(got.len(), 2, "{got:?}");
        assert!(matches!(&got[0], (7, PingEvent::Status(s)) if s.description.plain() == "Hi"));
        assert!(matches!(got[1], (7, PingEvent::Pong(_))));
    }

    #[test]
    fn legacy_request_layout() {
        let r = legacy_request("ab", 25565);
        let mut want = vec![0xFE, 0x01, 0xFA, 0, 11];
        for c in "MC|PingHost".encode_utf16() {
            want.extend_from_slice(&c.to_be_bytes());
        }
        // Payload: protocol 127, "ab" (2 units), port as an int.
        want.extend_from_slice(&[
            0,
            1 + 2 + 4 + 4,
            127,
            0,
            2,
            0,
            b'a',
            0,
            b'b',
            0,
            0,
            0x63,
            0xDD,
        ]);
        assert_eq!(r, want);
    }

    fn legacy_bytes(text: &str) -> Vec<u8> {
        let units: Vec<u16> = text.encode_utf16().collect();
        let mut out = vec![0xFF];
        out.extend_from_slice(&(units.len() as u16).to_be_bytes());
        out.extend(units.iter().flat_map(|u| u.to_be_bytes()));
        out
    }

    #[test]
    fn legacy_answers() {
        let b = legacy_bytes("§1\x00127\x001.6.4\x00§aHi \u{1F600}\x0012\x00x");
        assert_eq!(legacy_answer(&b[..b.len() - 1]), None, "not all there yet");
        assert_eq!(
            legacy_answer(&b),
            Some(Some(LegacyStatus {
                protocol: 127,
                version: "1.6.4".into(),
                motd: "§aHi \u{1F600}".into(),
                players: 12,
                max_players: -1,
            }))
        );
        // Pre-1.4 answers, other packets, too few fields: nothing.
        assert_eq!(legacy_answer(&legacy_bytes("A server§0§20")), Some(None));
        assert_eq!(legacy_answer(&[0x00, 1, 2]), Some(None));
        assert_eq!(
            legacy_answer(&legacy_bytes("§2\x0074\x00v\x00m\x001\x002")),
            Some(None)
        );
        assert_eq!(legacy_answer(&legacy_bytes("§1\x00127\x00v")), Some(None));
        // The sixth field keeps any further NULs.
        let b = legacy_bytes("§1\x00x\x00v\x00m\x001\x002\x00extra");
        assert_eq!(legacy_answer(&b).unwrap().unwrap().max_players, -1);
        assert_eq!(
            legacy_answer(&[0xFF, 0xFF, 0xFF]),
            Some(None),
            "negative length"
        );
    }

    /// A pre-1.7 server: it closes on the modern handshake and answers the legacy ping.
    #[test]
    fn falls_back_to_the_legacy_ping() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let listener = rt.block_on(TcpListener::bind("127.0.0.1:0")).unwrap();
        let port = listener.local_addr().unwrap().port();
        rt.spawn(async move {
            // The status ping: refused by closing.
            let (s, _) = listener.accept().await.unwrap();
            drop(s);
            let (mut s, _) = listener.accept().await.unwrap();
            let mut buf = vec![0u8; 3];
            s.read_exact(&mut buf).await.unwrap();
            assert_eq!(buf, [0xFE, 0x01, 0xFA]);
            s.write_all(&legacy_bytes(
                "§1\x0074\x001.6.4\x00Old §cserver\x003\x0020",
            ))
            .await
            .unwrap();
        });
        let pinger = Pinger::new().unwrap();
        pinger.ping(3, &format!("127.0.0.1:{port}"));
        let mut got = Vec::new();
        let deadline = Instant::now() + Duration::from_secs(10);
        while got.len() < 2 && Instant::now() < deadline {
            rt.block_on(async { tokio::time::sleep(Duration::from_millis(5)).await });
            got.extend(pinger.poll());
        }
        assert!(matches!(got[0], (3, PingEvent::Failed(_))), "{got:?}");
        assert_eq!(
            got[1],
            (
                3,
                PingEvent::Legacy(LegacyStatus {
                    protocol: 74,
                    version: "1.6.4".into(),
                    motd: "Old §cserver".into(),
                    players: 3,
                    max_players: 20,
                })
            )
        );
    }

    #[test]
    fn reports_refused_connections() {
        let pinger = Pinger::new().unwrap();
        // Bind and drop a listener to find a port nothing listens on.
        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        pinger.ping(1, &format!("127.0.0.1:{port}"));
        pinger.ping(2, "this-host-does-not-exist.invalid:1");
        let mut got = Vec::new();
        let deadline = Instant::now() + Duration::from_secs(10);
        while got.len() < 2 && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
            got.extend(pinger.poll());
        }
        got.sort_by_key(|(id, _)| *id);
        assert!(
            matches!(&got[0], (1, PingEvent::Failed(PingError::Connection(_)))),
            "{got:?}"
        );
        assert_eq!(got[1], (2, PingEvent::Failed(PingError::UnknownHost)));
    }
}

//! A connection to a server for playing, from the handshake into play, on a thread of its
//! own (research 01 §3, §12.1).
//!
//! The thread does what vanilla does on its Netty thread: framing, compression and
//! encryption ([`crate::transport`]); the whole login (`ClientHandshakePacketListenerImpl`,
//! whose handlers don't move to the main thread), with the session join on a blocking
//! thread; keep-alive replies, at once, in every phase after it (`handleKeepAlive`); and
//! disconnect packets. Every other configuration and play packet is decoded here and
//! handed on, in order, as an [`Event`] for the game thread, which answers through
//! [`Connection::send`].
//!
//! A terminal packet (`finish_configuration`, `start_configuration`) stops decoding,
//! as vanilla's `ProtocolSwapHandler` does, until the game thread has handled it and
//! called [`Connection::switch`] with its acknowledgement: that packet goes out in the
//! old protocol, then both directions use the new one and decoding resumes with the bytes
//! already received.

use std::net::{IpAddr, SocketAddr};
use std::sync::mpsc as std_mpsc;
use std::thread::JoinHandle;
use std::time::Duration;

use ruthenium_data::v26_3::version::PROTOCOL_VERSION;
use ruthenium_protocol::nbt::Tag;
use ruthenium_protocol::packets::login::{self, Intent};
use ruthenium_protocol::packets::{ClientInformation, Common, GameProfile, configuration, play};
use ruthenium_protocol::wire::{Reader, WireError};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::sync::mpsc;
use tokio::time::Instant;

use crate::address::ServerAddress;
use crate::crypt;
use crate::resolve::Resolver;
use crate::transport::Transport;

/// Netty's default connect timeout and vanilla's read timeout (`Connection.java`).
const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);
const READ_TIMEOUT: Duration = Duration::from_secs(30);
/// The handshake's host field limit (`ClientIntentionPacket`).
const MAX_HOST: usize = 255;

/// The phases after the login, whose packets the game thread handles.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Protocol {
    Configuration,
    Play,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    Login,
    After(Protocol),
}

/// Who logs in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Identity {
    pub name: String,
    pub id: u128,
}

/// Joins the server's session for an online-mode login (`MinecraftSessionService.joinServer`):
/// blocks on the network, and says why if it fails.
pub trait SessionJoin: Send + Sync + 'static {
    fn join(&self, server_hash: &str) -> Result<(), String>;
}

pub enum Auth {
    /// No session (development builds only, PLAN D3): servers that authenticate are
    /// refused.
    #[cfg(any(test, feature = "dev-offline"))]
    Offline,
    Online(std::sync::Arc<dyn SessionJoin>),
}

impl std::fmt::Debug for Auth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            #[cfg(any(test, feature = "dev-offline"))]
            Auth::Offline => "Offline",
            Auth::Online(_) => "Online",
        })
    }
}

#[derive(Debug)]
pub struct ConnectOptions {
    /// The server address as typed.
    pub address: String,
    pub identity: Identity,
    pub auth: Auth,
    /// Sent in `minecraft:brand` once the login is done.
    pub brand: String,
    pub information: ClientInformation,
    /// Refuse servers that aren't on this machine or the local network.
    pub local_only: bool,
}

/// Where the login is, for the connecting screen (vanilla's `connect.*` messages).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    Connecting,
    Authorizing,
    Encrypting,
    Negotiating,
    Joining,
}

/// Why the connection closed.
#[derive(Debug, Clone, PartialEq)]
pub enum Reason {
    /// The server's reason during the login, as JSON text.
    Json(String),
    /// The server's reason after the login, as NBT text.
    Text(Tag),
    Local(Local),
}

/// Reasons from this end.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Local {
    UnknownHost,
    /// `local_only`, and the server is elsewhere.
    NotLocal,
    /// Connecting failed (refused, unreachable...).
    Connect(String),
    TimedOut,
    /// The server closed the connection without a reason.
    EndOfStream,
    /// Reading or writing failed after connecting.
    Io(String),
    /// A packet that doesn't parse, or one not allowed where it came.
    Protocol(String),
    /// An offline login and a server that authenticates players.
    OnlineMode,
    /// The session join failed.
    Authentication(String),
    /// [`Connection::close`].
    Closed,
}

impl std::fmt::Display for Local {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Local::UnknownHost => f.write_str("Unknown host"),
            Local::NotLocal => f.write_str(
                "Development builds only join servers on this computer or the local network",
            ),
            Local::Connect(e) | Local::Io(e) => f.write_str(e),
            Local::TimedOut => f.write_str("Timed out"),
            Local::EndOfStream => f.write_str("End of stream"),
            Local::Protocol(e) => write!(f, "Network protocol error: {e}"),
            Local::OnlineMode => f.write_str(
                "This server checks accounts; development builds only join offline-mode servers",
            ),
            Local::Authentication(e) => write!(f, "Failed to log in: {e}"),
            Local::Closed => f.write_str("Disconnected"),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Closed {
    /// Closed before the login finished (vanilla's "Failed to connect to the server"
    /// rather than "Connection Lost").
    pub during_login: bool,
    pub reason: Reason,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    Stage(Stage),
    /// The login is done and the configuration has begun.
    LoggedIn {
        profile: GameProfile,
    },
    Configuration(configuration::Clientbound),
    Play(play::Clientbound),
    /// The last event.
    Closed(Closed),
}

enum Command {
    Send(Vec<u8>),
    Switch { to: Protocol, ack: Vec<u8> },
    Close,
}

/// A connection on its own thread. Dropping it closes the connection.
pub struct Connection {
    commands: mpsc::UnboundedSender<Command>,
    events: std_mpsc::Receiver<Event>,
    thread: Option<JoinHandle<()>>,
}

impl std::fmt::Debug for Connection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Connection").finish_non_exhaustive()
    }
}

impl Connection {
    pub fn open(options: ConnectOptions) -> std::io::Result<Connection> {
        let (commands, rx) = mpsc::unbounded_channel();
        let (tx, events) = std_mpsc::channel();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        let thread = std::thread::Builder::new()
            .name("connection".into())
            .spawn(move || {
                runtime.block_on(async move {
                    let mut rx = rx;
                    let mut during_login = true;
                    let reason = run(options, &mut rx, &tx, &mut during_login).await;
                    let _ = tx.send(Event::Closed(Closed {
                        during_login,
                        reason,
                    }));
                });
                runtime.shutdown_background();
            })?;
        Ok(Connection {
            commands,
            events,
            thread: Some(thread),
        })
    }

    /// Sends a packet (its id and fields) in the current protocol.
    pub fn send(&self, packet: Vec<u8>) {
        let _ = self.commands.send(Command::Send(packet));
    }

    /// After a terminal packet: sends `ack` in the old protocol, then switches both
    /// directions to `to` and goes on decoding.
    pub fn switch(&self, to: Protocol, ack: Vec<u8>) {
        let _ = self.commands.send(Command::Switch { to, ack });
    }

    /// Closes the connection; a [`Local::Closed`] event follows.
    pub fn close(&self) {
        let _ = self.commands.send(Command::Close);
    }

    /// What happened, in order; the last is [`Event::Closed`].
    pub fn events(&self) -> &std_mpsc::Receiver<Event> {
        &self.events
    }
}

impl Drop for Connection {
    fn drop(&mut self) {
        self.close();
        // The thread ends once the socket closes; don't wait for it.
        drop(self.thread.take());
    }
}

/// Whether `ip` is this machine or the local network: loopback, private IPv4, link-local,
/// IPv6 unique-local, or IPv4 mapped into IPv6 that is one of those.
pub fn is_local(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => v4.is_loopback() || v4.is_private() || v4.is_link_local(),
        IpAddr::V6(v6) => {
            if let Some(v4) = v6.to_ipv4_mapped() {
                return is_local(IpAddr::V4(v4));
            }
            let first = v6.segments()[0];
            v6.is_loopback() || (first & 0xFE00) == 0xFC00 || (first & 0xFFC0) == 0xFE80
        }
    }
}

struct Net<'a> {
    read: OwnedReadHalf,
    write: OwnedWriteHalf,
    transport: Transport,
    state: State,
    /// After a terminal packet, until the game thread switches the protocol.
    paused: bool,
    events: &'a std_mpsc::Sender<Event>,
    /// The login has seen the server's hello.
    hello_seen: bool,
}

impl Net<'_> {
    async fn send(&mut self, packet: &[u8]) -> Result<(), Reason> {
        let bytes = self
            .transport
            .encode(packet)
            .map_err(|e| Reason::Local(Local::Protocol(e.to_string())))?;
        self.write
            .write_all(&bytes)
            .await
            .map_err(|e| Reason::Local(Local::Io(e.to_string())))
    }

    fn event(&self, e: Event) {
        let _ = self.events.send(e);
    }

    /// Handles one received packet.
    async fn packet(&mut self, options: &Opened, packet: &[u8]) -> Result<(), Reason> {
        let protocol = |e: WireError| Reason::Local(Local::Protocol(e.to_string()));
        let mut r = Reader::new(packet);
        let id = r.varint().map_err(protocol)? as u32;
        let body = r.rest();
        match self.state {
            State::Login => self.login(options, id, body).await,
            State::After(Protocol::Configuration) => {
                let p = configuration::Clientbound::decode(id, body).map_err(protocol)?;
                match p {
                    configuration::Clientbound::Common(Common::KeepAlive { id }) => {
                        self.send(&configuration::keep_alive(id)).await
                    }
                    configuration::Clientbound::Common(Common::Disconnect { reason }) => {
                        Err(Reason::Text(reason))
                    }
                    p => {
                        self.paused = p.is_terminal();
                        self.event(Event::Configuration(p));
                        Ok(())
                    }
                }
            }
            State::After(Protocol::Play) => {
                let p = play::Clientbound::decode(id, body).map_err(protocol)?;
                match p {
                    play::Clientbound::Common(Common::KeepAlive { id }) => {
                        self.send(&play::keep_alive(id)).await
                    }
                    play::Clientbound::Common(Common::Disconnect { reason }) => {
                        Err(Reason::Text(reason))
                    }
                    p => {
                        self.paused = p.is_terminal();
                        self.event(Event::Play(p));
                        Ok(())
                    }
                }
            }
        }
    }

    async fn login(&mut self, o: &Opened, id: u32, body: &[u8]) -> Result<(), Reason> {
        let protocol = |e: String| Reason::Local(Local::Protocol(e));
        let p = login::Clientbound::decode(id, body).map_err(|e| protocol(e.to_string()))?;
        match p {
            login::Clientbound::Disconnect { reason } => Err(Reason::Json(reason)),
            login::Clientbound::Hello(hello) => {
                if self.hello_seen {
                    return Err(protocol("a second encryption request".into()));
                }
                self.hello_seen = true;
                self.event(Event::Stage(Stage::Authorizing));
                let secret = crypt::secret().map_err(protocol)?;
                let key = crypt::rsa_encrypt(&hello.public_key, &secret).map_err(protocol)?;
                let challenge =
                    crypt::rsa_encrypt(&hello.public_key, &hello.challenge).map_err(protocol)?;
                if hello.should_authenticate {
                    let hash = crypt::server_hash(&hello.server_id, &secret, &hello.public_key);
                    let Some(join) = o.join.clone() else {
                        return Err(Reason::Local(Local::OnlineMode));
                    };
                    tokio::task::spawn_blocking(move || join.join(&hash))
                        .await
                        .map_err(|e| Reason::Local(Local::Authentication(e.to_string())))?
                        .map_err(|e| Reason::Local(Local::Authentication(e)))?;
                }
                self.event(Event::Stage(Stage::Encrypting));
                self.send(&login::key(&key, &challenge)).await?;
                // Everything after the key packet is encrypted, both ways.
                self.transport.set_encryption(&secret);
                Ok(())
            }
            login::Clientbound::Finished { profile, .. } => {
                self.event(Event::Stage(Stage::Joining));
                self.send(&login::login_acknowledged()).await?;
                self.state = State::After(Protocol::Configuration);
                self.send(&configuration::brand(&o.brand)).await?;
                self.send(&configuration::client_information(&o.information))
                    .await?;
                self.event(Event::LoggedIn { profile });
                Ok(())
            }
            login::Clientbound::Compression { threshold } => {
                self.transport.set_compression(threshold);
                Ok(())
            }
            login::Clientbound::CustomQuery { transaction_id, .. } => {
                self.event(Event::Stage(Stage::Negotiating));
                self.send(&login::custom_query_answer(transaction_id)).await
            }
            // No cookies yet: they come with transfers.
            login::Clientbound::CookieRequest { key } => {
                self.send(&login::cookie_response(&key, None)).await
            }
        }
    }
}

/// What the login needs from the options.
struct Opened {
    brand: String,
    information: ClientInformation,
    /// `None`: an offline login.
    join: Option<std::sync::Arc<dyn SessionJoin>>,
}

async fn run(
    options: ConnectOptions,
    commands: &mut mpsc::UnboundedReceiver<Command>,
    events: &std_mpsc::Sender<Event>,
    during_login: &mut bool,
) -> Reason {
    let ConnectOptions {
        address,
        identity,
        auth,
        brand,
        information,
        local_only,
    } = options;
    let opened = Opened {
        brand,
        information,
        join: match auth {
            #[cfg(any(test, feature = "dev-offline"))]
            Auth::Offline => None,
            Auth::Online(j) => Some(j),
        },
    };
    let _ = events.send(Event::Stage(Stage::Connecting));
    let address = ServerAddress::parse(&address);
    let resolver = Resolver::new();
    let connect = async {
        let Some((host, socket)) = resolver.resolve_named(&address).await else {
            return Err(Reason::Local(Local::UnknownHost));
        };
        if local_only && !is_local(socket.ip()) {
            return Err(Reason::Local(Local::NotLocal));
        }
        let stream = tokio::time::timeout(CONNECT_TIMEOUT, TcpStream::connect(socket))
            .await
            .map_err(|_| Reason::Local(Local::TimedOut))?
            .map_err(|e| Reason::Local(Local::Connect(connect_message(&e, &host, socket))))?;
        Ok((host, socket, stream))
    };
    // Closing while connecting gives up at once.
    let (host, socket, stream) = tokio::select! {
        r = connect => match r {
            Ok(c) => c,
            Err(reason) => return reason,
        },
        _ = closed(commands) => return Reason::Local(Local::Closed),
    };
    let _ = stream.set_nodelay(true);
    let (read, write) = stream.into_split();
    let mut net = Net {
        read,
        write,
        transport: Transport::new(),
        state: State::Login,
        paused: false,
        events,
        hello_seen: false,
    };
    let mut host = host;
    if host.encode_utf16().count() > MAX_HOST {
        host = host.chars().take(MAX_HOST).collect();
    }
    // The intention and the hello go out together, without waiting (`ConnectScreen`).
    let opening = [
        login::intention(PROTOCOL_VERSION, &host, socket.port(), Intent::Login),
        login::hello(&identity.name, identity.id),
    ];
    for p in &opening {
        if let Err(r) = net.send(p).await {
            return r;
        }
    }
    let result = drive(&mut net, &opened, commands).await;
    *during_login = net.state == State::Login;
    result
}

/// Waits for a close command (or the handle going away).
async fn closed(commands: &mut mpsc::UnboundedReceiver<Command>) {
    loop {
        match commands.recv().await {
            Some(Command::Close) | None => return,
            Some(_) => {}
        }
    }
}

fn connect_message(e: &std::io::Error, host: &str, socket: SocketAddr) -> String {
    // Vanilla shows Java's message, e.g. "Connection refused: localhost/127.0.0.1:25565".
    match e.kind() {
        std::io::ErrorKind::ConnectionRefused => {
            format!("Connection refused: {host}/{socket}")
        }
        _ => e.to_string(),
    }
}

/// Reads, decodes and handles packets, and carries out commands, until the end.
async fn drive(
    net: &mut Net<'_>,
    opened: &Opened,
    commands: &mut mpsc::UnboundedReceiver<Command>,
) -> Reason {
    let mut buf = vec![0u8; 64 * 1024];
    let mut deadline = Instant::now() + READ_TIMEOUT;
    loop {
        while !net.paused {
            match net.transport.next_packet() {
                Ok(Some(p)) => {
                    if let Err(r) = net.packet(opened, &p).await {
                        return r;
                    }
                }
                Ok(None) => break,
                Err(e) => return Reason::Local(Local::Protocol(e.to_string())),
            }
        }
        let paused = net.paused;
        tokio::select! {
            biased;
            command = commands.recv() => match command {
                Some(Command::Send(p)) => {
                    if let Err(r) = net.send(&p).await {
                        return r;
                    }
                }
                Some(Command::Switch { to, ack }) => {
                    if !net.paused || net.state == State::Login {
                        return Reason::Local(Local::Protocol("protocol switch without a terminal packet".into()));
                    }
                    if let Err(r) = net.send(&ack).await {
                        return r;
                    }
                    net.state = State::After(to);
                    net.paused = false;
                    deadline = Instant::now() + READ_TIMEOUT;
                }
                Some(Command::Close) | None => return Reason::Local(Local::Closed),
            },
            n = net.read.read(&mut buf), if !paused => match n {
                Ok(0) => return Reason::Local(Local::EndOfStream),
                Ok(n) => {
                    net.transport.receive(&buf[..n]);
                    deadline = Instant::now() + READ_TIMEOUT;
                }
                Err(e) => return Reason::Local(Local::Io(e.to_string())),
            },
            _ = tokio::time::sleep_until(deadline), if !paused => {
                return Reason::Local(Local::TimedOut);
            }
        }
    }
}

#[cfg(test)]
mod tests;

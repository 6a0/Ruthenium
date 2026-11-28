//! Transfers between two fake servers on local ports: one in configuration, one in play,
//! with cookies stored on one server and asked for by the next, in its login and after.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{Ipv4Addr, TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use ruthenium_data::v26_3::packets::{
    configuration as cfg_ids, login as login_ids, play as play_ids,
};
use ruthenium_net::ServerAddress;
use ruthenium_net::connection::{
    Auth, Closed, ConnectOptions, Identity, Reason, SessionJoin, TransferState,
};
use ruthenium_net::transport::Transport;
use ruthenium_protocol::nbt::Tag;
use ruthenium_protocol::packets::ClientInformation;
use ruthenium_protocol::wire::{self, Reader};
use ruthenium_sim::session::{ServerPackSetup, Session, SessionEvent, SessionOptions, Transfer};

/// Never asked: the fake servers are in offline mode.
struct NoJoin;

impl SessionJoin for NoJoin {
    fn join(&self, _: &str) -> Result<(), String> {
        Err("an offline-mode server asked for a session join".into())
    }
}

/// The server's end of a connection, blocking.
struct Peer {
    stream: TcpStream,
    transport: Transport,
}

impl Peer {
    fn accept(listener: &TcpListener) -> Peer {
        let (stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(20)))
            .unwrap();
        Peer {
            stream,
            transport: Transport::new(),
        }
    }

    /// The next packet, or `None` once the client has closed the connection.
    fn try_recv(&mut self) -> Option<(u32, Vec<u8>)> {
        loop {
            if let Some(p) = self.transport.next_packet().unwrap() {
                let mut r = Reader::new(&p);
                let id = r.varint().unwrap() as u32;
                return Some((id, r.rest().to_vec()));
            }
            let mut buf = [0u8; 4096];
            let n = self.stream.read(&mut buf).unwrap_or(0);
            if n == 0 {
                return None;
            }
            self.transport.receive(&buf[..n]);
        }
    }

    fn recv(&mut self) -> (u32, Vec<u8>) {
        self.try_recv().expect("the client closed the connection")
    }

    fn send(&mut self, id: u32, body: impl FnOnce(&mut Vec<u8>)) {
        let p = wire::packet(id, body);
        let bytes = self.transport.encode(&p).unwrap();
        self.stream.write_all(&bytes).unwrap();
    }

    /// The handshake (its intent) and the hello; returns the player's name.
    fn opening(&mut self, port: u16) -> (i32, String) {
        let (id, body) = self.recv();
        assert_eq!(id, 0, "intention");
        let mut r = Reader::new(&body);
        r.varint().unwrap();
        assert_eq!(r.string(255).unwrap(), "127.0.0.1");
        assert_eq!(r.u16().unwrap(), port);
        let intent = r.varint().unwrap();
        let (id, body) = self.recv();
        assert_eq!(id, login_ids::Serverbound::Hello.id());
        (intent, Reader::new(&body).string(16).unwrap().to_owned())
    }

    /// Asks for a cookie with `request` and reads the answer, sent with `response`.
    fn cookie(&mut self, request: u32, response: u32, key: &str) -> Option<Vec<u8>> {
        self.send(request, |o| wire::write_string(o, key));
        let (id, body) = self.recv();
        assert_eq!(id, response, "cookie_response");
        let mut r = Reader::new(&body);
        assert_eq!(r.identifier().unwrap(), key);
        let payload = r
            .bool()
            .unwrap()
            .then(|| r.byte_array(5120).unwrap().to_vec());
        r.finish().unwrap();
        payload
    }

    fn login_cookie(&mut self, key: &str) -> Option<Vec<u8>> {
        self.cookie(
            login_ids::Clientbound::CookieRequest.id(),
            login_ids::Serverbound::CookieResponse.id(),
            key,
        )
    }

    fn config_cookie(&mut self, key: &str) -> Option<Vec<u8>> {
        self.cookie(
            cfg_ids::Clientbound::CookieRequest.id(),
            cfg_ids::Serverbound::CookieResponse.id(),
            key,
        )
    }

    /// The login's end, the acknowledgement, the brand and the client's options.
    fn finish_login(&mut self, name: &str) {
        self.send(login_ids::Clientbound::LoginFinished.id(), |o| {
            wire::write_uuid(o, 1);
            wire::write_string(o, name);
            wire::write_varint(o, 0);
            // The session's id.
            wire::write_uuid(o, 42);
        });
        assert_eq!(
            self.recv().0,
            login_ids::Serverbound::LoginAcknowledged.id()
        );
        assert_eq!(
            self.recv().0,
            cfg_ids::Serverbound::CustomPayload.id(),
            "brand"
        );
        assert_eq!(self.recv().0, cfg_ids::Serverbound::ClientInformation.id());
    }

    fn store(&mut self, id: u32, key: &str, payload: &[u8]) {
        self.send(id, |o| {
            wire::write_string(o, key);
            wire::write_byte_array(o, payload);
        });
    }

    fn transfer(&mut self, id: u32, port: u16) {
        self.send(id, |o| {
            wire::write_string(o, "127.0.0.1");
            wire::write_varint(o, i32::from(port));
        });
    }

    /// Offers a resource pack (no prompt text, not required).
    fn push_pack(&mut self, id: u128, url: &str, hash: &str) {
        self.send(cfg_ids::Clientbound::ResourcePackPush.id(), |o| {
            wire::write_uuid(o, id);
            wire::write_string(o, url);
            wire::write_string(o, hash);
            wire::write_bool(o, false);
            wire::write_bool(o, false);
        });
    }

    /// The pack statuses the client sends next, `n` of them, as (pack, action).
    fn pack_statuses(&mut self, n: usize) -> Vec<(u128, i32)> {
        (0..n)
            .map(|_| {
                let (id, body) = self.recv();
                assert_eq!(id, cfg_ids::Serverbound::ResourcePack.id(), "resource_pack");
                let mut r = Reader::new(&body);
                (r.uuid().unwrap(), r.varint().unwrap())
            })
            .collect()
    }

    /// What the client sends until it closes the connection.
    fn rest(&mut self) -> Vec<u32> {
        std::iter::from_fn(|| self.try_recv().map(|(id, _)| id)).collect()
    }
}

fn options(to: ServerAddress, transfer: Option<TransferState>) -> SessionOptions {
    SessionOptions {
        connect: ConnectOptions {
            address: to,
            identity: Identity {
                name: "Dev".into(),
                id: 7,
            },
            auth: Auth::Online(Arc::new(NoJoin)),
            brand: "ruthenium".into(),
            information: ClientInformation::default(),
            local_only: false,
            transfer,
        },
        known_packs: Vec::new(),
        physics: None,
        sound: Default::default(),
        profile_keys: None,
        server_packs: ServerPackSetup::default(),
    }
}

fn cookies(list: &[(&str, &[u8])]) -> HashMap<String, Vec<u8>> {
    list.iter()
        .map(|(k, v)| ((*k).to_owned(), v.to_vec()))
        .collect()
}

/// A pack host on a local port: every request gets `body`; counts the requests.
fn pack_host(body: &'static [u8]) -> (u16, Arc<AtomicUsize>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let gets = Arc::new(AtomicUsize::new(0));
    let counted = gets.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut s) = stream else { continue };
            let mut head = Vec::new();
            let mut buf = [0u8; 1024];
            while !head.windows(4).any(|w| w == b"\r\n\r\n") {
                match s.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => head.extend_from_slice(&buf[..n]),
                }
            }
            counted.fetch_add(1, Ordering::SeqCst);
            let _ = write!(
                s,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            let _ = s.write_all(body);
        }
    });
    (port, gets)
}

/// A server pack goes on through a transfer in play (vanilla's `disconnect(screen,
/// true)`): A pushes it, B pushes it again and gets it from the cache, not the host, as
/// the same pack; it is dropped only when the chain ends.
#[test]
fn server_packs_go_on_through_a_transfer_in_play() {
    const PACK: u128 = 0x0123_4567_89ab_cdef_0123_4567_89ab_cdef;
    // sha1("a pack for the transfer test")
    const HASH: &str = "72f14704cb2ab76aa8ff80fd4cfb9e7ac61d0d1f";
    let (http, gets) = pack_host(b"a pack for the transfer test");
    let url = format!("http://127.0.0.1:{http}/p.zip");
    let a = TcpListener::bind("127.0.0.1:0").unwrap();
    let b = TcpListener::bind("127.0.0.1:0").unwrap();
    let (port_a, port_b) = (
        a.local_addr().unwrap().port(),
        b.local_addr().unwrap().port(),
    );
    let served = url.clone();
    let servers = std::thread::spawn(move || {
        // (ACCEPTED, DOWNLOADED, SUCCESSFULLY_LOADED.)
        let applied = vec![(PACK, 3), (PACK, 4), (PACK, 0)];
        let mut s = Peer::accept(&a);
        let (_, name) = s.opening(port_a);
        s.finish_login(&name);
        s.push_pack(PACK, &served, HASH);
        assert_eq!(s.pack_statuses(3), applied, "A");
        s.send(cfg_ids::Clientbound::FinishConfiguration.id(), |_| {});
        assert_eq!(s.recv().0, cfg_ids::Serverbound::FinishConfiguration.id());
        s.transfer(play_ids::Clientbound::Transfer.id(), port_b);
        s.rest();

        let mut s = Peer::accept(&b);
        let (intent, name) = s.opening(port_b);
        assert_eq!(intent, 3, "TRANSFER");
        s.finish_login(&name);
        s.push_pack(PACK, &served, HASH);
        assert_eq!(s.pack_statuses(3), applied, "B");
        s.send(cfg_ids::Clientbound::Disconnect.id(), |o| {
            ruthenium_protocol::nbt::write_network(o, &Tag::String("Bye".into())).unwrap()
        });
        s.rest();
    });

    let cache = tempfile::tempdir().unwrap();
    let packs = ServerPackSetup {
        status: ruthenium_net::server_list::PackStatus::Enabled,
        cache_dir: Some(cache.path().to_owned()),
        check: Arc::new(|_| true),
        ..ServerPackSetup::default()
    };
    let start = |to, transfer: Option<Transfer>| {
        let mut o = options(to, transfer.as_ref().map(|t| t.state.clone()));
        o.server_packs = ServerPackSetup {
            kept: transfer.map(|t| t.packs).unwrap_or_default(),
            ..packs.clone()
        };
        Session::start(o).unwrap()
    };
    let mut session = start(ServerAddress::new("127.0.0.1", port_a), None);
    // The reloads asked for, by session.
    let mut reloads: Vec<(usize, Vec<(u128, PathBuf)>)> = Vec::new();
    let (mut sessions, mut next, mut kept) = (0, None, Vec::new());
    let deadline = Instant::now() + Duration::from_secs(30);
    'run: loop {
        assert!(Instant::now() < deadline, "the run didn't end: {reloads:?}");
        session.frame();
        let events: Vec<SessionEvent> = session.poll().collect();
        for e in events {
            match e {
                SessionEvent::ReloadResources(r) => {
                    reloads.push((sessions, r.packs));
                    session.resources_reloaded(true);
                }
                SessionEvent::Transfer(t) => {
                    kept.push(t.packs.kept());
                    next = Some(t);
                }
                SessionEvent::Closed(_) => match next.take() {
                    Some(t) => {
                        sessions += 1;
                        session = start(t.to.clone(), Some(t));
                    }
                    None => break 'run,
                },
                _ => {}
            }
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    drop(session);
    servers.join().expect("the servers saw what they expected");
    assert_eq!(kept, [true]);
    assert_eq!(reloads.len(), 3, "{reloads:?}");
    let pack = reloads[0].1.clone();
    assert_eq!(pack.len(), 1);
    assert!(pack[0].1.starts_with(cache.path()), "{pack:?}");
    assert_eq!(
        reloads,
        [(0, pack.clone()), (1, pack), (1, Vec::new())],
        "A's pack, the same again from B, and none once the chain ends"
    );
    assert_eq!(gets.load(Ordering::SeqCst), 1, "B's came from the cache");
}

#[test]
fn transfers_carry_the_cookies_from_server_to_server() {
    let a = TcpListener::bind("127.0.0.1:0").unwrap();
    let b = TcpListener::bind("127.0.0.1:0").unwrap();
    let (port_a, port_b) = (
        a.local_addr().unwrap().port(),
        b.local_addr().unwrap().port(),
    );
    let servers = std::thread::spawn(move || {
        // A, joined: stores a cookie and transfers in configuration.
        let mut s = Peer::accept(&a);
        let (intent, name) = s.opening(port_a);
        assert_eq!(intent, 2, "LOGIN");
        assert_eq!(s.login_cookie("test:a"), None);
        s.finish_login(&name);
        s.store(cfg_ids::Clientbound::StoreCookie.id(), "test:a", &[1]);
        s.transfer(cfg_ids::Clientbound::Transfer.id(), port_b);
        // Behind the transfer: not answered.
        s.send(cfg_ids::Clientbound::Ping.id(), |o| wire::write_i32(o, 3));
        assert_eq!(
            s.rest(),
            Vec::<u32>::new(),
            "A heard more after the transfer"
        );

        // B: a transfer's login, with A's cookie; stores its own, and transfers in play.
        let mut s = Peer::accept(&b);
        let (intent, name) = s.opening(port_b);
        assert_eq!(intent, 3, "TRANSFER");
        assert_eq!(s.login_cookie("test:a"), Some(vec![1]));
        assert_eq!(s.login_cookie("test:b"), None);
        s.finish_login(&name);
        assert_eq!(s.config_cookie("test:a"), Some(vec![1]));
        s.store(cfg_ids::Clientbound::StoreCookie.id(), "test:b", &[2, 2]);
        s.send(cfg_ids::Clientbound::FinishConfiguration.id(), |_| {});
        assert_eq!(s.recv().0, cfg_ids::Serverbound::FinishConfiguration.id());
        assert_eq!(
            s.cookie(
                play_ids::Clientbound::CookieRequest.id(),
                play_ids::Serverbound::CookieResponse.id(),
                "test:b"
            ),
            Some(vec![2, 2])
        );
        s.store(play_ids::Clientbound::StoreCookie.id(), "test:c", &[3]);
        s.transfer(play_ids::Clientbound::Transfer.id(), port_a);
        assert_eq!(
            s.rest(),
            Vec::<u32>::new(),
            "B heard more after the transfer"
        );

        // Back to A, transferred, with every cookie so far.
        let mut s = Peer::accept(&a);
        let (intent, name) = s.opening(port_a);
        assert_eq!(intent, 3, "TRANSFER");
        assert_eq!(s.login_cookie("test:a"), Some(vec![1]));
        assert_eq!(s.login_cookie("test:b"), Some(vec![2, 2]));
        assert_eq!(s.login_cookie("test:c"), Some(vec![3]));
        s.finish_login(&name);
        assert_eq!(s.config_cookie("test:c"), Some(vec![3]));
        s.send(cfg_ids::Clientbound::Disconnect.id(), |o| {
            ruthenium_protocol::nbt::write_network(o, &Tag::String("Bye".into())).unwrap()
        });
        assert_eq!(s.rest(), Vec::<u32>::new());
    });

    // The client: follows each transfer as the menus do.
    let mut session =
        Session::start(options(ServerAddress::new("127.0.0.1", port_a), None)).unwrap();
    let mut transfers: Vec<Transfer> = Vec::new();
    let mut ends: Vec<Closed> = Vec::new();
    let mut next = None;
    let deadline = Instant::now() + Duration::from_secs(30);
    'run: loop {
        assert!(Instant::now() < deadline, "the run didn't end: {ends:?}");
        session.frame();
        let events: Vec<SessionEvent> = session.poll().collect();
        for e in events {
            match e {
                SessionEvent::Transfer(t) => {
                    transfers.push(t.clone());
                    next = Some(t);
                }
                SessionEvent::Closed(c) => {
                    ends.push(c);
                    match next.take() {
                        Some(t) => {
                            let mut o = options(t.to, Some(t.state));
                            o.server_packs.kept = t.packs;
                            session = Session::start(o).unwrap();
                        }
                        None => break 'run,
                    }
                }
                _ => {}
            }
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    drop(session);
    servers.join().expect("the servers saw what they expected");

    assert_eq!(
        transfers
            .iter()
            .map(|t| (t.to.clone(), t.state.clone(), t.packs.kept()))
            .collect::<Vec<_>>(),
        [
            (
                ServerAddress::new("127.0.0.1", port_b),
                TransferState {
                    cookies: cookies(&[("test:a", &[1])]),
                    from: Some(Ipv4Addr::LOCALHOST.into()),
                },
                // In configuration: the server packs are dropped.
                false,
            ),
            (
                ServerAddress::new("127.0.0.1", port_a),
                TransferState {
                    cookies: cookies(&[("test:a", &[1]), ("test:b", &[2, 2]), ("test:c", &[3])]),
                    from: Some(Ipv4Addr::LOCALHOST.into()),
                },
                // In play: kept.
                true,
            ),
        ]
    );
    let reasons: Vec<String> = ends
        .iter()
        .map(|c| match &c.reason {
            Reason::Text(t) => format!("{t:?}"),
            r => format!("{r:?}"),
        })
        .collect();
    assert_eq!(ends.len(), 3, "{reasons:?}");
    assert!(reasons[0].contains("disconnect.transfer"), "{reasons:?}");
    assert!(reasons[1].contains("disconnect.transfer"), "{reasons:?}");
    assert_eq!(ends[2].reason, Reason::Text(Tag::String("Bye".into())));
}

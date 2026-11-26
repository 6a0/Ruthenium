//! A fake server on a local port plays the server's side.

use super::*;
use rsa::pkcs8::{DecodePrivateKey, EncodePublicKey};
use ruthenium_protocol::nbt::Compound;
use ruthenium_protocol::packets::{KnownPack, configuration as cfg};
use ruthenium_protocol::wire;
use tokio::net::TcpListener;

/// A 1024-bit key, as vanilla servers use (PKCS#8 DER).
const PRIVATE_KEY: &[&str] = &[
    "30820276020100300d06092a864886f70d0101010500048202603082025c02010002818100c5cbe01fd3ffba1de62496",
    "612571bdfb3f75f71b6b1908c1d8060e3864d23ca992e7b034d3a03a4c342c8ead54f823e3fdb8b52e5f35d30cfddfc6",
    "0bd116c970ea612f547093f3a12c2003f3cc5accc10fc1630d6cf398ed965d106ecb48a8c4cc476c3e91ae5cba8d6ed6",
    "4df708f8265ce26e887ac7dbb61fbfc4b9c9cf01a302030100010281804abb19a5ccda4cbe692a9faa7a943bdc4680d2",
    "690890bfcf45481c6d5022628b0b17006b251ddf88d519949a8684ee4280a51dc7346fc89a9afcc5fdc6f4de0c6200d0",
    "7bd54601bd9bdf00b9b0f184c0aee1d2dbbb1e0198e352ff803cf4d8d52c91200f15401130d42ba8add2f28116769e07",
    "4a5e5115c01b185dccef9e6e11024100f1069b5e1fad6449acc51230723009a7d295bdb04941e8a337178edf20d7b4ab",
    "d0da47af6653a2314a2bcab60bbd617a12dc29cc951b483f851cfdc5976469eb024100d215ba034343ee0a0856ab1461",
    "91465bc0a2496cd583b6fb184154bd69115f8495ab1b7ac728bf0393eba46d70d1675f86bd499ec18ba80e692b82677d",
    "c8612902407308f3386e280478816421754bda8f913099d54a00fcdafaa33db42bdc71f74215d1486b778c2d1a174b65",
    "5993aad48e3dd8247f071f59512fc8117aaff1f5f7024100831543675dc538f5d1962251293fb822efe352e531a469bb",
    "eaba78e166798c9bf1b73e0231da5eb520cadb61de0f0aed33e94fd25af984f6d8f06901a21e316102405b575f2177c5",
    "734f6c7b7d6d1260056cb3bc6c1a67f56381a71610be19a30a64dec51d91abeeb3c77354f3519a00d7a9b893abab29eb",
    "09487ce947658c89b0a5",
];

fn private_key() -> rsa::PrivateKey {
    let hex: String = PRIVATE_KEY.concat();
    let der: Vec<u8> = (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
        .collect();
    rsa::PrivateKey::from_pkcs8_der(&der).unwrap()
}

/// The server's end of a connection.
struct Server {
    stream: TcpStream,
    transport: Transport,
}

impl Server {
    async fn accept(listener: &TcpListener) -> Server {
        let (stream, _) = listener.accept().await.unwrap();
        Server {
            stream,
            transport: Transport::new(),
        }
    }

    /// The next packet: id and body.
    async fn recv(&mut self) -> (u32, Vec<u8>) {
        self.try_recv()
            .await
            .expect("the client closed the connection")
    }

    /// The next packet, or `None` if the client closes the connection.
    async fn try_recv(&mut self) -> Option<(u32, Vec<u8>)> {
        loop {
            if let Some(p) = self.transport.next_packet().unwrap() {
                let mut r = Reader::new(&p);
                let id = r.varint().unwrap() as u32;
                return Some((id, r.rest().to_vec()));
            }
            let mut buf = [0u8; 4096];
            let n = self.stream.read(&mut buf).await.unwrap_or(0);
            if n == 0 {
                return None;
            }
            self.transport.receive(&buf[..n]);
        }
    }

    async fn send(&mut self, id: u32, body: impl FnOnce(&mut Vec<u8>)) {
        let p = wire::packet(id, body);
        let bytes = self.transport.encode(&p).unwrap();
        self.stream.write_all(&bytes).await.unwrap();
    }

    /// Reads the handshake and the hello, checks them, and returns the player's name.
    async fn opening(&mut self, port: u16) -> String {
        let (id, body) = self.recv().await;
        assert_eq!(id, 0);
        let mut r = Reader::new(&body);
        assert_eq!(r.varint().unwrap(), PROTOCOL_VERSION);
        assert_eq!(r.string(255).unwrap(), "127.0.0.1");
        assert_eq!(r.u16().unwrap(), port);
        assert_eq!(r.varint().unwrap(), 2);
        let (id, body) = self.recv().await;
        assert_eq!(id, 0);
        let mut r = Reader::new(&body);
        let name = r.string(16).unwrap().to_owned();
        assert_eq!(r.uuid().unwrap(), crypt::offline_uuid(&name));
        name
    }

    /// Sends the login's end and checks the client's acknowledgement, brand and options.
    async fn finish_login(&mut self, name: &str) {
        self.send(2, |o| {
            wire::write_uuid(o, crypt::offline_uuid(name));
            wire::write_string(o, name);
            wire::write_varint(o, 0);
            wire::write_uuid(o, 42);
        })
        .await;
        assert_eq!(self.recv().await, (3, vec![]));
        let (id, body) = self.recv().await;
        assert_eq!(id, 2);
        let mut r = Reader::new(&body);
        assert_eq!(r.identifier().unwrap(), "minecraft:brand");
        assert_eq!(
            ruthenium_protocol::packets::read_brand(r.rest()).unwrap(),
            "ruthenium"
        );
        assert_eq!(self.recv().await.0, 0, "client_information");
    }
}

fn options(port: u16, auth: Auth) -> ConnectOptions {
    ConnectOptions {
        address: format!("127.0.0.1:{port}"),
        identity: Identity {
            name: "Dev".into(),
            id: crypt::offline_uuid("Dev"),
        },
        auth,
        brand: "ruthenium".into(),
        information: ClientInformation::default(),
        local_only: true,
    }
}

/// Runs `server` on a fresh local port and connects to it; returns the client's events
/// up to and including `Closed`, answering terminal packets as a game thread would.
fn session<F>(auth: Auth, server: impl FnOnce(TcpListener, u16) -> F) -> Vec<Event>
where
    F: std::future::Future<Output = ()> + Send + 'static,
{
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .unwrap();
    let listener = rt.block_on(TcpListener::bind("127.0.0.1:0")).unwrap();
    let port = listener.local_addr().unwrap().port();
    let done = rt.spawn(server(listener, port));
    let c = Connection::open(options(port, auth)).unwrap();
    let mut got = Vec::new();
    loop {
        let e = c
            .events()
            .recv_timeout(Duration::from_secs(20))
            .expect("an event in time");
        match &e {
            Event::Configuration(cfg::Clientbound::Finish) => {
                c.switch(Protocol::Play, cfg::finish_configuration());
            }
            Event::Configuration(cfg::Clientbound::Common(Common::Ping { id })) => {
                c.send(cfg::pong(*id));
            }
            Event::Play(play::Clientbound::StartConfiguration) => {
                c.switch(Protocol::Configuration, play::configuration_acknowledged());
            }
            _ => {}
        }
        let end = matches!(e, Event::Closed(_));
        got.push(e);
        if end {
            break;
        }
    }
    rt.block_on(done).unwrap();
    got
}

fn closed(events: &[Event]) -> &Closed {
    match events.last() {
        Some(Event::Closed(c)) => c,
        other => panic!("{other:?}"),
    }
}

#[test]
fn offline_login_configuration_and_play() {
    let events = session(Auth::Offline, |listener, port| async move {
        let mut s = Server::accept(&listener).await;
        let name = s.opening(port).await;
        // Compression from here on, both ways.
        s.send(3, |o| wire::write_varint(o, 64)).await;
        s.transport.set_compression(64);
        s.finish_login(&name).await;
        // Configuration: known packs, a big registry (compressed), a keep-alive the
        // connection answers by itself, and a ping for the game thread.
        s.send(15, |o| {
            wire::write_varint(o, 1);
            for f in ["minecraft", "core", "26.3"] {
                wire::write_string(o, f);
            }
        })
        .await;
        s.send(7, |o| {
            wire::write_string(o, "minecraft:damage_type");
            wire::write_varint(o, 50);
            for i in 0..50 {
                wire::write_string(o, &format!("minecraft:type_{i}"));
                wire::write_bool(o, false);
            }
        })
        .await;
        s.send(4, |o| wire::write_i64(o, 77)).await;
        let (id, body) = s.recv().await;
        assert_eq!((id, body), (4, 77i64.to_be_bytes().to_vec()));
        s.send(5, |o| wire::write_i32(o, 9)).await;
        assert_eq!(s.recv().await, (5, 9i32.to_be_bytes().to_vec()));
        // Finish, and a play packet in the same write: the client must not read it as
        // a configuration packet.
        let mut both = s.transport.encode(&wire::packet(3, |_| {})).unwrap();
        both.extend(
            s.transport
                .encode(&wire::packet(45, |o| wire::write_i64(o, 5)))
                .unwrap(),
        );
        s.stream.write_all(&both).await.unwrap();
        assert_eq!(s.recv().await, (3, vec![]), "finish_configuration");
        // The play keep-alive, answered with the play id.
        assert_eq!(s.recv().await, (28, 5i64.to_be_bytes().to_vec()));
        // Back to configuration and out again.
        s.send(120, |_| {}).await;
        assert_eq!(s.recv().await, (16, vec![]), "configuration_acknowledged");
        s.send(3, |_| {}).await;
        assert_eq!(s.recv().await, (3, vec![]));
        let mut reason = Compound::new();
        reason.put("text", Tag::String("Bye".into()));
        s.send(32, |o| {
            ruthenium_protocol::nbt::write_network(o, &Tag::Compound(reason)).unwrap()
        })
        .await;
    });
    assert_eq!(events[0], Event::Stage(Stage::Connecting));
    assert!(events.contains(&Event::Stage(Stage::Joining)));
    assert!(
        events
            .iter()
            .any(|e| matches!(e, Event::LoggedIn { profile, .. } if profile.name == "Dev"))
    );
    assert!(
        events.contains(&Event::Configuration(cfg::Clientbound::SelectKnownPacks(
            vec![KnownPack::vanilla("core", "26.3")]
        )))
    );
    assert!(events.iter().any(|e| matches!(e,
        Event::Configuration(cfg::Clientbound::RegistryData { entries, .. }) if entries.len() == 50)));
    // Keep-alives never reach the game thread.
    assert!(!events.iter().any(|e| matches!(
        e,
        Event::Configuration(cfg::Clientbound::Common(Common::KeepAlive { .. }))
            | Event::Play(play::Clientbound::Common(Common::KeepAlive { .. }))
    )));
    let c = closed(&events);
    assert!(!c.during_login);
    assert!(matches!(&c.reason, Reason::Text(Tag::Compound(t)) if t.string("text") == Some("Bye")));
}

/// An encrypting server: hello, then the key packet, then everything encrypted.
async fn encrypting_server(listener: TcpListener, port: u16, authenticate: bool) -> Option<String> {
    let key = private_key();
    let public = key.to_public_key().to_public_key_der().unwrap().into_vec();
    let mut s = Server::accept(&listener).await;
    let name = s.opening(port).await;
    s.send(1, |o| {
        wire::write_string(o, "");
        wire::write_byte_array(o, &public);
        wire::write_byte_array(o, &[1, 2, 3, 4]);
        wire::write_bool(o, authenticate);
    })
    .await;
    // An offline client gives up instead of answering.
    let (id, body) = s.try_recv().await?;
    assert_eq!(id, 1);
    let mut r = Reader::new(&body);
    let secret = key
        .decrypt(rsa::Pkcs1v15Encrypt, r.byte_array(256).unwrap())
        .unwrap();
    let challenge = key
        .decrypt(rsa::Pkcs1v15Encrypt, r.byte_array(256).unwrap())
        .unwrap();
    assert_eq!(challenge, [1, 2, 3, 4]);
    let secret: [u8; 16] = secret.try_into().unwrap();
    s.transport.set_encryption(&secret);
    s.finish_login(&name).await;
    s.send(2, |o| {
        ruthenium_protocol::nbt::write_network(o, &Tag::String("done".into())).unwrap()
    })
    .await;
    Some(crypt::server_hash("", &secret, &public))
}

#[test]
fn encryption_without_authentication() {
    let events = session(Auth::Offline, |l, port| async move {
        assert!(encrypting_server(l, port, false).await.is_some());
    });
    assert!(events.contains(&Event::Stage(Stage::Encrypting)));
    assert!(matches!(&closed(&events).reason, Reason::Text(Tag::String(s)) if s == "done"));
}

#[test]
fn offline_logins_refuse_online_mode() {
    let events = session(Auth::Offline, |l, port| async move {
        let mut s = Server::accept(&l).await;
        s.opening(port).await;
        s.send(1, |o| {
            wire::write_string(o, "");
            wire::write_byte_array(o, &[1]);
            wire::write_byte_array(o, &[1]);
            wire::write_bool(o, true);
        })
        .await;
    });
    let c = closed(&events);
    assert!(c.during_login);
    // The bad key is found before the question of authenticating comes up.
    assert!(
        matches!(c.reason, Reason::Local(Local::Protocol(_))),
        "{c:?}"
    );

    let events = session(Auth::Offline, |l, port| async move {
        assert!(encrypting_server(l, port, true).await.is_none());
    });
    assert_eq!(closed(&events).reason, Reason::Local(Local::OnlineMode));
}

struct RecordJoin(std::sync::Mutex<Vec<String>>);

impl SessionJoin for RecordJoin {
    fn join(&self, hash: &str) -> Result<(), String> {
        self.0.lock().unwrap().push(hash.to_owned());
        Ok(())
    }
}

#[test]
fn online_logins_join_the_session_with_the_servers_hash() {
    let join = std::sync::Arc::new(RecordJoin(Default::default()));
    let (tx, rx) = std_mpsc::channel();
    let events = session(Auth::Online(join.clone()), move |l, port| async move {
        tx.send(encrypting_server(l, port, true).await).unwrap();
    });
    assert!(
        matches!(&closed(&events).reason, Reason::Text(_)),
        "{events:?}"
    );
    let want = rx.recv().unwrap().unwrap();
    assert_eq!(*join.0.lock().unwrap(), [want]);
}

#[test]
fn chunk_batch_size_follows_vanilla() {
    let t0 = std::time::Instant::now();
    let mut c = ChunkBatchSize::default();
    // 2 ms a chunk at first: 3.5 chunks a tick.
    assert_eq!(c.desired_chunks_per_tick(), 3.5);
    // A 10-chunk batch in 10 ms: 1 ms a chunk, averaged with the start (weight 1).
    c.start(t0);
    c.finish(10, t0 + Duration::from_millis(10));
    assert_eq!(
        c.desired_chunks_per_tick(),
        (7_000_000.0f64 / 1_500_000.0) as f32
    );
    // A very slow batch is clamped to three times the average.
    c.start(t0);
    c.finish(1, t0 + Duration::from_secs(1));
    let avg = (1_500_000.0 * 2.0 + 4_500_000.0) / 3.0;
    assert_eq!(c.desired_chunks_per_tick(), (7_000_000.0 / avg) as f32);
    // Empty batches change nothing.
    c.finish(0, t0);
    assert_eq!(c.desired_chunks_per_tick(), (7_000_000.0 / avg) as f32);
}

#[test]
fn login_disconnects_and_local_only() {
    let events = session(Auth::Offline, |l, port| async move {
        let mut s = Server::accept(&l).await;
        s.opening(port).await;
        s.send(0, |o| wire::write_string(o, r#"{"text":"Whitelist"}"#))
            .await;
    });
    let c = closed(&events);
    assert!(c.during_login);
    assert_eq!(c.reason, Reason::Json(r#"{"text":"Whitelist"}"#.into()));

    let mut o = options(1, Auth::Offline);
    o.address = "8.8.8.8:1".into();
    let c = Connection::open(o).unwrap();
    let last = c
        .events()
        .iter()
        .find(|e| matches!(e, Event::Closed(_)))
        .unwrap();
    assert_eq!(
        last,
        Event::Closed(Closed {
            during_login: true,
            reason: Reason::Local(Local::NotLocal)
        })
    );
    assert!(is_local("192.168.1.20".parse().unwrap()));
    assert!(is_local("::ffff:10.0.0.1".parse().unwrap()));
    assert!(is_local("fd12::1".parse().unwrap()));
    assert!(!is_local("2001:db8::1".parse().unwrap()));
}

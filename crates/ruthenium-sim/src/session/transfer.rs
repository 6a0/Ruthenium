//! Transfers (`ClientCommonPacketListenerImpl.handleTransfer`, `ClientCommonPacketListenerImpl.java:315`).
//!
//! A server's transfer packet ends the session the way vanilla's does: the connection is
//! closed with `disconnect.transfer`, and of the packets that arrived behind it only
//! those storing cookies and further transfers are handled (`shouldHandleMessage` while
//! `isTransferring`, `ClientCommonPacketListenerImpl.java:139`); a further transfer is
//! ignored, as vanilla's ConnectScreen refuses a second ("Attempt to connect while
//! already connecting"). No more ticks run: vanilla's `Minecraft.disconnect` drops the
//! level and the player. Once the connection is closed, the session reports a
//! [`super::SessionEvent::Transfer`] with the cookies, then its
//! [`super::SessionEvent::Closed`]; the
//! client opens the next session there (`ConnectScreen.startConnecting` with a
//! `TransferState`), keeping the server list entry it joined (vanilla hands the same
//! `ServerData` on).
//!
//! The address is the packet's host and port as vanilla's `new ServerAddress(host, port)`
//! takes them ([`ServerAddress::from_parts`]). Where vanilla's constructor throws, it has
//! already disconnected (`ClientCommonPacketListenerImpl.java:322`): its screen says
//! "Transferred to another server" and nothing follows; so here.
//!
//! The address is the server's to choose: like a join, it goes through the resolver
//! (SRV and Mojang's list of blocked servers, `ServerNameResolver.resolveAddress`), and
//! vanilla checks nothing else, so a transfer may name this machine or the local network.

use ruthenium_net::ServerAddress;
use ruthenium_net::connection::{Closed, Reason, TransferState};
use ruthenium_protocol::nbt::{Compound, Tag};
use ruthenium_protocol::packets::Common;

use super::Game;

/// Where a server sent the client, and what goes with it: the next session connects to
/// `to` with `state` in its [`ruthenium_net::connection::ConnectOptions::transfer`].
#[derive(Debug, Clone, PartialEq)]
pub struct Transfer {
    pub to: ServerAddress,
    pub state: TransferState,
}

/// A session leaving for a transfer: where to (`None`: an address vanilla can't make).
#[derive(Debug)]
pub(super) struct Leaving {
    to: Option<ServerAddress>,
}

/// `disconnect.transfer`: why the session ended.
fn transferred() -> Reason {
    let mut c = Compound::new();
    c.put("translate", Tag::String("disconnect.transfer".into()));
    Reason::Text(Tag::Compound(c))
}

/// The packets still handled once a transfer has begun (`shouldHandleMessage`).
pub(super) fn handled_while_transferring(p: &Common) -> bool {
    matches!(p, Common::StoreCookie { .. } | Common::Transfer { .. })
}

impl Game {
    /// The session is leaving for a transfer.
    pub(super) fn transferring(&self) -> bool {
        self.transfer.is_some()
    }

    /// `handleTransfer`.
    pub(super) fn handle_transfer(&mut self, host: &str, port: i32) {
        if self.transfer.is_some() {
            eprintln!("[session] a second transfer, to {host:?}:{port}: ignored");
            return;
        }
        let to = ServerAddress::from_parts(host, port);
        match &to {
            Some(a) => eprintln!("[session] the server transfers to {a}"),
            None => eprintln!(
                "[session] the server transfers to {host:?}:{port}, which isn't an address: leaving"
            ),
        }
        self.transfer = Some(Leaving { to });
        self.close_reason = Some(transferred());
        self.connection.close();
    }

    /// The connection closed: if for a transfer, its reason is the transfer's (whatever
    /// else ended it meanwhile), and the transfer to report before the session's end.
    pub(super) fn transfer_closed(&mut self, c: &mut Closed) -> Option<Transfer> {
        let leaving = self.transfer.take()?;
        c.during_login = false;
        c.reason = transferred();
        Some(Transfer {
            to: leaving.to?,
            state: TransferState {
                cookies: std::mem::take(&mut self.cookies),
                from: self.packs.peer,
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::{DEFAULT_DIRTY_CAPACITY, Scene};
    use crate::session::SessionEvent;
    use ruthenium_net::connection::{Connection, Event, Local, Remote, Sent};
    use ruthenium_protocol::packets::configuration as cfg;
    use ruthenium_protocol::packets::{ClientInformation, play};
    use std::collections::HashMap;
    use std::sync::mpsc::{Receiver, channel};

    fn game() -> (Game, Remote, Receiver<SessionEvent>, Scene) {
        let (connection, remote) = Connection::detached();
        let (publisher, scene) = Scene::new(DEFAULT_DIRTY_CAPACITY);
        let (tx, events) = channel();
        let mut game = Game::new(
            connection,
            Vec::new(),
            None,
            tx,
            publisher,
            ClientInformation::default(),
        );
        game.manual_ms = Some(0);
        game.login_busy_ms = 0;
        (game, remote, events, scene)
    }

    fn store(key: &str, payload: &[u8]) -> Common {
        Common::StoreCookie {
            key: key.into(),
            payload: payload.to_vec(),
        }
    }

    fn transfer(host: &str, port: i32) -> Common {
        Common::Transfer {
            host: host.into(),
            port,
        }
    }

    fn cookies(list: &[(&str, &[u8])]) -> HashMap<String, Vec<u8>> {
        list.iter()
            .map(|(k, v)| ((*k).to_owned(), v.to_vec()))
            .collect()
    }

    /// Runs frames until the session ends, and returns what it reported.
    fn end(
        game: &mut Game,
        remote: &Remote,
        events: &Receiver<SessionEvent>,
        how: Local,
    ) -> Vec<SessionEvent> {
        remote.event(Event::Closed(Closed {
            during_login: false,
            reason: Reason::Local(how),
        }));
        let mut frames = 0;
        while game.frame(Default::default()) {
            frames += 1;
            assert!(frames < 10, "the session doesn't end");
        }
        events.try_iter().collect()
    }

    #[test]
    fn a_transfer_in_configuration_takes_the_cookies_and_ends_the_session() {
        let (mut game, mut remote, events, _scene) = game();
        game.cookies = cookies(&[("test:old", b"from before")]);
        let c = |p: Common| Event::Configuration(cfg::Clientbound::Common(p));
        remote.event(c(store("test:a", b"one")));
        remote.event(c(transfer("127.0.0.1", 25622)));
        // Behind it: a cookie is still stored, nothing else is handled.
        remote.event(c(store("test:b", b"two")));
        remote.event(c(Common::Ping { id: 5 }));
        remote.event(c(Common::CookieRequest {
            key: "test:a".into(),
        }));
        remote.event(c(transfer("127.0.0.1", 1)));
        remote.event(Event::Configuration(cfg::Clientbound::Finish));
        assert!(game.frame(Default::default()));
        assert_eq!(remote.sent(), vec![Sent::Close]);
        let got = end(&mut game, &remote, &events, Local::Closed);
        let want = Transfer {
            to: ServerAddress::new("127.0.0.1", 25622),
            state: TransferState {
                cookies: cookies(&[
                    ("test:old", b"from before"),
                    ("test:a", b"one"),
                    ("test:b", b"two"),
                ]),
                from: None,
            },
        };
        assert_eq!(
            got[got.len() - 2..],
            [
                SessionEvent::Transfer(want),
                SessionEvent::Closed(Closed {
                    during_login: false,
                    reason: transferred(),
                }),
            ]
        );
    }

    #[test]
    fn a_transfer_in_play_goes_even_if_the_server_hangs_up_first() {
        let (mut game, mut remote, events, _scene) = game();
        game.phase = super::super::Phase::Play;
        // The server left, as the login found it: where it may send the client
        // depends on it (`transfer_allowed`).
        let peer: std::net::IpAddr = "203.0.113.7".parse().unwrap();
        game.packs.peer = Some(peer);
        let p = |p: Common| Event::Play(play::Clientbound::Common(p));
        remote.event(p(store("test:a", b"one")));
        remote.event(Event::Bundle(vec![
            play::Clientbound::Common(transfer("[::1]", 25565)),
            play::Clientbound::Common(store("test:b", b"two")),
            play::Clientbound::Common(Common::Ping { id: 1 }),
        ]));
        assert!(game.frame(Default::default()));
        // The ping behind the transfer isn't answered.
        assert_eq!(remote.sent(), vec![Sent::Close]);
        let got = end(&mut game, &remote, &events, Local::EndOfStream);
        assert_eq!(
            got,
            [
                SessionEvent::Transfer(Transfer {
                    to: ServerAddress::new("::1", 25565),
                    state: TransferState {
                        cookies: cookies(&[("test:a", b"one"), ("test:b", b"two")]),
                        from: Some(peer),
                    },
                }),
                SessionEvent::Closed(Closed {
                    during_login: false,
                    reason: transferred(),
                }),
            ]
        );
    }

    #[test]
    fn no_ticks_run_once_a_transfer_has_begun() {
        let (mut game, mut remote, _events, _scene) = game();
        let mut overworld = Compound::new();
        overworld.put("min_y", Tag::Int(0));
        overworld.put("height", Tag::Int(64));
        overworld.put("has_skylight", Tag::Byte(1));
        overworld.put("has_ceiling", Tag::Byte(0));
        game.configured.registries = vec![(
            crate::environment::DIMENSION_TYPE.into(),
            vec![ruthenium_protocol::packets::configuration::RegistryEntry {
                id: "minecraft:overworld".into(),
                data: Some(Tag::Compound(overworld)),
            }],
        )];
        game.phase = super::super::Phase::Play;
        remote.event(Event::Play(play::Clientbound::Login(Box::new(
            play::Login {
                player_id: 7,
                hardcore: false,
                levels: vec!["minecraft:overworld".into()],
                max_players: 20,
                chunk_radius: 2,
                simulation_distance: 2,
                reduced_debug_info: false,
                show_death_screen: true,
                limited_crafting: false,
                spawn: play::SpawnInfo {
                    dimension_type: 0,
                    dimension: "minecraft:overworld".into(),
                    seed: 0,
                    game_type: play::CREATIVE,
                    previous_game_type: None,
                    is_debug: false,
                    is_flat: true,
                    last_death: None,
                    portal_cooldown: 0,
                    sea_level: 0,
                },
                online_mode: false,
                enforces_secure_chat: false,
            },
        ))));
        assert!(game.frame(Default::default()));
        // A level: two ticks' worth of time, two ticks' ends.
        game.manual_ms = Some(100);
        assert!(game.frame(Default::default()));
        let tick_end = Sent::Packet(play::client_tick_end());
        let ends = |sent: &[Sent]| sent.iter().filter(|s| **s == tick_end).count();
        assert_eq!(ends(&remote.sent()), 2);
        remote.event(Event::Play(play::Clientbound::Common(transfer(
            "127.0.0.1",
            25622,
        ))));
        game.manual_ms = Some(200);
        assert!(game.frame(Default::default()));
        game.manual_ms = Some(300);
        assert!(game.frame(Default::default()));
        assert_eq!(remote.sent(), vec![Sent::Close]);
    }

    #[test]
    fn a_transfer_vanilla_cant_address_only_ends_the_session() {
        for (host, port) in [("example.com:25", 25565), ("example.com", 65536), ("a", -1)] {
            let (mut game, remote, events, _scene) = game();
            game.phase = super::super::Phase::Play;
            remote.event(Event::Play(play::Clientbound::Common(transfer(host, port))));
            assert!(game.frame(Default::default()));
            let got = end(&mut game, &remote, &events, Local::Closed);
            assert_eq!(
                got,
                [SessionEvent::Closed(Closed {
                    during_login: false,
                    reason: transferred(),
                })],
                "{host:?} {port}"
            );
        }
    }
}

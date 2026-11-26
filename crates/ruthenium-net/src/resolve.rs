//! Turns a server address into a socket address as vanilla does
//! (`multiplayer/resolver/ServerNameResolver.java`, `ServerRedirectHandler.java`):
//!
//! 1. the host through the system resolver (Java's `InetAddress.getByName`, which prefers
//!    IPv4: the first IPv4 address, else the first address);
//! 2. if the port is the default 25565, the SRV record `_minecraft._tcp.<host>`: when
//!    there is one, its first record's target and port are resolved instead, whatever
//!    step 1 found.
//!
//! Vanilla takes the first SRV record as the DNS answer lists it, ignoring priority and
//! weight; so does this.
//!
//! Servers on Mojang's list of blocked servers do not resolve (`AddressCheck`,
//! `ServerNameResolver`): the list is fetched once, the first time an address is
//! resolved, and the address as typed, the host it resolved to (by name and by IP) and an
//! SRV target are checked against it; a blocked one is an unknown host, in the list's
//! ping and when joining alike. See [`BlockedServers`].

use std::collections::HashSet;
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};

use hickory_resolver::TokioResolver;
use hickory_resolver::proto::rr::RData;
use sha1::{Digest, Sha1};
use tokio::sync::OnceCell;

use crate::address::{DEFAULT_PORT, ServerAddress};

/// Where Mojang publishes the blocked servers' hashes (patchy's `MojangBlockListSupplier`).
pub const BLOCKED_SERVERS_URL: &str = "https://sessionserver.mojang.com/blockedservers";

/// Mojang's list of blocked servers (patchy's `BlockedServers`): SHA-1 hashes, in lower
/// case hex, of lower-cased host patterns. A host is blocked when the list has
///
/// - the host itself (without an `_minecraft._tcp.` prefix or trailing full stops);
/// - for a name, `*.` and the name, or `*.` and the name without its first one or more
///   labels (`*.example.com` blocks `mc.example.com`);
/// - for an IPv4 address, its first one to three numbers and `.*` (`10.0.*`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BlockedServers {
    hashes: HashSet<String>,
}

impl BlockedServers {
    /// The list as Mojang serves it: one hash a line.
    pub fn from_text(text: &str) -> BlockedServers {
        BlockedServers {
            hashes: text.lines().map(str::to_owned).collect(),
        }
    }

    /// A list blocking exactly these patterns (for tests).
    pub fn of_patterns<'a>(patterns: impl IntoIterator<Item = &'a str>) -> BlockedServers {
        BlockedServers {
            hashes: patterns.into_iter().map(hash).collect(),
        }
    }

    pub fn len(&self) -> usize {
        self.hashes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.hashes.is_empty()
    }

    /// Whether `host` (a name or an IP address) is blocked.
    pub fn blocks(&self, host: &str) -> bool {
        if host.is_empty() || self.hashes.is_empty() {
            return false;
        }
        let host = host.strip_prefix("_minecraft._tcp.").unwrap_or(host);
        let host = host.trim_end_matches('.');
        if host.is_empty() {
            return false;
        }
        let listed = |pattern: &str| self.hashes.contains(&hash(pattern));
        if listed(host) {
            return true;
        }
        let mut parts: Vec<&str> = host.split('.').collect();
        // Java's `Integer.parseInt`, which takes a sign.
        let ip = parts.len() == 4
            && parts
                .iter()
                .all(|p| p.parse::<i32>().is_ok_and(|n| (0..=255).contains(&n)));
        if !ip && listed(&format!("*.{host}")) {
            return true;
        }
        while parts.len() > 1 {
            let pattern = if ip {
                parts.pop();
                format!("{}.*", parts.join("."))
            } else {
                parts.remove(0);
                format!("*.{}", parts.join("."))
            };
            if listed(&pattern) {
                return true;
            }
        }
        false
    }
}

/// A pattern's hash: SHA-1 of its lower case in ISO 8859-1 (other characters as `?`).
fn hash(pattern: &str) -> String {
    let bytes: Vec<u8> = pattern
        .to_lowercase()
        .chars()
        .map(|c| u8::try_from(u32::from(c)).unwrap_or(b'?'))
        .collect();
    Sha1::digest(&bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Whether resolving checks Mojang's list, which the game's binaries turn on
/// ([`check_blocked_servers`]); off for libraries and tests.
static CHECK: AtomicU8 = AtomicU8::new(0);
/// The list, fetched the first time it is needed (`None`: it couldn't be).
static MOJANG_LIST: OnceCell<Option<Arc<BlockedServers>>> = OnceCell::const_new();

/// Turns the check against Mojang's list of blocked servers on or off for every resolver.
pub fn check_blocked_servers(on: bool) {
    CHECK.store(u8::from(on), Ordering::Relaxed);
}

/// Fetches Mojang's list (once a run: a list that couldn't be fetched stays unfetched,
/// as in vanilla, where every server is then allowed).
async fn mojang_list() -> Option<Arc<BlockedServers>> {
    MOJANG_LIST
        .get_or_init(|| async {
            let fetched = tokio::task::spawn_blocking(fetch_blocked_servers)
                .await
                .map_err(|e| e.to_string())
                .and_then(|r| r);
            match fetched {
                Ok(list) => {
                    eprintln!("[net] {} blocked servers", list.len());
                    Some(Arc::new(list))
                }
                Err(e) => {
                    eprintln!("[net] couldn't fetch the blocked servers: {e}");
                    None
                }
            }
        })
        .await
        .clone()
}

/// A plain GET of [`BLOCKED_SERVERS_URL`], as the game makes it.
fn fetch_blocked_servers() -> Result<BlockedServers, String> {
    let agent = ureq::Agent::config_builder()
        .timeout_global(Some(std::time::Duration::from_secs(30)))
        .build()
        .new_agent();
    let text = agent
        .get(BLOCKED_SERVERS_URL)
        .call()
        .map_err(|e| e.to_string())?
        .body_mut()
        .read_to_string()
        .map_err(|e| e.to_string())?;
    Ok(BlockedServers::from_text(&text))
}

/// Which list a resolver checks addresses against.
#[derive(Debug, Clone)]
enum Check {
    /// Mojang's, if checking is on.
    Mojang,
    Fixed(Arc<BlockedServers>),
}

/// The SRV lookup's resolver (system DNS configuration) and the system resolver.
pub struct Resolver {
    srv: Option<TokioResolver>,
    check: Check,
}

/// A host resolved to an address (`ResolvedServerAddress`).
struct Resolved {
    socket: SocketAddr,
    /// The host's name: as asked for, or for an IP address its reverse lookup's name
    /// (when that name leads back to it).
    name: Option<String>,
}

impl std::fmt::Debug for Resolver {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Resolver")
            .field("srv", &self.srv.is_some())
            .finish()
    }
}

impl Resolver {
    /// Must be called inside a Tokio runtime. Without a usable DNS configuration, SRV
    /// records are not looked up (vanilla logs and carries on the same way).
    pub fn new() -> Resolver {
        let srv = match TokioResolver::builder_tokio().and_then(|b| b.build()) {
            Ok(r) => Some(r),
            Err(e) => {
                eprintln!("[net] no SRV lookups, some servers may not resolve: {e}");
                None
            }
        };
        Resolver {
            srv,
            check: Check::Mojang,
        }
    }

    /// Like [`Resolver::new`], checking addresses against `list` instead of Mojang's.
    pub fn with_blocked_servers(list: BlockedServers) -> Resolver {
        Resolver {
            check: Check::Fixed(Arc::new(list)),
            ..Resolver::new()
        }
    }

    /// The list to check against, if any.
    async fn blocked(&self) -> Option<Arc<BlockedServers>> {
        match &self.check {
            Check::Fixed(l) => Some(l.clone()),
            Check::Mojang if CHECK.load(Ordering::Relaxed) == 1 => mojang_list().await,
            Check::Mojang => None,
        }
        .filter(|l| !l.is_empty())
    }

    /// A resolved host's name and IP (as Java writes it).
    async fn names(&self, r: &Resolved) -> (String, String) {
        let ip = java_ip(r.socket.ip());
        let name = match &r.name {
            Some(n) => n.clone(),
            None => self
                .reverse(r.socket.ip())
                .await
                .unwrap_or_else(|| ip.clone()),
        };
        (name, ip)
    }

    /// Whether a resolved host is allowed, by its name and its IP.
    async fn allowed(&self, list: &BlockedServers, r: &Resolved) -> bool {
        let (name, ip) = self.names(r).await;
        permitted(list, "", Some((&name, &ip)))
    }

    /// The name an IP address has in DNS, if that name resolves back to it (Java's
    /// `InetAddress.getHostName`).
    async fn reverse(&self, ip: IpAddr) -> Option<String> {
        let lookup = self.srv.as_ref()?.reverse_lookup(ip).await.ok()?;
        let name = lookup.answers().iter().find_map(|r| match &r.data {
            RData::PTR(p) => Some(p.0.to_string()),
            _ => None,
        })?;
        let back = tokio::net::lookup_host((name.as_str(), 0)).await.ok()?;
        back.into_iter()
            .any(|a| a.ip() == ip)
            .then(|| name.trim_end_matches('.').to_owned())
    }

    /// Resolves `address` as `ServerNameResolver.resolveAddress` does: the host, then the
    /// SRV redirect if there is one; `None` for a host that doesn't resolve or is blocked.
    /// Gives the host connected to (the SRV target, if any) with the address.
    async fn resolve_checked(&self, address: &ServerAddress) -> Option<(String, SocketAddr)> {
        let host = address.host_ascii();
        let direct = system(&host, address.port()).await.map(|socket| Resolved {
            socket,
            name: named(&host),
        });
        let list = self.blocked().await;
        if let Some(list) = &list {
            let names = match &direct {
                Some(r) => Some(self.names(r).await),
                None => None,
            };
            let resolved = names.as_ref().map(|(n, ip)| (n.as_str(), ip.as_str()));
            if !permitted(list, &host, resolved) {
                eprintln!("[net] {host} is on the list of blocked servers");
                return None;
            }
        }
        match self.redirect(address).await {
            Some(target) => {
                let host = target.host_ascii();
                let r = system(&host, target.port()).await.map(|socket| Resolved {
                    socket,
                    name: named(&host),
                })?;
                if let Some(list) = &list
                    && !self.allowed(list, &r).await
                {
                    eprintln!("[net] {host} is on the list of blocked servers");
                    return None;
                }
                Some((host, r.socket))
            }
            None => direct.map(|r| (host, r.socket)),
        }
    }

    /// The socket address to connect to, or `None` if the host does not resolve (or is
    /// blocked).
    pub async fn resolve(&self, address: &ServerAddress) -> Option<SocketAddr> {
        self.resolve_checked(address).await.map(|(_, a)| a)
    }

    /// The host the connection is to (the SRV target if there is one, as vanilla sends
    /// it in the login handshake) and the socket address, or `None` if it doesn't
    /// resolve.
    ///
    /// Vanilla sends `InetSocketAddress.getHostName()`, which for an address typed as
    /// an IP literal is a reverse lookup; this sends the literal instead.
    pub async fn resolve_named(&self, address: &ServerAddress) -> Option<(String, SocketAddr)> {
        self.resolve_checked(address).await
    }

    /// The SRV redirect for `address`, if it has the default port and a record.
    pub async fn redirect(&self, address: &ServerAddress) -> Option<ServerAddress> {
        let srv = self.srv.as_ref()?;
        let host = address.host_ascii();
        if address.port() != DEFAULT_PORT || host.is_empty() {
            return None;
        }
        let lookup = srv
            .srv_lookup(format!("_minecraft._tcp.{host}"))
            .await
            .ok()?;
        lookup.answers().iter().find_map(|r| match &r.data {
            RData::SRV(s) => Some(ServerAddress::new(&s.target.to_string(), s.port)),
            _ => None,
        })
    }
}

impl Default for Resolver {
    fn default() -> Resolver {
        Resolver::new()
    }
}

/// `ServerNameResolver`'s check: the host as resolved (by name and by IP), if it
/// resolved, and the host as typed.
fn permitted(list: &BlockedServers, typed: &str, resolved: Option<(&str, &str)>) -> bool {
    resolved.is_none_or(|(name, ip)| !list.blocks(name) && !list.blocks(ip)) && !list.blocks(typed)
}

/// A host's name, unless it is an IP address (whose name takes a reverse lookup).
fn named(host: &str) -> Option<String> {
    host.parse::<IpAddr>().is_err().then(|| host.to_owned())
}

/// An IP address as Java's `getHostAddress` writes it: IPv6 in eight groups, none left
/// out.
fn java_ip(ip: IpAddr) -> String {
    match ip {
        IpAddr::V4(v4) => v4.to_string(),
        IpAddr::V6(v6) => v6
            .segments()
            .iter()
            .map(|s| format!("{s:x}"))
            .collect::<Vec<_>>()
            .join(":"),
    }
}

/// The system resolver, preferring IPv4 as Java does.
async fn system(host: &str, port: u16) -> Option<SocketAddr> {
    if host.is_empty() {
        return None;
    }
    let addrs: Vec<SocketAddr> = tokio::net::lookup_host((host, port)).await.ok()?.collect();
    addrs
        .iter()
        .find(|a| a.is_ipv4())
        .or_else(|| addrs.first())
        .copied()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn offline(list: BlockedServers) -> Resolver {
        Resolver {
            srv: None,
            check: Check::Fixed(Arc::new(list)),
        }
    }

    #[test]
    fn hashes_are_sha1_of_lower_case() {
        // printf 'mc.example.com' | sha1sum
        assert_eq!(
            hash("MC.Example.com"),
            "5303a76c19617a55ae2c2102319038f225fcc328"
        );
        assert_eq!(hash("é"), hash("\u{E9}"));
        assert_eq!(hash("\u{3042}"), hash("?"));
    }

    /// What patchy's `BlockedServers.test` (the game's own class) answers for these hosts
    /// against lists of these patterns.
    #[test]
    fn blocks_as_the_games_list_does() {
        let hosts = [
            "evil.test",
            "EVIL.test",
            "evil.test.",
            "_minecraft._tcp.evil.test",
            "bad.example",
            "mc.bad.example",
            "a.b.bad.example",
            "solo.test",
            "10.1.2.3",
            "10.1.0.0",
            "notevil.test",
            "evil.test.example",
            "example",
            "10.2.1.3",
            "10.1",
            "1.10.1.3",
            "",
            "mc.evil.test",
            "0.1.2.3",
            "10.example.com",
            "x.1.2.3",
            "+10.1.2.3",
            "10.1.2.300",
            "10.01.2.3",
            "10.1..3",
            "127.0.0.1",
            "localhost",
            "LOCALHOST.",
        ];
        for (patterns, blocked) in [
            (
                &["*.bad.example", "evil.test", "10.1.*", "*.solo.test"][..],
                [
                    true, true, true, true, true, true, true, true, true, true, false, false,
                    false, false, false, false, false, false, false, false, false, false, false,
                    false, false, false, false, false,
                ],
            ),
            (
                &["*.1.2.3", "10.example.*"][..],
                [
                    false, false, false, false, false, false, false, false, false, false, false,
                    false, false, false, false, false, false, false, false, false, true, false,
                    false, false, false, false, false, false,
                ],
            ),
            (
                &["localhost", "127.0.*"][..],
                [
                    false, false, false, false, false, false, false, false, false, false, false,
                    false, false, false, false, false, false, false, false, false, false, false,
                    false, false, false, true, true, true,
                ],
            ),
        ] {
            let list = BlockedServers::of_patterns(patterns.iter().copied());
            for (host, want) in hosts.iter().zip(blocked) {
                assert_eq!(list.blocks(host), want, "{patterns:?} {host:?}");
            }
        }
        // Mojang's lines, as served.
        let text = format!("{}\n{}\n", hash("a.test"), hash("*.b.test"));
        let list = BlockedServers::from_text(&text);
        assert!(list.blocks("a.test") && list.blocks("c.b.test") && !list.blocks("c.test"));
    }

    #[test]
    fn the_typed_host_and_the_resolved_one_are_both_checked() {
        let list = BlockedServers::of_patterns(["typed.test", "10.0.*", "*.named.test"]);
        assert!(permitted(&list, "ok.test", None));
        assert!(permitted(&list, "ok.test", Some(("ok.test", "192.0.2.1"))));
        // Not resolved (an SRV record may still lead somewhere): the typed host counts.
        assert!(!permitted(&list, "typed.test", None));
        assert!(!permitted(&list, "ok.test", Some(("ok.test", "10.0.0.5"))));
        assert!(!permitted(
            &list,
            "ok.test",
            Some(("mc.named.test", "192.0.2.1"))
        ));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn blocked_servers_do_not_resolve() {
        let r = offline(BlockedServers::of_patterns(["127.0.*"]));
        assert_eq!(
            r.resolve(&ServerAddress::parse("127.0.0.1:25570")).await,
            None
        );
        assert_eq!(
            r.resolve(&ServerAddress::parse("localhost:25570")).await,
            None
        );
        let r = offline(BlockedServers::of_patterns(["localhost"]));
        assert_eq!(
            r.resolve(&ServerAddress::parse("localhost:25570")).await,
            None
        );
        assert!(
            r.resolve(&ServerAddress::parse("127.0.0.1:25570"))
                .await
                .is_some()
        );
        assert_eq!(java_ip("::1".parse().unwrap()), "0:0:0:0:0:0:0:1");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn literals_resolve_without_dns() {
        let r = offline(BlockedServers::default());
        let a = r.resolve(&ServerAddress::parse("127.0.0.1:25570")).await;
        assert_eq!(a, Some("127.0.0.1:25570".parse().unwrap()));
        let a = r.resolve(&ServerAddress::parse("[::1]")).await;
        assert_eq!(a, Some("[::1]:25565".parse().unwrap()));
        assert_eq!(r.resolve(&ServerAddress::new("", 1)).await, None);
    }
}

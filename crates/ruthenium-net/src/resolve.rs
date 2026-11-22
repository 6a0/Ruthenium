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
//! weight; so does this. Not yet: the Mojang server blocklist (`AddressCheck`).

use std::net::SocketAddr;

use hickory_resolver::TokioResolver;
use hickory_resolver::proto::rr::RData;

use crate::address::{DEFAULT_PORT, ServerAddress};

/// The SRV lookup's resolver (system DNS configuration) and the system resolver.
pub struct Resolver {
    srv: Option<TokioResolver>,
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
        Resolver { srv }
    }

    /// The socket address to connect to, or `None` if the host does not resolve.
    pub async fn resolve(&self, address: &ServerAddress) -> Option<SocketAddr> {
        let direct = system(&address.host_ascii(), address.port()).await;
        match self.redirect(address).await {
            Some(target) => system(&target.host_ascii(), target.port()).await,
            None => direct,
        }
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

    #[tokio::test(flavor = "current_thread")]
    async fn literals_resolve_without_dns() {
        let r = Resolver { srv: None };
        let a = r.resolve(&ServerAddress::parse("127.0.0.1:25570")).await;
        assert_eq!(a, Some("127.0.0.1:25570".parse().unwrap()));
        let a = r.resolve(&ServerAddress::parse("[::1]")).await;
        assert_eq!(a, Some("[::1]:25565".parse().unwrap()));
        assert_eq!(r.resolve(&ServerAddress::new("", 1)).await, None);
    }
}

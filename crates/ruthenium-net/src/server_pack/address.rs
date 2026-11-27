//! Which addresses a pack download may connect to, and a resolver that checks them.
//!
//! Vanilla connects wherever the URL points. A pack URL comes from the server, so it could
//! point the player's own machine at their router, a printer, a local admin page or
//! anything else on their network; we refuse every address that isn't on the public
//! internet, except the few the caller allows (the server's own, when it is local).
//! The check runs on the resolved addresses inside ureq's resolver, and ureq connects only
//! to the addresses the resolver returns, so a name can't resolve to a public address for
//! the check and a private one for the connection (DNS rebinding).
//!
//! IPv4 reached through IPv6 is judged as the IPv4 it reaches: mapped addresses, NAT64's
//! well-known prefix 64:ff9b::/96, 6to4, and the network's own NAT64 prefixes, which
//! RFC 7050 finds by asking the system resolver for `ipv4only.arpa` (its DNS64 then
//! answers with 192.0.0.170 and .171 inside the prefix). Without that, a name answering
//! only `<operator prefix>::c0a8:101` (192.168.1.1 through the operator's NAT64) would pass
//! as a public IPv6 address.

use std::fmt;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, ToSocketAddrs};
use std::sync::{Arc, Mutex, mpsc};
use std::time::Duration;

use ureq::config::Config;
use ureq::http::Uri;
use ureq::unversioned::resolver::{DefaultResolver, ResolvedSocketAddrs, Resolver};
use ureq::unversioned::transport::NextTimeout;
use ureq::unversioned::transport::time::Duration as UreqDuration;

/// A range of addresses: `addr/prefix_len`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AddressRange {
    addr: IpAddr,
    prefix_len: u8,
}

impl AddressRange {
    /// Exactly one address (/32 or /128). IPv4-mapped IPv6 is normalised to IPv4.
    pub fn host(ip: IpAddr) -> AddressRange {
        let addr = unmap(ip);
        let prefix_len = if addr.is_ipv4() { 32 } else { 128 };
        AddressRange { addr, prefix_len }
    }

    /// `ip/prefix_len`; None when `prefix_len` is over 32 (IPv4) or 128 (IPv6). An
    /// IPv4-mapped range of /96 or longer is the IPv4 range it maps.
    pub fn new(ip: IpAddr, prefix_len: u8) -> Option<AddressRange> {
        let (addr, prefix_len) = match ip {
            IpAddr::V6(v6) if prefix_len >= 96 && v6.to_ipv4_mapped().is_some() => {
                (unmap(ip), prefix_len - 96)
            }
            _ => (ip, prefix_len),
        };
        let max = if addr.is_ipv4() { 32 } else { 128 };
        (prefix_len <= max).then_some(AddressRange { addr, prefix_len })
    }

    /// Whether `ip` (IPv4-mapped IPv6 as its IPv4) is in the range.
    pub fn contains(&self, ip: IpAddr) -> bool {
        match (self.addr, unmap(ip)) {
            (IpAddr::V4(net), IpAddr::V4(ip)) => in_v4(ip, net.octets(), self.prefix_len.into()),
            (IpAddr::V6(net), IpAddr::V6(ip)) => in_v6(ip, net.segments(), self.prefix_len.into()),
            _ => false,
        }
    }
}

fn unmap(ip: IpAddr) -> IpAddr {
    match ip {
        IpAddr::V6(v6) => v6.to_ipv4_mapped().map_or(ip, IpAddr::V4),
        IpAddr::V4(_) => ip,
    }
}

/// A NAT64 prefix (RFC 6052): an IPv6 address's first `len` bits, with the IPv4 address
/// it reaches in the bits after.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Nat64Prefix {
    bytes: [u8; 16],
    len: u8,
}

/// The prefix lengths RFC 6052 allows.
const NAT64_LENGTHS: [u8; 6] = [96, 64, 56, 48, 40, 32];

/// The IPv4 address embedded after a prefix of `len` bits (RFC 6052 section 2.2: bits 64
/// to 71 are zero and skipped).
fn embedded_v4(b: [u8; 16], len: u8) -> Option<Ipv4Addr> {
    let v4 = match len {
        96 => [b[12], b[13], b[14], b[15]],
        64 => [b[9], b[10], b[11], b[12]],
        56 => [b[7], b[9], b[10], b[11]],
        48 => [b[6], b[7], b[9], b[10]],
        40 => [b[5], b[6], b[7], b[9]],
        32 => [b[4], b[5], b[6], b[7]],
        _ => return None,
    };
    (len == 96 || b[8] == 0).then(|| Ipv4Addr::from(v4))
}

impl Nat64Prefix {
    /// `ip`'s first `len` bits as a prefix.
    pub fn new(ip: Ipv6Addr, len: u8) -> Option<Nat64Prefix> {
        if !NAT64_LENGTHS.contains(&len) {
            return None;
        }
        let mut bytes = [0u8; 16];
        let n = usize::from(len / 8);
        bytes[..n].copy_from_slice(&ip.octets()[..n]);
        Some(Nat64Prefix { bytes, len })
    }

    /// Whether `ip` falls inside this prefix's leading `len` bits.
    fn matches_prefix(&self, ip: Ipv6Addr) -> bool {
        let b = ip.octets();
        let n = usize::from(self.len / 8);
        b[..n] == self.bytes[..n]
    }

    /// The IPv4 address `ip` reaches through this prefix, if it is in it and conforms to
    /// RFC 6052 (`b[8] == 0` when `len != 96`).
    fn embedded(&self, ip: Ipv6Addr) -> Option<Ipv4Addr> {
        if !self.matches_prefix(ip) {
            return None;
        }
        embedded_v4(ip.octets(), self.len)
    }
}

/// RFC 7050's well-known IPv4 addresses of `ipv4only.arpa`.
const IPV4ONLY: [Ipv4Addr; 2] = [Ipv4Addr::new(192, 0, 0, 170), Ipv4Addr::new(192, 0, 0, 171)];

/// RFC 7050 section 3: the prefixes in which `ipv4only.arpa`'s IPv6 answers carry its
/// well-known IPv4 addresses. Every length that fits is kept (more is stricter).
pub(crate) fn nat64_prefixes(answers: impl IntoIterator<Item = IpAddr>) -> Vec<Nat64Prefix> {
    let mut out = Vec::new();
    for ip in answers {
        let IpAddr::V6(v6) = ip else { continue };
        if v6.to_ipv4_mapped().is_some() {
            continue;
        }
        for len in NAT64_LENGTHS {
            if embedded_v4(v6.octets(), len).is_some_and(|v4| IPV4ONLY.contains(&v4))
                && let Some(p) = Nat64Prefix::new(v6, len)
                && !out.contains(&p)
            {
                out.push(p);
            }
        }
    }
    out
}

/// Asks the system resolver for `ipv4only.arpa` (RFC 7050), giving up after `timeout`:
/// returns `Err(())` when the lookup timed out or couldn't run so callers can fail closed
/// instead of caching an empty prefix list on transient timeouts.
pub(crate) fn discover_nat64(timeout: Duration) -> Result<Vec<Nat64Prefix>, ()> {
    let (tx, rx) = mpsc::sync_channel(1);
    let spawned = std::thread::Builder::new()
        .name("nat64-discovery".into())
        .spawn(move || {
            let answers: Vec<IpAddr> = ("ipv4only.arpa", 0)
                .to_socket_addrs()
                .map(|a| a.map(|s| s.ip()).collect())
                .unwrap_or_default();
            let _ = tx.send(answers);
        });
    if spawned.is_err() {
        return Err(());
    }
    let answers = rx.recv_timeout(timeout).map_err(|_| ())?;
    let prefixes = nat64_prefixes(answers);
    if !prefixes.is_empty() {
        eprintln!("[packs] NAT64 prefixes of this network: {prefixes:?}");
    }
    Ok(prefixes)
}

/// What a download may connect to.
#[derive(Debug, Clone, Default)]
pub(crate) struct Policy {
    /// Non-public addresses still allowed (the server's own, when it is local).
    pub private_allowed: Vec<AddressRange>,
    /// The network's NAT64 prefixes besides the well-known one.
    pub nat64: Vec<Nat64Prefix>,
}

impl Policy {
    /// Whether a download may connect to `ip`: a public address, or one the caller
    /// allows; never an address no server can have.
    pub fn allowed(&self, ip: IpAddr) -> bool {
        if never(ip) {
            return false;
        }
        let base = canonical(ip);
        if never(base) {
            return false;
        }
        if let IpAddr::V6(v6) = base {
            let mut matched_nat64 = false;
            for prefix in &self.nat64 {
                if !prefix.matches_prefix(v6) {
                    continue;
                }
                matched_nat64 = true;
                // A non-zero reserved `u` octet (byte 8) in a non-/96 NAT64 prefix is
                // non-standard and may be ignored by permissive NAT64 translators; refuse it.
                let Some(v4) = prefix.embedded(v6) else {
                    return false;
                };
                let reached = IpAddr::V4(v4);
                if never(reached) {
                    return false;
                }
                let ok = is_public_v4(v4)
                    || self
                        .private_allowed
                        .iter()
                        .any(|r| r.contains(ip) || r.contains(reached));
                if !ok {
                    return false;
                }
            }
            if matched_nat64 {
                return true;
            }
        }
        is_public(base)
            || self
                .private_allowed
                .iter()
                .any(|r| r.contains(ip) || r.contains(base))
    }

    /// Whether the answer depends on the network's NAT64 prefixes: an IPv6 address that
    /// would otherwise pass.
    fn needs_nat64(&self, ip: IpAddr) -> bool {
        matches!(canonical(ip), IpAddr::V6(_)) && self.allowed(ip)
    }
}

/// IPv4 written as IPv6 (mapped, or NAT64's well-known prefix) as the IPv4 it reaches.
fn canonical(ip: IpAddr) -> IpAddr {
    match ip {
        IpAddr::V6(v6) => {
            if let Some(v4) = v6.to_ipv4_mapped() {
                return v4.into();
            }
            let s = v6.segments();
            // 64:ff9b::/96 (RFC 6052): a NAT64 gateway turns it into the embedded IPv4.
            if s[..6] == [0x64, 0xff9b, 0, 0, 0, 0] {
                let [_, _, _, _, _, _, a, b] = s;
                return Ipv4Addr::from((u32::from(a) << 16) | u32::from(b)).into();
            }
            ip
        }
        IpAddr::V4(_) => ip,
    }
}

/// Addresses no web server has, local or not: unspecified, broadcast, multicast.
fn never(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => v4.octets()[0] == 0 || v4.is_broadcast() || v4.is_multicast(),
        IpAddr::V6(v6) => v6.is_unspecified() || v6.is_multicast(),
    }
}

/// Whether `ip` (canonical) is a unicast address on the public internet.
pub(crate) fn is_public(ip: IpAddr) -> bool {
    match canonical(ip) {
        IpAddr::V4(v4) => is_public_v4(v4),
        IpAddr::V6(v6) => is_public_v6(v6),
    }
}

fn in_v4(ip: Ipv4Addr, net: [u8; 4], bits: u32) -> bool {
    let mask = if bits == 0 {
        0
    } else {
        u32::MAX << (32 - bits)
    };
    u32::from(ip) & mask == u32::from(Ipv4Addr::from(net)) & mask
}

/// Refused IPv4 ranges (IANA's special-purpose registry, RFC 6890 and later).
const V4_REFUSED: &[([u8; 4], u32)] = &[
    ([0, 0, 0, 0], 8),       // "this network"
    ([10, 0, 0, 0], 8),      // private (RFC 1918)
    ([100, 64, 0, 0], 10),   // carrier-grade NAT (RFC 6598)
    ([127, 0, 0, 0], 8),     // loopback
    ([169, 254, 0, 0], 16),  // link-local (and cloud metadata at 169.254.169.254)
    ([172, 16, 0, 0], 12),   // private
    ([192, 0, 0, 0], 24),    // IETF protocol assignments
    ([192, 0, 2, 0], 24),    // documentation (TEST-NET-1)
    ([192, 88, 99, 0], 24),  // 6to4 relay anycast (deprecated)
    ([192, 168, 0, 0], 16),  // private
    ([198, 18, 0, 0], 15),   // benchmarking
    ([198, 51, 100, 0], 24), // documentation (TEST-NET-2)
    ([203, 0, 113, 0], 24),  // documentation (TEST-NET-3)
    ([224, 0, 0, 0], 4),     // multicast
    ([240, 0, 0, 0], 4),     // reserved, and broadcast
];

fn is_public_v4(ip: Ipv4Addr) -> bool {
    !V4_REFUSED.iter().any(|&(net, bits)| in_v4(ip, net, bits))
}

fn in_v6(ip: Ipv6Addr, net: [u16; 8], bits: u32) -> bool {
    let mask = if bits == 0 {
        0
    } else {
        u128::MAX << (128 - bits)
    };
    u128::from(ip) & mask == u128::from(Ipv6Addr::from(net)) & mask
}

/// Refused ranges inside 2000::/3, the only IPv6 space handed out for public unicast.
const V6_REFUSED: &[([u16; 8], u32)] = &[
    ([0x2001, 0, 0, 0, 0, 0, 0, 0], 23), // IETF protocol assignments (Teredo, ORCHID, ...)
    ([0x2001, 0xdb8, 0, 0, 0, 0, 0, 0], 32), // documentation
    ([0x3fff, 0, 0, 0, 0, 0, 0, 0], 20), // documentation (RFC 9637)
];

fn is_public_v6(ip: Ipv6Addr) -> bool {
    // Everything outside 2000::/3: loopback, IPv4-compatible, discard-only, ULA (fc00::/7),
    // link-local (fe80::/10), site-local, multicast, SRv6 SIDs (5f00::/16), NAT64's local
    // prefix (64:ff9b:1::/48), ...
    if !in_v6(ip, [0x2000, 0, 0, 0, 0, 0, 0, 0], 3) {
        return false;
    }
    if V6_REFUSED.iter().any(|&(net, bits)| in_v6(ip, net, bits)) {
        return false;
    }
    // 6to4 (2002::/16) reaches the IPv4 address it embeds.
    let s = ip.segments();
    if s[0] == 0x2002 {
        let v4 = Ipv4Addr::from((u32::from(s[1]) << 16) | u32::from(s[2]));
        return is_public_v4(v4);
    }
    true
}

/// ureq's resolver, keeping only answers the policy allows: a name with any refused
/// address is refused as a whole (a public pack host never answers with a private one).
pub(crate) struct CheckedResolver {
    inner: DefaultResolver,
    policy: Policy,
    /// The network's NAT64 prefixes: given (tests), or found on the first IPv6 answer.
    nat64: Mutex<Option<Vec<Nat64Prefix>>>,
    discovery_timeout: Duration,
    /// Addresses tests treat as public (the test server is on loopback).
    pretend_public: Vec<IpAddr>,
    /// The last address refused, for the error.
    pub refused: Arc<Mutex<Option<IpAddr>>>,
}

impl CheckedResolver {
    pub fn new(
        private_allowed: Vec<AddressRange>,
        nat64: Option<Vec<Nat64Prefix>>,
        discovery_timeout: Duration,
        pretend_public: Vec<IpAddr>,
    ) -> CheckedResolver {
        CheckedResolver {
            inner: DefaultResolver::default(),
            policy: Policy {
                private_allowed,
                nat64: Vec::new(),
            },
            nat64: Mutex::new(nat64),
            discovery_timeout,
            pretend_public,
            refused: Arc::default(),
        }
    }

    fn allowed(&self, ip: IpAddr, timeout: NextTimeout) -> bool {
        if self.pretend_public.contains(&ip) {
            return true;
        }
        if !self.policy.needs_nat64(ip) {
            return self.policy.allowed(ip);
        }
        let nat64 = {
            let guard = self.nat64.lock().unwrap_or_else(|e| e.into_inner());
            guard.clone()
        };
        let nat64 = match nat64 {
            Some(list) => list,
            None => {
                let budget = match timeout.after {
                    UreqDuration::Exact(d) => self.discovery_timeout.min(d),
                    _ => self.discovery_timeout,
                };
                if budget.is_zero() {
                    return false;
                }
                let Ok(found) = discover_nat64(budget) else {
                    // Fail closed if NAT64 discovery timed out, without permanently caching.
                    return false;
                };
                let mut guard = self.nat64.lock().unwrap_or_else(|e| e.into_inner());
                *guard = Some(found.clone());
                found
            }
        };
        Policy {
            private_allowed: self.policy.private_allowed.clone(),
            nat64,
        }
        .allowed(ip)
    }
}

impl fmt::Debug for CheckedResolver {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CheckedResolver")
            .field("policy", &self.policy)
            .finish_non_exhaustive()
    }
}

impl Resolver for CheckedResolver {
    fn resolve(
        &self,
        uri: &Uri,
        config: &Config,
        timeout: NextTimeout,
    ) -> Result<ResolvedSocketAddrs, ureq::Error> {
        let addrs = self.inner.resolve(uri, config, timeout)?;
        for addr in addrs.iter() {
            let ip = addr.ip();
            if !self.allowed(ip, timeout) {
                if let Ok(mut r) = self.refused.lock() {
                    *r = Some(ip);
                }
                return Err(ureq::Error::Io(std::io::Error::new(
                    std::io::ErrorKind::PermissionDenied,
                    format!("refused to connect to {ip}"),
                )));
            }
        }
        Ok(addrs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(s: &str) -> IpAddr {
        s.parse().unwrap()
    }

    /// The policy with nothing private allowed, or everything.
    fn allowed(ip: IpAddr, private: bool) -> bool {
        let all = || {
            vec![
                AddressRange::new("0.0.0.0".parse().unwrap(), 0).unwrap(),
                AddressRange::new("::".parse().unwrap(), 0).unwrap(),
            ]
        };
        Policy {
            private_allowed: if private { all() } else { Vec::new() },
            nat64: Vec::new(),
        }
        .allowed(ip)
    }

    fn only(ranges: &[AddressRange]) -> Policy {
        Policy {
            private_allowed: ranges.to_vec(),
            nat64: Vec::new(),
        }
    }

    #[test]
    fn ranges_hold_what_they_say() {
        let host = AddressRange::host(ip("192.168.1.20"));
        assert!(host.contains(ip("192.168.1.20")));
        assert!(host.contains(ip("::ffff:192.168.1.20")));
        assert!(!host.contains(ip("192.168.1.21")));
        assert!(!host.contains(ip("::1")));
        assert_eq!(
            AddressRange::host(ip("::ffff:10.0.0.1")),
            AddressRange::host(ip("10.0.0.1"))
        );
        let net = AddressRange::new(ip("10.1.2.3"), 16).unwrap();
        assert!(net.contains(ip("10.1.255.1")));
        assert!(!net.contains(ip("10.2.0.1")));
        let mapped = AddressRange::new(ip("::ffff:10.1.0.0"), 112).unwrap();
        assert_eq!(mapped, AddressRange::new(ip("10.1.0.0"), 16).unwrap());
        let v6 = AddressRange::new(ip("fd00:1::"), 64).unwrap();
        assert!(v6.contains(ip("fd00:1::5")));
        assert!(!v6.contains(ip("fd00:2::5")));
        assert!(AddressRange::new(ip("10.0.0.0"), 33).is_none());
        assert!(AddressRange::new(ip("::"), 129).is_none());
        assert!(AddressRange::new(ip("::"), 128).is_some());
    }

    #[test]
    fn only_the_listed_private_addresses_are_allowed() {
        let p = only(&[
            AddressRange::host(ip("192.168.1.20")),
            AddressRange::host(ip("::1")),
        ]);
        assert!(p.allowed(ip("192.168.1.20")));
        assert!(p.allowed(ip("::ffff:192.168.1.20")));
        assert!(p.allowed(ip("::1")));
        assert!(p.allowed(ip("8.8.8.8")));
        for s in [
            "192.168.1.21",
            "127.0.0.1",
            "10.0.0.1",
            "169.254.169.254",
            "fe80::1",
        ] {
            assert!(!p.allowed(ip(s)), "{s}");
        }
        // Never an address no server has, listed or not.
        let all = only(&[
            AddressRange::new(ip("0.0.0.0"), 0).unwrap(),
            AddressRange::new(ip("::"), 0).unwrap(),
        ]);
        for s in [
            "0.0.0.0",
            "255.255.255.255",
            "224.0.0.1",
            "::",
            "ff02::1",
            "::ffff:224.0.0.1",
        ] {
            assert!(!all.allowed(ip(s)), "{s}");
        }
    }

    /// `v4` inside `prefix` (of `len` bits) as RFC 6052 writes it.
    fn synthesize(prefix: &str, len: u8, v4: &str) -> IpAddr {
        let mut b = prefix.parse::<Ipv6Addr>().unwrap().octets();
        let v4: Ipv4Addr = v4.parse().unwrap();
        let o = v4.octets();
        let positions: &[usize] = match len {
            96 => &[12, 13, 14, 15],
            64 => &[9, 10, 11, 12],
            56 => &[7, 9, 10, 11],
            48 => &[6, 7, 9, 10],
            40 => &[5, 6, 7, 9],
            _ => &[4, 5, 6, 7],
        };
        for (&i, &x) in positions.iter().zip(&o) {
            b[i] = x;
        }
        IpAddr::V6(Ipv6Addr::from(b))
    }

    #[test]
    fn an_operators_nat64_prefix_is_found_and_judged_by_the_ipv4_it_reaches() {
        for (prefix, len) in [
            ("2a00:1234:5678:9abc::", 96),
            ("2a00:1234:5678:9abc::", 64),
            ("2a00:1234:5678:9a00::", 56),
            ("2a00:1234:5678::", 48),
            ("2a00:1234:5600::", 40),
            ("2a00:1234::", 32),
        ] {
            // What ipv4only.arpa answers on that network: its A records and the DNS64's.
            let answers = [
                ip("192.0.0.170"),
                ip("192.0.0.171"),
                synthesize(prefix, len, "192.0.0.170"),
                synthesize(prefix, len, "192.0.0.171"),
            ];
            let found = nat64_prefixes(answers);
            assert!(
                found.contains(&Nat64Prefix::new(prefix.parse().unwrap(), len).unwrap()),
                "/{len}: {found:?}"
            );
            let policy = Policy {
                private_allowed: Vec::new(),
                nat64: found,
            };
            for private in [
                "192.168.1.1",
                "10.0.0.1",
                "127.0.0.1",
                "169.254.169.254",
                "100.64.0.1",
            ] {
                let through = synthesize(prefix, len, private);
                assert!(
                    !policy.allowed(through),
                    "/{len}: {through} reaches {private}"
                );
                // Without the prefix it would have passed for a public IPv6 address.
                assert!(Policy::default().allowed(through), "{through}");
            }
            assert!(policy.allowed(synthesize(prefix, len, "8.8.8.8")), "/{len}");
            // Outside the prefix the same bits are just an IPv6 address.
            let elsewhere = synthesize("2a00:9999:8888:7777::", len, "192.168.1.1");
            assert!(policy.allowed(elsewhere), "/{len}: {elsewhere}");
            // A local server's own address through NAT64 is allowed like the address itself.
            let lan = Policy {
                private_allowed: vec![AddressRange::host(ip("192.168.1.20"))],
                nat64: policy.nat64.clone(),
            };
            assert!(lan.allowed(synthesize(prefix, len, "192.168.1.20")));
            assert!(!lan.allowed(synthesize(prefix, len, "192.168.1.21")));
        }
    }

    #[test]
    fn a_network_without_dns64_has_no_prefix() {
        // ipv4only.arpa's own records only, or nothing at all.
        assert_eq!(nat64_prefixes([ip("192.0.0.170"), ip("192.0.0.171")]), []);
        assert_eq!(nat64_prefixes([]), []);
        // An IPv6 answer that carries neither well-known address isn't a prefix.
        assert_eq!(nat64_prefixes([ip("2a00:1234::1")]), []);
        // RFC 6052's bits 64 to 71 are zero below /96: otherwise it isn't a /64 prefix.
        let IpAddr::V6(v6) = synthesize("2a00:1234:5678:9abc::", 64, "192.0.0.170") else {
            unreachable!()
        };
        let mut b = v6.octets();
        b[8] = 0xff;
        let found = nat64_prefixes([IpAddr::V6(Ipv6Addr::from(b))]);
        assert!(found.iter().all(|p| p.len == 96), "{found:?}");
        // Nor does a mapped address count.
        assert_eq!(nat64_prefixes([ip("::ffff:192.0.0.170")]), []);
    }

    #[test]
    fn public_addresses_are_allowed() {
        for s in [
            "1.1.1.1",
            "8.8.8.8",
            "93.184.215.14",
            "100.63.255.255",
            "100.128.0.0",
            "172.15.255.255",
            "172.32.0.0",
            "192.167.255.255",
            "192.169.0.0",
            "198.17.255.255",
            "198.20.0.0",
            "223.255.255.255",
            "2606:4700:4700::1111",
            "2a00:1450:4001::200e",
            "2001:200::1",
            "::ffff:1.1.1.1",
            "64:ff9b::808:808",
            "2002:0808:0808::1",
        ] {
            assert!(allowed(ip(s), false), "{s}");
            assert!(allowed(ip(s), true), "{s}");
        }
    }

    #[test]
    fn local_and_special_addresses_are_refused() {
        for s in [
            "127.0.0.1",
            "127.255.255.254",
            "10.0.0.1",
            "10.255.255.255",
            "172.16.0.1",
            "172.31.255.255",
            "192.168.1.1",
            "100.64.0.1",
            "100.127.255.255",
            "169.254.169.254",
            "192.0.0.8",
            "192.0.2.1",
            "198.51.100.7",
            "203.0.113.9",
            "198.18.0.1",
            "198.19.255.255",
            "192.88.99.1",
            "240.0.0.1",
            "::1",
            "fc00::1",
            "fd12:3456::1",
            "fe80::1",
            "fec0::1",
            "100::1",
            "2001:db8::1",
            "2001::1",
            "2001:10::1",
            "3fff::1",
            "5f00::1",
            "64:ff9b:1::1",
            "::ffff:127.0.0.1",
            "::ffff:10.0.0.1",
            "::ffff:169.254.169.254",
            "::127.0.0.1",
            "::10.0.0.1",
            "64:ff9b::7f00:1",
            "64:ff9b::c0a8:101",
            "2002:7f00:0001::1",
            "2002:c0a8:0101::1",
        ] {
            assert!(!allowed(ip(s), false), "{s}");
        }
    }

    #[test]
    fn a_local_server_allows_local_packs_but_never_impossible_addresses() {
        for s in [
            "127.0.0.1",
            "192.168.1.20",
            "10.0.0.5",
            "fd00::5",
            "fe80::1",
            "::1",
        ] {
            assert!(allowed(ip(s), true), "{s}");
        }
        for s in [
            "0.0.0.0",
            "0.1.2.3",
            "255.255.255.255",
            "224.0.0.1",
            "239.255.255.250",
            "::",
            "ff02::1",
            "ff05::fb",
            "::ffff:0.0.0.0",
            "::ffff:224.0.0.1",
            "64:ff9b::ffff:ffff",
        ] {
            assert!(!allowed(ip(s), true), "{s}");
            assert!(!allowed(ip(s), false), "{s}");
        }
    }

    #[test]
    fn nat64_non_zero_u_octet_and_multi_prefix_are_refused() {
        // Non-zero RFC 6052 reserved octet b[8] inside a discovered non-/96 prefix must
        // fail closed rather than falling through as a plain public IPv6 address.
        let p64 = Nat64Prefix::new("2a00:1234:5678:9abc::".parse().unwrap(), 64).unwrap();
        let IpAddr::V6(v6) = synthesize("2a00:1234:5678:9abc::", 64, "127.0.0.1") else {
            unreachable!()
        };
        let mut b = v6.octets();
        b[8] = 0x01;
        let policy = Policy {
            private_allowed: Vec::new(),
            nat64: vec![p64],
        };
        assert!(!policy.allowed(IpAddr::V6(Ipv6Addr::from(b))));

        // Multiple overlapping NAT64 prefixes: even if the first extracts a public IPv4,
        // a second matching prefix that extracts a private IPv4 must still refuse it.
        let p32 = Nat64Prefix::new("2a00:1234::".parse().unwrap(), 32).unwrap();
        let p64_priv = Nat64Prefix::new("2a00:1234:0808:0808::".parse().unwrap(), 64).unwrap();
        // Bytes 4..8 = 8.8.8.8 (public under /32), bytes 9..13 = 127.0.0.1 (private under /64).
        let addr = synthesize("2a00:1234:0808:0808::", 64, "127.0.0.1");
        let multi = Policy {
            private_allowed: Vec::new(),
            nat64: vec![p32, p64_priv],
        };
        assert!(!multi.allowed(addr));
    }
}

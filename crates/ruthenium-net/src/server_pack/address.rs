//! Which addresses a pack download may connect to, and a resolver that checks them.
//!
//! Vanilla connects wherever the URL points. A pack URL comes from the server, so it could
//! point the player's own machine at their router, a printer, a local admin page or
//! anything else on their network; we refuse every address that isn't on the public
//! internet unless the server we're playing on is itself local (then its packs may be too).
//! The check runs on the resolved addresses inside ureq's resolver, and ureq connects only
//! to the addresses the resolver returns, so a name can't resolve to a public address for
//! the check and a private one for the connection (DNS rebinding).

use std::fmt;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::sync::{Arc, Mutex};

use ureq::config::Config;
use ureq::http::Uri;
use ureq::unversioned::resolver::{DefaultResolver, ResolvedSocketAddrs, Resolver};
use ureq::unversioned::transport::NextTimeout;

/// Whether a download may connect to `ip`. With `allow_private` (the server we play on is
/// local), everything but addresses no server can have.
pub(crate) fn allowed(ip: IpAddr, allow_private: bool) -> bool {
    let ip = canonical(ip);
    if never(ip) {
        return false;
    }
    allow_private || is_public(ip)
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
    allow_private: bool,
    /// Addresses tests treat as public (the test server is on loopback).
    pretend_public: Vec<IpAddr>,
    /// The last address refused, for the error.
    pub refused: Arc<Mutex<Option<IpAddr>>>,
}

impl CheckedResolver {
    pub fn new(allow_private: bool, pretend_public: Vec<IpAddr>) -> CheckedResolver {
        CheckedResolver {
            inner: DefaultResolver::default(),
            allow_private,
            pretend_public,
            refused: Arc::default(),
        }
    }
}

impl fmt::Debug for CheckedResolver {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CheckedResolver")
            .field("allow_private", &self.allow_private)
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
            if !self.pretend_public.contains(&ip) && !allowed(ip, self.allow_private) {
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
}

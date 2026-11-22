//! Server addresses as players type them (`multiplayer/resolver/ServerAddress.java`,
//! which parses with Guava's `HostAndPort`):
//!
//! - `host` or `host:port`; a host with several colons and no brackets is an IPv6
//!   address without a port;
//! - `[v6]` or `[v6]:port`;
//! - the port is a decimal number from 0 to 65535 (no `+`); an empty port, as in
//!   `host:`, means the default 25565.
//!
//! Anything else, or an empty host, is invalid; vanilla then uses `server.invalid`.
//! The host goes to the resolver and the handshake as IDNA ASCII; a host that cannot be
//! converted becomes empty.

use std::fmt;

pub const DEFAULT_PORT: u16 = 25565;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ServerAddress {
    /// As typed, without brackets.
    host: String,
    port: u16,
}

impl ServerAddress {
    pub fn new(host: &str, port: u16) -> ServerAddress {
        ServerAddress {
            host: host.to_owned(),
            port,
        }
    }

    /// What vanilla uses for an address it cannot parse.
    pub fn invalid() -> ServerAddress {
        ServerAddress::new("server.invalid", DEFAULT_PORT)
    }

    /// Parses `input`, falling back to [`ServerAddress::invalid`].
    pub fn parse(input: &str) -> ServerAddress {
        match split(input) {
            Ok((host, port)) if !host.is_empty() => ServerAddress {
                host: host.to_owned(),
                port: port.unwrap_or(DEFAULT_PORT),
            },
            _ => ServerAddress::invalid(),
        }
    }

    /// Whether `input` parses with a non-empty host that converts to ASCII (what the
    /// Done and Join buttons check).
    pub fn is_valid(input: &str) -> bool {
        matches!(split(input), Ok((host, _)) if !host.is_empty() && to_ascii(host).is_some())
    }

    /// The host as typed.
    pub fn host(&self) -> &str {
        &self.host
    }

    /// The host for lookups and the handshake: IDNA ASCII, or empty if it has none.
    pub fn host_ascii(&self) -> String {
        to_ascii(&self.host).unwrap_or_default()
    }

    pub fn port(&self) -> u16 {
        self.port
    }
}

impl fmt::Display for ServerAddress {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.host.contains(':') {
            write!(f, "[{}]:{}", self.host, self.port)
        } else {
            write!(f, "{}:{}", self.host, self.port)
        }
    }
}

/// Host and optional port, or why not.
fn split(input: &str) -> Result<(&str, Option<u16>), &'static str> {
    let (host, port) = if let Some(rest) = input.strip_prefix('[') {
        let close = input.rfind(']').ok_or("no closing bracket")?;
        let colon = input.find(':').ok_or("no colon in brackets")?;
        if close < colon {
            return Err("invalid bracketed host");
        }
        let host = &rest[..close - 1];
        let after = &input[close + 1..];
        if after.is_empty() {
            (host, "")
        } else {
            let port = after
                .strip_prefix(':')
                .ok_or("only a colon may follow ']'")?;
            if !port.bytes().all(|b| b.is_ascii_digit()) {
                return Err("port must be numeric");
            }
            (host, port)
        }
    } else {
        match input.find(':') {
            Some(i) if input.rfind(':') == Some(i) => (&input[..i], &input[i + 1..]),
            // No colon, or several: all host (an IPv6 address without a port).
            _ => (input, ""),
        }
    };
    if port.is_empty() {
        return Ok((host, None));
    }
    if port.starts_with('+') || !port.is_ascii() {
        return Err("unparseable port");
    }
    let n: i32 = port.parse().map_err(|_| "unparseable port")?;
    let n = u16::try_from(n).map_err(|_| "port out of range")?;
    Ok((host, Some(n)))
}

/// Java's `IDN.toASCII` without flags, closely enough: ASCII labels pass unchanged (63
/// bytes at most), others become punycode.
fn to_ascii(host: &str) -> Option<String> {
    if host.is_ascii() {
        let ok = host.split('.').all(|label| label.len() <= 63);
        return ok.then(|| host.to_owned());
    }
    idna::domain_to_ascii(host).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(s: &str) -> (String, u16) {
        let a = ServerAddress::parse(s);
        (a.host().to_owned(), a.port())
    }

    #[test]
    fn hosts_and_ports() {
        assert_eq!(parse("mc.hypixel.net"), ("mc.hypixel.net".into(), 25565));
        assert_eq!(parse("localhost:25570"), ("localhost".into(), 25570));
        assert_eq!(parse("localhost:"), ("localhost".into(), 25565));
        assert_eq!(parse("[::1]:25570"), ("::1".into(), 25570));
        assert_eq!(parse("[::1]"), ("::1".into(), 25565));
        assert_eq!(parse("::1"), ("::1".into(), 25565));
        assert_eq!(parse("2001:db8::1"), ("2001:db8::1".into(), 25565));
        assert_eq!(parse("Example.COM:0"), ("Example.COM".into(), 0));
    }

    #[test]
    fn invalid_falls_back() {
        for bad in [
            "",
            ":25565",
            "host:65536",
            "host:-1",
            "host:+5",
            "host:x",
            "[::1]x",
            "[::1",
            "[]:5",
        ] {
            assert_eq!(
                ServerAddress::parse(bad),
                ServerAddress::invalid(),
                "{bad:?}"
            );
        }
        assert!(!ServerAddress::is_valid(""));
        assert!(!ServerAddress::is_valid("host:99999"));
        assert!(ServerAddress::is_valid("play.example.org"));
        assert!(!ServerAddress::is_valid(&"a".repeat(64)));
    }

    #[test]
    fn idna_hosts() {
        let a = ServerAddress::parse("bücher.example:25566");
        assert_eq!(a.host(), "bücher.example");
        assert_eq!(a.host_ascii(), "xn--bcher-kva.example");
        assert_eq!(
            ServerAddress::parse("Mixed.Case").host_ascii(),
            "Mixed.Case"
        );
        assert_eq!(ServerAddress::parse("[::1]:5").to_string(), "[::1]:5");
    }
}

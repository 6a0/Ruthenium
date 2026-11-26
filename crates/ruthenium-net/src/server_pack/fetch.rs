//! The HTTP side of a pack download: vanilla's `HttpURLConnection` GET, on ureq.
//!
//! What vanilla does, and keeps here: a GET with exactly the caller's headers, following
//! redirects as `HttpURLConnection` does (statuses 300 to 307 but 304 and 306, a
//! `Location` read as `java.net.URL` reads it, relative to the URL it came from, and never
//! to another protocol). What we add: certificates checked by the platform's verifier, at
//! most [`Limits::max_redirects`] hops each checked like the first, no proxy (vanilla uses
//! the game's, and we have none; the environment's proxy variables are ignored), no cookies,
//! no `Accept`/`Accept-Encoding`/`User-Agent` of ureq's own, timeouts on every phase and
//! on the whole download, the address policy of [`super::address`], the player's name and
//! UUID only for the host the server named, and error bodies never read.

use std::net::IpAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use ureq::Agent;
use ureq::config::AutoHeaderValue;
use ureq::http::{Response, header};
use ureq::tls::{RootCerts, TlsConfig};
use ureq::unversioned::transport::DefaultConnector;

use super::DownloadError;
use super::address::CheckedResolver;
use super::java_url::{self, JavaUrl, UrlError};

/// Every limit a download runs under. [`Limits::default`] is what the game uses; tests
/// shorten the times.
#[derive(Debug, Clone)]
pub(crate) struct Limits {
    /// Vanilla's `MAX_PACK_SIZE_BYTES` (250 MiB), on the bytes as read.
    pub max_size: u64,
    /// Redirect hops after the first request. `HttpURLConnection` allows 20; pack hosts use
    /// one or two (a CDN, a signed link); five leaves room without letting a server bounce
    /// us around.
    pub max_redirects: u32,
    /// Looking the host up (each hop).
    pub resolve: Duration,
    /// Connecting, with the TLS handshake (each hop).
    pub connect: Duration,
    /// Sending the request.
    pub send: Duration,
    /// From the request sent to the response's headers in.
    pub headers: Duration,
    /// The longest wait for any read: a stalled server is dropped.
    pub per_read: Duration,
    /// The whole download, redirects and body: vanilla has none, so a server trickling a
    /// byte at a time could hold the download forever.
    pub whole: Duration,
    /// Bytes this game run may receive into one cache, all downloads together.
    pub run_budget: u64,
    /// Downloads in one batch; the rest fail at once.
    pub max_requests: usize,
    /// Addresses treated as public (tests: the test server is on loopback).
    pub pretend_public: Vec<IpAddr>,
}

impl Default for Limits {
    fn default() -> Self {
        Limits {
            max_size: super::MAX_PACK_SIZE,
            max_redirects: 5,
            resolve: Duration::from_secs(10),
            connect: Duration::from_secs(15),
            send: Duration::from_secs(15),
            headers: Duration::from_secs(30),
            per_read: Duration::from_secs(30),
            // 250 MiB in 15 minutes needs about 290 KB/s.
            whole: Duration::from_secs(15 * 60),
            // Sixteen packs of the largest size: far beyond what any network sends a player
            // in one sitting, and a bound on what a hostile one can make us write.
            run_budget: 16 * super::MAX_PACK_SIZE,
            // Servers send one pack, a few at most.
            max_requests: 32,
            pretend_public: Vec::new(),
        }
    }
}

/// Request headers only the host the server named gets: the player's identity.
const IDENTITY_HEADERS: [&str; 2] = ["X-Minecraft-Username", "X-Minecraft-UUID"];

/// An agent for one batch of downloads.
pub(crate) struct Client {
    agent: Agent,
    refused: Arc<Mutex<Option<IpAddr>>>,
    max_redirects: u32,
}

impl std::fmt::Debug for Client {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Client").finish_non_exhaustive()
    }
}

impl Client {
    pub fn new(limits: &Limits, allow_private: bool) -> Client {
        let tls = TlsConfig::builder()
            .root_certs(RootCerts::PlatformVerifier)
            .build();
        let config = Agent::config_builder()
            .http_status_as_error(false)
            .https_only(false)
            .proxy(None)
            .max_redirects(0)
            .user_agent(AutoHeaderValue::None)
            .accept(AutoHeaderValue::None)
            .accept_encoding(AutoHeaderValue::None)
            .max_response_header_size(64 * 1024)
            .timeout_resolve(Some(limits.resolve))
            .timeout_connect(Some(limits.connect))
            .timeout_send_request(Some(limits.send))
            .timeout_recv_response(Some(limits.headers))
            .timeout_per_read(Some(limits.per_read))
            .tls_config(tls)
            .build();
        let resolver = CheckedResolver::new(allow_private, limits.pretend_public.clone());
        let refused = resolver.refused.clone();
        Client {
            agent: Agent::with_parts(config, DefaultConnector::default(), resolver),
            refused,
            max_redirects: limits.max_redirects,
        }
    }

    /// GETs the URL and follows its redirects, until a response with the file (2xx).
    pub fn get(
        &self,
        url: &JavaUrl,
        headers: &[(String, String)],
        deadline: Instant,
    ) -> Result<Response<ureq::Body>, DownloadError> {
        let first = Target::new(url)?;
        let mut url = url.clone();
        let mut target = first.clone();
        let mut hops = 0;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(DownloadError::TimedOut);
            }
            let same_origin = target.origin() == first.origin();
            let mut request = self.agent.get(&target.uri);
            for (name, value) in headers {
                if !same_origin
                    && IDENTITY_HEADERS
                        .iter()
                        .any(|h| h.eq_ignore_ascii_case(name))
                {
                    continue;
                }
                request = request.header(name, value);
            }
            if let Ok(mut r) = self.refused.lock() {
                *r = None;
            }
            let response = request
                .config()
                .timeout_global(Some(remaining))
                .build()
                .call()
                .map_err(|e| self.error(e))?;
            let status = response.status().as_u16();
            match status {
                // A gzip body we didn't ask for comes decoded (ureq's doing; vanilla would
                // keep the encoded bytes): the size limit counts the decoded bytes, so a
                // small body inflating to gigabytes still stops at the limit.
                200..=299 => return Ok(response),
                305 => return Err(DownloadError::BadRedirect("a proxy redirect (305)")),
                300..=307 if status != 304 && status != 306 => {
                    let Some(location) = response.headers().get(header::LOCATION) else {
                        return Err(DownloadError::BadRedirect("a redirect with no Location"));
                    };
                    let Ok(location) = location.to_str() else {
                        return Err(DownloadError::BadRedirect("a Location that isn't text"));
                    };
                    let next = resolve_location(&url, location)?;
                    hops += 1;
                    if hops > self.max_redirects {
                        return Err(DownloadError::TooManyRedirects);
                    }
                    target = Target::new(&next)?;
                    url = next;
                    // The response is dropped unread.
                }
                _ => return Err(DownloadError::Status(status)),
            }
        }
    }

    fn error(&self, e: ureq::Error) -> DownloadError {
        if let Some(ip) = self.refused.lock().ok().and_then(|r| *r) {
            return DownloadError::RefusedAddress(ip);
        }
        network_error(e)
    }
}

pub(crate) fn network_error(e: ureq::Error) -> DownloadError {
    match e {
        ureq::Error::Timeout(_) => DownloadError::TimedOut,
        e => DownloadError::Network(e.to_string()),
    }
}

/// What a body read failing means.
pub(crate) fn read_error(e: &std::io::Error) -> DownloadError {
    if e.kind() == std::io::ErrorKind::TimedOut
        || e.get_ref()
            .and_then(|i| i.downcast_ref::<ureq::Error>())
            .is_some_and(|u| matches!(u, ureq::Error::Timeout(_)))
    {
        DownloadError::TimedOut
    } else {
        DownloadError::Network(e.to_string())
    }
}

/// `HttpURLConnection.followRedirect`'s reading of a `Location`: an absolute URL of the
/// same protocol, or else one relative to where it came from.
pub(crate) fn resolve_location(base: &JavaUrl, location: &str) -> Result<JavaUrl, DownloadError> {
    match java_url::parse(None, location) {
        Ok(u) if u.https == base.https => Ok(u),
        Ok(_) | Err(UrlError::OtherProtocol) => {
            Err(DownloadError::BadRedirect("a redirect to another protocol"))
        }
        Err(UrlError::Malformed) => java_url::parse(Some(base), location)
            .map_err(|_| DownloadError::BadRedirect("a Location that isn't a URL")),
    }
}

/// What we ask for: a URL vanilla would try, made into one we will.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Target {
    https: bool,
    /// Lower-case name, or an address (IPv6 in brackets).
    host: String,
    port: u16,
    pub uri: String,
}

impl Target {
    pub fn new(url: &JavaUrl) -> Result<Target, DownloadError> {
        let unusable = DownloadError::UnusableUrl;
        let host = if let Some(inner) = url.host.strip_prefix('[').and_then(|h| h.strip_suffix(']'))
        {
            if inner.contains('%') {
                return Err(unusable("an IPv6 address with a scope"));
            }
            let chars: Vec<char> = inner.chars().collect();
            match java_url::parse_ipv6(&chars) {
                Some(IpAddr::V6(v6)) => format!("[{v6}]"),
                Some(IpAddr::V4(v4)) => v4.to_string(),
                None => return Err(unusable("a malformed IPv6 address")),
            }
        } else {
            host_name(&url.host)?
        };
        let default_port = if url.https { 443 } else { 80 };
        let port = match url.port {
            -1 => default_port,
            p => u16::try_from(p)
                .ok()
                .filter(|&p| p != 0)
                .ok_or(unusable("a port out of range"))?,
        };
        let mut path_and_query = String::new();
        if !url.path.starts_with('/') {
            path_and_query.push('/');
        }
        encode_into(&url.path, &mut path_and_query)?;
        if let Some(q) = &url.query {
            path_and_query.push('?');
            encode_into(q, &mut path_and_query)?;
        }
        let scheme = url.protocol();
        let uri = if port == default_port {
            format!("{scheme}://{host}{path_and_query}")
        } else {
            format!("{scheme}://{host}:{port}{path_and_query}")
        };
        // ureq must read it as the same URL.
        if uri.parse::<ureq::http::Uri>().is_err() {
            return Err(unusable("a URL ureq can't read"));
        }
        Ok(Target {
            https: url.https,
            host,
            port,
            uri,
        })
    }

    fn origin(&self) -> (bool, &str, u16) {
        (self.https, &self.host, self.port)
    }
}

/// A host name we will look up: ASCII letters, digits, `-`, `_` and dots, at most 253
/// characters; one ending in a number must be a plain dotted IPv4 address (`127.1`,
/// `0x7f.1` and `2130706433` are refused: one spelling per address).
fn host_name(host: &str) -> Result<String, DownloadError> {
    let unusable = DownloadError::UnusableUrl;
    if host.is_empty() {
        return Err(unusable("no host"));
    }
    if host.len() > 253 {
        return Err(unusable("a host name too long"));
    }
    if !host
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
    {
        return Err(unusable("a host name with characters DNS doesn't have"));
    }
    let trimmed = host.strip_suffix('.').unwrap_or(host);
    if trimmed.is_empty() || trimmed.split('.').any(str::is_empty) {
        return Err(unusable("a host name with an empty label"));
    }
    let last = trimmed.rsplit('.').next().unwrap_or(trimmed);
    if last.starts_with(|c: char| c.is_ascii_digit()) {
        return match trimmed.parse::<std::net::Ipv4Addr>() {
            Ok(v4) => Ok(v4.to_string()),
            Err(_) => Err(unusable("a numeric host that isn't a dotted IPv4 address")),
        };
    }
    Ok(host.to_ascii_lowercase())
}

/// Percent-encodes what a request line can't hold as it is (spaces, non-ASCII as UTF-8,
/// quotes, brackets...); existing escapes stay. Control characters are refused: vanilla
/// sends them raw, which is a broken request at best.
fn encode_into(s: &str, out: &mut String) -> Result<(), DownloadError> {
    for c in s.chars() {
        if c.is_ascii_control() {
            return Err(DownloadError::UnusableUrl(
                "a control character in the path",
            ));
        }
        if c.is_ascii_alphanumeric() || "-._~!$&'()*+,;=:@/?%".contains(c) {
            out.push(c);
        } else {
            let mut buf = [0u8; 4];
            for b in c.encode_utf8(&mut buf).bytes() {
                out.push_str(&format!("%{b:02X}"));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target(url: &str) -> Result<Target, DownloadError> {
        Target::new(&java_url::parse(None, url).unwrap())
    }

    #[test]
    fn urls_vanilla_accepts_become_requests_or_are_refused() {
        let ok = |url: &str, uri: &str| assert_eq!(target(url).unwrap().uri, uri, "{url}");
        ok(
            "https://Example.COM/pack.zip",
            "https://example.com/pack.zip",
        );
        ok("https://example.com:443/p", "https://example.com/p");
        ok(
            "http://example.com:8080/p?x=1#f",
            "http://example.com:8080/p?x=1",
        );
        ok("https://example.com", "https://example.com/");
        ok("https://example.com?x", "https://example.com/?x");
        ok(
            "https://example.com/a b/\u{e9}",
            "https://example.com/a%20b/%C3%A9",
        );
        ok("https://example.com/a%20b", "https://example.com/a%20b");
        ok(
            "https://example.com/\"<>\\^\u{60}{|}",
            "https://example.com/%22%3C%3E%5C%5E%60%7B%7C%7D",
        );
        ok("https://user:pw@example.com/p", "https://example.com/p");
        ok("https://[::1]:8443/p", "https://[::1]:8443/p");
        ok("https://[::ffff:1.2.3.4]/p", "https://1.2.3.4/p");
        ok("https://1.2.3.4/p", "https://1.2.3.4/p");
        ok("https://example.com./p", "https://example.com./p");
        ok("https://example.com:-1/p", "https://example.com/p");

        let refused = |url: &str| assert!(target(url).is_err(), "{url}");
        refused("http:///p");
        refused("http:p");
        refused("https://user@pass@example.com/");
        refused("https://:80/");
        refused("https://example.com:0/");
        refused("https://example.com:65536/");
        refused("https://example.com:-0/");
        refused("https://ex\u{e4}mple.com/");
        refused("https://exa!mple.com/");
        refused("https://exa%41mple.com/");
        refused("https://a..b/");
        refused("https://.a/");
        refused("http://127.1/");
        refused("http://2130706433/");
        refused("http://0x7f.0.0.1/");
        refused("http://017.0.0.1/");
        refused("https://[fe80::1%eth0]/");
        refused("https://example.com/a\nb");
        refused("https://example.com/a\rb");
        refused("https://example.com/a\tb");
        refused(&format!("https://{}.com/", "a".repeat(260)));
    }

    #[test]
    fn locations_resolve_like_java() {
        let base = java_url::parse(None, "https://example.com/a/b?q").unwrap();
        let to = |loc: &str| resolve_location(&base, loc).map(|u| Target::new(&u).unwrap().uri);
        assert_eq!(to("c").unwrap(), "https://example.com/a/c");
        assert_eq!(
            to("//cdn.example.net/x").unwrap(),
            "https://cdn.example.net/x"
        );
        assert!(to("http://example.com/x").is_err());
        assert!(to("ftp://example.com/x").is_err());
        assert!(to("javascript:alert(1)").is_err());
        assert!(to("file:///etc/passwd").is_err());
    }
}

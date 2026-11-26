//! A port of how `java.net.URL` reads a URL (OpenJDK 21 and 25: the `URL(URL, String)`
//! constructor, `URLStreamHandler.parseURL` and `setURL`, and `IPAddressUtil`'s host,
//! user-info and authority checks with early URL parsing, the default), for http and https.
//!
//! Vanilla reads a pushed pack's URL with `new URL(string)` and keeps it only for http or
//! https (`ClientCommonPacketListenerImpl.parseResourcePackUrl`); `HttpURLConnection` reads
//! each redirect's `Location` the same way, relative to the URL it came from when it isn't
//! absolute. Java is lenient (no host, any port up to 2^31, spaces and non-ASCII in the
//! path, ports in any script's digits), so what this accepts is only what vanilla would try:
//! [`super::fetch`] then decides which of those we will actually ask for.
//!
//! Strings are walked as `char`s where Java walks UTF-16 units: every test here is on ASCII
//! or on single BMP characters, so the two agree. One known gap: whether a non-ASCII
//! character may stand in a protocol name (`Character.isLetter`) is approximated by
//! `char::is_alphabetic`; that only decides how a redirect to some other protocol fails.

/// A URL as `java.net.URL` holds it, for http or https.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct JavaUrl {
    /// `https` rather than `http`.
    pub https: bool,
    pub authority: Option<String>,
    pub user_info: Option<String>,
    /// May be empty, or an IPv6 literal in brackets.
    pub host: String,
    /// -1 when none was given (Java's value; anything up to `i32::MAX` parses).
    pub port: i32,
    pub path: String,
    pub query: Option<String>,
}

impl JavaUrl {
    pub fn protocol(&self) -> &'static str {
        if self.https { "https" } else { "http" }
    }

    /// `URL.getFile`: the path and the query.
    #[cfg(test)]
    pub fn file(&self) -> String {
        match &self.query {
            Some(q) => format!("{}?{q}", self.path),
            None => self.path.clone(),
        }
    }
}

/// Why a string isn't an http or https URL.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum UrlError {
    /// No protocol, or one Java would reject (`MalformedURLException`).
    Malformed,
    /// A well-formed protocol name other than http and https (Java may have a handler for
    /// it, as for `ftp` or `file`, but vanilla only takes http and https).
    OtherProtocol,
}

/// `new URL(context, spec)`, keeping only http and https.
pub(crate) fn parse(context: Option<&JavaUrl>, spec: &str) -> Result<JavaUrl, UrlError> {
    let s: Vec<char> = spec.chars().collect();
    let mut limit = s.len();
    while limit > 0 && s[limit - 1] <= ' ' {
        limit -= 1;
    }
    let mut start = 0;
    while start < limit && s[start] <= ' ' {
        start += 1;
    }
    if region_matches_ignore_case(&s, start, "url:") {
        start += 4;
    }
    let a_ref = start < s.len() && s[start] == '#';
    let mut new_protocol: Option<String> = None;
    if !a_ref {
        let mut i = start;
        while i < limit && s[i] != '/' {
            if s[i] == ':' {
                let p: String = s[start..i].iter().collect::<String>().to_lowercase();
                if is_valid_protocol(&p) {
                    new_protocol = Some(p);
                    start = i + 1;
                }
                break;
            }
            i += 1;
        }
    }

    // The fields a relative URL inherits.
    let mut url = Fields::default();
    let mut protocol = new_protocol.clone();
    let mut is_relative = false;
    if let Some(ctx) = context
        && new_protocol
            .as_deref()
            .is_none_or(|p| p.eq_ignore_ascii_case(ctx.protocol()))
    {
        // A hierarchical context with a matching protocol: "http:foo" is a relative "foo".
        if ctx.path.starts_with('/') {
            new_protocol = None;
        }
        if new_protocol.is_none() {
            protocol = Some(ctx.protocol().to_owned());
            url.authority = ctx.authority.clone();
            url.user_info = ctx.user_info.clone();
            url.host = Some(ctx.host.clone());
            url.port = ctx.port;
            url.path = Some(ctx.path.clone());
            is_relative = true;
        }
    }
    let https = match protocol.as_deref() {
        None => return Err(UrlError::Malformed),
        Some("http") => false,
        Some("https") => true,
        Some(_) => return Err(UrlError::OtherProtocol),
    };

    if let Some(i) = index_of(&s, '#', start, s.len()) {
        limit = i;
    }
    if is_relative
        && start == limit
        && let Some(ctx) = context
    {
        url.query = ctx.query.clone();
    }
    parse_url(&mut url, &s, start, limit)?;
    let host = url.host.unwrap_or_default();

    // setURL's checks.
    if let Some(e) = check_host(&host) {
        return Err(e);
    }
    if let Some(u) = &url.user_info
        && scan(u, USERINFO_MASK, &[]).is_some()
    {
        return Err(UrlError::Malformed);
    }
    if let Some(a) = &url.authority
        && scan(a, AUTH_MASK, &[]).is_some()
    {
        return Err(UrlError::Malformed);
    }
    Ok(JavaUrl {
        https,
        authority: url.authority,
        user_info: url.user_info,
        host,
        port: url.port,
        path: url.path.unwrap_or_default(),
        query: url.query,
    })
}

#[derive(Debug)]
struct Fields {
    authority: Option<String>,
    user_info: Option<String>,
    host: Option<String>,
    port: i32,
    path: Option<String>,
    query: Option<String>,
}

impl Default for Fields {
    fn default() -> Self {
        Fields {
            authority: None,
            user_info: None,
            host: None,
            port: -1,
            path: None,
            query: None,
        }
    }
}

/// `URLStreamHandler.parseURL`.
fn parse_url(
    u: &mut Fields,
    s: &[char],
    mut start: usize,
    mut limit: usize,
) -> Result<(), UrlError> {
    // The spec may be cut at its query: then it ends there.
    let mut spec_len = s.len();
    let mut query_only = false;
    if start < limit {
        let query_start = index_of(s, '?', 0, spec_len);
        query_only = query_start == Some(start);
        if let Some(q) = query_start
            && q < limit
        {
            u.query = Some(s[q + 1..limit].iter().collect());
            limit = q;
            spec_len = q;
        }
    }
    let spec = &s[..spec_len];

    let is_unc = start + 4 <= limit && spec[start..start + 4] == ['/'; 4];
    if !is_unc && start + 2 <= limit && spec[start] == '/' && spec[start + 1] == '/' {
        start += 2;
        let i = match index_of(spec, '/', start, spec_len).filter(|&i| i <= limit) {
            Some(i) => i,
            None => index_of(spec, '?', start, spec_len)
                .filter(|&i| i <= limit)
                .unwrap_or(limit),
        };
        let authority: Vec<char> = spec[start..i].to_vec();
        let mut host: Option<Vec<char>> = Some(authority.clone());
        let at = authority.iter().position(|&c| c == '@');
        if let Some(at) = at {
            if Some(at) != authority.iter().rposition(|&c| c == '@') {
                // More than one '@': not server based.
                u.user_info = None;
                host = None;
            } else {
                u.user_info = Some(authority[..at].iter().collect());
                host = Some(authority[at + 1..].to_vec());
            }
        } else {
            u.user_info = None;
        }
        match host {
            Some(h) => {
                if !h.is_empty() && h[0] == '[' {
                    match h.iter().position(|&c| c == ']') {
                        Some(ind) if ind > 2 => {
                            if parse_ipv6(&h[1..ind]).is_none() {
                                return Err(UrlError::Malformed);
                            }
                            u.port = -1;
                            if h.len() > ind + 1 {
                                if h[ind + 1] != ':' {
                                    return Err(UrlError::Malformed);
                                }
                                if h.len() > ind + 2 {
                                    u.port = parse_int(&h[ind + 2..])?;
                                }
                            }
                            u.host = Some(h[..=ind].iter().collect());
                        }
                        _ => return Err(UrlError::Malformed),
                    }
                } else {
                    u.port = -1;
                    match h.iter().position(|&c| c == ':') {
                        Some(ind) => {
                            if h.len() > ind + 1 {
                                u.port = parse_int(&h[ind + 1..])?;
                            }
                            u.host = Some(h[..ind].iter().collect());
                        }
                        None => u.host = Some(h.iter().collect()),
                    }
                }
            }
            None => u.host = Some(String::new()),
        }
        if u.port < -1 {
            return Err(UrlError::Malformed);
        }
        start = i;
        // With an authority, the path comes from the spec alone.
        if !authority.is_empty() {
            u.path = Some(String::new());
        }
        u.authority = Some(authority.iter().collect());
    }
    if u.host.is_none() {
        u.host = Some(String::new());
    }

    let mut is_rel_path = false;
    if start < limit {
        let spec_str: String = spec[start..limit].iter().collect();
        if spec[start] == '/' {
            u.path = Some(spec_str);
        } else if let Some(path) = u.path.as_ref().filter(|p| !p.is_empty()) {
            is_rel_path = true;
            u.path = Some(match path.rfind('/') {
                None if u.authority.is_some() => format!("/{spec_str}"),
                None => spec_str,
                Some(ind) => format!("{}{spec_str}", &path[..=ind]),
            });
        } else {
            u.path = Some(if u.authority.is_some() {
                format!("/{spec_str}")
            } else {
                spec_str
            });
        }
    } else if query_only && let Some(path) = &u.path {
        u.path = Some(match path.rfind('/') {
            None => "/".to_owned(),
            Some(ind) => path[..=ind].to_owned(),
        });
    }
    let mut path = u.path.take().unwrap_or_default();
    if is_rel_path {
        remove_dot_segments(&mut path);
    }
    u.path = Some(path);
    Ok(())
}

/// parseURL's clean-up of a relative path, as Java does it (not RFC 3986's).
fn remove_dot_segments(path: &mut String) {
    while let Some(i) = path.find("/./") {
        path.replace_range(i..i + 2, "");
    }
    let mut i = 0;
    while let Some(found) = path.get(i..).and_then(|p| p.find("/../")).map(|f| f + i) {
        let prev = if found > 0 {
            path[..found].rfind('/')
        } else {
            None
        };
        match prev {
            Some(limit) if path[limit..].find("/../").map(|f| f + limit) != Some(0) => {
                path.replace_range(limit..found + 3, "");
                i = 0;
            }
            _ => i = found + 3,
        }
    }
    while path.ends_with("/..") {
        let i = path.find("/..").unwrap_or(0);
        match path[..i].rfind('/') {
            Some(limit) => path.truncate(limit + 1),
            None => break,
        }
    }
    if path.starts_with("./") && path.len() > 2 {
        path.replace_range(..2, "");
    }
    if path.ends_with("/.") {
        path.pop();
    }
}

fn index_of(s: &[char], c: char, from: usize, len: usize) -> Option<usize> {
    s.get(from..len)?
        .iter()
        .position(|&x| x == c)
        .map(|p| p + from)
}

fn region_matches_ignore_case(s: &[char], start: usize, what: &str) -> bool {
    let n = what.chars().count();
    start + n <= s.len()
        && s[start..start + n]
            .iter()
            .zip(what.chars())
            .all(|(a, b)| a.eq_ignore_ascii_case(&b))
}

/// `URL.isValidProtocol` (on the lower-cased name).
fn is_valid_protocol(p: &str) -> bool {
    let mut chars = p.chars();
    chars.next().is_some_and(char::is_alphabetic)
        && chars.all(|c| c.is_alphanumeric() || matches!(c, '.' | '+' | '-'))
}

/// The BMP characters whose `Character.digit(c, 10)` is 0, from Java 25 itself: every
/// decimal digit is one of these plus 0 to 9.
const DIGIT_ZEROS: [u32; 37] = [
    0x0030, 0x0660, 0x06f0, 0x07c0, 0x0966, 0x09e6, 0x0a66, 0x0ae6, 0x0b66, 0x0be6, 0x0c66, 0x0ce6,
    0x0d66, 0x0de6, 0x0e50, 0x0ed0, 0x0f20, 0x1040, 0x1090, 0x17e0, 0x1810, 0x1946, 0x19d0, 0x1a80,
    0x1a90, 0x1b50, 0x1bb0, 0x1c40, 0x1c50, 0xa620, 0xa8d0, 0xa900, 0xa9d0, 0xa9f0, 0xaa50, 0xabf0,
    0xff10,
];

/// `Character.digit(c, 10)` for a UTF-16 unit.
fn java_digit(c: char) -> Option<i64> {
    let c = c as u32;
    DIGIT_ZEROS
        .iter()
        .find(|&&z| (z..z + 10).contains(&c))
        .map(|&z| i64::from(c - z))
}

/// `Integer.parseInt(s, 10)`: an optional sign and digits of any script, within i32.
fn parse_int(s: &[char]) -> Result<i32, UrlError> {
    let (negative, digits) = match s.first() {
        Some('-') => (true, &s[1..]),
        Some('+') => (false, &s[1..]),
        _ => (false, s),
    };
    if digits.is_empty() {
        return Err(UrlError::Malformed);
    }
    let mut value: i64 = 0;
    for &c in digits {
        value = value * 10 + java_digit(c).ok_or(UrlError::Malformed)?;
        if value > 1 << 31 {
            return Err(UrlError::Malformed);
        }
    }
    let value = if negative { -value } else { value };
    i32::try_from(value).map_err(|_| UrlError::Malformed)
}

// IPAddressUtil's character masks: bit n of LOW is character n, bit n of HIGH is 64 + n.
const L_IPV6_DELIMS: u64 = 0x0;
const H_IPV6_DELIMS: u64 = 0x2800_0000;
const L_AUTH_DELIMS: u64 = 0x0400_0000_0000_0000;
const H_AUTH_DELIMS: u64 = 0x2800_0001;
const L_COLON: u64 = 0x0400_0000_0000_0000;
const H_COLON: u64 = 0x0;
const L_SLASH: u64 = 0x8000_0000_0000;
const H_SLASH: u64 = 0x0;
const L_BACKSLASH: u64 = 0x0;
const H_BACKSLASH: u64 = 0x1000_0000;
const L_NON_PRINTABLE: u64 = 0xffff_ffff;
const H_NON_PRINTABLE: u64 = 0x8000_0000_0000_0000;
const L_EXCLUDE: u64 = 0x8400_8008_ffff_ffff;
const H_EXCLUDE: u64 = 0x8000_0000_3800_0001;
const L_EXCLUDED_DELIMS: u64 = 0x5000_0005_0000_0000;
const H_EXCLUDED_DELIMS: u64 = 0x0;
const L_UNWISE: u64 = 0x0;
const H_UNWISE: u64 = 0x3800_0001_7800_0000;
const L_FRAGMENT: u64 = 0x0000_0008_0000_0000;
const H_FRAGMENT: u64 = 0x0;
const L_QUERY: u64 = 0x8000_0000_0000_0000;
const H_QUERY: u64 = 0x0;

type Mask = (u64, u64);
const HOSTNAME_MASK: Mask = (
    L_EXCLUDE | L_NON_PRINTABLE | L_SLASH | L_UNWISE | L_EXCLUDED_DELIMS,
    H_EXCLUDE | H_NON_PRINTABLE | H_SLASH | H_UNWISE | H_EXCLUDED_DELIMS,
);
const SCOPE_MASK: Mask = (
    L_NON_PRINTABLE | L_IPV6_DELIMS | L_SLASH | L_BACKSLASH | L_FRAGMENT | L_QUERY,
    H_NON_PRINTABLE | H_IPV6_DELIMS | H_SLASH | H_BACKSLASH | H_FRAGMENT | H_QUERY,
);
const USERINFO_MASK: Mask = (L_EXCLUDE & !L_COLON, H_EXCLUDE & !H_COLON);
const AUTH_MASK: Mask = (L_EXCLUDE & !L_AUTH_DELIMS, H_EXCLUDE & !H_AUTH_DELIMS);
/// Characters that look like delimiters (IPAddressUtil.OTHERS), refused in host names.
const OTHERS: [u32; 17] = [
    8263, 8264, 8265, 8448, 8449, 8453, 8454, 10868, 65109, 65110, 65119, 65131, 65283, 65295,
    65306, 65311, 65312,
];

fn matches(c: char, (low, high): Mask) -> bool {
    let c = c as u32;
    match c {
        0..64 => low >> c & 1 != 0,
        64..128 => high >> (c - 64) & 1 != 0,
        _ => false,
    }
}

/// The first character in the mask or in `others`.
fn scan(s: &str, mask: Mask, others: &[u32]) -> Option<usize> {
    s.chars()
        .position(|c| matches(c, mask) || others.contains(&(c as u32)))
}

/// `IPAddressUtil.checkHost`: an error when the host has a character Java refuses.
fn check_host(host: &str) -> Option<UrlError> {
    if let Some(inner) = host.strip_prefix('[').and_then(|h| h.strip_suffix(']')) {
        let chars: Vec<char> = inner.chars().collect();
        if parse_ipv6(&chars).is_none() {
            return Some(UrlError::Malformed);
        }
        if let Some(i) = inner.find('%')
            && scan(&inner[i..], SCOPE_MASK, &[]).is_some()
        {
            return Some(UrlError::Malformed);
        }
        return None;
    }
    scan(host, HOSTNAME_MASK, &OTHERS).map(|_| UrlError::Malformed)
}

/// `IPAddressUtil.textToNumericFormatV4`: one to four decimal parts, the last filling the
/// rest of the address (`127.1` is 127.0.0.1).
pub(crate) fn parse_ipv4(s: &[char]) -> Option<[u8; 4]> {
    if s.is_empty() || s.len() > 15 {
        return None;
    }
    let mut res = [0u8; 4];
    let mut value: i64 = 0;
    let mut byte = 0;
    let mut new_octet = true;
    for &c in s {
        if c == '.' {
            if new_octet || value > 0xff || byte == 3 {
                return None;
            }
            res[byte] = value as u8;
            byte += 1;
            value = 0;
            new_octet = true;
        } else {
            value = value * 10 + i64::from(c.to_digit(10)?);
            new_octet = false;
        }
    }
    if new_octet || value >= 1 << ((4 - byte) * 8) {
        return None;
    }
    let rest = (value as u32).to_be_bytes();
    res[byte..].copy_from_slice(&rest[byte..]);
    Some(res)
}

/// `IPAddressUtil.textToNumericFormatV6` (anything after a `%` is a scope, ignored):
/// 16 bytes, or 4 for an IPv4-mapped address.
pub(crate) fn parse_ipv6(s: &[char]) -> Option<std::net::IpAddr> {
    if s.len() < 2 {
        return None;
    }
    let mut len = s.len();
    match s.iter().position(|&c| c == '%') {
        Some(pc) if pc == s.len() - 1 => return None,
        Some(pc) => len = pc,
        None => {}
    }
    let mut dst = [0u8; 16];
    let mut colonp: Option<usize> = None;
    let mut i = 0;
    let mut j = 0;
    if s[i] == ':' {
        i += 1;
        if s[i] != ':' {
            return None;
        }
    }
    let mut curtok = i;
    let mut saw_xdigit = false;
    let mut val: u32 = 0;
    while i < len {
        let ch = s[i];
        i += 1;
        if let Some(d) = ch.to_digit(16) {
            val = (val << 4) | d;
            if val > 0xffff {
                return None;
            }
            saw_xdigit = true;
            continue;
        }
        if ch == ':' {
            curtok = i;
            if !saw_xdigit {
                if colonp.is_some() {
                    return None;
                }
                colonp = Some(j);
                continue;
            } else if i == len {
                return None;
            }
            if j + 2 > 16 {
                return None;
            }
            dst[j] = (val >> 8) as u8;
            dst[j + 1] = val as u8;
            j += 2;
            saw_xdigit = false;
            val = 0;
            continue;
        }
        if ch == '.' && j + 4 <= 16 {
            let ia4 = &s[curtok..len];
            if ia4.iter().filter(|&&c| c == '.').count() != 3 {
                return None;
            }
            let v4 = parse_ipv4(ia4)?;
            dst[j..j + 4].copy_from_slice(&v4);
            j += 4;
            saw_xdigit = false;
            break;
        }
        return None;
    }
    if saw_xdigit {
        if j + 2 > 16 {
            return None;
        }
        dst[j] = (val >> 8) as u8;
        dst[j + 1] = val as u8;
        j += 2;
    }
    if let Some(colonp) = colonp {
        let n = j - colonp;
        if j == 16 {
            return None;
        }
        for k in 1..=n {
            dst[16 - k] = dst[colonp + n - k];
            dst[colonp + n - k] = 0;
        }
        j = 16;
    }
    if j != 16 {
        return None;
    }
    let v6 = std::net::Ipv6Addr::from(dst);
    Some(match v6.to_ipv4_mapped() {
        Some(v4) => v4.into(),
        None => v6.into(),
    })
}

#[cfg(test)]
#[path = "url_golden.rs"]
mod golden;

#[cfg(test)]
mod tests {
    use super::golden;
    use super::*;

    fn describe(r: Result<JavaUrl, UrlError>) -> String {
        match r {
            Ok(u) => format!("{}|{}|{}|{}", u.protocol(), u.host, u.port, u.file()),
            Err(_) => "!".to_owned(),
        }
    }

    /// `HttpURLConnection.followRedirect`'s reading of a Location (see [`super::super::fetch`]).
    fn redirect(base: &str, loc: &str) -> String {
        let Ok(base) = parse(None, base) else {
            return "!".to_owned();
        };
        describe(super::super::fetch::resolve_location(&base, loc).map_err(|_| UrlError::Malformed))
    }

    fn vanilla(expected: &str) -> String {
        if expected.starts_with('!') {
            "!".to_owned()
        } else {
            expected.to_owned()
        }
    }

    #[test]
    fn pack_urls_parse_as_java_does() {
        let mut wrong = Vec::new();
        for &(url, expected) in golden::PACK_URLS {
            let ours = describe(parse(None, url));
            if ours != vanilla(expected) {
                wrong.push(format!("{url:?}: vanilla {expected:?}, ours {ours:?}"));
            }
        }
        assert!(wrong.is_empty(), "{}", wrong.join("\n"));
    }

    #[test]
    fn redirects_resolve_as_java_does() {
        let mut wrong = Vec::new();
        for &(base, loc, expected) in golden::REDIRECTS {
            let ours = redirect(base, loc);
            if ours != vanilla(expected) {
                wrong.push(format!(
                    "{base:?} + {loc:?}: vanilla {expected:?}, ours {ours:?}"
                ));
            }
        }
        assert!(wrong.is_empty(), "{}", wrong.join("\n"));
    }

    /// The generator the golden tool shares: a 64-bit LCG over made-up pieces of URLs.
    struct Gen(u64);

    impl Gen {
        fn next(&mut self, n: usize) -> usize {
            self.0 = self
                .0
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            ((self.0 >> 33) % n as u64) as usize
        }

        fn url(&mut self) -> String {
            let mut s = STARTS[self.next(STARTS.len())].to_owned();
            let n = self.next(9);
            for _ in 0..n {
                s.push_str(TOKENS[self.next(TOKENS.len())]);
            }
            s
        }
    }

    const STARTS: &[&str] = &[
        "http://",
        "https://",
        "HTTP://",
        "hTtPs:",
        "http:",
        "url:http://",
        " \t http://",
        "",
        "//",
        "ftp://",
        "http:/",
        "http:///",
        "http:////",
        "#http://",
    ];
    const TOKENS: &[&str] = &[
        "a",
        "b.c",
        "example.com",
        "127.0.0.1",
        "::1",
        "[::1]",
        "[::ffff:1.2.3.4]",
        "[fe80::1%eth0]",
        "[",
        "]",
        ":",
        ":80",
        ":8080",
        ":-1",
        ":+5",
        ":65536",
        ":0",
        ":99999999999",
        ":\u{663}",
        ":\u{ff18}\u{ff10}",
        "/",
        "//",
        "?",
        "#",
        "@",
        "@@",
        "user:pw@",
        "%",
        "%41",
        " ",
        "\t",
        "\n",
        "\r",
        "\0",
        "\\",
        ".",
        "..",
        "./",
        "../",
        "/./",
        "/../",
        "~",
        "!",
        "$",
        "<",
        ">",
        "\"",
        "{",
        "|",
        "^",
        "\u{60}",
        "\u{e9}",
        "\u{2047}",
        "\u{2100}",
        "\u{ff03}",
        "\u{130}",
        "\u{212a}",
        "url:",
        "x",
        "-",
        "_",
        "+",
        "=",
        "&",
        ";",
        ",",
        "*",
        "'",
        "(",
        ")",
    ];
    const BASES: &[&str] = &[
        "http://example.com/a/b/c?q=1#f",
        "https://h:8443/dir/",
        "http://h",
        "http://u@h/x/y/../z",
        "https://[::1]/p?x",
    ];

    fn fnv(mut h: u64, s: &str) -> u64 {
        for b in s.bytes().chain(std::iter::once(b'\n')) {
            h ^= u64::from(b);
            h = h.wrapping_mul(0x100000001b3);
        }
        h
    }

    #[test]
    fn generated_urls_parse_as_java_does() {
        let mut g = Gen(0x5eed0f5e7e5);
        let mut wrong = Vec::new();
        for (batch, &expected) in golden::GENERATED.iter().enumerate() {
            let mut h = 0xcbf29ce484222325;
            for _ in 0..golden::GENERATED_BATCH {
                let url = g.url();
                let base = BASES[g.next(BASES.len())];
                h = fnv(h, &describe(parse(None, &url)));
                h = fnv(h, &redirect(base, &url));
            }
            if h != expected {
                wrong.push(batch);
            }
        }
        assert!(
            wrong.is_empty(),
            "batches {wrong:?} differ (the tool prints any case)"
        );
    }

    #[test]
    fn ipv4_literals_as_java_reads_them() {
        let p = |s: &str| parse_ipv4(&s.chars().collect::<Vec<_>>());
        assert_eq!(p("127.0.0.1"), Some([127, 0, 0, 1]));
        assert_eq!(p("127.1"), Some([127, 0, 0, 1]));
        assert_eq!(p("10.1.2"), Some([10, 1, 0, 2]));
        assert_eq!(p("2130706433"), Some([127, 0, 0, 1]));
        assert_eq!(p("4294967296"), None);
        assert_eq!(p("256.0.0.1"), None);
        assert_eq!(p("1.2.3.4.5"), None);
        assert_eq!(p("1..2"), None);
        assert_eq!(p("0x7f.1"), None);
    }
}

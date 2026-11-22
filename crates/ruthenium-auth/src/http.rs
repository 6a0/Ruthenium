//! HTTP for sign-in: a small trait so the flows can be tested without a network, and its
//! implementation on ureq (rustls, the platform's certificate verifier).

use std::time::Duration;

use serde_json::Value;

use crate::AuthError;

/// A response: status and body text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reply {
    pub status: u16,
    pub body: String,
}

impl Reply {
    pub fn ok(&self) -> bool {
        (200..300).contains(&self.status)
    }

    /// The body as JSON (`Null` when it is not JSON).
    pub fn json(&self) -> Value {
        serde_json::from_str(&self.body).unwrap_or(Value::Null)
    }
}

pub trait Http: Send + Sync {
    /// POST `form` as `application/x-www-form-urlencoded`.
    fn post_form(&self, url: &str, form: &[(&str, &str)]) -> Result<Reply, AuthError>;
    /// POST `body` as JSON, with a bearer token if given.
    fn post_json(&self, url: &str, body: &Value, bearer: Option<&str>) -> Result<Reply, AuthError>;
    /// GET with a bearer token.
    fn get(&self, url: &str, bearer: Option<&str>) -> Result<Reply, AuthError>;
}

/// [`Http`] over ureq.
pub struct UreqHttp {
    agent: ureq::Agent,
}

impl std::fmt::Debug for UreqHttp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UreqHttp").finish_non_exhaustive()
    }
}

const USER_AGENT: &str = concat!("Ruthenium/", env!("CARGO_PKG_VERSION"));

impl UreqHttp {
    pub fn new() -> UreqHttp {
        let tls = ureq::tls::TlsConfig::builder()
            .root_certs(ureq::tls::RootCerts::PlatformVerifier)
            .build();
        let config = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .timeout_global(Some(Duration::from_secs(30)))
            .user_agent(USER_AGENT)
            .tls_config(tls)
            .build();
        UreqHttp {
            agent: ureq::Agent::new_with_config(config),
        }
    }

    fn finish(
        r: Result<ureq::http::Response<ureq::Body>, ureq::Error>,
    ) -> Result<Reply, AuthError> {
        let mut r = r.map_err(|e| AuthError::Network(e.to_string()))?;
        let status = r.status().as_u16();
        let body = r
            .body_mut()
            .read_to_string()
            .map_err(|e| AuthError::Network(e.to_string()))?;
        Ok(Reply { status, body })
    }
}

impl Default for UreqHttp {
    fn default() -> UreqHttp {
        UreqHttp::new()
    }
}

impl Http for UreqHttp {
    fn post_form(&self, url: &str, form: &[(&str, &str)]) -> Result<Reply, AuthError> {
        Self::finish(
            self.agent
                .post(url)
                .header("Accept", "application/json")
                .send_form(form.iter().copied()),
        )
    }

    fn post_json(&self, url: &str, body: &Value, bearer: Option<&str>) -> Result<Reply, AuthError> {
        let mut req = self.agent.post(url).header("Accept", "application/json");
        if let Some(t) = bearer {
            req = req.header("Authorization", &format!("Bearer {t}"));
        }
        Self::finish(req.send_json(body))
    }

    fn get(&self, url: &str, bearer: Option<&str>) -> Result<Reply, AuthError> {
        let mut req = self.agent.get(url).header("Accept", "application/json");
        if let Some(t) = bearer {
            req = req.header("Authorization", &format!("Bearer {t}"));
        }
        Self::finish(req.call())
    }
}

/// Percent-encodes a query value (RFC 3986 unreserved characters stay).
pub fn encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-._~".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// Decodes a query value (`+` is a space).
pub fn decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'+' => out.push(b' '),
            b'%' if i + 2 < b.len()
                && b[i + 1].is_ascii_hexdigit()
                && b[i + 2].is_ascii_hexdigit() =>
            {
                let hex = |c: u8| (c as char).to_digit(16).unwrap() as u8;
                out.push(hex(b[i + 1]) << 4 | hex(b[i + 2]));
                i += 2;
            }
            c => out.push(c),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percent_coding_round_trips() {
        let s = "XboxLive.signin offline_access&x=1/é";
        assert_eq!(encode("a b"), "a%20b");
        assert_eq!(decode(&encode(s)), s);
        assert_eq!(decode("M.C5+x%2F"), "M.C5 x/");
        assert_eq!(decode("bad%zz%"), "bad%zz%");
    }
}

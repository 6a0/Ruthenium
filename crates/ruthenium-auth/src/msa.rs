//! Microsoft identity platform, consumers tenant, as a public client with the scope
//! `XboxLive.signin offline_access`:
//!
//! - authorization code with PKCE: the system browser signs in and redirects to
//!   `http://localhost:<port>`, where [`Loopback`] waits for the code;
//! - device code, for signing in on another device;
//! - refresh tokens, which roll: keep the newest.

use std::io::{Read, Write};
use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use serde_json::Value;

use crate::AuthError;
use crate::http::{Http, Reply, decode, encode};
use crate::pkce::{Pkce, random_token};

const AUTHORITY: &str = "https://login.microsoftonline.com/consumers/oauth2/v2.0";
pub const SCOPE: &str = "XboxLive.signin offline_access";
/// How long the browser sign-in may take.
const BROWSER_TIMEOUT: Duration = Duration::from_secs(300);

/// Tokens from the token endpoint.
#[derive(Clone, PartialEq, Eq)]
pub struct MsaTokens {
    pub access_token: String,
    pub refresh_token: String,
    pub expires_in: u64,
}

impl std::fmt::Debug for MsaTokens {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MsaTokens")
            .field("expires_in", &self.expires_in)
            .finish_non_exhaustive()
    }
}

fn token_reply(r: &Reply) -> Result<MsaTokens, AuthError> {
    let v = r.json();
    if !r.ok() {
        return Err(oauth_error(&v, r.status));
    }
    let s = |k: &str| v.get(k).and_then(Value::as_str).map(str::to_owned);
    Ok(MsaTokens {
        access_token: s("access_token")
            .ok_or_else(|| AuthError::BadResponse("no access_token".into()))?,
        refresh_token: s("refresh_token")
            .ok_or_else(|| AuthError::BadResponse("no refresh_token".into()))?,
        expires_in: v.get("expires_in").and_then(Value::as_u64).unwrap_or(3600),
    })
}

fn oauth_error(v: &Value, status: u16) -> AuthError {
    let code = v.get("error").and_then(Value::as_str).unwrap_or("");
    let desc = v
        .get("error_description")
        .and_then(Value::as_str)
        .map(|d| d.lines().next().unwrap_or(d).to_owned())
        .unwrap_or_else(|| format!("HTTP {status}"));
    match code {
        "invalid_grant" => AuthError::SignInExpired,
        "authorization_declined" | "access_denied" => AuthError::Denied(desc),
        _ => AuthError::Microsoft(format!("{code}: {desc}")),
    }
}

/// The sign-in page's address for `redirect`, with the PKCE challenge and `state`.
pub fn authorize_url(client_id: &str, redirect: &str, challenge: &str, state: &str) -> String {
    format!(
        "{AUTHORITY}/authorize?client_id={}&response_type=code&redirect_uri={}&response_mode=query\
         &scope={}&state={}&code_challenge={}&code_challenge_method=S256&prompt=select_account",
        encode(client_id),
        encode(redirect),
        encode(SCOPE),
        encode(state),
        encode(challenge),
    )
}

/// Listens on localhost for the browser's redirect.
pub struct Loopback {
    listeners: Vec<TcpListener>,
    pub port: u16,
}

impl std::fmt::Debug for Loopback {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Loopback")
            .field("port", &self.port)
            .finish()
    }
}

const DONE_PAGE: &str = "<!doctype html><meta charset=utf-8><title>Ruthenium</title>\
<body style=\"font:16px system-ui,sans-serif;background:#0b0a1a;color:#e8eaff;display:grid;\
place-items:center;height:90vh\"><div><h2>Signed in</h2><p>You can close this tab and go back to \
Ruthenium.</p></div>";
const FAILED_PAGE: &str = "<!doctype html><meta charset=utf-8><title>Ruthenium</title>\
<body style=\"font:16px system-ui,sans-serif;background:#0b0a1a;color:#e8eaff;display:grid;\
place-items:center;height:90vh\"><div><h2>Sign-in didn't finish</h2><p>Go back to Ruthenium to \
try again.</p></div>";

impl Loopback {
    /// Binds a free port on 127.0.0.1, and the same port on ::1 where possible (browsers
    /// may try either for `localhost`).
    pub fn bind() -> Result<Loopback, AuthError> {
        let v4 = TcpListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, 0)))
            .map_err(|e| AuthError::Network(format!("can't listen on localhost: {e}")))?;
        let port = v4
            .local_addr()
            .map_err(|e| AuthError::Network(e.to_string()))?
            .port();
        let mut listeners = vec![v4];
        if let Ok(v6) = TcpListener::bind(SocketAddr::from((Ipv6Addr::LOCALHOST, port))) {
            listeners.push(v6);
        }
        for l in &listeners {
            l.set_nonblocking(true)
                .map_err(|e| AuthError::Network(e.to_string()))?;
        }
        Ok(Loopback { listeners, port })
    }

    pub fn redirect_uri(&self) -> String {
        format!("http://localhost:{}", self.port)
    }

    /// Waits for the redirect carrying `state` and returns its code.
    pub fn wait_for_code(
        &self,
        state: &str,
        cancel: &AtomicBool,
        timeout: Duration,
    ) -> Result<String, AuthError> {
        let deadline = Instant::now() + timeout;
        loop {
            if cancel.load(Ordering::Relaxed) {
                return Err(AuthError::Cancelled);
            }
            if Instant::now() > deadline {
                return Err(AuthError::TimedOut);
            }
            for l in &self.listeners {
                if let Ok((stream, _)) = l.accept()
                    && let Some(result) = handle(stream, state)
                {
                    return result;
                }
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }
}

/// Reads one request: `None` if it is not the redirect (a favicon request), else the
/// code or the error it carries.
fn handle(mut stream: TcpStream, state: &str) -> Option<Result<String, AuthError>> {
    let _ = stream.set_nonblocking(false);
    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
    let mut buf = Vec::with_capacity(2048);
    let mut chunk = [0u8; 1024];
    while !buf.windows(4).any(|w| w == b"\r\n\r\n") && buf.len() < 16384 {
        match stream.read(&mut chunk) {
            Ok(0) | Err(_) => break,
            Ok(n) => buf.extend_from_slice(&chunk[..n]),
        }
    }
    let head = String::from_utf8_lossy(&buf);
    let target = head.lines().next()?.split(' ').nth(1)?.to_owned();
    let result = parse_redirect(&target, state);
    let (status, page) = match &result {
        None => ("404 Not Found", ""),
        Some(Ok(_)) => ("200 OK", DONE_PAGE),
        Some(Err(_)) => ("200 OK", FAILED_PAGE),
    };
    let _ = write!(
        stream,
        "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\n\
         Connection: close\r\nCache-Control: no-store\r\n\r\n{page}",
        page.len()
    );
    result
}

/// The code (or error) in a redirect target like `/?code=...&state=...`.
pub fn parse_redirect(target: &str, state: &str) -> Option<Result<String, AuthError>> {
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    if path != "/" {
        return None;
    }
    let get = |key: &str| {
        query
            .split('&')
            .filter_map(|p| p.split_once('='))
            .find(|(k, _)| *k == key)
            .map(|(_, v)| decode(v))
    };
    if get("state").as_deref() != Some(state) {
        return Some(Err(AuthError::Microsoft(
            "the sign-in reply didn't match this request".into(),
        )));
    }
    if let Some(err) = get("error") {
        let desc = get("error_description").unwrap_or_else(|| err.clone());
        return Some(Err(if err == "access_denied" {
            AuthError::Denied(desc)
        } else {
            AuthError::Microsoft(format!("{err}: {desc}"))
        }));
    }
    Some(get("code").ok_or_else(|| AuthError::BadResponse("no code in the redirect".into())))
}

/// Signs in through the system browser: `open` gets the page to show.
pub fn sign_in_browser(
    http: &dyn Http,
    client_id: &str,
    open: &mut dyn FnMut(&str),
    cancel: &AtomicBool,
) -> Result<MsaTokens, AuthError> {
    let loopback = Loopback::bind()?;
    let pkce = Pkce::new();
    let state = random_token(16);
    let redirect = loopback.redirect_uri();
    open(&authorize_url(
        client_id,
        &redirect,
        &pkce.challenge,
        &state,
    ));
    let code = loopback.wait_for_code(&state, cancel, BROWSER_TIMEOUT)?;
    let reply = http.post_form(
        &format!("{AUTHORITY}/token"),
        &[
            ("client_id", client_id),
            ("grant_type", "authorization_code"),
            ("code", &code),
            ("redirect_uri", &redirect),
            ("code_verifier", &pkce.verifier),
            ("scope", SCOPE),
        ],
    )?;
    token_reply(&reply)
}

/// What to show for a device-code sign-in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceCode {
    pub user_code: String,
    pub verification_uri: String,
    /// Microsoft's own instruction text.
    pub message: String,
    pub expires_in: u64,
}

/// Signs in with a device code: `show` gets the code to enter; this then polls until the
/// player has signed in, declined, or the code expires.
pub fn sign_in_device(
    http: &dyn Http,
    client_id: &str,
    show: &mut dyn FnMut(&DeviceCode),
    cancel: &AtomicBool,
) -> Result<MsaTokens, AuthError> {
    let r = http.post_form(
        &format!("{AUTHORITY}/devicecode"),
        &[("client_id", client_id), ("scope", SCOPE)],
    )?;
    let v = r.json();
    if !r.ok() {
        return Err(oauth_error(&v, r.status));
    }
    let s = |k: &str| v.get(k).and_then(Value::as_str).unwrap_or("").to_owned();
    let device_code = s("device_code");
    let code = DeviceCode {
        user_code: s("user_code"),
        verification_uri: s("verification_uri"),
        message: s("message"),
        expires_in: v.get("expires_in").and_then(Value::as_u64).unwrap_or(900),
    };
    if device_code.is_empty() || code.user_code.is_empty() {
        return Err(AuthError::BadResponse("no device code".into()));
    }
    show(&code);
    let mut interval = v
        .get("interval")
        .and_then(Value::as_u64)
        .unwrap_or(5)
        .max(1);
    let deadline = Instant::now() + Duration::from_secs(code.expires_in);
    loop {
        let wait_until = Instant::now() + Duration::from_secs(interval);
        while Instant::now() < wait_until {
            if cancel.load(Ordering::Relaxed) {
                return Err(AuthError::Cancelled);
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        if Instant::now() > deadline {
            return Err(AuthError::TimedOut);
        }
        let r = http.post_form(
            &format!("{AUTHORITY}/token"),
            &[
                ("client_id", client_id),
                ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
                ("device_code", &device_code),
            ],
        )?;
        if r.ok() {
            return token_reply(&r);
        }
        match r.json().get("error").and_then(Value::as_str) {
            Some("authorization_pending") => {}
            Some("slow_down") => interval += 5,
            Some("expired_token") => return Err(AuthError::TimedOut),
            _ => return Err(oauth_error(&r.json(), r.status)),
        }
    }
}

/// New tokens from a refresh token (which is replaced by the one returned).
pub fn refresh(
    http: &dyn Http,
    client_id: &str,
    refresh_token: &str,
) -> Result<MsaTokens, AuthError> {
    let r = http.post_form(
        &format!("{AUTHORITY}/token"),
        &[
            ("client_id", client_id),
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh_token),
            ("scope", SCOPE),
        ],
    )?;
    token_reply(&r)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn authorize_url_carries_pkce_and_state() {
        let u = authorize_url("cid", "http://localhost:5000", "chal", "st8");
        assert!(
            u.starts_with("https://login.microsoftonline.com/consumers/oauth2/v2.0/authorize?")
        );
        assert!(u.contains("redirect_uri=http%3A%2F%2Flocalhost%3A5000"));
        assert!(u.contains("scope=XboxLive.signin%20offline_access"));
        assert!(u.contains("code_challenge=chal&code_challenge_method=S256"));
        assert!(u.contains("state=st8"));
    }

    #[test]
    fn redirects_parse() {
        assert_eq!(parse_redirect("/favicon.ico", "s"), None);
        assert_eq!(
            parse_redirect("/?code=M.C5%2Fx&state=s", "s"),
            Some(Ok("M.C5/x".into()))
        );
        assert!(matches!(
            parse_redirect("/?code=x&state=other", "s"),
            Some(Err(AuthError::Microsoft(_)))
        ));
        assert!(matches!(
            parse_redirect("/?error=access_denied&error_description=The+user+said+no&state=s", "s"),
            Some(Err(AuthError::Denied(d))) if d == "The user said no"
        ));
    }

    #[test]
    fn loopback_returns_the_code_and_answers_the_browser() {
        let lb = Loopback::bind().unwrap();
        let port = lb.port;
        let browser = std::thread::spawn(move || {
            let mut favicon = TcpStream::connect(("127.0.0.1", port)).unwrap();
            favicon
                .write_all(b"GET /favicon.ico HTTP/1.1\r\nHost: x\r\n\r\n")
                .unwrap();
            let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
            s.write_all(b"GET /?code=abc&state=xyz HTTP/1.1\r\nHost: localhost\r\n\r\n")
                .unwrap();
            let mut page = String::new();
            s.read_to_string(&mut page).unwrap();
            page
        });
        let cancel = AtomicBool::new(false);
        let code = lb
            .wait_for_code("xyz", &cancel, Duration::from_secs(10))
            .unwrap();
        assert_eq!(code, "abc");
        let page = browser.join().unwrap();
        assert!(page.starts_with("HTTP/1.1 200 OK") && page.contains("Signed in"));
        cancel.store(true, Ordering::Relaxed);
        assert!(matches!(
            lb.wait_for_code("xyz", &cancel, Duration::from_secs(1)),
            Err(AuthError::Cancelled)
        ));
    }

    #[test]
    fn token_errors_map() {
        let r = Reply {
            status: 400,
            body: r#"{"error":"invalid_grant","error_description":"AADSTS70000: expired\nTrace"}"#
                .into(),
        };
        assert_eq!(token_reply(&r), Err(AuthError::SignInExpired));
        let r = Reply {
            status: 200,
            body: r#"{"access_token":"a","refresh_token":"r","expires_in":3599}"#.into(),
        };
        let t = token_reply(&r).unwrap();
        assert_eq!(
            (
                t.access_token.as_str(),
                t.refresh_token.as_str(),
                t.expires_in
            ),
            ("a", "r", 3599)
        );
    }
}

//! Downloading the resource packs a server pushes, into the pack cache, safely.
//!
//! Vanilla: `DownloadQueue.runDownload` and `HttpUtil.downloadFile`. A server sends a URL
//! and maybe a SHA-1 (`ClientboundResourcePackPushPacket`); the client keeps the file in
//! `downloads/<pack id>/<sha1>`, reuses a cached file whose hash matches, downloads at most
//! 250 MiB hashing as it goes, and checks the hash. We do the same, and treat the server,
//! the URL's host and every byte they send as hostile; each limit and its reason is in
//! [`fetch::Limits`], the address policy in [`address`], the cache's rules in [`cache`].
//!
//! Deviations from vanilla, each for safety: hosts that resolve to private, loopback or
//! other non-public addresses are refused unless the server itself is local; timeouts on
//! every phase and on the whole download; at most five redirects; no proxy; redirects
//! that aren't followed (308, a missing Location, another protocol) fail instead of
//! keeping the redirect's own body as the pack; the player's name and UUID go only to the
//! host the server named; a gzip body we didn't ask for comes decoded (counted decoded); a
//! file is written under a temporary name and moved into place only once complete (and,
//! with a hash, verified); URLs vanilla would try but which can't be a public web server
//! (no host, port 0, a host with characters DNS doesn't have) fail; a run may receive at
//! most [`fetch::Limits::run_budget`] bytes into one cache; no `log.json` of downloads.

mod address;
mod cache;
mod fetch;
mod java_url;
#[cfg(test)]
mod tests;

use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Instant;
use std::{fmt, io};

use sha1::{Digest, Sha1};

/// Vanilla's limit on a pack's size (`MAX_PACK_SIZE_BYTES`): 250 MiB.
pub const MAX_PACK_SIZE: u64 = 262_144_000;

/// How many cached files opening the cache keeps (`DownloadQueue.MAX_KEPT_PACKS`).
pub const MAX_KEPT_PACKS: usize = 20;

/// One pack to download.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackRequest {
    /// The pack's id (a UUID).
    pub id: u128,
    /// As the server sent it.
    pub url: String,
    /// vanilla's SHA1 regex ^[a-fA-F0-9]{40}$ parsed, else None
    pub sha1: Option<[u8; 20]>,
}

/// Where and how a batch downloads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DownloadConfig {
    /// The pack cache (vanilla: `<game dir>/downloads`).
    pub cache_dir: PathBuf,
    /// Sent with every request, exactly these (vanilla's `X-Minecraft-*` and
    /// `User-Agent`); the player's name and UUID only to the host the server named.
    pub headers: Vec<(String, String)>,
    /// whether the server we're connected to is itself on a loopback/private/link-local address (then pack hosts may be too)
    pub allow_private_hosts: bool,
}

/// Why a pack didn't download.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum DownloadError {
    /// Not a URL vanilla would try (`parseResourcePackUrl` fails).
    InvalidUrl,
    /// A URL vanilla would try, but not one a public web server can have.
    UnusableUrl(&'static str),
    /// The host is at an address the policy refuses (private, loopback, ...).
    RefusedAddress(IpAddr),
    /// More redirects than we follow.
    TooManyRedirects,
    /// A redirect we don't follow.
    BadRedirect(&'static str),
    /// The server answered with this status.
    Status(u16),
    /// Larger than [`MAX_PACK_SIZE`], by its `Content-Length` or by what was read.
    TooLarge,
    /// A phase or the whole download took too long.
    TimedOut,
    /// The file's SHA-1 isn't the one the server asked for.
    HashMismatch {
        expected: [u8; 20],
        actual: [u8; 20],
    },
    /// The connection failed.
    Network(String),
    /// The cache couldn't be read or written.
    Cache(String),
    /// More packs in one batch than we download.
    TooManyRequests,
    /// This run already received as much as it may into the cache.
    BudgetExhausted,
    /// [`Progress::cancelled`] said to stop.
    Cancelled,
}

impl fmt::Display for DownloadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DownloadError::InvalidUrl => write!(f, "not an http or https URL"),
            DownloadError::UnusableUrl(why) => write!(f, "refused the URL: {why}"),
            DownloadError::RefusedAddress(ip) => {
                write!(f, "refused to connect to {ip}: not a public address")
            }
            DownloadError::TooManyRedirects => write!(f, "too many redirects"),
            DownloadError::BadRedirect(why) => write!(f, "didn't follow {why}"),
            DownloadError::Status(s) => write!(f, "the server answered {s}"),
            DownloadError::TooLarge => {
                write!(f, "larger than the limit of {MAX_PACK_SIZE} bytes")
            }
            DownloadError::TimedOut => write!(f, "timed out"),
            DownloadError::HashMismatch { expected, actual } => write!(
                f,
                "the file's hash {} isn't the requested {}",
                cache::hex(actual),
                cache::hex(expected)
            ),
            DownloadError::Network(e) => write!(f, "{e}"),
            DownloadError::Cache(e) => write!(f, "the pack cache: {e}"),
            DownloadError::TooManyRequests => write!(f, "too many packs at once"),
            DownloadError::BudgetExhausted => {
                write!(f, "downloaded as much as one run may")
            }
            DownloadError::Cancelled => write!(f, "cancelled"),
        }
    }
}

impl std::error::Error for DownloadError {}

fn cache_error(e: io::Error) -> DownloadError {
    DownloadError::Cache(e.to_string())
}

/// Progress for vanilla's toast (count, total, bytes so far, Content-Length if any).
///
/// `size` is the response's `Content-Length`, which the server chooses: the bytes may pass
/// it (a percentage should be clamped), and a length of 0 comes as `None`.
pub trait Progress: Send {
    fn request_start(&mut self);
    fn download_start(&mut self, size: Option<u64>);
    fn downloaded(&mut self, bytes: u64);
    fn request_finished(&mut self, ok: bool);
    /// Whether to stop (the player left the server): checked before each pack and between
    /// reads; the rest of the batch fails with [`DownloadError::Cancelled`].
    fn cancelled(&mut self) -> bool {
        false
    }
}

/// DownloadQueue.runDownload + HttpUtil.downloadFile for one batch, in order, blocking (call it on a thread). Ok(path) is a verified file in the cache.
pub fn download_batch(
    config: &DownloadConfig,
    requests: &[PackRequest],
    progress: &mut dyn Progress,
) -> Vec<(u128, Result<PathBuf, DownloadError>)> {
    download_batch_with(config, requests, progress, &fetch::Limits::default())
}

/// Parses the hash field as DownloadedPackSource.tryParseSha1Hash does.
pub fn parse_sha1(hash: &str) -> Option<[u8; 20]> {
    let bytes = hash.as_bytes();
    if bytes.len() != 40 || !bytes.iter().all(u8::is_ascii_hexdigit) {
        return None;
    }
    let mut out = [0u8; 20];
    for (i, pair) in bytes.chunks(2).enumerate() {
        let s = std::str::from_utf8(pair).ok()?;
        out[i] = u8::from_str_radix(s, 16).ok()?;
    }
    Some(out)
}

/// Whether a URL string is one vanilla would try (parseResourcePackUrl: java.net.URL, protocol http or https).
pub fn is_pack_url(url: &str) -> bool {
    java_url::parse(None, url).is_ok()
}

/// What a game run has done with each cache: opened (cleaned) it, and received how much.
static RUNS: Mutex<Vec<(PathBuf, u64)>> = Mutex::new(Vec::new());

/// Opens the cache: refuses a symlink or a non-directory, makes it if missing, and the
/// first time in a run cleans it as `DownloadQueue`'s constructor does. Returns the bytes
/// this run has received into it.
fn open_cache(dir: &Path) -> io::Result<u64> {
    cache::ensure_dir(dir, true)?;
    let mut runs = RUNS.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((_, received)) = runs.iter().find(|(d, _)| d == dir) {
        return Ok(*received);
    }
    cache::vacuum(dir, MAX_KEPT_PACKS);
    runs.push((dir.to_path_buf(), 0));
    Ok(0)
}

fn add_received(dir: &Path, bytes: u64) {
    let mut runs = RUNS.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((_, received)) = runs.iter_mut().find(|(d, _)| d == dir) {
        *received = received.saturating_add(bytes);
    }
}

pub(crate) fn download_batch_with(
    config: &DownloadConfig,
    requests: &[PackRequest],
    progress: &mut dyn Progress,
    limits: &fetch::Limits,
) -> Vec<(u128, Result<PathBuf, DownloadError>)> {
    let opened = open_cache(&config.cache_dir);
    let client = fetch::Client::new(limits, config.allow_private_hosts);
    let mut results = Vec::with_capacity(requests.len());
    for (n, request) in requests.iter().enumerate() {
        progress.request_start();
        let result = match &opened {
            Err(e) => Err(DownloadError::Cache(e.to_string())),
            Ok(_) if n >= limits.max_requests => Err(DownloadError::TooManyRequests),
            Ok(_) if progress.cancelled() => Err(DownloadError::Cancelled),
            Ok(_) => download(config, limits, &client, request, progress),
        };
        if let Err(e) = &result {
            eprintln!(
                "[packs] couldn't download pack {}: {e}",
                cache::uuid_string(request.id)
            );
        }
        progress.request_finished(result.is_ok());
        results.push((request.id, result));
    }
    results
}

/// `HttpUtil.downloadFile` for one pack.
fn download(
    config: &DownloadConfig,
    limits: &fetch::Limits,
    client: &fetch::Client,
    request: &PackRequest,
    progress: &mut dyn Progress,
) -> Result<PathBuf, DownloadError> {
    let url = java_url::parse(None, &request.url).map_err(|_| DownloadError::InvalidUrl)?;
    let dir = config.cache_dir.join(cache::uuid_string(request.id));
    // A pack directory that is a symlink is refused before anything is read through it.
    if std::fs::symlink_metadata(&dir).is_ok() {
        cache::ensure_dir(&dir, false).map_err(cache_error)?;
    }
    let target = match &request.sha1 {
        Some(hash) => {
            let target = dir.join(cache::hex(hash));
            match cache::matches(&target, hash, limits.max_size) {
                Ok(true) => {
                    eprintln!("[packs] using the cached {}", target.display());
                    cache::touch(&target);
                    return Ok(target);
                }
                Ok(false) => {}
                Err(e) => eprintln!("[packs] couldn't check {}: {e}", target.display()),
            }
            cache::remove(&target).map_err(cache_error)?;
            Some(target)
        }
        None => None,
    };

    let received = open_cache(&config.cache_dir).map_err(cache_error)?;
    let budget = limits.run_budget.saturating_sub(received);
    if budget == 0 {
        return Err(DownloadError::BudgetExhausted);
    }
    let deadline = Instant::now() + limits.whole;
    let response = client.get(&url, &config.headers, deadline)?;
    let size = response
        .headers()
        .get(ureq::http::header::CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.trim().parse::<u64>().ok());
    cache::ensure_dir(&dir, false).map_err(cache_error)?;
    progress.download_start(size.filter(|&s| s > 0));
    if size.is_some_and(|s| s > limits.max_size) {
        return Err(DownloadError::TooLarge);
    }
    if size.is_some_and(|s| s > budget) {
        return Err(DownloadError::BudgetExhausted);
    }

    let mut temp = cache::Temp::new(&dir).map_err(cache_error)?;
    let mut reader = response
        .into_body()
        .into_with_config()
        .limit(u64::MAX)
        .reader();
    let mut hasher = Sha1::new();
    let mut buf = vec![0u8; 64 * 1024];
    let mut so_far = 0u64;
    loop {
        if Instant::now() >= deadline {
            return Err(DownloadError::TimedOut);
        }
        if progress.cancelled() {
            return Err(DownloadError::Cancelled);
        }
        let n = match io::Read::read(&mut reader, &mut buf) {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(fetch::read_error(&e)),
        };
        so_far += n as u64;
        add_received(&config.cache_dir, n as u64);
        progress.downloaded(so_far);
        if so_far > limits.max_size {
            return Err(DownloadError::TooLarge);
        }
        if so_far > budget {
            return Err(DownloadError::BudgetExhausted);
        }
        temp.write_all(&buf[..n]).map_err(cache_error)?;
        hasher.update(&buf[..n]);
    }
    let actual: [u8; 20] = hasher.finalize().into();

    match (target, &request.sha1) {
        (Some(target), Some(expected)) => {
            if &actual != expected {
                return Err(DownloadError::HashMismatch {
                    expected: *expected,
                    actual,
                });
            }
            temp.persist(&target).map_err(cache_error)?;
            Ok(target)
        }
        _ => {
            let target = dir.join(cache::hex(&actual));
            if cache::matches(&target, &actual, limits.max_size).map_err(cache_error)? {
                cache::touch(&target);
                drop(temp);
            } else {
                cache::remove(&target).map_err(cache_error)?;
                temp.persist(&target).map_err(cache_error)?;
            }
            Ok(target)
        }
    }
}

//! A small HTTP server on a local port plays the pack host.

use super::*;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;
use std::time::{Duration, SystemTime};

/// A request as the server saw it.
#[derive(Debug, Clone)]
struct Seen {
    path: String,
    /// Names lower-cased, in order.
    headers: Vec<(String, String)>,
}

impl Seen {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v.as_str())
    }
}

enum Reply {
    /// A status, extra headers and a body with its length.
    Full(u16, Vec<(&'static str, String)>, Vec<u8>),
    /// A chunked body.
    Chunked(Vec<Vec<u8>>),
    /// Bytes written as they are.
    Raw(Vec<u8>),
    /// A head, then `count` writes of `chunk`, `delay` apart.
    Trickle {
        head: Vec<u8>,
        chunk: Vec<u8>,
        delay: Duration,
        count: usize,
    },
    /// Nothing, for this long.
    Silent(Duration),
}

fn ok(body: &[u8]) -> Reply {
    Reply::Full(200, vec![], body.to_vec())
}

fn redirect(status: u16, location: &str) -> Reply {
    Reply::Full(
        status,
        vec![("Location", location.to_owned())],
        b"moved".to_vec(),
    )
}

type Handler = dyn Fn(&Seen) -> Reply + Send + Sync;

struct Server {
    addr: SocketAddr,
    seen: Arc<Mutex<Vec<Seen>>>,
    connections: Arc<AtomicUsize>,
}

impl Server {
    fn url(&self, path: &str) -> String {
        format!("http://{}{path}", self.addr)
    }

    fn seen(&self) -> Vec<Seen> {
        self.seen.lock().unwrap().clone()
    }
}

fn serve(handler: impl Fn(&Seen) -> Reply + Send + Sync + 'static) -> Server {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let connections = Arc::new(AtomicUsize::new(0));
    let handler: Arc<Handler> = Arc::new(handler);
    let (s, c) = (seen.clone(), connections.clone());
    thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(stream) = stream else { continue };
            c.fetch_add(1, Ordering::SeqCst);
            let (s, handler) = (s.clone(), handler.clone());
            thread::spawn(move || answer(stream, &s, &*handler));
        }
    });
    Server {
        addr,
        seen,
        connections,
    }
}

fn answer(mut stream: TcpStream, seen: &Mutex<Vec<Seen>>, handler: &Handler) {
    let mut head = Vec::new();
    let mut byte = [0u8; 1];
    while !head.ends_with(b"\r\n\r\n") && head.len() < 65536 {
        match stream.read(&mut byte) {
            Ok(1) => head.push(byte[0]),
            _ => return,
        }
    }
    let text = String::from_utf8_lossy(&head);
    let mut lines = text.split("\r\n");
    let path = lines
        .next()
        .and_then(|l| l.split(' ').nth(1))
        .unwrap_or("")
        .to_owned();
    let headers = lines
        .filter_map(|l| l.split_once(':'))
        .map(|(n, v)| (n.trim().to_ascii_lowercase(), v.trim().to_owned()))
        .collect();
    let request = Seen { path, headers };
    seen.lock().unwrap().push(request.clone());
    let _ = match handler(&request) {
        Reply::Full(status, extra, body) => {
            let mut out = format!(
                "HTTP/1.1 {status} X\r\nContent-Length: {}\r\nConnection: close\r\n",
                body.len()
            );
            for (n, v) in extra {
                out.push_str(&format!("{n}: {v}\r\n"));
            }
            out.push_str("\r\n");
            let mut bytes = out.into_bytes();
            bytes.extend_from_slice(&body);
            stream.write_all(&bytes)
        }
        Reply::Chunked(chunks) => {
            let mut bytes =
                b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n"
                    .to_vec();
            for c in chunks {
                bytes.extend_from_slice(format!("{:x}\r\n", c.len()).as_bytes());
                bytes.extend_from_slice(&c);
                bytes.extend_from_slice(b"\r\n");
            }
            bytes.extend_from_slice(b"0\r\n\r\n");
            stream.write_all(&bytes)
        }
        Reply::Raw(bytes) => stream.write_all(&bytes),
        Reply::Trickle {
            head,
            chunk,
            delay,
            count,
        } => {
            let mut r = stream.write_all(&head);
            for _ in 0..count {
                thread::sleep(delay);
                r = stream.write_all(&chunk);
                if r.is_err() {
                    break;
                }
            }
            r
        }
        Reply::Silent(d) => {
            thread::sleep(d);
            Ok(())
        }
    };
    let _ = stream.flush();
}

/// A directory of its own under the system's temporary directory, removed after.
struct TempDir(PathBuf);

impl TempDir {
    fn new() -> TempDir {
        let mut r = [0u8; 8];
        getrandom::fill(&mut r).unwrap();
        let name: String = r.iter().map(|b| format!("{b:02x}")).collect();
        let dir = std::env::temp_dir().join(format!("ruthenium-pack-test-{name}"));
        std::fs::create_dir_all(&dir).unwrap();
        TempDir(dir)
    }

    fn cache(&self) -> PathBuf {
        self.0.join("downloads")
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

const ID: u128 = 0x0123_4567_89ab_cdef_0011_2233_4455_6677;
const ID_DIR: &str = "01234567-89ab-cdef-0011-223344556677";

fn headers() -> Vec<(String, String)> {
    [
        ("X-Minecraft-Username", "Developer"),
        ("X-Minecraft-UUID", "0123456789abcdef0123456789abcdef"),
        ("X-Minecraft-Version", "26.3"),
        ("X-Minecraft-Version-ID", "26.3"),
        ("X-Minecraft-Pack-Format", "88"),
        ("User-Agent", "Minecraft Java/26.3"),
    ]
    .iter()
    .map(|(n, v)| (n.to_string(), v.to_string()))
    .collect()
}

fn config(dir: &TempDir) -> DownloadConfig {
    DownloadConfig {
        cache_dir: dir.cache(),
        headers: headers(),
        private_allowed: vec![AddressRange::host("127.0.0.1".parse().unwrap())],
    }
}

/// Generous times: other builds may share the machine.
fn limits() -> fetch::Limits {
    fetch::Limits {
        per_read: Duration::from_secs(5),
        headers: Duration::from_secs(5),
        whole: Duration::from_secs(20),
        nat64: Some(Vec::new()),
        ..fetch::Limits::default()
    }
}

#[derive(Debug, Default)]
struct Log(Vec<String>);

impl Progress for Log {
    fn request_start(&mut self) {
        self.0.push("start".into());
    }
    fn download_start(&mut self, size: Option<u64>) {
        self.0.push(format!("size {size:?}"));
    }
    fn downloaded(&mut self, bytes: u64) {
        match self.0.last_mut() {
            Some(last) if last.starts_with("bytes ") => *last = format!("bytes {bytes}"),
            _ => self.0.push(format!("bytes {bytes}")),
        }
    }
    fn request_finished(&mut self, ok: bool) {
        self.0.push(format!("finished {ok}"));
    }
}

fn sha1(bytes: &[u8]) -> [u8; 20] {
    Sha1::digest(bytes).into()
}

/// A body that looks like nothing in particular.
fn body(len: usize) -> Vec<u8> {
    (0..len).map(|i| (i * 7 + i / 251) as u8).collect()
}

fn one(
    config: &DownloadConfig,
    url: &str,
    sha1: Option<[u8; 20]>,
    limits: &fetch::Limits,
) -> (Result<PathBuf, DownloadError>, Vec<String>) {
    let mut log = Log::default();
    let request = PackRequest {
        id: ID,
        url: url.to_owned(),
        sha1,
    };
    let mut results = download_batch_with(config, &[request], &mut log, limits);
    assert_eq!(results.len(), 1);
    let (id, result) = results.remove(0);
    assert_eq!(id, ID);
    (result, log.0)
}

/// Every file under the cache, relative and sorted.
fn files(dir: &Path) -> Vec<String> {
    fn walk(root: &Path, dir: &Path, out: &mut Vec<String>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for e in entries.flatten() {
            let p = e.path();
            if e.file_type().unwrap().is_dir() {
                walk(root, &p, out);
            } else {
                out.push(
                    p.strip_prefix(root)
                        .unwrap()
                        .to_string_lossy()
                        .replace('\\', "/"),
                );
            }
        }
    }
    let mut out = Vec::new();
    walk(dir, dir, &mut out);
    out.sort();
    out
}

fn assert_no_files(dir: &TempDir) {
    assert_eq!(files(&dir.cache()), Vec::<String>::new());
}

#[test]
fn downloads_a_pack_with_its_hash() {
    let pack = body(100_000);
    let p = pack.clone();
    let server = serve(move |_| ok(&p));
    let dir = TempDir::new();
    let hash = sha1(&pack);
    let (result, log) = one(
        &config(&dir),
        &server.url("/pack.zip"),
        Some(hash),
        &limits(),
    );
    let path = result.unwrap();
    assert_eq!(path, dir.cache().join(ID_DIR).join(cache::hex(&hash)));
    assert_eq!(std::fs::read(&path).unwrap(), pack);
    assert_eq!(
        log,
        [
            "start",
            "size Some(100000)",
            "bytes 100000",
            "finished true"
        ]
    );
    assert_eq!(
        files(&dir.cache()),
        [format!("{ID_DIR}/{}", cache::hex(&hash))]
    );

    // Exactly vanilla's headers, and nothing of ureq's own.
    let seen = server.seen();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].path, "/pack.zip");
    let mut names: Vec<&str> = seen[0].headers.iter().map(|(n, _)| n.as_str()).collect();
    names.sort();
    assert_eq!(
        names,
        [
            "host",
            "user-agent",
            "x-minecraft-pack-format",
            "x-minecraft-username",
            "x-minecraft-uuid",
            "x-minecraft-version",
            "x-minecraft-version-id"
        ]
    );
    assert_eq!(seen[0].header("user-agent"), Some("Minecraft Java/26.3"));
    assert_eq!(
        seen[0].header("x-minecraft-uuid"),
        Some("0123456789abcdef0123456789abcdef")
    );
}

#[test]
fn only_the_callers_headers_are_sent() {
    let server = serve(|_| ok(b"pack"));
    let dir = TempDir::new();
    let mut cfg = config(&dir);
    cfg.headers = vec![("X-Minecraft-Version".into(), "26.3".into())];
    assert!(one(&cfg, &server.url("/p"), None, &limits()).0.is_ok());
    let mut names: Vec<String> = server.seen()[0]
        .headers
        .iter()
        .map(|h| h.0.clone())
        .collect();
    names.sort();
    assert_eq!(names, ["host", "x-minecraft-version"]);
}

/// Run by [`proxies_in_the_environment_are_ignored`] in a child process with proxy
/// variables set: downloads from the URL in `RUTHENIUM_TEST_PACK_URL`.
#[test]
#[ignore = "run by proxies_in_the_environment_are_ignored"]
fn proxy_child() {
    let Ok(url) = std::env::var("RUTHENIUM_TEST_PACK_URL") else {
        return;
    };
    let dir = TempDir::new();
    let (result, _) = one(&config(&dir), &url, None, &limits());
    assert_eq!(std::fs::read(result.unwrap()).unwrap(), b"pack");
}

#[test]
fn proxies_in_the_environment_are_ignored() {
    let server = serve(|_| ok(b"pack"));
    let proxy = serve(|_| ok(b"proxied"));
    let proxy_url = format!("http://{}", proxy.addr);
    let status = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "server_pack::tests::proxy_child",
            "--ignored",
            "--quiet",
        ])
        .env("RUTHENIUM_TEST_PACK_URL", server.url("/p"))
        .env("http_proxy", &proxy_url)
        .env("HTTP_PROXY", &proxy_url)
        .env("https_proxy", &proxy_url)
        .env("HTTPS_PROXY", &proxy_url)
        .env("ALL_PROXY", &proxy_url)
        .env("all_proxy", &proxy_url)
        .env_remove("NO_PROXY")
        .env_remove("no_proxy")
        .status()
        .unwrap();
    assert!(status.success());
    assert_eq!(server.seen().len(), 1);
    assert_eq!(proxy.connections.load(Ordering::SeqCst), 0);
}

#[test]
fn a_pack_without_a_hash_is_named_by_its_own() {
    let pack = body(5000);
    let p = pack.clone();
    let server = serve(move |_| Reply::Chunked(p.chunks(1000).map(<[u8]>::to_vec).collect()));
    let dir = TempDir::new();
    let (result, log) = one(&config(&dir), &server.url("/p"), None, &limits());
    let path = result.unwrap();
    assert_eq!(
        path,
        dir.cache().join(ID_DIR).join(cache::hex(&sha1(&pack)))
    );
    assert_eq!(std::fs::read(&path).unwrap(), pack);
    assert_eq!(log, ["start", "size None", "bytes 5000", "finished true"]);

    // Vanilla downloads it again (nothing to compare with first), and keeps the one file.
    let old = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000);
    std::fs::File::options()
        .write(true)
        .open(&path)
        .unwrap()
        .set_modified(old)
        .unwrap();
    let (again, _) = one(&config(&dir), &server.url("/p"), None, &limits());
    assert_eq!(again.unwrap(), path);
    assert_eq!(server.seen().len(), 2);
    assert!(std::fs::metadata(&path).unwrap().modified().unwrap() > old);
    assert_eq!(files(&dir.cache()).len(), 1);
}

#[test]
fn a_hash_mismatch_fails_and_keeps_nothing() {
    let server = serve(|_| ok(b"not the pack"));
    let dir = TempDir::new();
    let wanted = sha1(b"the pack");
    let (result, log) = one(&config(&dir), &server.url("/p"), Some(wanted), &limits());
    assert_eq!(
        result,
        Err(DownloadError::HashMismatch {
            expected: wanted,
            actual: sha1(b"not the pack")
        })
    );
    assert_eq!(log.last().unwrap(), "finished false");
    assert_no_files(&dir);
}

#[test]
fn a_cached_pack_is_reused_without_asking() {
    let pack = body(3000);
    let p = pack.clone();
    let server = serve(move |_| ok(&p));
    let dir = TempDir::new();
    let hash = sha1(&pack);
    let path = one(&config(&dir), &server.url("/p"), Some(hash), &limits())
        .0
        .unwrap();
    let old = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000);
    std::fs::File::options()
        .write(true)
        .open(&path)
        .unwrap()
        .set_modified(old)
        .unwrap();

    let (again, log) = one(&config(&dir), &server.url("/p"), Some(hash), &limits());
    assert_eq!(again.unwrap(), path);
    assert_eq!(log, ["start", "finished true"]);
    assert_eq!(server.seen().len(), 1);
    assert!(std::fs::metadata(&path).unwrap().modified().unwrap() > old);

    // A damaged cached file is deleted and downloaded again.
    std::fs::write(&path, b"damaged").unwrap();
    let (fixed, _) = one(&config(&dir), &server.url("/p"), Some(hash), &limits());
    assert_eq!(std::fs::read(fixed.unwrap()).unwrap(), pack);
    assert_eq!(server.seen().len(), 2);
}

#[test]
fn a_cached_file_larger_than_the_limit_isnt_trusted() {
    // Its hash is the one asked for, but no pack that large could have been kept.
    let big = body(1001);
    let hash = sha1(&big);
    let b = big.clone();
    let server = serve(move |_| ok(&b));
    let dir = TempDir::new();
    let target = dir.cache().join(ID_DIR).join(cache::hex(&hash));
    std::fs::create_dir_all(target.parent().unwrap()).unwrap();
    std::fs::write(&target, &big).unwrap();
    let (result, _) = one(&config(&dir), &server.url("/p"), Some(hash), &small());
    assert_eq!(result, Err(DownloadError::TooLarge));
    assert_no_files(&dir);
}

#[test]
fn a_mismatching_cached_file_is_deleted_even_when_the_download_fails() {
    let server = serve(|_| Reply::Full(404, vec![], vec![]));
    let dir = TempDir::new();
    let hash = sha1(b"pack");
    let target = dir.cache().join(ID_DIR).join(cache::hex(&hash));
    std::fs::create_dir_all(target.parent().unwrap()).unwrap();
    std::fs::write(&target, b"old").unwrap();
    let (result, _) = one(&config(&dir), &server.url("/p"), Some(hash), &limits());
    assert_eq!(result, Err(DownloadError::Status(404)));
    assert_no_files(&dir);
}

fn small() -> fetch::Limits {
    fetch::Limits {
        max_size: 1000,
        ..limits()
    }
}

#[test]
fn a_pack_too_large_by_its_length_is_refused_before_reading() {
    let server = serve(|_| ok(&body(1001)));
    let dir = TempDir::new();
    let (result, log) = one(&config(&dir), &server.url("/p"), None, &small());
    assert_eq!(result, Err(DownloadError::TooLarge));
    assert_eq!(log, ["start", "size Some(1001)", "finished false"]);
    assert_no_files(&dir);

    // Exactly the limit is allowed.
    let server = serve(|_| ok(&body(1000)));
    assert!(
        one(&config(&dir), &server.url("/p"), None, &small())
            .0
            .is_ok()
    );
}

#[test]
fn a_pack_too_large_by_its_body_is_cut_off() {
    let server = serve(|_| Reply::Chunked(vec![body(600), body(600)]));
    let dir = TempDir::new();
    let (result, log) = one(&config(&dir), &server.url("/p"), None, &small());
    assert_eq!(result, Err(DownloadError::TooLarge));
    assert_eq!(log, ["start", "size None", "bytes 1200", "finished false"]);
    assert_no_files(&dir);

    // Close-delimited, no length at all.
    let mut raw = b"HTTP/1.1 200 OK\r\nConnection: close\r\n\r\n".to_vec();
    raw.extend(body(5000));
    let server = serve(move |_| Reply::Raw(raw.clone()));
    let (result, _) = one(&config(&dir), &server.url("/p"), None, &small());
    assert_eq!(result, Err(DownloadError::TooLarge));
    assert_no_files(&dir);
}

#[test]
fn a_lying_content_length_never_gives_a_pack_vanilla_wouldnt() {
    let dir = TempDir::new();
    // Says more than it sends: the download fails.
    let mut raw = b"HTTP/1.1 200 OK\r\nContent-Length: 900\r\nConnection: close\r\n\r\n".to_vec();
    raw.extend(body(100));
    let server = serve(move |_| Reply::Raw(raw.clone()));
    let (result, _) = one(&config(&dir), &server.url("/p"), None, &small());
    assert!(
        matches!(result, Err(DownloadError::Network(_))),
        "{result:?}"
    );
    assert_no_files(&dir);

    // Says less than it sends: only that much is read, as vanilla reads it.
    let mut raw = b"HTTP/1.1 200 OK\r\nContent-Length: 10\r\nConnection: close\r\n\r\n".to_vec();
    raw.extend(body(5000));
    let server = serve(move |_| Reply::Raw(raw.clone()));
    let (result, _) = one(&config(&dir), &server.url("/p"), None, &small());
    assert_eq!(std::fs::read(result.unwrap()).unwrap(), body(10));

    // A small length with a large chunked body: never more than the limit.
    let mut raw =
        b"HTTP/1.1 200 OK\r\nContent-Length: 10\r\nTransfer-Encoding: chunked\r\n\r\n".to_vec();
    for _ in 0..5 {
        raw.extend(format!("{:x}\r\n", 600).as_bytes());
        raw.extend(body(600));
        raw.extend(b"\r\n");
    }
    raw.extend(b"0\r\n\r\n");
    let dir = TempDir::new();
    let server = serve(move |_| Reply::Raw(raw.clone()));
    let (result, _) = one(&config(&dir), &server.url("/p"), None, &small());
    match result {
        Ok(path) => assert!(std::fs::metadata(path).unwrap().len() <= 1000),
        Err(e) => {
            assert!(
                matches!(e, DownloadError::TooLarge | DownloadError::Network(_)),
                "{e:?}"
            );
            assert_no_files(&dir);
        }
    }
}

/// Each timeout test shortens one limit and leaves the others long, so each is tested alone.
fn quick(headers: u64, per_read: u64, whole: u64) -> fetch::Limits {
    fetch::Limits {
        headers: Duration::from_millis(headers),
        per_read: Duration::from_millis(per_read),
        whole: Duration::from_millis(whole),
        nat64: Some(Vec::new()),
        ..fetch::Limits::default()
    }
}

#[test]
fn a_server_that_never_answers_times_out() {
    let server = serve(|_| Reply::Silent(Duration::from_secs(8)));
    let dir = TempDir::new();
    let start = Instant::now();
    let (result, _) = one(
        &config(&dir),
        &server.url("/p"),
        None,
        &quick(400, 6000, 6000),
    );
    assert_eq!(result, Err(DownloadError::TimedOut));
    assert!(
        start.elapsed() < Duration::from_secs(3),
        "{:?}",
        start.elapsed()
    );
}

#[test]
fn a_stalled_body_times_out() {
    let server = serve(|_| Reply::Trickle {
        head: b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\n\r\nab".to_vec(),
        chunk: b"c".to_vec(),
        delay: Duration::from_secs(2),
        count: 2,
    });
    let dir = TempDir::new();
    let start = Instant::now();
    let (result, _) = one(
        &config(&dir),
        &server.url("/p"),
        None,
        &quick(6000, 400, 6000),
    );
    assert_eq!(result, Err(DownloadError::TimedOut));
    assert!(
        start.elapsed() < Duration::from_millis(1800),
        "{:?}",
        start.elapsed()
    );
    assert_no_files(&dir);
}

#[test]
fn a_trickling_server_hits_the_whole_download_limit() {
    // A byte every 100 ms never trips the per-read limit, only the whole one.
    let server = serve(|_| Reply::Trickle {
        head: b"HTTP/1.1 200 OK\r\nConnection: close\r\n\r\n".to_vec(),
        chunk: b"x".to_vec(),
        delay: Duration::from_millis(100),
        count: 100,
    });
    let dir = TempDir::new();
    let start = Instant::now();
    let (result, _) = one(
        &config(&dir),
        &server.url("/p"),
        None,
        &quick(6000, 6000, 1500),
    );
    assert_eq!(result, Err(DownloadError::TimedOut));
    let took = start.elapsed();
    assert!(
        took >= Duration::from_millis(1400) && took < Duration::from_secs(4),
        "{took:?}"
    );
    assert_no_files(&dir);
}

#[test]
fn redirects_are_followed_as_java_follows_them() {
    let server = serve(|r| match r.path.as_str() {
        "/a" => redirect(301, "/b"),
        "/b" => redirect(302, "c"),
        "/c" => redirect(303, "./d?x=1"),
        "/d?x=1" => redirect(307, "/e"),
        "/e" => ok(b"pack"),
        _ => Reply::Full(404, vec![], vec![]),
    });
    let dir = TempDir::new();
    let (result, _) = one(&config(&dir), &server.url("/a"), None, &limits());
    assert_eq!(std::fs::read(result.unwrap()).unwrap(), b"pack");
    let paths: Vec<String> = server.seen().iter().map(|s| s.path.clone()).collect();
    assert_eq!(paths, ["/a", "/b", "/c", "/d?x=1", "/e"]);
    // The same host keeps every header.
    assert!(
        server
            .seen()
            .iter()
            .all(|s| s.header("x-minecraft-uuid").is_some())
    );
}

#[test]
fn a_long_redirect_chain_is_refused() {
    let server = serve(|r| {
        let n: u32 = r.path[1..].parse().unwrap_or(0);
        if n < 10 {
            redirect(302, &format!("/{}", n + 1))
        } else {
            ok(b"pack")
        }
    });
    let dir = TempDir::new();
    // Five hops are followed...
    let (result, _) = one(&config(&dir), &server.url("/5"), None, &limits());
    assert!(result.is_ok(), "{result:?}");
    // ...six are not.
    let (result, _) = one(&config(&dir), &server.url("/4"), None, &limits());
    assert_eq!(result, Err(DownloadError::TooManyRedirects));
    assert_eq!(server.seen().len(), 6 + 6);
    let (result, _) = one(&config(&dir), &server.url("/loop"), None, &limits());
    assert!(result.is_ok() || result == Err(DownloadError::TooManyRedirects));
}

#[test]
fn redirects_vanilla_wouldnt_follow_fail() {
    let server = serve(|r| match r.path.as_str() {
        "/https" => redirect(302, "https://127.0.0.1:1/p"),
        "/ftp" => redirect(302, "ftp://127.0.0.1/p"),
        "/file" => redirect(302, "file:///etc/passwd"),
        "/js" => redirect(302, "javascript:alert(1)"),
        "/308" => redirect(308, "/p"),
        "/305" => redirect(305, "http://127.0.0.1:1/"),
        "/304" => Reply::Full(304, vec![], vec![]),
        "/none" => Reply::Full(302, vec![], b"x".to_vec()),
        "/bad" => redirect(302, "http://[::1/"),
        _ => ok(b"pack"),
    });
    let dir = TempDir::new();
    for (path, expected) in [
        ("/https", "didn't follow a redirect to another protocol"),
        ("/ftp", "didn't follow a redirect to another protocol"),
        ("/file", "didn't follow a redirect to another protocol"),
        ("/js", "didn't follow a redirect to another protocol"),
        ("/308", "the server answered 308"),
        ("/305", "didn't follow a proxy redirect (305)"),
        ("/304", "the server answered 304"),
        ("/none", "didn't follow a redirect with no Location"),
        ("/bad", "didn't follow a Location that isn't a URL"),
    ] {
        let (result, _) = one(&config(&dir), &server.url(path), None, &limits());
        assert_eq!(result.unwrap_err().to_string(), expected, "{path}");
    }
    // Only the redirects themselves were asked for.
    assert_eq!(server.seen().len(), 9);
    assert_no_files(&dir);
}

#[test]
fn the_player_is_named_only_to_the_host_the_server_named() {
    let cdn = serve(|_| ok(b"pack"));
    let cdn_url = cdn.url("/pack.zip");
    let origin = serve(move |_| redirect(302, &cdn_url));
    let dir = TempDir::new();
    let (result, _) = one(&config(&dir), &origin.url("/p"), None, &limits());
    assert!(result.is_ok(), "{result:?}");
    let first = &origin.seen()[0];
    assert_eq!(first.header("x-minecraft-username"), Some("Developer"));
    assert!(first.header("x-minecraft-uuid").is_some());
    let second = &cdn.seen()[0];
    assert_eq!(second.header("x-minecraft-username"), None);
    assert_eq!(second.header("x-minecraft-uuid"), None);
    assert_eq!(second.header("x-minecraft-version"), Some("26.3"));
    assert_eq!(second.header("x-minecraft-pack-format"), Some("88"));
    assert_eq!(second.header("user-agent"), Some("Minecraft Java/26.3"));
}

#[test]
fn cookies_are_never_sent_back() {
    let server = serve(|r| match r.path.as_str() {
        "/a" => Reply::Full(
            302,
            vec![
                ("Location", "/b".into()),
                ("Set-Cookie", "session=1; Path=/".into()),
            ],
            vec![],
        ),
        _ => ok(b"pack"),
    });
    let dir = TempDir::new();
    assert!(
        one(&config(&dir), &server.url("/a"), None, &limits())
            .0
            .is_ok()
    );
    assert!(server.seen().iter().all(|s| s.header("cookie").is_none()));
}

#[test]
fn local_hosts_are_refused_unless_the_server_is_local() {
    let server = serve(|_| ok(b"pack"));
    let dir = TempDir::new();
    let mut cfg = config(&dir);
    cfg.private_allowed = Vec::new();
    let (result, log) = one(&cfg, &server.url("/p"), None, &limits());
    assert_eq!(
        result,
        Err(DownloadError::RefusedAddress("127.0.0.1".parse().unwrap()))
    );
    assert_eq!(log, ["start", "finished false"]);
    let localhost = format!("http://localhost:{}/p", server.addr.port());
    let (result, _) = one(&cfg, &localhost, None, &limits());
    assert!(
        matches!(result, Err(DownloadError::RefusedAddress(ip)) if ip.is_loopback()),
        "{result:?}"
    );
    // Never connected.
    assert_eq!(server.connections.load(Ordering::SeqCst), 0);
    assert_no_files(&dir);
}

#[test]
fn a_local_server_allows_its_own_address_and_nothing_else() {
    let server = serve(|r| match r.path.as_str() {
        "/to-neighbour" => redirect(302, "http://127.0.0.2:9/admin"),
        "/to-router" => redirect(302, "http://192.168.1.1/admin"),
        _ => ok(b"pack"),
    });
    let dir = TempDir::new();
    // config() allows the server's own address, 127.0.0.1.
    assert!(
        one(&config(&dir), &server.url("/p"), None, &limits())
            .0
            .is_ok()
    );
    for (path, ip) in [
        ("/to-neighbour", "127.0.0.2"),
        ("/to-router", "192.168.1.1"),
    ] {
        let (result, _) = one(&config(&dir), &server.url(path), None, &limits());
        assert_eq!(
            result,
            Err(DownloadError::RefusedAddress(ip.parse().unwrap())),
            "{path}"
        );
    }
}

#[test]
fn a_redirect_to_a_private_address_is_refused() {
    let server = serve(|r| match r.path.as_str() {
        "/to-loopback" => redirect(302, "http://127.0.0.2:9/admin"),
        "/to-router" => redirect(302, "http://192.168.1.1/admin"),
        "/to-metadata" => redirect(302, "http://169.254.169.254/latest/meta-data/"),
        "/to-v6" => redirect(302, "http://[::1]:9/"),
        "/to-mapped" => redirect(302, "http://[::ffff:10.0.0.1]/"),
        "/to-nat64" => redirect(302, "http://[2a00:1234:5678:9abc::c0a8:101]:9/"),
        _ => ok(b"pack"),
    });
    let dir = TempDir::new();
    let mut cfg = config(&dir);
    cfg.private_allowed = Vec::new();
    // The test server stands in for a public host, on a network whose DNS64 synthesises
    // with an operator's prefix.
    let pretend = fetch::Limits {
        pretend_public: vec!["127.0.0.1".parse().unwrap()],
        nat64: Some(vec![
            address::Nat64Prefix::new("2a00:1234:5678:9abc::".parse().unwrap(), 96).unwrap(),
        ]),
        ..limits()
    };
    assert!(one(&cfg, &server.url("/p"), None, &pretend).0.is_ok());
    for (path, ip) in [
        ("/to-loopback", "127.0.0.2"),
        ("/to-router", "192.168.1.1"),
        ("/to-metadata", "169.254.169.254"),
        ("/to-v6", "::1"),
        ("/to-mapped", "10.0.0.1"),
        // 192.168.1.1 through the operator's NAT64.
        ("/to-nat64", "2a00:1234:5678:9abc::c0a8:101"),
    ] {
        let (result, _) = one(&cfg, &server.url(path), None, &pretend);
        assert_eq!(
            result,
            Err(DownloadError::RefusedAddress(ip.parse().unwrap())),
            "{path}"
        );
    }
}

#[test]
fn urls_vanilla_wouldnt_try_fail_at_once() {
    let dir = TempDir::new();
    for url in [
        "ftp://example.com/p",
        "file:///etc/passwd",
        "example.com/p",
        "",
        "jar:http://x!/",
    ] {
        let (result, log) = one(&config(&dir), url, None, &limits());
        assert_eq!(result, Err(DownloadError::InvalidUrl), "{url}");
        assert_eq!(log, ["start", "finished false"]);
    }
    for url in [
        "http:///p",
        "http://a@b@c/",
        "http://host:0/",
        "http://host:70000/",
        "http://ex ample/",
    ] {
        let (result, _) = one(&config(&dir), url, None, &limits());
        assert!(
            matches!(
                result,
                Err(DownloadError::UnusableUrl(_) | DownloadError::InvalidUrl)
            ),
            "{url}: {result:?}"
        );
    }
    assert_no_files(&dir);
}

#[test]
fn a_compressed_body_counts_as_decoded() {
    // 100 kB of zeros in a few hundred bytes of gzip: stopped at the limit all the same.
    let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::best());
    gz.write_all(&[0u8; 100_000]).unwrap();
    let gz = gz.finish().unwrap();
    assert!(gz.len() < 1000);
    let server =
        serve(move |_| Reply::Full(200, vec![("Content-Encoding", "gzip".into())], gz.clone()));
    let dir = TempDir::new();
    let (result, log) = one(&config(&dir), &server.url("/p"), None, &small());
    assert_eq!(result, Err(DownloadError::TooLarge));
    assert_eq!(log[..2], ["start", "size None"]);
    assert_eq!(log.last().unwrap(), "finished false");
    assert_no_files(&dir);
    // Not even gzip: fails.
    let server = serve(|_| Reply::Full(200, vec![("Content-Encoding", "gzip".into())], body(100)));
    assert!(
        one(&config(&dir), &server.url("/p"), None, &small())
            .0
            .is_err()
    );
    assert_no_files(&dir);
}

#[test]
fn an_error_response_is_not_read() {
    // Says 100 MiB and sends a little: we never wait for the rest.
    let server = serve(|_| Reply::Trickle {
        head: b"HTTP/1.1 500 X\r\nContent-Length: 104857600\r\n\r\n".to_vec(),
        chunk: vec![b'e'; 1000],
        delay: Duration::from_millis(50),
        count: 200,
    });
    let dir = TempDir::new();
    let start = Instant::now();
    let (result, _) = one(&config(&dir), &server.url("/p"), None, &limits());
    assert_eq!(result, Err(DownloadError::Status(500)));
    assert!(
        start.elapsed() < Duration::from_secs(2),
        "{:?}",
        start.elapsed()
    );
}

#[test]
fn https_is_never_downgraded_to_plain_http() {
    let server = serve(|_| ok(b"pack"));
    let dir = TempDir::new();
    let url = format!("https://{}/p", server.addr);
    let (result, _) = one(&config(&dir), &url, None, &quick(400, 400, 1500));
    // The handshake gets no answer it can use.
    assert!(
        matches!(
            result,
            Err(DownloadError::Network(_) | DownloadError::TimedOut)
        ),
        "{result:?}"
    );
    // What arrived was a TLS handshake, never a plain request.
    assert!(server.seen().iter().all(|s| s.path != "/p"));
    assert_no_files(&dir);
}

#[test]
fn an_empty_length_is_no_length() {
    let server = serve(|_| ok(b""));
    let dir = TempDir::new();
    let (result, log) = one(&config(&dir), &server.url("/p"), None, &limits());
    assert_eq!(log, ["start", "size None", "finished true"]);
    assert_eq!(std::fs::read(result.unwrap()).unwrap(), b"");
}

/// Stops after the given number of checks.
struct CancelAfter(usize, Log);

impl Progress for CancelAfter {
    fn request_start(&mut self) {
        self.1.request_start();
    }
    fn download_start(&mut self, size: Option<u64>) {
        self.1.download_start(size);
    }
    fn downloaded(&mut self, bytes: u64) {
        self.1.downloaded(bytes);
    }
    fn request_finished(&mut self, ok: bool) {
        self.1.request_finished(ok);
    }
    fn cancelled(&mut self) -> bool {
        self.0 = self.0.saturating_sub(1);
        self.0 == 0
    }
}

#[test]
fn a_cancelled_batch_stops() {
    let server = serve(|_| Reply::Trickle {
        head: b"HTTP/1.1 200 OK
Content-Length: 1000

"
        .to_vec(),
        chunk: vec![b'x'; 100],
        delay: Duration::from_millis(20),
        count: 10,
    });
    let dir = TempDir::new();
    let requests: Vec<PackRequest> = (0..3u128)
        .map(|i| PackRequest {
            id: i,
            url: server.url("/p"),
            sha1: None,
        })
        .collect();
    // Before the first pack, then in its body: nothing more is asked for.
    let mut progress = CancelAfter(3, Log::default());
    let results = download_batch_with(&config(&dir), &requests, &mut progress, &limits());
    assert!(
        results.iter().all(|r| r.1 == Err(DownloadError::Cancelled)),
        "{results:?}"
    );
    assert_eq!(server.seen().len(), 1);
    assert_no_files(&dir);
}

#[test]
fn a_batch_downloads_in_order_up_to_its_limits() {
    let server = serve(|r| ok(r.path.as_bytes()));
    let dir = TempDir::new();
    let requests: Vec<PackRequest> = (0..4u128)
        .map(|i| PackRequest {
            id: i,
            url: server.url(&format!("/{i}")),
            sha1: None,
        })
        .collect();
    let limits = fetch::Limits {
        max_requests: 3,
        ..limits()
    };
    let mut log = Log::default();
    let results = download_batch_with(&config(&dir), &requests, &mut log, &limits);
    let ids: Vec<u128> = results.iter().map(|r| r.0).collect();
    assert_eq!(ids, [0, 1, 2, 3]);
    assert!(results[..3].iter().all(|r| r.1.is_ok()));
    assert_eq!(results[3].1, Err(DownloadError::TooManyRequests));
    let paths: Vec<String> = server.seen().iter().map(|s| s.path.clone()).collect();
    assert_eq!(paths, ["/0", "/1", "/2"]);
    assert_eq!(log.0.iter().filter(|e| *e == "start").count(), 4);
    assert_eq!(log.0.last().unwrap(), "finished false");
}

#[test]
fn a_run_receives_at_most_its_budget() {
    let server = serve(|r| match r.path.as_str() {
        "/sized" => ok(&body(700)),
        _ => Reply::Chunked(vec![body(700)]),
    });
    let dir = TempDir::new();
    let limits = fetch::Limits {
        run_budget: 1000,
        ..limits()
    };
    assert!(
        one(&config(&dir), &server.url("/a"), None, &limits)
            .0
            .is_ok()
    );
    // 300 bytes left: refused by its length, then cut off by its body.
    let (result, log) = one(&config(&dir), &server.url("/sized"), None, &limits);
    assert_eq!(result, Err(DownloadError::BudgetExhausted));
    assert_eq!(log, ["start", "size Some(700)", "finished false"]);
    let (result, _) = one(&config(&dir), &server.url("/b"), None, &limits);
    assert_eq!(result, Err(DownloadError::BudgetExhausted));
    // Nothing left: refused before asking.
    let asked = server.seen().len();
    let (result, _) = one(&config(&dir), &server.url("/c"), None, &limits);
    assert_eq!(result, Err(DownloadError::BudgetExhausted));
    assert_eq!(server.seen().len(), asked);
}

#[test]
fn a_run_may_use_its_whole_budget() {
    let server = serve(|r| match r.path.as_str() {
        "/a" => ok(&body(700)),
        _ => Reply::Chunked(vec![body(300)]),
    });
    let dir = TempDir::new();
    let limits = fetch::Limits {
        run_budget: 1000,
        ..limits()
    };
    assert!(
        one(&config(&dir), &server.url("/a"), None, &limits)
            .0
            .is_ok()
    );
    assert!(
        one(&config(&dir), &server.url("/b"), None, &limits)
            .0
            .is_ok()
    );
    let (result, _) = one(&config(&dir), &server.url("/c"), None, &limits);
    assert_eq!(result, Err(DownloadError::BudgetExhausted));
    assert_eq!(server.seen().len(), 2);
}

#[test]
fn a_cache_dir_that_isnt_a_real_directory_is_refused() {
    let server = serve(|_| ok(b"pack"));
    let dir = TempDir::new();
    // A file.
    std::fs::write(dir.cache(), b"x").unwrap();
    let (result, _) = one(&config(&dir), &server.url("/p"), None, &limits());
    assert!(matches!(result, Err(DownloadError::Cache(_))), "{result:?}");
    assert_eq!(server.seen().len(), 0);
}

#[cfg(unix)]
#[test]
fn symlinks_in_the_cache_are_never_followed() {
    use std::os::unix::fs::symlink;
    let server = serve(|_| ok(b"pack"));
    let dir = TempDir::new();
    let outside = dir.0.join("outside");
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::write(outside.join("precious"), b"keep me").unwrap();

    // The cache itself a symlink.
    symlink(&outside, dir.cache()).unwrap();
    let (result, _) = one(&config(&dir), &server.url("/p"), None, &limits());
    assert!(matches!(result, Err(DownloadError::Cache(_))), "{result:?}");
    std::fs::remove_file(dir.cache()).unwrap();

    // A pack's directory a symlink, to a file that would pass for the cached pack: never
    // handed out, nothing asked for.
    std::fs::create_dir_all(dir.cache()).unwrap();
    std::fs::write(outside.join(cache::hex(&sha1(b"pack"))), b"pack").unwrap();
    symlink(&outside, dir.cache().join(ID_DIR)).unwrap();
    let (result, _) = one(
        &config(&dir),
        &server.url("/p"),
        Some(sha1(b"pack")),
        &limits(),
    );
    assert!(matches!(result, Err(DownloadError::Cache(_))), "{result:?}");
    assert_eq!(server.seen().len(), 0);
    std::fs::remove_file(dir.cache().join(ID_DIR)).unwrap();

    // A pack's file a symlink: the link is replaced, its target untouched.
    let hash = sha1(b"pack");
    std::fs::create_dir_all(dir.cache().join(ID_DIR)).unwrap();
    let target = dir.cache().join(ID_DIR).join(cache::hex(&hash));
    symlink(outside.join("precious"), &target).unwrap();
    let (result, _) = one(&config(&dir), &server.url("/p"), Some(hash), &limits());
    assert_eq!(result.unwrap(), target);
    assert!(
        !std::fs::symlink_metadata(&target)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert_eq!(std::fs::read(outside.join("precious")).unwrap(), b"keep me");

    // Cleaning skips links to files and directories outside.
    symlink(
        outside.join("precious"),
        dir.cache().join(ID_DIR).join("link"),
    )
    .unwrap();
    symlink(&outside, dir.cache().join("dirlink")).unwrap();
    cache::vacuum(&dir.cache(), 0);
    assert_eq!(std::fs::read(outside.join("precious")).unwrap(), b"keep me");
    assert!(std::fs::symlink_metadata(dir.cache().join(ID_DIR).join("link")).is_ok());
    assert!(!target.exists());
}

#[cfg(unix)]
#[test]
fn cache_files_are_the_players_alone() {
    use std::os::unix::fs::PermissionsExt;
    let server = serve(|_| ok(b"pack"));
    let dir = TempDir::new();
    let path = one(&config(&dir), &server.url("/p"), None, &limits())
        .0
        .unwrap();
    let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode(&path), 0o600);
    assert_eq!(mode(path.parent().unwrap()), 0o700);
    assert_eq!(mode(&dir.cache()), 0o700);
}

/// vanilla's DownloadCacheCleaner on a made-up cache (a private tool runs the 26.3 class
/// on the same files): what it keeps for each limit.
const VACUUM_FILES: &[(&str, u64)] = &[
    ("log.json", 100),
    ("a/f1", 10),
    ("a/f2", 20),
    ("a/f3", 30),
    ("a/f4", 40),
    ("a/f5", 50),
    ("b/g1", 15),
    ("c/h1", 5),
    ("c/h2", 60),
    ("d/i1", 1),
    ("d/i2", 2),
    ("d/i3", 3),
    ("d/i4", 70),
    ("d/i5", 80),
    ("d/i6", 90),
    ("d/i7", 25),
    ("d/i8", 35),
    ("e/j1", 45),
    ("f/k1", 55),
    ("f/k2", 65),
    ("g/deep/l1", 12),
    ("g/deep/l2", 13),
    ("h/m1", 8),
];

const VACUUM_KEPT: &[(usize, &str)] = &[
    (
        25,
        "a/ a/f1 a/f2 a/f3 a/f4 a/f5 b/ b/g1 c/ c/h1 c/h2 d/ d/i1 d/i2 d/i3 d/i4 d/i5 d/i6 d/i7 d/i8 e/ e/j1 f/ f/k1 f/k2 g/ g/deep/ g/deep/l1 g/deep/l2 h/ h/m1 log.json",
    ),
    (
        22,
        "a/ a/f1 a/f2 a/f3 a/f4 a/f5 b/ b/g1 c/ c/h1 c/h2 d/ d/i1 d/i2 d/i3 d/i4 d/i5 d/i6 d/i7 d/i8 e/ e/j1 f/ f/k1 f/k2 g/ g/deep/ g/deep/l1 g/deep/l2 h/ h/m1 log.json",
    ),
    (
        20,
        "a/ a/f1 a/f2 a/f3 a/f4 a/f5 b/ b/g1 c/ c/h1 c/h2 d/ d/i3 d/i4 d/i5 d/i6 d/i7 d/i8 e/ e/j1 f/ f/k1 f/k2 g/ g/deep/ g/deep/l1 g/deep/l2 h/ h/m1 log.json",
    ),
    (
        15,
        "a/ a/f3 a/f4 a/f5 b/ b/g1 c/ c/h1 c/h2 d/ d/i4 d/i5 d/i6 e/ e/j1 f/ f/k1 f/k2 g/ g/deep/ g/deep/l1 g/deep/l2 h/ h/m1 log.json",
    ),
    (
        8,
        "a/ a/f5 b/ b/g1 c/ c/h2 d/ d/i6 e/ e/j1 f/ f/k2 g/ g/deep/ g/deep/l2 h/ h/m1 log.json",
    ),
    (3, "c/ c/h2 d/ d/i6 f/ f/k2 g/ log.json"),
    (0, "g/ log.json"),
];

fn listing(dir: &Path) -> String {
    fn walk(root: &Path, dir: &Path, out: &mut Vec<String>) {
        for e in std::fs::read_dir(dir).unwrap().flatten() {
            let p = e.path();
            let rel = p
                .strip_prefix(root)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            if e.file_type().unwrap().is_dir() {
                out.push(format!("{rel}/"));
                walk(root, &p, out);
            } else {
                out.push(rel);
            }
        }
    }
    let mut out = Vec::new();
    walk(dir, dir, &mut out);
    out.sort();
    out.join(" ")
}

#[test]
fn cleaning_keeps_what_vanilla_keeps() {
    for &(keep, expected) in VACUUM_KEPT {
        let dir = TempDir::new();
        let cache = dir.cache();
        for &(name, t) in VACUUM_FILES {
            let p = cache.join(name);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(&p, name).unwrap();
            let time = SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000 + t);
            std::fs::File::options()
                .write(true)
                .open(&p)
                .unwrap()
                .set_modified(time)
                .unwrap();
        }
        cache::vacuum(&cache, keep);
        assert_eq!(listing(&cache), expected, "keeping {keep}");
    }
}

#[test]
fn opening_the_cache_cleans_it_once_a_run() {
    let server = serve(|r| ok(r.path.as_bytes()));
    let dir = TempDir::new();
    let cache = dir.cache();
    for i in 0..25 {
        let p = cache.join(format!("old-{i:02}")).join("f");
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, b"x").unwrap();
        let time = SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000 + i);
        std::fs::File::options()
            .write(true)
            .open(&p)
            .unwrap()
            .set_modified(time)
            .unwrap();
    }
    assert!(
        one(&config(&dir), &server.url("/new"), None, &limits())
            .0
            .is_ok()
    );
    // The five oldest went before the download; the new one is the 21st.
    let left = files(&cache);
    assert_eq!(left.len(), 21);
    assert!(
        !left
            .iter()
            .any(|f| f.starts_with("old-0") && f < &"old-05".to_string())
    );
    // Not again in the same run.
    assert!(
        one(&config(&dir), &server.url("/newer"), None, &limits())
            .0
            .is_ok()
    );
    assert_eq!(files(&cache).len(), 22);
}

#[test]
fn hashes_parse_as_vanilla_parses_them() {
    let hex = "0123456789abcdefABCDEF0123456789abcdef01";
    assert_eq!(
        parse_sha1(hex),
        Some([
            0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef, 0xab, 0xcd, 0xef, 0x01, 0x23, 0x45,
            0x67, 0x89, 0xab, 0xcd, 0xef, 0x01
        ])
    );
    for bad in [
        "",
        &hex[1..],
        &format!("{hex}0"),
        &format!("{hex}\n"),
        &format!(" {}", &hex[1..]),
        "g123456789abcdefABCDEF0123456789abcdef01",
        "\u{ff10}123456789abcdefABCDEF0123456789abcdef0",
    ] {
        assert_eq!(parse_sha1(bad), None, "{bad:?}");
    }
}

#[test]
fn pack_urls_are_what_vanilla_tries() {
    assert!(is_pack_url("https://example.com/pack.zip"));
    assert!(is_pack_url("http://example.com"));
    assert!(is_pack_url(" HTTPS://example.com/p "));
    assert!(!is_pack_url("ftp://example.com/p"));
    assert!(!is_pack_url("example.com/p"));
    assert!(!is_pack_url("https://exa mple.com/"));
    assert!(!is_pack_url("https://[::1/"));
}

#[test]
fn zero_output_gzip_flood_is_stopped_by_wire_limit() {
    // A valid gzip stream with ~650 KiB of empty deflate blocks that decompresses to 1 byte
    // ('x'): decoded size is 1 byte, so only the raw wire-byte guard can stop it.
    let mut gz = vec![0x1f, 0x8b, 0x08, 0x00, 0, 0, 0, 0, 0x00, 0x03];
    for _ in 0..130_000 {
        gz.extend_from_slice(&[0x00, 0x00, 0x00, 0xFF, 0xFF]);
    }
    gz.extend_from_slice(&[0x01, 0x01, 0x00, 0xFE, 0xFF, b'x']);
    // CRC32 of b"x" = 0x8cdc1683, ISIZE = 1
    gz.extend_from_slice(&0x8cdc_1683u32.to_le_bytes());
    gz.extend_from_slice(&1u32.to_le_bytes());
    // Send with Content-Encoding: gzip (chunked so there is no Content-Length header).
    let mut raw = b"HTTP/1.1 200 OK\r\nContent-Encoding: gzip\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n".to_vec();
    raw.extend_from_slice(format!("{:x}\r\n", gz.len()).as_bytes());
    raw.extend_from_slice(&gz);
    raw.extend_from_slice(b"\r\n0\r\n\r\n");
    let server = serve(move |_| Reply::Raw(raw.clone()));
    let dir = TempDir::new();
    let limits = fetch::Limits {
        max_size: 16 * 1024,
        ..limits()
    };
    let (result, _) = one(&config(&dir), &server.url("/gzip-bloat"), None, &limits);
    assert_eq!(result, Err(DownloadError::TooLarge));
}

#[test]
fn cancel_token_aborts_stalled_download_immediately() {
    struct TokenProgress(CancelToken);
    impl Progress for TokenProgress {
        fn request_start(&mut self) {}
        fn download_start(&mut self, _: Option<u64>) {}
        fn downloaded(&mut self, _: u64) {}
        fn request_finished(&mut self, _: bool) {}
        fn cancel_token(&self) -> Option<CancelToken> {
            Some(self.0.clone())
        }
    }

    let server = serve(|_| Reply::Silent(Duration::from_secs(10)));
    let dir = TempDir::new();
    let token = CancelToken::new();
    let trigger = token.clone();
    let handle = thread::spawn(move || {
        thread::sleep(Duration::from_millis(60));
        trigger.cancel();
    });
    let started = std::time::Instant::now();
    let limits = fetch::Limits {
        headers: Duration::from_secs(5),
        per_read: Duration::from_secs(5),
        whole: Duration::from_secs(10),
        ..limits()
    };
    let mut progress = TokenProgress(token);
    let results = download_batch_with(
        &config(&dir),
        &[PackRequest {
            id: ID,
            url: server.url("/stall"),
            sha1: None,
        }],
        &mut progress,
        &limits,
    );
    handle.join().unwrap();
    assert_eq!(results[0].1, Err(DownloadError::Cancelled));
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "cancellation waited {:?}",
        started.elapsed()
    );
}

#[test]
fn failed_download_does_not_leave_empty_uuid_dir() {
    // Chunked, so the size is only found too large while reading, after the directory
    // was made.
    let server = serve(|_| Reply::Chunked(vec![body(5000)]));
    let dir = TempDir::new();
    let limits = fetch::Limits {
        max_size: 100,
        ..limits()
    };
    let (result, _) = one(&config(&dir), &server.url("/big"), None, &limits);
    assert_eq!(result, Err(DownloadError::TooLarge));
    assert!(
        !dir.cache().join(ID_DIR).exists(),
        "empty UUID directory should not remain after failed download"
    );
}

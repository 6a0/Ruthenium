//! A vanilla 26.3 dedicated server for one benchmark trial (`cargo xtask bench
//! dedicated`): started from a locally supplied server jar in a folder of its own under
//! the ignored `cache/`, in offline mode, bound to 127.0.0.1 on a free port, with the
//! white-list off.
//!
//! The benchmark's controller talks to it over RCON, not its console: every command
//! gets its own reply, so each step can be checked and the server's tick times read
//! back, where the console's answers would have to be picked out of the log. RCON
//! listens on loopback only, with a password made for the trial.
//!
//! The server must never outlive the task. It runs under a supervisor (this binary
//! again, `bench supervise`) that holds a pipe from the task: when the task lets go of
//! it, by stopping the server, failing, timing out or being killed, the supervisor
//! kills the server.

use std::io::{Read, Write};
use std::net::{Ipv4Addr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};

use crate::fetch_compile_deps::sha1_of;

/// The SHA-1 of 26.3's `server.jar`, from Mojang's version manifest.
pub const JAR_SHA1: &str = "33680f5f2ac32864d6d7cf5e56a705fdb3e05f4c";
/// The server's heap. Mojang gives no arguments for servers; these are fixed here so
/// every trial runs alike.
pub const JVM_ARGS: &[&str] = &["-Xms1G", "-Xmx2G"];
const EULA_URL: &str = "https://aka.ms/MinecraftEULA";
/// The kernel reports process times in these units (`USER_HZ`).
const CLOCK_TICKS_PER_SECOND: f64 = 100.0;

/// What a trial's server is started with.
#[derive(Debug)]
pub struct Settings<'a> {
    pub java: &'a Path,
    pub jar: &'a Path,
    /// Where the server jar unpacks its libraries, shared by every trial.
    pub runtime: &'a Path,
    pub view_distance: i32,
    /// The person running the task has said they accept Mojang's EULA.
    pub accept_eula: bool,
}

/// The server jar: `explicit`, else the local reference copy. It must be 26.3's.
pub fn find_jar(root: &Path, explicit: Option<&Path>) -> Result<PathBuf, String> {
    let jar = match explicit {
        Some(p) => p.to_owned(),
        None => root.join("reference/26.3/jars/server.jar"),
    };
    if !jar.is_file() {
        return Err(format!(
            "no server jar at {}: pass --server-jar <26.3 server.jar> (the dedicated server is not downloaded for you)",
            jar.display()
        ));
    }
    match sha1_of(&jar) {
        Some(sha1) if sha1 == JAR_SHA1 => std::path::absolute(&jar).map_err(|e| e.to_string()),
        Some(sha1) => Err(format!(
            "{} is not the 26.3 server (SHA-1 {sha1}, expected {JAR_SHA1})",
            jar.display()
        )),
        None => Err(format!("can't read {}", jar.display())),
    }
}

fn epoch_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}

/// Two free loopback ports, held together so they differ.
fn free_ports() -> Result<(u16, u16), String> {
    let bind = || TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).map_err(|e| e.to_string());
    let (a, b) = (bind()?, bind()?);
    let port = |l: &TcpListener| l.local_addr().map(|a| a.port()).map_err(|e| e.to_string());
    Ok((port(&a)?, port(&b)?))
}

/// A Source RCON connection, as the vanilla server speaks it: packets of a length, a
/// request id, a type and a NUL-terminated text.
#[derive(Debug)]
struct Rcon {
    stream: TcpStream,
    next_id: i32,
}

impl Rcon {
    const LOGIN: i32 = 3;
    const COMMAND: i32 = 2;

    fn connect(port: u16, password: &str) -> std::io::Result<Rcon> {
        let stream = TcpStream::connect((Ipv4Addr::LOCALHOST, port))?;
        stream.set_read_timeout(Some(Duration::from_secs(30)))?;
        stream.set_nodelay(true)?;
        let mut rcon = Rcon { stream, next_id: 1 };
        let (id, _) = rcon.exchange(Self::LOGIN, password)?;
        if id == -1 {
            return Err(std::io::Error::other(
                "the server refused the RCON password",
            ));
        }
        Ok(rcon)
    }

    /// Sends one packet and reads its answer: the id it answers and its text. The
    /// replies here are far shorter than the 4096 bytes the server splits at.
    fn exchange(&mut self, kind: i32, text: &str) -> std::io::Result<(i32, String)> {
        let id = self.next_id;
        self.next_id += 1;
        let mut packet = Vec::with_capacity(text.len() + 14);
        packet.extend((text.len() as i32 + 10).to_le_bytes());
        packet.extend(id.to_le_bytes());
        packet.extend(kind.to_le_bytes());
        packet.extend(text.as_bytes());
        packet.extend([0, 0]);
        self.stream.write_all(&packet)?;
        let mut word = [0u8; 4];
        self.stream.read_exact(&mut word)?;
        let length = i32::from_le_bytes(word);
        if !(10..=8192).contains(&length) {
            return Err(std::io::Error::other(format!(
                "an RCON packet of {length} bytes"
            )));
        }
        let mut body = vec![0u8; length as usize];
        self.stream.read_exact(&mut body)?;
        let answered = i32::from_le_bytes([body[0], body[1], body[2], body[3]]);
        let reply = String::from_utf8_lossy(&body[8..body.len() - 2]).into_owned();
        Ok((answered, reply))
    }
}

/// A running server. Dropping it kills the server if it hasn't been stopped.
#[derive(Debug)]
pub struct Server {
    /// The supervisor, and the pipe that keeps the server alive while it is held.
    supervisor: Child,
    lifeline: Option<ChildStdin>,
    rcon: Rcon,
    /// The server's own process, for sampling.
    pub pid: u32,
    pub port: u16,
    pub log: PathBuf,
    /// Every command sent and its reply, in order.
    pub commands: Vec<(String, String)>,
}

impl Server {
    /// Starts a server in `dir`, which already holds the world as `world`, and waits
    /// until it answers RCON.
    pub fn start(dir: &Path, settings: &Settings<'_>) -> Result<Server, String> {
        if !settings.accept_eula {
            return Err(format!(
                "the dedicated server needs Mojang's EULA accepted ({EULA_URL}): read it, then pass --accept-eula"
            ));
        }
        let (port, rcon_port) = free_ports()?;
        // Loopback only, and gone with the trial: unguessable enough from the clock.
        let password =
            crate::routes::fnv1a(&format!("{}-{}-{port}", epoch_ms(), std::process::id()));
        let io = |e: std::io::Error| format!("{}: {e}", dir.display());
        std::fs::write(
            dir.join("eula.txt"),
            format!("# Accepted with --accept-eula by the person who ran cargo xtask bench dedicated ({EULA_URL}).\neula=true\n"),
        )
        .map_err(io)?;
        std::fs::write(
            dir.join("server.properties"),
            properties(port, rcon_port, &password, settings.view_distance),
        )
        .map_err(io)?;
        let log = dir.join("server.log");
        let out = std::fs::File::create(&log).map_err(io)?;
        let pid_file = dir.join("server.pid");
        let _ = std::fs::remove_file(&pid_file);
        let mut supervisor = Command::new(std::env::current_exe().map_err(|e| e.to_string())?)
            .args(["bench", "supervise"])
            .arg(&pid_file)
            .arg(settings.java)
            .args(JVM_ARGS)
            .arg(format!("-DbundlerRepoDir={}", settings.runtime.display()))
            .arg("-jar")
            .arg(settings.jar)
            .arg("--nogui")
            .current_dir(dir)
            .stdin(Stdio::piped())
            .stdout(out.try_clone().map_err(io)?)
            .stderr(out)
            .spawn()
            .map_err(|e| format!("starting the server's supervisor: {e}"))?;
        let lifeline = supervisor.stdin.take();
        // From here on, an early return drops `starting` and so kills the server.
        let mut starting = Starting {
            supervisor: Some(supervisor),
            lifeline,
        };
        let started = Instant::now();
        let (pid, rcon) = loop {
            std::thread::sleep(Duration::from_millis(100));
            let child = starting
                .supervisor
                .as_mut()
                .expect("held until the server is up");
            if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
                return Err(format!(
                    "the server exited ({status}) before it was ready:\n{}",
                    tail(&log, 15)
                ));
            }
            if started.elapsed() > Duration::from_secs(180) {
                return Err(format!(
                    "the server wasn't ready after 3 minutes:\n{}",
                    tail(&log, 15)
                ));
            }
            let Some(pid) = std::fs::read_to_string(&pid_file)
                .ok()
                .and_then(|p| p.trim().parse::<u32>().ok())
            else {
                continue;
            };
            // RCON opens once the world is loaded ("Done").
            if let Ok(rcon) = Rcon::connect(rcon_port, &password) {
                break (pid, rcon);
            }
        };
        Ok(Server {
            supervisor: starting
                .supervisor
                .take()
                .expect("held until the server is up"),
            lifeline: starting.lifeline.take(),
            rcon,
            pid,
            port,
            log,
            commands: Vec::new(),
        })
    }

    pub fn address(&self) -> String {
        format!("127.0.0.1:{}", self.port)
    }

    /// Runs a console command that changes something, keeps it and its reply in
    /// [`Server::commands`], and returns the reply. Fails if the server didn't
    /// understand it or the reply lacks `expect`.
    pub fn command(&mut self, command: &str, expect: &str) -> Result<String, String> {
        let reply = self.ask(command, expect);
        let logged = reply.clone().unwrap_or_else(|e| e);
        self.commands.push((command.to_owned(), logged));
        reply
    }

    /// Runs a console command that only reads.
    fn ask(&mut self, command: &str, expect: &str) -> Result<String, String> {
        let (_, reply) = self
            .rcon
            .exchange(Rcon::COMMAND, command)
            .map_err(|e| format!("RCON {command:?}: {e}"))?;
        if reply.contains("<--[HERE]") || !reply.contains(expect) {
            return Err(format!("the server answered {command:?} with {reply:?}"));
        }
        Ok(reply)
    }

    /// The names of the players online.
    pub fn players(&mut self) -> Result<Vec<String>, String> {
        let reply = self.ask("list", "players online")?;
        let names = reply.split_once("online:").map_or("", |(_, n)| n);
        Ok(names
            .split(',')
            .map(|n| n.trim().to_owned())
            .filter(|n| !n.is_empty())
            .collect())
    }

    /// The server's own tick-time query (`tick query`): the average and the
    /// percentiles of its last hundred ticks, in milliseconds.
    pub fn tick_health(&mut self) -> Result<Value, String> {
        let reply = self.ask("tick query", "Average time per tick")?;
        let number = |label: &str| {
            let rest = &reply[reply.find(label)? + label.len()..];
            let end = rest
                .find(|c: char| !c.is_ascii_digit() && c != '.')
                .unwrap_or(rest.len());
            rest[..end].trim_end_matches('.').parse::<f64>().ok()
        };
        Ok(json!({
            "epochMs": epoch_ms(),
            "targetRate": number("Target tick rate: "),
            "averageMs": number("Average time per tick: "),
            "p50Ms": number("P50: "),
            "p95Ms": number("P95: "),
            "p99Ms": number("P99: "),
            "runningNormally": reply.contains("running normally"),
            "reply": reply,
        }))
    }

    /// The server process's resident memory and the CPU time it has used so far:
    /// `[epoch ms, RSS KiB, CPU seconds]`.
    pub fn sample(&self) -> Value {
        json!([
            epoch_ms(),
            crate::bench::rss_kib(self.pid),
            cpu_seconds(self.pid)
        ])
    }

    pub fn running(&mut self) -> bool {
        matches!(self.supervisor.try_wait(), Ok(None))
    }

    /// Stops the server with its own `stop` command, which saves the world; kills it
    /// if it hasn't exited after a minute.
    pub fn stop(mut self) -> Result<(), String> {
        let asked = self.command("stop", "Stopping");
        let limit = Instant::now() + Duration::from_secs(60);
        while Instant::now() < limit {
            if !self.running() {
                return asked.map(|_| ());
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        Err("the server didn't stop within a minute and was killed".into())
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        end(&mut self.supervisor, &mut self.lifeline);
    }
}

/// The supervisor while the server starts: dropped on an early return.
struct Starting {
    supervisor: Option<Child>,
    lifeline: Option<ChildStdin>,
}

impl Drop for Starting {
    fn drop(&mut self) {
        if let Some(supervisor) = &mut self.supervisor {
            end(supervisor, &mut self.lifeline);
        }
    }
}

/// Lets go of the lifeline, on which the supervisor kills the server and exits, and
/// waits for that.
fn end(supervisor: &mut Child, lifeline: &mut Option<ChildStdin>) {
    drop(lifeline.take());
    let limit = Instant::now() + Duration::from_secs(15);
    while matches!(supervisor.try_wait(), Ok(None)) {
        if Instant::now() > limit {
            let _ = supervisor.kill();
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    let _ = supervisor.wait();
}

/// `cargo xtask bench supervise <pid file> <program> [arguments]`: runs the program,
/// writes its process id to the file, and kills it as soon as this process's standard
/// input closes, which it does when the task that started it lets go or dies.
pub fn supervise(args: &[String]) -> Result<(), String> {
    let [pid_file, program, arguments @ ..] = args else {
        return Err("usage: cargo xtask bench supervise <pid file> <program> [arguments]".into());
    };
    let mut child = Command::new(program)
        .args(arguments)
        .stdin(Stdio::null())
        .spawn()
        .map_err(|e| format!("running {program}: {e}"))?;
    std::fs::write(pid_file, child.id().to_string()).map_err(|e| format!("{pid_file}: {e}"))?;
    let orphaned = Arc::new(AtomicBool::new(false));
    let flag = orphaned.clone();
    std::thread::spawn(move || {
        let _ = std::io::stdin().read_to_end(&mut Vec::new());
        flag.store(true, Ordering::Relaxed);
    });
    loop {
        if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
            return if status.success() {
                Ok(())
            } else {
                Err(format!("{program} exited with {status}"))
            };
        }
        if orphaned.load(Ordering::Relaxed) {
            let _ = child.kill();
            let _ = child.wait();
            return Err(format!(
                "{program} was killed: the task that started it let go"
            ));
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// `server.properties`: what the benchmark fixes, the rest at the server's defaults.
/// Players join in adventure mode; the controller's switch to creative, the last thing
/// it does to a player, is how a client knows its scene is set.
fn properties(port: u16, rcon_port: u16, password: &str, view_distance: i32) -> String {
    format!(
        "online-mode=false\n\
         server-ip=127.0.0.1\n\
         server-port={port}\n\
         white-list=false\n\
         enforce-whitelist=false\n\
         enforce-secure-profile=false\n\
         enable-rcon=true\n\
         rcon.port={rcon_port}\n\
         rcon.password={password}\n\
         broadcast-rcon-to-ops=false\n\
         view-distance={view_distance}\n\
         simulation-distance={view_distance}\n\
         gamemode=adventure\n\
         force-gamemode=true\n\
         difficulty=normal\n\
         spawn-protection=0\n\
         pause-when-empty-seconds=-1\n\
         max-players=4\n\
         level-name=world\n"
    )
}

/// The CPU time a process has used, user and system, from `/proc`.
fn cpu_seconds(pid: u32) -> Option<f64> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    // The fields after the command name, which is in brackets and may hold spaces.
    let fields: Vec<&str> = stat.rsplit_once(')')?.1.split_whitespace().collect();
    let ticks = fields.get(11)?.parse::<f64>().ok()? + fields.get(12)?.parse::<f64>().ok()?;
    Some(ticks / CLOCK_TICKS_PER_SECOND)
}

/// The last lines of a log.
pub fn tail(path: &Path, lines: usize) -> String {
    let text = std::fs::read_to_string(path).unwrap_or_default();
    let all: Vec<&str> = text.lines().collect();
    all[all.len().saturating_sub(lines)..].join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refuses_without_the_eula() {
        let settings = Settings {
            java: Path::new("java"),
            jar: Path::new("server.jar"),
            runtime: Path::new("runtime"),
            view_distance: 12,
            accept_eula: false,
        };
        let dir = std::env::temp_dir().join("ruthenium-xtask-no-eula");
        let e = Server::start(&dir, &settings).unwrap_err();
        assert!(e.contains("--accept-eula") && e.contains(EULA_URL));
        // Nothing was written, least of all an eula.txt.
        assert!(!dir.exists());
    }

    #[test]
    fn properties_keep_the_server_local() {
        let p = properties(25001, 25002, "secret", 12);
        for line in [
            "online-mode=false",
            "server-ip=127.0.0.1",
            "server-port=25001",
            "white-list=false",
            "rcon.port=25002",
            "view-distance=12",
            "simulation-distance=12",
        ] {
            assert!(p.lines().any(|l| l == line), "{line}");
        }
    }

    #[test]
    fn free_ports_differ() {
        let (a, b) = free_ports().unwrap();
        assert_ne!(a, b);
    }
}

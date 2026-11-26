//! The statuses and requests of [`ServerPacks`] against vanilla's `ServerPackManager`:
//! `ServerPackGolden.java` drives vanilla's class (with `DownloadedPackSource`'s
//! scheduler and reload logic and `Minecraft`'s reload queue copied around it) through
//! random pushes, pops, prompt answers, task runs, download results (some failed, some
//! files without valid metadata), reload results, frame starts and disconnects, and
//! writes what each run told the server and asked for. Both sides draw the events from
//! the same generator; `golden.rs` holds the first runs in full and hashes of the rest.

use super::golden::{BATCHES, FULL};
use super::*;

struct Lcg(u64);

impl Lcg {
    fn rnd(&mut self, n: u64) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (self.0 >> 33) % n
    }
}

fn id(k: u64) -> u128 {
    (u128::from(0x1000 + k) << 64) | u128::from(0x2000 + k)
}

fn idx(id: u128) -> u64 {
    ((id >> 64) as u64) - 0x1000
}

struct Driver {
    r: Lcg,
    out: String,
    packs: ServerPacks<u64>,
    outstanding: Vec<Batch>,
}

impl Driver {
    fn flush(&mut self) {
        for e in self.packs.take_effects() {
            match e {
                Effect::Feedback(id, f) => {
                    self.out += &format!("F {} {}\n", idx(id), f.name());
                }
                Effect::Download(b) => {
                    self.out += &format!("D {}", b.batch);
                    for r in &b.requests {
                        self.out += &format!(" {}", idx(r.id));
                    }
                    self.out += "\n";
                    self.outstanding.push(b);
                }
                Effect::Reload(r) => {
                    self.out += &format!("R {}", u8::from(r.recovery));
                    for k in &r.packs {
                        self.out += &format!(" {k}");
                    }
                    self.out += "\n";
                }
                Effect::Abort => self.out += "A\n",
            }
        }
    }

    fn configure(&mut self, s: u64) {
        self.packs.configure_for_server_control(match s {
            0 => PromptStatus::Pending,
            1 => PromptStatus::Allowed,
            _ => PromptStatus::Declined,
        });
    }

    fn step(&mut self) {
        let op = self.r.rnd(24);
        if std::env::var_os("TRACE").is_some() {
            self.out += &format!("op {op}\n");
        }
        match op {
            0..=3 => {
                let k = self.r.rnd(4);
                let hash = (self.r.rnd(2) == 0).then_some([k as u8 + 1; 20]);
                self.packs.push(id(k), format!("http://h/{k}"), hash);
            }
            4 => {
                let k = self.r.rnd(4);
                self.packs.pop(id(k));
            }
            5 => self.packs.pop_all(),
            6 => self.packs.allow(),
            7 => self.packs.reject(),
            8..=11 | 21..=23 => self.packs.run_tasks(),
            12..=15 => {
                if !self.outstanding.is_empty() {
                    let at = self.r.rnd(self.outstanding.len() as u64) as usize;
                    let b = self.outstanding.remove(at);
                    let result = b
                        .requests
                        .iter()
                        .enumerate()
                        .map(|(i, req)| {
                            let ok = self.r.rnd(5) != 0;
                            let broken = self.r.rnd(6) == 0;
                            let path = format!(
                                "/c/{}/{i}{}",
                                b.batch,
                                if broken { "-broken" } else { "" }
                            );
                            (req.id, if ok { Ok(PathBuf::from(path)) } else { Err(()) })
                        })
                        .collect();
                    self.packs.downloaded(b.batch, result);
                }
            }
            16..=18 => {
                if self.packs.reloading() {
                    let ok = self.r.rnd(4) != 0;
                    self.packs.reloaded(ok);
                }
            }
            19 => self.packs.frame_start(),
            _ => {
                if self.r.rnd(2) == 0 {
                    self.flush();
                    self.out += "X\n";
                    self.packs.cleanup_after_disconnect();
                    let s = self.r.rnd(3);
                    self.configure(s);
                } else {
                    self.packs.run_tasks();
                }
            }
        }
        self.flush();
    }
}

/// `loadRequestedPacks`: a file whose name says so has no valid metadata.
fn open(packs: &[(u128, PathBuf)]) -> Option<Vec<u64>> {
    packs
        .iter()
        .rev()
        .map(|(id, path)| (!path.to_string_lossy().contains("broken")).then(|| idx(*id)))
        .collect::<Option<Vec<u64>>>()
        .map(|mut v| {
            v.reverse();
            v
        })
}

fn run(case: u64) -> String {
    let mut d = Driver {
        r: Lcg(case.wrapping_mul(0x9E37_79B9_7F4A_7C15).wrapping_add(1)),
        out: String::new(),
        packs: ServerPacks::new(open),
        outstanding: Vec::new(),
    };
    let s = [0, 1, 1, 2][d.r.rnd(4) as usize];
    d.configure(s);
    d.flush();
    for _ in 0..60 {
        d.step();
    }
    d.out
}

fn fnv(s: &str) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in s.bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

/// Prints one case's transcript (`CASE=n`, with `TRACE=1` the events drawn too), to
/// set beside `java ServerPackGolden x n` (`-Dtrace=1`) when they differ.
#[test]
fn print_case() {
    if let Some(c) = std::env::var_os("CASE") {
        print!("{}", run(c.to_str().unwrap().parse().unwrap()));
    }
}

#[test]
fn the_statuses_are_vanillas() {
    for (case, want) in FULL.iter().enumerate() {
        let got = run(case as u64);
        assert_eq!(got, *want, "case {case}");
    }
    let per = 100;
    for (b, want) in BATCHES.iter().enumerate() {
        let start = FULL.len() + b * per;
        let mut all = String::new();
        for c in start..start + per {
            all += &run(c as u64);
            all.push('|');
        }
        assert_eq!(fnv(&all), *want, "batch {b} (cases {start}..)");
    }
}

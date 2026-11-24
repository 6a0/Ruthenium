//! The routes of the dedicated-server benchmark (`cargo xtask bench dedicated`): one
//! definition, written to a file per trial that Ruthenium (`ruthenium connect --route`)
//! and the vanilla bench client (`tools/java/bench`) both read, and the checks on what
//! each of them recorded.
//!
//! A route is keyed by client tick. Tick 0 is the first tick after the player has stood
//! at the start for a second with the start's chunks loaded. Before tick n, the turn of
//! its segment is applied (`Entity.turn`'s amounts) and its keys are held; after it, the
//! client records the position and rotation. The warm-up is the first ticks, the
//! capture the ones after, and a short tail without input follows. The clients record
//! the loaded chunks (a mark) at tick 0, every [`MARK_EVERY`] ticks up to the capture's
//! end, and once more when the player and the chunks have come to rest.
//!
//! Two checksums identify a run, both FNV-1a (64 bits) of text:
//! - positions: every tick's record one after the other, each the bits of x, y, z
//!   (doubles) and of the yaw and pitch (floats) as 64 hex digits;
//! - footprint: the first mark's chunks as `x,z;` each (sorted by x, then z), a `|`,
//!   and the last mark's.
//!
//! The marks in between are taken while chunks stream in, so they are checked against
//! the expected footprint with a tolerance and left out of the checksum.

use serde_json::{Value, json};

pub const SCENES: &[&str] = &["idle", "spin", "walk", "fly"];
/// Ticks without input after the capture, before the end is recorded.
const TAIL_TICKS: u64 = 40;
/// Ticks between marks.
pub const MARK_EVERY: u64 = 100;
/// The fly route starts this far above the spawn, as the integrated-server scene does.
const FLY_HEIGHT: f64 = 40.0;
const TICKS_PER_SECOND: f64 = 20.0;

/// A stretch of a route: ticks, keys held, turn before each tick.
type Segment = (u64, &'static [&'static str], (f64, f64));

#[derive(Debug, Clone)]
pub struct Route {
    pub name: String,
    pub id: String,
    pub position: [f64; 3],
    pub yaw: f32,
    pub pitch: f32,
    pub flying: bool,
    stationary: bool,
    pub warmup_ticks: u64,
    pub capture_ticks: u64,
    pub view_distance: i32,
    segments: Vec<Segment>,
}

impl Route {
    /// The route of `scene` in a world whose spawn block is `spawn`, for a warm-up and a
    /// capture of this many seconds.
    ///
    /// - idle: standing at the spawn, facing south.
    /// - spin: turning on the spot, 12 counts (1.8 degrees) a tick: 36 degrees a second.
    /// - walk: forward with jump held (to climb and swim), turning 4 counts (0.6
    ///   degrees) a tick: a circle about 21 blocks in radius where nothing is in the way,
    ///   so the route stays near the spawn however long it is.
    /// - fly: creative sprint-flight south, 40 blocks above the spawn.
    pub fn new(
        scene: &str,
        spawn: [f64; 3],
        warmup: f64,
        duration: f64,
        view_distance: i32,
    ) -> Result<Route, String> {
        let (keys, turn, flying): (&'static [&'static str], _, _) = match scene {
            "idle" => (&[], (0.0, 0.0), false),
            "spin" => (&[], (12.0, 0.0), false),
            "walk" => (&["forward", "jump"], (4.0, 0.0), false),
            "fly" => (&["forward", "sprint"], (0.0, 0.0), true),
            other => return Err(format!("no route for the scene {other:?}")),
        };
        let warmup_ticks = (warmup * TICKS_PER_SECOND).round() as u64;
        let capture_ticks = (duration * TICKS_PER_SECOND).round() as u64;
        if capture_ticks == 0 {
            return Err("the capture is shorter than a tick".into());
        }
        let height = if flying { FLY_HEIGHT } else { 0.0 };
        let mut route = Route {
            name: scene.into(),
            id: String::new(),
            position: [spawn[0] + 0.5, spawn[1] + height, spawn[2] + 0.5],
            yaw: 0.0,
            pitch: 0.0,
            flying,
            stationary: matches!(scene, "idle" | "spin"),
            warmup_ticks,
            capture_ticks,
            view_distance,
            segments: vec![
                (warmup_ticks + capture_ticks, keys, turn),
                (TAIL_TICKS, &[], (0.0, 0.0)),
            ],
        };
        route.id = fnv1a(&route.json().to_string());
        Ok(route)
    }

    pub fn total(&self) -> u64 {
        self.segments.iter().map(|s| s.0).sum()
    }

    /// The route file. The identifier is the checksum of this text without it.
    pub fn json(&self) -> Value {
        let start = footprint(chunk_of(self.position), self.view_distance);
        let mut v = json!({
            "format": "ruthenium-route/1",
            "name": self.name,
            "viewDistance": self.view_distance,
            "start": {
                "position": self.position,
                "yaw": self.yaw,
                "pitch": self.pitch,
                "flying": self.flying,
                "chunks": start.iter().map(|(x, z)| json!([x, z])).collect::<Vec<_>>(),
            },
            "segments": self.segments.iter().map(|(ticks, keys, turn)| json!({
                "ticks": ticks,
                "keys": keys,
                "turn": [turn.0, turn.1],
            })).collect::<Vec<_>>(),
            "warmupTicks": self.warmup_ticks,
            "captureTicks": self.capture_ticks,
            "markEvery": MARK_EVERY,
            "stationary": self.stationary,
        });
        if !self.id.is_empty() {
            v["id"] = json!(self.id);
        }
        v
    }

    fn turn(&self, tick: u64) -> (f64, f64) {
        let mut first = 0;
        for (ticks, _, turn) in &self.segments {
            if tick < first + ticks {
                return *turn;
            }
            first += ticks;
        }
        (0.0, 0.0)
    }

    /// The ticks at which the clients record the loaded chunks.
    fn mark_ticks(&self) -> Vec<u64> {
        let capture_end = self.warmup_ticks + self.capture_ticks;
        let mut ticks: Vec<u64> = (0..=capture_end / MARK_EVERY)
            .map(|k| k * MARK_EVERY)
            .collect();
        ticks.push(self.total());
        ticks
    }
}

/// FNV-1a, 64 bits, as 16 hex digits.
pub fn fnv1a(text: &str) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for b in text.bytes() {
        hash = (hash ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{hash:016x}")
}

fn chunk_of(position: [f64; 3]) -> (i32, i32) {
    (
        (position[0].floor() as i32) >> 4,
        (position[2].floor() as i32) >> 4,
    )
}

/// The chunks a vanilla server keeps loaded on a client at `center` with this view
/// distance, sorted: the square out to the view distance plus one, without the corners
/// farther than the view distance from the square two chunks around the centre.
pub fn footprint(center: (i32, i32), view_distance: i32) -> Vec<(i32, i32)> {
    let reach = view_distance + 1;
    let mut chunks = Vec::new();
    for x in -reach..=reach {
        for z in -reach..=reach {
            let (dx, dz) = ((x.abs() - 2).max(0), (z.abs() - 2).max(0));
            if dx * dx + dz * dz < view_distance * view_distance {
                chunks.push((center.0 + x, center.1 + z));
            }
        }
    }
    chunks.sort_unstable();
    chunks
}

fn chunk_text(chunks: &[(i32, i32)]) -> String {
    chunks.iter().map(|(x, z)| format!("{x},{z};")).collect()
}

/// A tick's record back as position, yaw and pitch.
fn decode(record: &str) -> Option<([f64; 3], f32, f32)> {
    if record.len() != 64 || !record.is_ascii() {
        return None;
    }
    let double = |i: usize| {
        u64::from_str_radix(&record[i..i + 16], 16)
            .map(f64::from_bits)
            .ok()
    };
    let float = |i: usize| {
        u32::from_str_radix(&record[i..i + 8], 16)
            .map(f32::from_bits)
            .ok()
    };
    Some((
        [double(0)?, double(16)?, double(32)?],
        float(48)?,
        float(56)?,
    ))
}

fn position_of(v: &Value) -> Option<[f64; 3]> {
    Some([v[0].as_f64()?, v[1].as_f64()?, v[2].as_f64()?])
}

fn same(a: [f64; 3], b: [f64; 3]) -> bool {
    a.map(f64::to_bits) == b.map(f64::to_bits)
}

/// What a client's record of a route came to, once checked against the route.
#[derive(Debug, Clone)]
pub struct Checked {
    pub positions: String,
    pub footprint: String,
    ticks: Vec<String>,
    pub end_position: [f64; 3],
    pub start_chunks: usize,
    pub end_chunks: usize,
    /// The most chunks of the expected footprint missing at a mark taken on the move.
    pub most_missing: usize,
}

/// Checks a client's record (the `ruthenium-route-run/1` object) against the route: that
/// it is of this route and complete, that the player joined, faced where the route
/// turns it on every tick, stood still where the route does, that the loaded chunks at
/// the start and at rest are exactly the server's footprint around the player and
/// nearly so on the move, and that the checksums are those of the records.
pub fn check(route: &Route, run: &Value) -> Result<Checked, String> {
    if let Some(e) = run["error"].as_str() {
        return Err(e.to_owned());
    }
    if run["format"] != "ruthenium-route-run/1" {
        return Err(format!("no route record (format {})", run["format"]));
    }
    if run["route"]["id"] != route.id.as_str() {
        return Err(format!(
            "the record is of route {}, not {}",
            run["route"]["id"], route.id
        ));
    }
    position_of(&run["joinPosition"]).ok_or("the player never joined")?;
    let end_position = position_of(&run["endPosition"]).ok_or("no end position")?;
    let ticks: Vec<String> = run["ticks"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|t| t.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default();
    if ticks.len() as u64 != route.total() {
        return Err(format!(
            "{} ticks recorded of {}",
            ticks.len(),
            route.total()
        ));
    }
    let mut rotation = (route.yaw, route.pitch);
    for (tick, record) in ticks.iter().enumerate() {
        let (position, yaw, pitch) =
            decode(record).ok_or_else(|| format!("tick {tick}: unreadable record"))?;
        // Entity.turn, in floats as the game adds them.
        let (xo, yo) = route.turn(tick as u64);
        if (xo, yo) != (0.0, 0.0) {
            rotation = (
                rotation.0 + xo as f32 * 0.15,
                ((rotation.1 + yo as f32 * 0.15) % 360.0).clamp(-90.0, 90.0),
            );
        }
        if (yaw.to_bits(), pitch.to_bits()) != (rotation.0.to_bits(), rotation.1.to_bits()) {
            return Err(format!(
                "tick {tick}: the player faces {yaw} {pitch}, the route says {} {}",
                rotation.0, rotation.1
            ));
        }
        if route.stationary && !same(position, route.position) {
            return Err(format!(
                "tick {tick}: the player is at {position:?} on a route that stands at {:?}",
                route.position
            ));
        }
    }
    let positions = fnv1a(&ticks.concat());
    if run["checksums"]["positions"] != positions.as_str() {
        return Err(format!(
            "the client's position checksum {} is not that of its records ({positions})",
            run["checksums"]["positions"]
        ));
    }

    let marks = run["marks"].as_array().ok_or("no marks")?;
    let at: Vec<u64> = marks.iter().filter_map(|m| m["tick"].as_u64()).collect();
    if at != route.mark_ticks() {
        return Err(format!(
            "marks at ticks {at:?}, the route has {:?}",
            route.mark_ticks()
        ));
    }
    // On the move, the chunks of the two rows ahead may still be on their way and the
    // two behind not yet dropped.
    let slack = 2 * (2 * (route.view_distance as usize + 1) + 1);
    let mut most_missing = 0;
    let mut ends = Vec::new();
    for (i, mark) in marks.iter().enumerate() {
        let tick = at[i];
        let position = position_of(&mark["position"]).ok_or("a mark without a position")?;
        let mut chunks: Vec<(i32, i32)> = mark["chunks"]
            .as_array()
            .ok_or("a mark without chunks")?
            .iter()
            .filter_map(|c| Some((c[0].as_i64()? as i32, c[1].as_i64()? as i32)))
            .collect();
        chunks.sort_unstable();
        let expected = footprint(chunk_of(position), route.view_distance);
        let missing = expected
            .iter()
            .filter(|c| chunks.binary_search(c).is_err())
            .count();
        let extra = chunks
            .iter()
            .filter(|c| expected.binary_search(c).is_err())
            .count();
        let at_rest = i == 0 || i + 1 == marks.len();
        if at_rest && (missing, extra) != (0, 0) {
            return Err(format!(
                "at tick {tick}, at rest at {position:?}: {} chunks loaded, {missing} of the footprint's {} missing and {extra} outside it",
                chunks.len(),
                expected.len()
            ));
        }
        if missing > slack || extra > slack {
            return Err(format!(
                "at tick {tick}, at {position:?}: {missing} of the footprint's {} chunks missing and {extra} outside it (more than two rows, {slack})",
                expected.len()
            ));
        }
        if at_rest {
            ends.push((position, chunks));
        } else {
            most_missing = most_missing.max(missing);
        }
    }
    let [(start, start_chunks), (end, end_chunks)] = &ends[..] else {
        return Err("fewer than two marks".into());
    };
    if !same(*start, route.position) {
        return Err(format!(
            "the route began at {start:?}, not at its start {:?}",
            route.position
        ));
    }
    if !same(*end, end_position) {
        return Err("the last mark is not at the end position".into());
    }
    let footprint = fnv1a(&format!(
        "{}|{}",
        chunk_text(start_chunks),
        chunk_text(end_chunks)
    ));
    if run["checksums"]["footprint"] != footprint.as_str() {
        return Err(format!(
            "the client's footprint checksum {} is not that of its marks ({footprint})",
            run["checksums"]["footprint"]
        ));
    }
    for marker in [
        "joinedEpochMs",
        "sceneReadyEpochMs",
        "warmupStartEpochMs",
        "captureStartEpochMs",
        "captureEndEpochMs",
        "routeEndEpochMs",
    ] {
        if run["markers"][marker].as_u64().is_none() {
            return Err(format!("no {marker} marker"));
        }
    }
    Ok(Checked {
        positions,
        footprint,
        ticks,
        end_position,
        start_chunks: start_chunks.len(),
        end_chunks: end_chunks.len(),
        most_missing,
    })
}

/// Whether two checked records of one route are the same run: the same position on
/// every tick, the same place at rest, the same chunks at the start and the end.
pub fn compare(a: &Checked, b: &Checked) -> Result<(), String> {
    if let Some(tick) = (0..a.ticks.len().min(b.ticks.len())).find(|&i| a.ticks[i] != b.ticks[i]) {
        let show = |r: &str| {
            decode(r).map_or_else(String::new, |(p, yaw, pitch)| {
                format!("{} {} {} facing {yaw} {pitch}", p[0], p[1], p[2])
            })
        };
        return Err(format!(
            "the positions differ from tick {tick} on: {} against {}",
            show(&a.ticks[tick]),
            show(&b.ticks[tick])
        ));
    }
    if a.positions != b.positions {
        return Err(format!(
            "position checksums {} and {}",
            a.positions, b.positions
        ));
    }
    if !same(a.end_position, b.end_position) {
        return Err(format!(
            "the player came to rest at {:?} against {:?}",
            a.end_position, b.end_position
        ));
    }
    if a.footprint != b.footprint {
        return Err(format!(
            "footprint checksums {} ({} and {} chunks) and {} ({} and {} chunks)",
            a.footprint, a.start_chunks, a.end_chunks, b.footprint, b.start_chunks, b.end_chunks
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn footprint_is_a_square_without_its_far_corners() {
        let f = footprint((0, 0), 12);
        // 27 by 27, minus the corner chunks farther than 12 from the inner square.
        assert!(
            f.contains(&(13, 0))
                && f.contains(&(-13, 6))
                && !f.contains(&(-13, 7))
                && !f.contains(&(14, 0))
        );
        assert!(f.contains(&(10, 10)) && !f.contains(&(13, 13)) && !f.contains(&(11, 11)));
        assert_eq!(f.len(), 637);
        assert!(f.is_sorted());
        assert_eq!(footprint((3, -2), 12).len(), f.len());
    }

    #[test]
    fn routes_have_a_stable_identity() {
        let fly = Route::new("fly", [4.0, 71.0, 2.0], 10.0, 60.0, 12).unwrap();
        assert_eq!(fly.position, [4.5, 111.0, 2.5]);
        assert_eq!(
            (fly.total(), fly.warmup_ticks, fly.capture_ticks),
            (1440, 200, 1200)
        );
        assert_eq!(fly.mark_ticks().len(), 16);
        assert_eq!(fly.mark_ticks()[14..], [1400, 1440]);
        assert_eq!(
            fly.id,
            Route::new("fly", [4.0, 71.0, 2.0], 10.0, 60.0, 12)
                .unwrap()
                .id
        );
        assert_ne!(
            fly.id,
            Route::new("fly", [4.0, 71.0, 2.0], 10.0, 30.0, 12)
                .unwrap()
                .id
        );
        assert_eq!(fly.json()["id"], fly.id.as_str());
        assert!(Route::new("menu", [0.0; 3], 1.0, 1.0, 12).is_err());
    }

    /// A record as a client would write it for `route`, standing still.
    fn standing(route: &Route) -> Value {
        let mut rotation = (route.yaw, route.pitch);
        let ticks: Vec<String> = (0..route.total())
            .map(|t| {
                let (xo, _) = route.turn(t);
                if xo != 0.0 {
                    rotation.0 += xo as f32 * 0.15;
                }
                let p = route.position;
                format!(
                    "{:016x}{:016x}{:016x}{:08x}{:08x}",
                    p[0].to_bits(),
                    p[1].to_bits(),
                    p[2].to_bits(),
                    rotation.0.to_bits(),
                    rotation.1.to_bits()
                )
            })
            .collect();
        let chunks = footprint(chunk_of(route.position), route.view_distance);
        let list: Vec<Value> = chunks.iter().map(|(x, z)| json!([x, z])).collect();
        let marks: Vec<Value> = route
            .mark_ticks()
            .iter()
            .map(|t| json!({ "tick": t, "position": route.position, "chunks": list }))
            .collect();
        let text = chunk_text(&chunks);
        json!({
            "format": "ruthenium-route-run/1",
            "route": { "id": route.id },
            "joinPosition": route.position,
            "endPosition": route.position,
            "ticks": ticks,
            "marks": marks,
            "markers": {
                "joinedEpochMs": 1, "sceneReadyEpochMs": 2, "warmupStartEpochMs": 2,
                "captureStartEpochMs": 3, "captureEndEpochMs": 4, "routeEndEpochMs": 5,
            },
            "checksums": {
                "positions": fnv1a(&ticks.concat()),
                "footprint": fnv1a(&format!("{text}|{text}")),
            },
            "error": null,
        })
    }

    #[test]
    fn checks_a_record_against_its_route() {
        let spin = Route::new("spin", [4.0, 71.0, 2.0], 1.0, 2.0, 12).unwrap();
        let run = standing(&spin);
        let checked = check(&spin, &run).unwrap();
        assert_eq!((checked.start_chunks, checked.end_chunks), (637, 637));
        compare(&checked, &checked).unwrap();

        // Not turning where the route turns.
        let idle = Route::new("idle", [4.0, 71.0, 2.0], 1.0, 2.0, 12).unwrap();
        let mut wrong = standing(&idle);
        wrong["route"]["id"] = json!(spin.id);
        wrong["checksums"] = run["checksums"].clone();
        assert!(check(&spin, &wrong).unwrap_err().contains("faces"));

        // A chunk short at rest.
        let mut short = run.clone();
        short["marks"][0]["chunks"].as_array_mut().unwrap().pop();
        assert!(
            check(&spin, &short)
                .unwrap_err()
                .contains("1 of the footprint's 637 missing")
        );

        // A checksum that isn't the records'.
        let mut forged = run.clone();
        forged["checksums"]["positions"] = json!("0000000000000000");
        assert!(
            check(&spin, &forged)
                .unwrap_err()
                .contains("position checksum")
        );

        // A client's own error, and a record of another route.
        let mut failed = run.clone();
        failed["error"] = json!("the route never started");
        assert_eq!(
            check(&spin, &failed).unwrap_err(),
            "the route never started"
        );
        assert!(check(&idle, &run).unwrap_err().contains("not"));

        // Another position on one tick is another run.
        let mut moved = checked.clone();
        moved.ticks[7] = moved.ticks[7].replacen("4012", "4013", 1);
        assert!(
            compare(&checked, &moved)
                .unwrap_err()
                .contains("from tick 7 on")
        );
    }
}

//! The packet-order checker over vanilla's own packets: the outbound stream of every
//! golden trace recorded from vanilla's client, with its frames, ticks and the packets
//! it handled, breaks no rule. A rule vanilla breaks would be a wrong rule (PLAN §1.5).
//!
//! The traces start in the middle of a session (nothing known of what was sent before),
//! record only some handled packets' bytes, and the harness runs one tick per frame; the
//! checker is given what the trace says of the selected slot, the vehicle and the screen.

use std::collections::HashMap;
use std::path::PathBuf;

use ruthenium_protocol::packets::play::Clientbound;
use ruthenium_protocol::wire::Reader;
use ruthenium_sim::packet_check::{OnViolation, PacketChecker, Violation};
use ruthenium_trace::{Record, Trace};

fn traces() -> Vec<(String, Trace)> {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/golden/26.3");
    let mut paths: Vec<PathBuf> = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("{}: {e}", dir.display()))
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "jsonl"))
        .collect();
    paths.sort();
    paths
        .iter()
        .map(|p| {
            let text = std::fs::read_to_string(p).unwrap();
            let name = p.file_stem().unwrap().to_string_lossy().into_owned();
            let t = Trace::parse(&text).unwrap_or_else(|e| panic!("{}: {e}", p.display()));
            (name, t)
        })
        .collect()
}

/// A handled packet from its recorded bytes.
fn decode(bytes: &[u8]) -> Option<Clientbound> {
    let mut r = Reader::new(bytes);
    let id = r.varint().ok()? as u32;
    Clientbound::decode(id, r.rest()).ok()
}

/// Runs the checker over a trace the way the session runs it over its own stream.
fn check(t: &Trace) -> PacketChecker {
    check_records(&t.records)
}

fn check_records(records: &[Record]) -> PacketChecker {
    let mut c = PacketChecker::in_play(OnViolation::Collect);
    let states: HashMap<i64, &Record> = records
        .iter()
        .filter(|r| r.kind == "state")
        .filter_map(|s| Some((s.tick()?, s)))
        .collect();
    let screen = |n: i64| {
        states
            .get(&n)
            .and_then(|s| s.str("screen"))
            .map(|s| !s.is_empty())
    };
    let mut in_tick = false;
    for r in records {
        match r.kind.as_str() {
            "frame" => c.frame(),
            "tick" => {
                let n = r.tick().unwrap();
                c.begin_tick(n as u64, true);
                in_tick = true;
                // The slot as the tick began, then the hotbar keys pressed in it.
                if let Some(mut slot) = r.value["slot"].as_i64() {
                    for k in 0..9 {
                        let key = format!("key.hotbar.{}", k + 1);
                        if r.value["clicks"][key.as_str()].as_u64().unwrap_or(0) > 0 {
                            slot = k;
                        }
                    }
                    c.set_selected(slot as i32);
                }
                if let Some(s) = states.get(&n) {
                    c.set_riding(s.value.get("vehicle").is_some());
                }
                // A screen open as the last tick ended and as this one ends was open
                // where this one's key binds would run.
                if screen(n - 1) == Some(true) && screen(n) == Some(true) {
                    c.set_screen(true);
                }
            }
            "state" => {
                c.end_tick();
                in_tick = false;
            }
            "out" if r.str("thread") == Some("game") && r.str("proto") == Some("play") => {
                assert_eq!(
                    r.bool("inTick"),
                    Some(in_tick),
                    "an out record's inTick disagrees with the ticks"
                );
                c.sent(&r.bytes().unwrap());
            }
            "handle" if r.str("proto") == Some("play") => {
                match r.bytes().as_deref().and_then(decode) {
                    Some(p) => c.handled(&p),
                    None => c.handled_unread(r.str("name").unwrap()),
                }
            }
            _ => {}
        }
    }
    c
}

#[test]
fn vanilla_breaks_no_rule() {
    let mut all: Vec<(String, Violation)> = Vec::new();
    let mut checked = 0;
    for (name, t) in traces() {
        let c = check(&t);
        checked += t.of_kind("out").count();
        all.extend(c.violations().iter().map(|v| (name.clone(), v.clone())));
    }
    assert!(checked > 50_000, "only {checked} packets checked");
    let shown: Vec<String> = all
        .iter()
        .take(60)
        .map(|(n, v)| format!("{n}: {v}"))
        .collect();
    assert!(
        all.is_empty(),
        "{} violations in vanilla's streams:\n{}",
        all.len(),
        shown.join("\n")
    );
}

fn trace(name: &str) -> Trace {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/golden/26.3")
        .join(format!("{name}.jsonl"));
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    Trace::parse(&text).unwrap()
}

fn is_out(r: &Record, name: &str) -> bool {
    r.kind == "out" && r.str("name") == Some(name)
}

/// The invariants a stream breaks.
fn broken(records: &[Record]) -> Vec<&'static str> {
    let mut ids: Vec<_> = check_records(records)
        .violations()
        .iter()
        .map(|v| v.invariant)
        .collect();
    ids.dedup();
    ids
}

/// The checker sees vanilla's streams broken by hand, so the run above judges them.
#[test]
fn vanilla_streams_broken_by_hand_are_caught() {
    // A tick's end left out.
    let mut r = trace("walk_directions").records;
    let i = r
        .iter()
        .position(|r| is_out(r, "minecraft:client_tick_end"))
        .unwrap();
    r.remove(i);
    // (And the next tick's position counts in the same window: the server's kick.)
    assert_eq!(broken(&r), ["INV-T1", "INV-T2"]);

    // An attack's punch left out.
    let mut r = trace("attacking").records;
    let i = r
        .iter()
        .position(|r| is_out(r, "minecraft:attack"))
        .unwrap();
    assert!(is_out(&r[i + 1], "minecraft:punch"));
    r.remove(i + 1);
    assert_eq!(broken(&r), ["INV-O3"]);

    // A sprint command sent twice.
    let mut r = trace("sprint_jump").records;
    let i = r
        .iter()
        .position(|r| is_out(r, "minecraft:player_command"))
        .unwrap();
    let again = r[i].clone();
    r.insert(i + 1, again);
    assert!(broken(&r).contains(&"INV-C2"), "{:?}", broken(&r));

    // The same held slot sent again.
    let mut r = trace("combat_basics").records;
    let i = r
        .iter()
        .position(|r| is_out(r, "minecraft:set_carried_item"))
        .unwrap();
    let again = r[i].clone();
    r.insert(i + 1, again);
    assert_eq!(broken(&r), ["INV-C1"]);

    // A teleport not accepted.
    let mut r = trace("walk_directions").records;
    if let Some(i) = r
        .iter()
        .position(|r| is_out(r, "minecraft:accept_teleportation"))
    {
        r.remove(i);
        assert!(broken(&r).contains(&"INV-C9"), "{:?}", broken(&r));
    }

    // The input sent after the movement.
    let mut r = trace("sprint_jump").records;
    let i = r
        .iter()
        .position(|r| is_out(r, "minecraft:player_input"))
        .unwrap();
    let j = i + r[i..]
        .iter()
        .position(|r| {
            r.str("name")
                .is_some_and(|n| n.starts_with("minecraft:move_player"))
        })
        .unwrap();
    let input = r.remove(i);
    r.insert(j, input);
    assert!(broken(&r).contains(&"INV-O5"), "{:?}", broken(&r));
}

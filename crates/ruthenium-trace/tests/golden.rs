//! Checks the committed golden traces (`tests/golden/26.3`) against
//! ruthenium-data's generated packet ids and against vanilla's per-tick packet
//! rules, so a trace that does not look like vanilla is caught here.

use std::collections::BTreeMap;
use std::path::PathBuf;

use ruthenium_data::v26_3::packets::play;
use ruthenium_data::v26_3::version;
use ruthenium_trace::Trace;

fn traces() -> Vec<Trace> {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/golden/26.3");
    let mut paths: Vec<PathBuf> = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("{}: {e}", dir.display()))
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "jsonl"))
        .collect();
    paths.sort();
    assert!(!paths.is_empty(), "no golden traces in {}", dir.display());
    paths
        .iter()
        .map(|p| {
            let text = std::fs::read_to_string(p).unwrap();
            Trace::parse(&text).unwrap_or_else(|e| panic!("{}: {e}", p.display()))
        })
        .collect()
}

#[test]
fn headers_match_the_generated_version() {
    for t in traces() {
        assert_eq!(t.header["game"], version::ID, "{}", t.scenario());
        assert_eq!(
            t.header["protocol"],
            version::PROTOCOL_VERSION,
            "{}",
            t.scenario()
        );
    }
}

/// Vanilla's encoder wrote these bytes; their leading packet id must be the
/// one codegen generated for the packet's name.
#[test]
fn packet_ids_match_codegen() {
    let mut checked = 0;
    for t in traces() {
        for r in t.records.iter().filter(|r| r.str("proto") == Some("play")) {
            let (Some(name), Some(id)) = (r.str("name"), r.packet_id()) else {
                continue;
            };
            let expected = match r.kind.as_str() {
                "out" => play::Serverbound::from_name(name).map(|p| p.id()),
                "handle" => play::Clientbound::from_name(name).map(|p| p.id()),
                _ => continue,
            };
            assert_eq!(expected, Some(id), "{}: {name}", t.scenario());
            checked += 1;
        }
    }
    assert!(checked > 1000, "only {checked} packets checked");
}

#[test]
fn every_tick_has_inputs_and_a_state() {
    for t in traces() {
        let ticks: Vec<i64> = t.of_kind("tick").filter_map(|r| r.tick()).collect();
        let states: Vec<i64> = t.of_kind("state").filter_map(|r| r.tick()).collect();
        assert!(!ticks.is_empty(), "{}", t.scenario());
        assert_eq!(
            ticks,
            (0..ticks.len() as i64).collect::<Vec<_>>(),
            "{}",
            t.scenario()
        );
        assert_eq!(ticks, states, "{}", t.scenario());
        for s in t.of_kind("state") {
            assert!(
                s.vec3("pos").is_some() && s.vec3("vel").is_some(),
                "{}",
                t.scenario()
            );
            assert!(
                s.f32("yRot").is_some() && s.f32("xRot").is_some(),
                "{}",
                t.scenario()
            );
        }
    }
}

/// Vanilla 26.3 ends every client tick with `client_tick_end` and sends at
/// most one movement packet per tick (the server kicks for two).
#[test]
fn tick_packets_follow_vanilla_rules() {
    for t in traces() {
        let mut per_tick: BTreeMap<i64, Vec<&str>> = BTreeMap::new();
        for r in t.of_kind("out").filter(|r| r.bool("inTick") == Some(true)) {
            per_tick
                .entry(r.tick().unwrap())
                .or_default()
                .push(r.str("name").unwrap());
        }
        let ticks = t.of_kind("tick").count() as i64;
        assert_eq!(
            per_tick.len() as i64,
            ticks,
            "{}: a tick sent nothing",
            t.scenario()
        );
        for (tick, names) in &per_tick {
            let ends = names
                .iter()
                .filter(|n| **n == "minecraft:client_tick_end")
                .count();
            assert_eq!(ends, 1, "{} tick {tick}: {names:?}", t.scenario());
            assert_eq!(
                names.last(),
                Some(&"minecraft:client_tick_end"),
                "{} tick {tick}",
                t.scenario()
            );
            let moves = names
                .iter()
                .filter(|n| n.starts_with("minecraft:move_player_"))
                .count();
            assert!(moves <= 1, "{} tick {tick}: {names:?}", t.scenario());
        }
    }
}

#[test]
fn floats_parse_exactly() {
    let t = Trace::parse(concat!(
        r#"{"k":"header","format":"ruthenium-trace/1"}"#,
        "\n",
        r#"{"k":"state","n":0,"pos":[0.8274000124096872,-59.58000001311302,0.5],"yRot":-90.0,"xRot":0.1}"#
    ))
    .unwrap();
    let s = t.of_kind("state").next().unwrap();
    assert_eq!(
        s.vec3("pos").unwrap()[0].to_bits(),
        0.8274000124096872f64.to_bits()
    );
    assert_eq!(s.f32("xRot").unwrap().to_bits(), 0.1f32.to_bits());
}

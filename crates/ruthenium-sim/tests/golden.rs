//! Replays the golden traces (`tests/golden/26.3`) through the movement code: the
//! world each scenario built, the keys held and the camera's rotation each tick, and
//! the player's state and movement packets compared with vanilla's after every tick.
//!
//! Needs the prepared game's exports: `RUTHENIUM_GAME_DIR` names `<data>/game` (as
//! `ruthenium prepare` writes it). Skipped without it.

use std::path::{Path, PathBuf};

use ruthenium_data::v26_3::{Block, BlockState};
use ruthenium_sim::blocks::{BlockTags, Blocks};
use ruthenium_sim::player::{Env, Keys, LocalPlayer, Pose};
use ruthenium_sim::world::Level;
use ruthenium_trace::Trace;

fn game() -> Option<PathBuf> {
    Some(PathBuf::from(std::env::var_os("RUTHENIUM_GAME_DIR")?))
}

/// Vanilla's block tags from the jar's data pack (plain block lists only).
fn vanilla_tags(game: &Path) -> BlockTags {
    let jar = ruthenium_assets::zip::Zip::open(&game.join("versions/26.3/client.jar")).unwrap();
    let mut tags = BlockTags::new();
    // The tags movement reads.
    #[allow(clippy::single_element_loop)]
    for tag in ["climbable"] {
        let i = jar
            .find(&format!("data/minecraft/tags/block/{tag}.json"))
            .unwrap();
        let v: serde_json::Value = serde_json::from_slice(&jar.read(i).unwrap()).unwrap();
        let names: Vec<String> = v["values"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|n| n.as_str().map(str::to_owned))
            .collect();
        tags.set(
            &format!("minecraft:{tag}"),
            names.iter().map(String::as_str),
        );
    }
    tags
}

fn trace(name: &str) -> Trace {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/golden/26.3");
    Trace::parse(&std::fs::read_to_string(root.join(format!("{name}.jsonl"))).unwrap()).unwrap()
}

/// `minecraft:oak_stairs[facing=east]` to a state id.
fn parse_state(s: &str) -> u32 {
    let (name, props) = match s.split_once('[') {
        Some((n, rest)) => (n, rest.trim_end_matches(']')),
        None => (s, ""),
    };
    let block = Block::from_name(name).unwrap_or_else(|| panic!("unknown block {name}"));
    let mut state: BlockState = block.default_state();
    for kv in props.split(',').filter(|p| !p.is_empty()) {
        let (k, v) = kv.split_once('=').unwrap();
        state = state
            .with(k, v)
            .unwrap_or_else(|| panic!("{name} has no {k}={v}"));
    }
    state.id()
}

/// The scenario's world: the flat preset, then the header's reset and build commands.
fn build_level(t: &Trace) -> Level {
    let mut level = Level::new(-64, 384);
    for cx in -3..=4 {
        for cz in -3..=3 {
            level.add_empty_chunk(cx, cz);
        }
    }
    let layers = [
        (-64, "minecraft:bedrock"),
        (-63, "minecraft:dirt"),
        (-62, "minecraft:dirt"),
        (-61, "minecraft:grass_block"),
    ];
    for (y, name) in layers {
        let s = parse_state(name);
        for x in -48..80 {
            for z in -48..64 {
                level.set_block(x, y, z, s);
            }
        }
    }
    let commands = t.header["reset"]
        .as_array()
        .unwrap()
        .iter()
        .chain(t.header["build"].as_array().unwrap());
    for c in commands {
        let c = c.as_str().unwrap();
        let w: Vec<&str> = c.split_whitespace().collect();
        let n = |i: usize| w[i].parse::<i32>().unwrap();
        match w[0] {
            "fill" => {
                let s = parse_state(w[7]);
                for x in n(1).min(n(4))..=n(1).max(n(4)) {
                    for y in n(2).min(n(5))..=n(2).max(n(5)) {
                        for z in n(3).min(n(6))..=n(3).max(n(6)) {
                            level.set_block(x, y, z, s);
                        }
                    }
                }
            }
            "setblock" => level.set_block(n(1), n(2), n(3), parse_state(w[4])),
            _ => {}
        }
    }
    level
}

fn keys(down: &serde_json::Value) -> Keys {
    let has = |k: &str| down.as_array().unwrap().iter().any(|v| v == k);
    Keys {
        forward: has("key.forward"),
        backward: has("key.back"),
        left: has("key.left"),
        right: has("key.right"),
        jump: has("key.jump"),
        shift: has("key.sneak"),
        sprint: has("key.sprint"),
    }
}

/// Movement packets `sendChanges` sends.
const SENT: [&str; 6] = [
    "minecraft:player_input",
    "minecraft:player_command",
    "minecraft:move_player_pos",
    "minecraft:move_player_pos_rot",
    "minecraft:move_player_rot",
    "minecraft:move_player_status_only",
];

/// Replays a scenario; the first tick that differs from vanilla, described.
fn replay(name: &str, blocks: &Blocks, tags: &BlockTags) -> Result<usize, String> {
    // The trace starts some ticks after the last position was sent; that count isn't
    // recorded, so try each until the first heartbeat lines up.
    let mut first = None;
    for reminder in 0..20 {
        match replay_from(name, blocks, tags, reminder) {
            Ok(n) => return Ok(n),
            Err((tick, e)) => {
                if first.is_none() {
                    first = Some(e.clone());
                }
                if tick > 25 {
                    return Err(e);
                }
            }
        }
    }
    Err(first.unwrap())
}

fn replay_from(
    name: &str,
    blocks: &Blocks,
    tags: &BlockTags,
    reminder: i32,
) -> Result<usize, (i64, String)> {
    let t = trace(name);
    let level = build_level(&t);
    let env = Env {
        level: &level,
        blocks,
        tags,
    };
    let tp: Vec<f64> = t.header["teleport"]
        .as_str()
        .unwrap()
        .split_whitespace()
        .skip(2)
        .map(|v| v.parse().unwrap())
        .collect();
    let mut p = LocalPlayer::new();
    p.entity_id = t.header["entityId"].as_i64().unwrap_or(0) as i32;
    p.position = [tp[0], tp[1], tp[2]];
    p.y_rot = tp[3] as f32;
    p.x_rot = tp[4] as f32;
    p.stand(&env);
    p.set_position_reminder(reminder);
    let mut ticks = 0;
    let records = &t.records;
    let mut i = 0;
    while i < records.len() {
        let r = &records[i];
        i += 1;
        if r.kind != "tick" {
            continue;
        }
        let n = r.tick().unwrap();
        p.y_rot = r.f32("yRot").unwrap();
        p.x_rot = r.f32("xRot").unwrap();
        p.tick(&env, keys(&r.value["down"]));
        let sent = p.send_changes();
        // Vanilla's packets and state for this tick.
        let mut want = Vec::new();
        let mut state = None;
        while i < records.len() && records[i].kind != "tick" {
            let q = &records[i];
            if q.kind == "out" && q.tick() == Some(n) && SENT.contains(&q.str("name").unwrap()) {
                want.push(q.bytes().unwrap());
            }
            if q.kind == "state" && q.tick() == Some(n) {
                state = Some(q.clone());
            }
            i += 1;
        }
        let s = state.ok_or((n, format!("tick {n}: no state")))?;
        let pose = match s.str("pose").unwrap() {
            "STANDING" => Pose::Standing,
            "CROUCHING" => Pose::Crouching,
            _ => Pose::Swimming,
        };
        let got = (
            p.position,
            p.delta,
            p.on_ground,
            p.horizontal_collision,
            p.sprinting,
            p.pose,
        );
        let exp = (
            s.vec3("pos").unwrap(),
            s.vec3("vel").unwrap(),
            s.bool("onGround").unwrap(),
            s.bool("hColl").unwrap(),
            s.bool("sprinting").unwrap(),
            pose,
        );
        if got != exp || sent != want {
            return Err((
                n,
                format!(
                    "tick {n} (heartbeat count {reminder}):\n  got  pos {:?} vel {:?} ground {} hcoll {} sprint {} pose {:?}\n  want pos {:?} vel {:?} ground {} hcoll {} sprint {} pose {:?}\n  sent {:02x?}\n  want {:02x?}",
                    got.0,
                    got.1,
                    got.2,
                    got.3,
                    got.4,
                    got.5,
                    exp.0,
                    exp.1,
                    exp.2,
                    exp.3,
                    exp.4,
                    exp.5,
                    sent,
                    want
                ),
            ));
        }
        ticks += 1;
    }
    Ok(ticks)
}

/// Scenarios the movement code must reproduce exactly. The rest need what isn't
/// ported yet (fluids, climbing, the server's attribute and entity data echoes).
const EXACT: [&str; 9] = [
    "walk_directions",
    "mouse_turn",
    "step_up",
    "sneak_edge",
    "head_hitter",
    "sprint_jump",
    "ice",
    "slime_bounce",
    "combat_basics",
];

#[test]
fn golden_traces() {
    let Some(game) = game() else {
        eprintln!("RUTHENIUM_GAME_DIR not set; skipped");
        return;
    };
    let blocks = Blocks::load(&game.join("versions/26.3/exports")).unwrap();
    let tags = vanilla_tags(&game);
    let all = [
        "walk_directions",
        "mouse_turn",
        "step_up",
        "sneak_edge",
        "head_hitter",
        "sprint_jump",
        "ice",
        "slime_bounce",
        "ladder",
        "swim",
        "combat_basics",
    ];
    let mut failed = Vec::new();
    for name in all {
        match replay(name, &blocks, &tags) {
            Ok(n) => eprintln!("{name}: all {n} ticks match"),
            Err(e) => {
                eprintln!("{name}: {e}");
                if EXACT.contains(&name) {
                    failed.push(name);
                }
            }
        }
    }
    assert!(failed.is_empty(), "differ from vanilla: {failed:?}");
}

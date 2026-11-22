//! Replays the golden traces (`tests/golden/26.3`) through the movement code: the
//! world each scenario built, the packets the client handled (entities, teams, the
//! player's motion), the keys held and the camera's rotation each tick, and the
//! player's state and movement packets compared with vanilla's after every tick.
//!
//! Needs the prepared game's exports: `RUTHENIUM_GAME_DIR` names `<data>/game` (as
//! `ruthenium prepare` writes it). Skipped without it.

use std::path::{Path, PathBuf};

use ruthenium_data::v26_3::{Block, BlockState};
use ruthenium_protocol::packets::play::{self, Clientbound};
use ruthenium_protocol::wire::Reader;
use ruthenium_sim::blocks::{BlockTags, Blocks};
use ruthenium_sim::effect::Effect;
use ruthenium_sim::entity::{Entities, EntityTypes, Local, PlayerInfos, Teams, TickContext};
use ruthenium_sim::interact::{self, GameMode, UseKeys};
use ruthenium_sim::inventory::{Hand, SLOT_OFFHAND};
use ruthenium_sim::item::Items;
use ruthenium_sim::pick::HitResult;
use ruthenium_sim::player::{Env, Keys, LocalPlayer, Pose};
use ruthenium_sim::predict::Predictions;
use ruthenium_sim::session::pick_target;
use ruthenium_sim::world::Level;
use ruthenium_trace::Trace;

fn game() -> Option<PathBuf> {
    Some(PathBuf::from(std::env::var_os("RUTHENIUM_GAME_DIR")?))
}

/// Vanilla's block tags from the jar's data pack (all of them: movement's, and the
/// ones tools' rules name), nested tags expanded.
fn vanilla_tags(game: &Path) -> BlockTags {
    let jar = ruthenium_assets::zip::Zip::open(&game.join("versions/26.3/client.jar")).unwrap();
    const DIR: &str = "data/minecraft/tags/block/";
    fn blocks(jar: &ruthenium_assets::zip::Zip, tag: &str, out: &mut Vec<String>) {
        let Some(i) = jar.find(&format!("{DIR}{tag}.json")) else {
            return;
        };
        let v: serde_json::Value = serde_json::from_slice(&jar.read(i).unwrap()).unwrap();
        for n in v["values"].as_array().unwrap() {
            let n = n.as_str().or_else(|| n["id"].as_str()).unwrap();
            match n.strip_prefix("#minecraft:") {
                Some(nested) => blocks(jar, nested, out),
                None => out.push(n.to_owned()),
            }
        }
    }
    let mut tags = BlockTags::new();
    let names: Vec<String> = jar
        .with_prefix(DIR)
        .filter_map(|i| {
            let n = jar.name(i).strip_prefix(DIR)?.strip_suffix(".json")?;
            Some(n.to_owned())
        })
        .collect();
    for tag in names {
        let mut members = Vec::new();
        blocks(&jar, &tag, &mut members);
        tags.set(
            &format!("minecraft:{tag}"),
            members.iter().map(String::as_str),
        );
    }
    tags
}

/// Vanilla's item tags from the jar's data pack, nested tags expanded.
fn vanilla_item_tags(game: &Path) -> ruthenium_sim::item::ItemTags {
    let jar = ruthenium_assets::zip::Zip::open(&game.join("versions/26.3/client.jar")).unwrap();
    const DIR: &str = "data/minecraft/tags/item/";
    fn items(jar: &ruthenium_assets::zip::Zip, tag: &str, out: &mut Vec<String>) {
        let Some(i) = jar.find(&format!("{DIR}{tag}.json")) else {
            return;
        };
        let v: serde_json::Value = serde_json::from_slice(&jar.read(i).unwrap()).unwrap();
        for n in v["values"].as_array().unwrap() {
            let n = n.as_str().or_else(|| n["id"].as_str()).unwrap();
            match n.strip_prefix("#minecraft:") {
                Some(nested) => items(jar, nested, out),
                None => out.push(n.to_owned()),
            }
        }
    }
    let mut tags = ruthenium_sim::item::ItemTags::new();
    let names: Vec<String> = jar
        .with_prefix(DIR)
        .filter_map(|i| {
            Some(
                jar.name(i)
                    .strip_prefix(DIR)?
                    .strip_suffix(".json")?
                    .to_owned(),
            )
        })
        .collect();
    for tag in names {
        let mut members = Vec::new();
        items(&jar, &tag, &mut members);
        tags.set(
            &format!("minecraft:{tag}"),
            members.iter().map(String::as_str),
        );
    }
    tags
}

/// Vanilla's block transformers (axes, shovels, hoes) from the jar's data pack.
fn vanilla_transformers(game: &Path) -> ruthenium_sim::use_on::Transformers {
    let jar = ruthenium_assets::zip::Zip::open(&game.join("versions/26.3/client.jar")).unwrap();
    let files: Vec<(String, Vec<u8>)> = jar
        .with_prefix("data/minecraft/block_transformer/")
        .map(|i| (jar.name(i).to_owned(), jar.read(i).unwrap().into_owned()))
        .collect();
    ruthenium_sim::use_on::Transformers::from_json_files(files)
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

/// The chunks the scenarios' world is built in.
const CHUNKS: [(i32, i32); 56] = {
    let mut out = [(0, 0); 56];
    let mut i = 0;
    while i < 56 {
        out[i] = (i as i32 / 7 - 3, i as i32 % 7 - 3);
        i += 1;
    }
    out
};

/// The scenario's world: the flat preset, then the header's reset and build commands.
fn build_level(t: &Trace) -> Level {
    let mut level = Level::new(-64, 384);
    for (cx, cz) in CHUNKS {
        level.add_empty_chunk(cx, cz);
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

/// The packets compared: the held slot and `player_loaded` (`gameMode.tick`), a
/// respawn request (`Gui.tick`), the attacks and the hands' use (`handleKeybinds`), then
/// the player's tick: abilities when flight toggles, then `sendChanges`'s.
const SENT: [&str; 17] = [
    "minecraft:set_carried_item",
    "minecraft:player_loaded",
    "minecraft:client_command",
    "minecraft:attack",
    "minecraft:punch",
    "minecraft:spectator_action",
    "minecraft:player_action",
    "minecraft:interact",
    "minecraft:use_item_on",
    "minecraft:use_item",
    "minecraft:player_abilities",
    "minecraft:player_input",
    "minecraft:player_command",
    "minecraft:move_player_pos",
    "minecraft:move_player_pos_rot",
    "minecraft:move_player_rot",
    "minecraft:move_player_status_only",
];

/// Replays a scenario; the first tick that differs from vanilla, described.
/// What the replay loads from the prepared game.
struct Data {
    blocks: Blocks,
    tags: BlockTags,
    types: EntityTypes,
    items: Items,
    item_tags: ruthenium_sim::item::ItemTags,
    transformers: ruthenium_sim::use_on::Transformers,
}

fn replay(name: &str, d: &Data) -> Result<usize, String> {
    // The trace starts some ticks after the last position was sent; that count isn't
    // recorded, so try each until the first heartbeat lines up.
    let mut first = None;
    for reminder in 0..20 {
        match replay_from(name, d, reminder) {
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

/// What the replayed client knows besides its player.
struct Client {
    entities: Entities,
    teams: Teams,
    players: PlayerInfos,
    /// The local player's name, and its UUID once its player list entry is seen.
    name: String,
    uuid: Option<u128>,
    /// The player has loaded into the level. After a respawn the trace says when (vanilla
    /// waits for the level to be drawn around the camera).
    loaded: bool,
    /// The death screen, while open: the ticks since it opened.
    death_screen: Option<u32>,
    /// Its Respawn button was pressed (it then stays inactive).
    respawn_pressed: bool,
    /// What the replay sent while handling packets, not yet matched with vanilla's.
    sent_outside: std::collections::VecDeque<Vec<u8>>,
}

/// A record applied before the next tick: a packet the client handled, or one it sent
/// outside a tick (a respawn request: handling a death, or the death screen's button).
enum Pending {
    Handled(Vec<u8>),
    Sent(Vec<u8>),
}

/// A request to respawn vanilla sent outside a tick: one the replay sent handling a
/// packet, else the death screen's Respawn button, which must be there and working.
fn sent_outside(c: &mut Client, vanilla: &[u8], n: i64) -> Result<(), (i64, String)> {
    let ours = match c.sent_outside.pop_front() {
        Some(ours) => ours,
        None if c.death_screen.is_some_and(|t| t >= 20) && !c.respawn_pressed => {
            c.respawn_pressed = true;
            play::client_command(play::CLIENT_COMMAND_PERFORM_RESPAWN)
        }
        None => {
            return Err((
                n,
                format!("after tick {n}: vanilla sent {vanilla:02x?} with no working death screen"),
            ));
        }
    };
    if ours != vanilla {
        return Err((
            n,
            format!("after tick {n}: sent {ours:02x?}, vanilla {vanilla:02x?}"),
        ));
    }
    Ok(())
}

/// Whether an `out` record is a respawn request sent outside a tick.
fn outside_tick_request(q: &ruthenium_trace::Record) -> bool {
    q.kind == "out"
        && q.bool("inTick") == Some(false)
        && q.str("name") == Some("minecraft:client_command")
}

/// `Inventory` slot names of `/item replace` (`hotbar.0`, `weapon.offhand`...).
fn inventory_slot(name: &str, selected: i32) -> Option<usize> {
    let (kind, n) = name.split_once('.')?;
    let n_or = |n: &str| n.parse::<usize>().ok();
    match kind {
        "hotbar" => n_or(n),
        "inventory" => n_or(n).map(|i| i + 9),
        "weapon" if n == "offhand" => Some(SLOT_OFFHAND),
        "weapon" => usize::try_from(selected).ok(),
        "armor" => match n {
            "feet" => Some(36),
            "legs" => Some(37),
            "chest" => Some(38),
            "head" => Some(39),
            _ => None,
        },
        _ => None,
    }
}

/// The sequence of the first predicted action in the trace (`use_item`,
/// `use_item_on`, starting or finishing breaking a block): the replay's prediction
/// counter starts one before it.
fn first_sequence(t: &Trace) -> Option<i32> {
    t.records.iter().find_map(|r| {
        if r.kind != "out" {
            return None;
        }
        let name = r.str("name")?;
        let b = r.bytes()?;
        let mut rd = Reader::new(&b);
        rd.varint().ok()?;
        match name {
            "minecraft:use_item" => {
                rd.varint().ok()?;
                rd.varint().ok()
            }
            "minecraft:use_item_on" => {
                rd.varint().ok()?;
                rd.i64().ok()?;
                rd.varint().ok()?;
                for _ in 0..3 {
                    rd.f32().ok()?;
                }
                rd.bool().ok()?;
                rd.bool().ok()?;
                rd.varint().ok()
            }
            "minecraft:player_action" => {
                let action = rd.varint().ok()?;
                if action != play::ACTION_START_DESTROY_BLOCK
                    && action != play::ACTION_STOP_DESTROY_BLOCK
                {
                    return None;
                }
                rd.i64().ok()?;
                rd.u8().ok()?;
                rd.varint().ok()
            }
            _ => None,
        }
    })
}

/// The level as the replayed client has it: blocks, and the ones it predicted.
struct ClientLevel {
    level: Level,
    predictions: Predictions,
}

/// A packet the client handled, as `Session` handles it (the parts that reach the
/// player, the blocks and the entities around it). A teleport's packets (breaking
/// given up) aren't compared: the scenarios don't teleport while breaking.
fn handle(
    bytes: &[u8],
    c: &mut Client,
    p: &mut LocalPlayer,
    l: &mut ClientLevel,
    game_mode: &mut GameMode,
    d: &Data,
) {
    let (types, items) = (&d.types, &d.items);
    let mut r = Reader::new(bytes);
    let id = r.varint().unwrap() as u32;
    let Ok(packet) = Clientbound::decode(id, r.rest()) else {
        panic!("can't decode handled packet {id}");
    };
    match packet {
        Clientbound::AddEntity(a) => c.entities.add(&a, types, &c.players),
        Clientbound::RemoveEntities { ids } => {
            for id in ids {
                c.entities.remove(id);
            }
        }
        Clientbound::MoveEntity {
            entity_id,
            delta,
            rotation,
            on_ground,
        } => c
            .entities
            .move_entity(entity_id, delta.as_ref(), rotation, on_ground),
        Clientbound::TeleportEntity {
            entity_id,
            change,
            on_ground,
        } => c.entities.teleport(entity_id, &change, on_ground),
        Clientbound::EntityPositionSync {
            entity_id,
            path,
            y_rot,
            x_rot,
            on_ground,
        } => c
            .entities
            .position_sync(entity_id, &path, y_rot, x_rot, on_ground),
        Clientbound::SetEntityData {
            entity_id, values, ..
        } => {
            if entity_id == p.entity_id {
                p.set_data(&values, types);
            } else {
                c.entities.set_data(entity_id, &values, types);
            }
        }
        Clientbound::PlayerAbilities {
            flags,
            flying_speed,
            walking_speed,
        } => p.abilities.apply(flags, flying_speed, walking_speed),
        Clientbound::UpdateMobEffect {
            entity_id,
            effect,
            amplifier,
            duration,
            flags,
        } if entity_id == p.entity_id => p.update_effect(Effect {
            id: effect,
            amplifier,
            duration,
            flags,
        }),
        Clientbound::RemoveMobEffect { entity_id, effect } if entity_id == p.entity_id => {
            p.remove_effect(effect)
        }
        // CHANGE_GAME_MODE.
        Clientbound::GameEvent { event: 3, param } => p.set_game_type((param + 0.5).floor() as i32),
        Clientbound::PlayerInfoUpdate { actions, entries } => {
            if let Some(e) = entries
                .iter()
                .find(|e| e.name.as_deref() == Some(c.name.as_str()))
            {
                c.uuid = Some(e.uuid);
            }
            if let Some(mode) = c
                .uuid
                .and_then(|u| c.players.changed_game_mode(u, actions, &entries))
            {
                p.on_game_mode_changed(mode);
            }
            c.players.update(actions, &entries);
            if let Some(info) = c.uuid.and_then(|u| c.players.get(u)) {
                p.spectator = info.game_mode == 3;
                p.creative = info.game_mode == 1;
            }
        }
        Clientbound::SetPassengers {
            vehicle,
            passengers,
        } => c.entities.set_passengers(vehicle, &passengers),
        Clientbound::SetPlayerTeam {
            name,
            method,
            parameters,
            players,
        } => c.teams.apply(&name, method, parameters.as_ref(), &players),
        Clientbound::UpdateAttributes {
            entity_id,
            attributes,
        } => {
            if entity_id == p.entity_id {
                p.apply_attributes(&attributes);
            } else {
                c.entities.update_attributes(entity_id, &attributes);
            }
        }
        Clientbound::SetEntityMotion {
            entity_id,
            movement,
        } => {
            if entity_id == p.entity_id {
                p.delta = movement;
            } else {
                c.entities.set_motion(entity_id, movement);
            }
        }
        Clientbound::SetHealth {
            health,
            food,
            saturation,
        } => {
            p.health = health;
            p.food = food;
            p.saturation = saturation;
        }
        Clientbound::SetHeldSlot { slot } if (0..9).contains(&slot) => p.inventory.selected = slot,
        // `handleRespawn` in the same level: a new player that ticks last and waits for
        // the level to load; the death screen closes.
        Clientbound::Respawn {
            spawn,
            data_to_keep,
        } => {
            c.entities.local_respawned();
            *p = p.respawned(data_to_keep);
            p.set_game_type(spawn.game_type);
            c.loaded = false;
            c.death_screen = None;
            c.respawn_pressed = false;
        }
        // `handlePlayerCombatKill`: the death screen, or a respawn request at once.
        Clientbound::PlayerCombatKill { player_id, .. } if player_id == p.entity_id => {
            if p.show_death_screen {
                c.death_screen = Some(0);
            } else {
                c.sent_outside
                    .push_back(play::client_command(play::CLIENT_COMMAND_PERFORM_RESPAWN));
            }
        }
        // IMMEDIATE_RESPAWN.
        Clientbound::GameEvent { event: 11, param } => p.show_death_screen = param == 0.0,
        Clientbound::PlayerPosition(t) => {
            p.teleport(&t);
            l.predictions.on_teleport();
            game_mode.stop_destroy_block(p, &mut Vec::new());
        }
        Clientbound::BlockUpdate { pos, state } => {
            l.predictions.server_block(&mut l.level, pos, state)
        }
        Clientbound::SectionBlocksUpdate { section, changes } => {
            for (rel, state) in changes {
                let rel = i32::from(rel);
                let pos = [
                    section[0] * 16 + (rel >> 8 & 15),
                    section[1] * 16 + (rel & 15),
                    section[2] * 16 + (rel >> 4 & 15),
                ];
                l.predictions.server_block(&mut l.level, pos, state);
            }
        }
        Clientbound::BlockChangedAck { sequence } => {
            l.predictions
                .acknowledge(sequence, &mut l.level, &d.blocks, p)
        }
        Clientbound::EntityEvent { entity_id, event }
            if entity_id == p.entity_id && (24..=28).contains(&event) =>
        {
            p.permission_level = i32::from(event) - 24
        }
        Clientbound::ContainerSetContent {
            container_id: 0,
            state_id,
            items: stacks,
            carried,
        } => {
            let stacks = stacks.iter().map(|s| items.stack(s)).collect();
            p.inventory
                .initialize(state_id, stacks, items.stack(&carried));
        }
        Clientbound::ContainerSetSlot {
            container_id: 0,
            state_id,
            slot,
            item,
        } => p
            .inventory
            .set_menu_slot(i32::from(slot), state_id, items.stack(&item)),
        Clientbound::SetPlayerInventory { slot, item } => {
            if let Ok(slot) = usize::try_from(slot) {
                p.inventory.set(slot, items.stack(&item));
            }
        }
        Clientbound::SetCursorItem { item } => p.inventory.carried = items.stack(&item),
        Clientbound::Cooldown { group, duration } => {
            if duration == 0 {
                p.cooldowns.remove(&group);
            } else {
                p.cooldowns.add(&group, duration);
            }
        }
        Clientbound::EntityEvent {
            entity_id,
            event: 9,
        } if entity_id == p.entity_id => p.complete_using_item(),
        _ => {}
    }
}

fn replay_from(name: &str, d: &Data, reminder: i32) -> Result<usize, (i64, String)> {
    let (blocks, tags, items) = (&d.blocks, &d.tags, &d.items);
    let t = trace(name);
    let mut cl = ClientLevel {
        level: build_level(&t),
        predictions: Predictions::new(),
    };
    let env = Env {
        level: &cl.level,
        blocks,
        tags,
        colliders: &[],
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
    p.old_position = p.position;
    p.old_rot = (p.y_rot, p.x_rot);
    // What the build commands put in the inventory before the trace began.
    for c in t.header["build"].as_array().unwrap() {
        let w: Vec<&str> = c.as_str().unwrap().split_whitespace().collect();
        // `item replace entity @a <slot> with <item> [count]`.
        if w.len() >= 7 && w[0] == "item" && w[1] == "replace" && w[5] == "with" {
            let item = w[6].split('[').next().unwrap();
            let id = ruthenium_data::v26_3::registries::Item::from_name(item)
                .unwrap()
                .id();
            let count = w.get(7).map_or(1, |n| n.parse().unwrap());
            // The one component a scenario gives: a potion's contents.
            let mut added = Vec::new();
            if let Some(name) = w[6]
                .split_once("potion_contents={potion:\"")
                .and_then(|(_, rest)| rest.split('"').next())
            {
                let potion = ruthenium_data::v26_3::registries::Potion::from_name(name).unwrap();
                added.push((
                    ruthenium_data::v26_3::registries::DataComponentType::PotionContents.id()
                        as i32,
                    ruthenium_protocol::item::Component::PotionContents {
                        potion: Some(potion.id() as i32),
                    },
                ));
            }
            let stack = items.stack(&ruthenium_protocol::item::ItemStack {
                item: id as i32,
                count,
                patch: ruthenium_protocol::item::ComponentPatch {
                    added,
                    removed: Vec::new(),
                },
            });
            if let Some(slot) = inventory_slot(w[4], p.inventory.selected) {
                p.inventory.set(slot, stack);
            }
        }
    }
    // The client's item use: the first tick's slot was sent before the trace began.
    let mut game_mode = GameMode::default();
    cl.predictions
        .set_sequence(first_sequence(&t).map_or(0, |s| s - 1));
    let mut first_tick = true;
    let fluid_shapes = ruthenium_sim::pick::FluidShapes::new();
    let name = t.header["playerName"].as_str().unwrap_or("").to_owned();
    // The player's own list entry, added before the trace began (in survival).
    let uuid = t.header["playerUuid"]
        .as_str()
        .map(|u| u128::from_str_radix(&u.replace('-', ""), 16).unwrap());
    let mut players = PlayerInfos::default();
    if let Some(uuid) = uuid {
        players.update(
            play::PLAYER_INFO_ADD | play::PLAYER_INFO_GAME_MODE,
            &[play::PlayerInfoEntry {
                uuid,
                name: Some(name.clone()),
                game_mode: Some(0),
            }],
        );
    }
    let mut client = Client {
        entities: Entities::new(p.entity_id, false),
        teams: Teams::default(),
        players,
        name: name.clone(),
        uuid,
        loaded: true,
        death_screen: None,
        respawn_pressed: false,
        sent_outside: std::collections::VecDeque::new(),
    };
    // Whether dying opens the death screen (the login's flag, or a game rule's game event
    // before the trace began).
    p.show_death_screen = t.header["showDeathScreen"].as_bool().unwrap_or(true);
    for (cx, cz) in CHUNKS {
        client.entities.chunk_loaded(cx, cz);
    }
    let mut ticks = 0;
    let records = &t.records;
    // Traces recorded before `set` records existed don't check the predicted blocks.
    let records_sets = records.iter().any(|r| r.kind == "set");
    // Packets handled after a tick began, applied before the next one (packets are
    // handled at the start of a frame, never during a tick).
    let mut pending: Vec<Pending> = Vec::new();
    let mut i = 0;
    while i < records.len() {
        let r = &records[i];
        i += 1;
        if r.kind == "handle" {
            if let Some(b) = r.bytes() {
                handle(&b, &mut client, &mut p, &mut cl, &mut game_mode, d);
            }
            continue;
        }
        if outside_tick_request(r) {
            sent_outside(&mut client, &r.bytes().unwrap(), -1)?;
            continue;
        }
        if r.kind != "tick" {
            continue;
        }
        let n = r.tick().unwrap();
        p.y_rot = r.f32("yRot").unwrap();
        p.x_rot = r.f32("xRot").unwrap();
        let held = keys(&r.value["down"]);
        // The slot as the tick begins (scrolling moves it between ticks).
        if let Some(slot) = r.value["slot"].as_i64() {
            p.inventory.selected = slot as i32;
            if first_tick {
                game_mode.carried_index = slot as i32;
                first_tick = false;
            }
        }
        let clicks = |k: &str| r.value["clicks"][k].as_u64().unwrap_or(0) as u32;
        let down = |k: &str| r.value["down"].as_array().unwrap().iter().any(|v| v == k);
        let mut sent = Vec::new();
        // `rightClickDelay`, `gameMode.tick` (the held slot, then the connection's tick:
        // `player_loaded` once the level is ready, which the trace says), `pick`,
        // `Gui.tick`, then `handleKeybinds` unless a screen is open: one press per hotbar
        // key, then the hands.
        game_mode.tick_delay();
        game_mode.ensure_carried(&p.inventory, &mut sent);
        let loaded_now = records[i..]
            .iter()
            .take_while(|q| q.kind != "tick")
            .find(|q| q.kind == "state")
            .and_then(|q| q.bool("loaded"))
            .unwrap_or(true);
        let mut held = held;
        if !client.loaded && loaded_now {
            sent.push(play::player_loaded());
            client.loaded = true;
            // The level loading screen closes this tick (`Gui.tick`), and grabbing the
            // mouse again reads the keys physically held (`KeyMapping.setAll`, except on
            // macOS).
            if let Some(physical) = r.value.get("physical") {
                held = keys(physical);
            }
        }
        let env = Env {
            level: &cl.level,
            blocks,
            tags,
            colliders: &[],
        };
        let hit = pick_target(&env, &client.entities, &client.players, &p);
        // `Gui.tick`: a dead player with no screen open gets the death screen, which lets
        // go of the keys at once, or asks to respawn (every tick until the server answers);
        // then the death screen ticks.
        let mut keys_released = false;
        if client.loaded && client.death_screen.is_none() && p.is_dead_or_dying() {
            if p.show_death_screen {
                client.death_screen = Some(0);
                keys_released = true;
            } else {
                sent.push(play::client_command(play::CLIENT_COMMAND_PERFORM_RESPAWN));
            }
        }
        if let Some(t) = &mut client.death_screen {
            *t += 1;
        }
        let screen_open = client.death_screen.is_some() || !client.loaded;
        for slot in 0..9 {
            if clicks(&format!("key.hotbar.{}", slot + 1)) > 0 && !p.spectator && !screen_open {
                p.inventory.selected = slot;
            }
        }
        let mut use_keys = UseKeys {
            use_down: down("key.use"),
            attack_down: down("key.attack"),
            use_clicks: clicks("key.use"),
            swap_clicks: clicks("key.swapOffhand"),
            attack_clicks: clicks("key.attack"),
            pick_clicks: clicks("key.pickItem"),
        };
        let mut world = interact::World::new(
            &mut cl.level,
            &mut cl.predictions,
            blocks,
            tags,
            &client.entities,
            &fluid_shapes,
        )
        .with_players(&client.players)
        .with_items(items)
        .with_item_tags(&d.item_tags)
        .with_transformers(&d.transformers);
        world.set_log = Some(Vec::new());
        if screen_open {
            game_mode.screen_open();
        } else {
            game_mode.handle_keybinds(&mut p, &mut world, &hit, &mut use_keys, &mut sent);
        }
        let sets_got = world.set_log.take().unwrap_or_default();
        let env = Env {
            level: &cl.level,
            blocks,
            tags,
            colliders: &[],
        };
        let cx = TickContext {
            env: &env,
            teams: &client.teams,
            players: &client.players,
            local: Local {
                name: &name,
                spectator: p.spectator,
                alive: p.is_alive(),
            },
            tick_rate: 20.0,
            runs_normally: true,
        };
        let held = if keys_released { Keys::default() } else { held };
        let loaded = client.loaded;
        // The player ticks once loaded and while still in the level; `sendChanges` only
        // for a player still in it after the tick.
        client.entities.tick(&cx, &mut p, |p, colliders| {
            if loaded && p.is_alive() {
                p.tick(&Env { colliders, ..env }, held);
                if p.is_alive() {
                    sent.extend(p.send_changes());
                }
            }
        });
        // Vanilla's packets and state for this tick.
        let mut want = Vec::new();
        let mut state = None;
        let mut sets_want = Vec::new();
        while i < records.len() && records[i].kind != "tick" {
            let q = &records[i];
            if q.kind == "set" && q.tick() == Some(n) && q.bool("predicting") == Some(true) {
                let at = &q.value["pos"];
                let c = |k: usize| at[k].as_i64().unwrap() as i32;
                sets_want.push((
                    [c(0), c(1), c(2)],
                    q.value["state"].as_u64().unwrap() as u32,
                    q.value["flags"].as_u64().unwrap() as u32,
                    q.value["limit"].as_i64().unwrap() as i32,
                ));
            }
            if q.kind == "out"
                && q.tick() == Some(n)
                && q.bool("inTick") != Some(false)
                && SENT.contains(&q.str("name").unwrap())
            {
                want.push(q.bytes().unwrap());
            }
            if outside_tick_request(q) {
                pending.push(Pending::Sent(q.bytes().unwrap()));
            }
            if q.kind == "state" && q.tick() == Some(n) {
                state = Some(q.clone());
            }
            if q.kind == "handle"
                && let Some(b) = q.bytes()
            {
                pending.push(Pending::Handled(b));
            }
            i += 1;
        }
        let s = state.ok_or((n, format!("tick {n}: no state")))?;
        // The blocks the client set while predicting, where the trace records them.
        if records_sets && sets_got != sets_want {
            let show = |v: &[([i32; 3], u32, u32, i32)]| {
                v.iter()
                    .map(|(at, s, f, l)| {
                        format!(
                            "{at:?} {} f{f} l{l}",
                            ruthenium_sim::block::state::describe(*s)
                        )
                    })
                    .collect::<Vec<_>>()
            };
            return Err((
                n,
                format!(
                    "tick {n}: the predicted blocks differ\n  got  {:?}\n  want {:?}",
                    show(&sets_got),
                    show(&sets_want)
                ),
            ));
        }
        // The hands and the use, where the trace records them.
        if s.value.get("using").is_some() {
            let hit_kind = match hit {
                HitResult::Entity { .. } => "ENTITY",
                HitResult::Block(ref b) if b.miss => "MISS",
                HitResult::Block(_) => "BLOCK",
            };
            let use_got = (
                hit_kind.to_owned(),
                p.is_using_item(),
                p.is_using_item().then(|| {
                    (
                        p.used_hand().name().to_owned(),
                        p.use_item().describe(),
                        p.use_remaining(),
                    )
                }),
                p.hand(Hand::Main).describe(),
                p.hand(Hand::Off).describe(),
                p.food,
            );
            let use_exp = (
                s.str("hit").unwrap().to_owned(),
                s.bool("using").unwrap(),
                s.bool("using").unwrap().then(|| {
                    (
                        s.str("useHand").unwrap().to_owned(),
                        s.str("useItem").unwrap().to_owned(),
                        s.value["useRemaining"].as_i64().unwrap() as i32,
                    )
                }),
                s.str("main").unwrap().to_owned(),
                s.str("off").unwrap().to_owned(),
                s.value["food"].as_i64().unwrap() as i32,
            );
            if use_got != use_exp {
                return Err((
                    n,
                    format!(
                        "tick {n} (heartbeat count {reminder}): the hands differ\n  got  {use_got:?}\n  want {use_exp:?}\n  sent {sent:02x?}"
                    ),
                ));
            }
        }
        // The block being broken, where the trace records it.
        if let Some(exp) = s.value.get("destroying") {
            let got = serde_json::json!({
                "destroying": game_mode.is_destroying,
                "pos": game_mode.destroy_pos,
                "progress": game_mode.destroy_progress.to_bits(),
                "ticks": game_mode.destroy_ticks.to_bits(),
                "delay": game_mode.destroy_delay,
                "state": cl.level.block(
                    game_mode.destroy_pos[0],
                    game_mode.destroy_pos[1],
                    game_mode.destroy_pos[2],
                ),
            });
            let want = serde_json::json!({
                "destroying": exp.as_bool().unwrap(),
                "pos": s.value["destroyPos"],
                "progress": s.f32("destroyProgress").unwrap().to_bits(),
                "ticks": s.f32("destroyTicks").unwrap().to_bits(),
                "delay": s.value["destroyDelay"],
                "state": s.value["destroyState"],
            });
            if got != want {
                return Err((
                    n,
                    format!(
                        "tick {n} (heartbeat count {reminder}): breaking differs\n  got  {got}\n  want {want}\n  sent {sent:02x?}"
                    ),
                ));
            }
        }
        // Dying, where the trace records it: the ticks since, whether the player is out
        // of the level, and the death screen.
        if let Some(dt) = s.value.get("deathTime").and_then(serde_json::Value::as_i64) {
            let got = (p.death_time, p.removed, client.death_screen.is_some());
            let want = (
                dt as i32,
                s.bool("removed").unwrap(),
                s.str("screen") == Some("DeathScreen"),
            );
            if got != want {
                return Err((
                    n,
                    format!(
                        "tick {n}: dying differs (death time, removed, death screen)\n  got  {got:?}\n  want {want:?}"
                    ),
                ));
            }
        }
        let pose = Pose::from_name(s.str("pose").unwrap()).unwrap();
        let got = (
            p.position,
            p.delta,
            p.on_ground,
            p.horizontal_collision,
            p.sprinting,
            p.pose,
            p.abilities.flying,
        );
        let exp = (
            s.vec3("pos").unwrap(),
            s.vec3("vel").unwrap(),
            s.bool("onGround").unwrap(),
            s.bool("hColl").unwrap(),
            s.bool("sprinting").unwrap(),
            pose,
            s.bool("flying").unwrap(),
        );
        if got != exp || sent != want {
            return Err((
                n,
                format!(
                    "tick {n} (heartbeat count {reminder}):\n  got  pos {:?} vel {:?} ground {} hcoll {} sprint {} pose {:?} flying {}\n  want pos {:?} vel {:?} ground {} hcoll {} sprint {} pose {:?} flying {}\n  sent {:02x?}\n  want {:02x?}",
                    got.0,
                    got.1,
                    got.2,
                    got.3,
                    got.4,
                    got.5,
                    got.6,
                    exp.0,
                    exp.1,
                    exp.2,
                    exp.3,
                    exp.4,
                    exp.5,
                    exp.6,
                    sent,
                    want
                ),
            ));
        }
        for e in std::mem::take(&mut pending) {
            match e {
                Pending::Handled(b) => handle(&b, &mut client, &mut p, &mut cl, &mut game_mode, d),
                Pending::Sent(b) => sent_outside(&mut client, &b, n)?,
            }
        }
        ticks += 1;
    }
    if let Some(ours) = client.sent_outside.front() {
        return Err((
            ticks as i64,
            format!("the replay sent {ours:02x?}, vanilla didn't"),
        ));
    }
    Ok(ticks)
}

/// Scenarios the movement code must reproduce exactly (all of them now).
const EXACT: [&str; 51] = [
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
    "water_current",
    "waterfall",
    "lava",
    "combat_basics",
    "attacking",
    "entity_push",
    "entity_moving",
    "entity_rules",
    "flying",
    "spectator",
    "effects",
    "eating",
    "offhand",
    "ranged",
    "throwing",
    "breaking",
    "placing",
    "using_blocks",
    "tools_on_blocks",
    "scaffolding",
    "shallow_water",
    "swim_under",
    "crawl",
    "fence_slime_water",
    "ice_lanes",
    "slime_lanes",
    "soul_sand",
    "climbing",
    "stair_sneak",
    "water_cube",
    "block_shapes_a",
    "block_shapes_b",
    "bounce",
    "speed_zero",
    "bubble_columns",
    "cobwebs",
    "berry_bushes",
    "powder_snow",
    "honey_slide",
    "death_screen",
    "death_immediate",
];

#[test]
fn golden_traces() {
    let Some(game) = game() else {
        eprintln!("RUTHENIUM_GAME_DIR not set; skipped");
        return;
    };
    let exports = game.join("versions/26.3/exports");
    let d = Data {
        blocks: Blocks::load(&exports).unwrap(),
        tags: vanilla_tags(&game),
        types: EntityTypes::load(&exports).unwrap(),
        items: Items::load(&exports, &game.join("versions/26.3/generated/reports")).unwrap(),
        item_tags: vanilla_item_tags(&game),
        transformers: vanilla_transformers(&game),
    };
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
        "water_current",
        "waterfall",
        "lava",
        "combat_basics",
        "attacking",
        "entity_push",
        "entity_moving",
        "entity_rules",
        "flying",
        "spectator",
        "effects",
        "eating",
        "offhand",
        "ranged",
        "throwing",
        "breaking",
        "placing",
        "using_blocks",
        "tools_on_blocks",
        "scaffolding",
        "shallow_water",
        "swim_under",
        "crawl",
        "fence_slime_water",
        "ice_lanes",
        "slime_lanes",
        "soul_sand",
        "climbing",
        "stair_sneak",
        "water_cube",
        "block_shapes_a",
        "block_shapes_b",
        "bounce",
        "speed_zero",
        "bubble_columns",
        "cobwebs",
        "berry_bushes",
        "powder_snow",
        "honey_slide",
        "death_screen",
        "death_immediate",
    ];
    let mut failed = Vec::new();
    for name in all {
        match replay(name, &d) {
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

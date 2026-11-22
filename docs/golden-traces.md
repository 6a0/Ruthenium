# Golden traces

Golden traces record what the vanilla 26.3 client does, tick by tick, in scripted scenarios: the inputs it reads, the packets it sends, the packets it processes and the player's state after every tick. Ruthenium's simulation will replay the inputs and inbound packets and must reproduce the positions, velocities and outbound packet bytes exactly.

The committed traces are in `tests/golden/26.3/`. `crates/ruthenium-trace` reads them. Its tests check every trace against vanilla's per-tick packet rules and check every recorded packet id against the ids `cargo xtask codegen` generated.

## Recording

```sh
cargo xtask trace                      # all scenarios into cache/traces/26.3
cargo xtask trace --only ice,swim      # some of them
cp cache/traces/26.3/*.jsonl tests/golden/26.3/   # update the committed traces
```

A game window opens for about 90 seconds. The client runs from your own 26.3 install (found like `ruthenium locate` does), with Mojang's Java runtime from that launcher, in offline developer mode (`--offlineDeveloperMode`): singleplayer only, and your account is never used.

On top of vanilla it loads:

- **Fabric Loader** 0.20.1;
- **Fabric API's client gametest module** and the two modules it needs (`fabric-api-base`, `fabric-networking-api-v1`, `fabric-resource-loader-v1`). These are not the whole of Fabric API, so no other parts of the game are replaced;
- **`tools/java/trace`**, Ruthenium's recorder: a few hooks at tick and frame boundaries, on `Connection.send` and on inbound packet processing, plus the scenarios.

All jars are pinned by SHA-1 (`xtask/src/fabric.rs`) and kept in git-ignored `cache/`.

### The test harness

Fabric's client gametests run the game in lockstep. Each client tick is one frame, the integrated server runs exactly one tick per client tick, and test code runs between ticks while both threads wait. A key pressed in a script is therefore read on a known tick, and the game-thread records (`tick`, `state`, `out`) of two recordings are identical down to every float bit. That was checked by recording every scenario twice and comparing.

Things this harness changes compared with normal play, which replay tests must allow for:

- **Frame and tick timing.** There is one tick per frame and no catch-up, so vanilla's `DeltaTracker` frame/tick timing is not exercised. Between ticks the harness also renders a few frames while test code runs. Timing tests use the simulated clocks in the plan instead.
- **Singleplayer.** The connection is the integrated server's in-memory one. The recorder encodes each packet itself with the protocol in effect, which gives the same bytes the network encoder would write before framing, compression and encryption.
- **Inbound arrival.** Packets from the server arrive on the network thread at times that vary from run to run. `recv` records and the tick on which a `handle` happens (for example `set_time`, sent every 20 server ticks) can differ between recordings. Each trace records where they actually happened, which is what a replay needs.

## Replaying

`crates/ruthenium-sim/tests/golden.rs` replays every trace through Ruthenium's movement code: it builds the scenario's world from the header's commands on the flat preset, applies the packets the client handled (entities added, moved and removed, their data, teams, the player's motion, teleports and attributes, the player's own entity data, abilities, effects, operator level and game mode, from the game event and from its player-list entry, which the header's `playerUuid` names, its inventory, item cooldowns, food and the server's "item use complete" event, and block updates and block-change acknowledgements through the client's predictions) where the trace handled them. Items the header's `build` commands gave (`item replace`) start in the inventory. Then each tick it sets the camera's rotation and the selected slot, sends the held slot if it changed, picks what the crosshair is on, plays the recorded hotbar, use, swap-hands and attack presses and the held use and attack keys through the client's item use, attacks and block breaking (both hands, the items' own predictions, the server's echo of the use, the miss penalty, the attack strength and the attacker's own slowdown, the breaking progress and the predicted removal of the broken block), holds the recorded movement keys, runs the entities' tick with the player's in its place, and compares the position, velocity, ground and collision flags, sprinting, pose and flight with the `state` record (and, where the record has them, what the crosshair is on, the item in use, the hands, the food level and the block being broken: its position and state, the progress, ticks and delay, bit for bit), and the packets the tick sends (`set_carried_item`, `attack`, `punch`, `spectator_action`, `player_action`, `interact`, `use_item_on`, `use_item`, `player_abilities`, `player_input`, `player_command`, `move_player_*`) with the `out` records, byte for byte. The prediction sequence starts one before the trace's first predicted action (`use_item`, `use_item_on`, or a `player_action` that starts or finishes breaking a block). It needs the prepared game (block shapes, hardness and item classes from the exporters, the items' default components from the data generator's reports, every block tag from the jar):

```sh
RUTHENIUM_GAME_DIR=~/.local/share/ruthenium/game cargo test -p ruthenium-sim --test golden -- --nocapture
```

Without `RUTHENIUM_GAME_DIR` it is skipped. `crates/ruthenium-sim/tests/mouse.rs` needs nothing but the committed trace and runs everywhere: it feeds `mouse_turn`'s cursor moves through the client's own mouse path (the first motion after a grab dropped, motion summed per frame, vanilla's sensitivity curve, `Entity.turn` in floats) and checks every recorded yaw and pitch bit for bit. The scenarios listed in `EXACT` must match every tick; the others are reported. The trace doesn't record how many ticks had passed since the last position packet when it starts, so the replay tries each count until the first heartbeat lines up.

## Scenarios

Each scenario first clears a 41 × 25 area around the origin of a flat world and lays a stone floor (the `reset` commands in the header). It then builds its own blocks with commands (`build`) and teleports the player (`teleport`). The header therefore describes the whole world the trace ran in. No world files are involved. The player stands at y = -60; yaw 0 faces +Z, yaw -90 faces +X.

| Scenario | What it does |
|---|---|
| `walk_directions` | 15 ticks in each of the eight key directions, 5 idle ticks between |
| `sprint_jump` | forward + sprint + jump held for 40 ticks |
| `sneak_edge` | sneak into the edge of a 3-high platform, sneak along it, walk off |
| `ice` | sprint from ice onto blue ice, release, slide |
| `slime_bounce` | walk off a 10-block pillar onto slime, bounce, jump on slime |
| `ladder` | climb a ladder, hold on by sneaking, slide down |
| `swim` | walk into a 2-deep pool, sprint-swim, swim up |
| `water_current` | a walled channel of settled flowing water: walk upstream, jump, drift, sneak downstream, strafe, jump in the shallow end |
| `waterfall` | drop into a 1×1 shaft of falling water, sink with the current, swim up |
| `lava` | with fire resistance, wade into a 2-deep lava pool, sink, swim with jump held |
| `step_up` | walk over a slab, carpet, snow layers and stairs, jump onto a block |
| `head_hitter` | sprint-jump under a ceiling 2 blocks above the floor |
| `mouse_turn` | walk while the mouse turns the camera every tick |
| `combat_basics` | hotbar keys and scrolling, attacks on an armor stand (summoned as the script starts), a missed swing, using a shield on it (the stand takes the click) |
| `attacking` | sprinting hits that slow the attacker on the client: a fist on a boat with sprint held (the sprint restarts in the same tick) and a weaker hit after it, a sword on a minecart after a double-tapped sprint (the sprint ends); a sword on a zombie, which the client doesn't slow for (mobs' `hurtClient` refuses); two clicks at the sky in one tick (the miss swallows the second) and the attack key held there; a spear jab refused before it is charged, then made |
| `entity_push` | mobs push the player: a zombie overlapping it, walking into it, a baby zombie, a slime |
| `entity_moving` | a zombie moved through the standing player a little every tick (stepped moves, interpolated pushes), another sliding sideways, both killed |
| `entity_rules` | team collision rules (never, push other teams, push own team), then a boat that pushes the player and that it steps onto |
| `flying` | creative flight: double-tap jump to take off and rise, fly, sprint-fly, strafe, sink, hover, double-tap to drop, take off again and land, which ends the flight |
| `spectator` | spectator mode: sink through the floor and back, fly through a wall, sprint-fly, then back to survival in the air and fall |
| `effects` | jump boost II jumps, levitation V, slow falling while walking, speed III, blindness ending a sprint and stopping the next, dolphin's grace sprint-swimming |
| `eating` | cooked beef refused at full hunger, eaten while walking once hunger has drained, eaten while sprinting and let go half way; a golden apple, a water potion, a honey bottle and milk to the end; a golden apple looking at the floor |
| `offhand` | a golden apple eaten from the off hand with an empty main hand, past a sword and past a bow without arrows, kept on through a hotbar change; a shield blocking from the off hand while walking; swapping hands while blocking and while eating |
| `ranged` | a bow drawn while walking and shot, a crossbow charged and fired, a trident readied and thrown, a spyglass, a goat horn and its cooldown |
| `breaking` | dirt by hand (finished, then the floor behind started and given up); stone with a wooden pickaxe, its face changed half way, then two floor blocks in turn; a torch broken at once, twice, and once while digging the floor (breaking is given up twice in that tick); the block under the player (it falls, and digs on in the air); haste II, mining fatigue, digging while jumping (a miss gives breaking up, and the same block then goes on without a new start, as vanilla does); a stone pickaxe with efficiency III; iron ore with a wooden pickaxe (fast, but the wrong tier); a waterlogged pane (its water left behind); redstone ore lit by the first hit; creative breaking every sixth tick, a waterlogged pane there, and a sword that breaks nothing; adventure mode with a pickaxe that may break only stone |
| `throwing` | experience bottles thrown with two clicks a tick, then a golden apple clicked the same way (each click starts it, the next tick lets it go, the server's late echo starts it again: it is never eaten), then held and eaten; snowballs thrown by holding use; ender pearls straight up, held back by their cooldown, landing back on the player |

They are defined in `tools/java/trace/src/dev/ruthenium/trace/Scenarios.java`. To add one, add an entry there and record again. Scripts can run server commands while the trace records, so entities a scenario needs are summoned inside its script: their packets are then in the trace. A scenario that leaves mobs behind must remove them and wait until they are gone (dying mobs push for 20 ticks, and a large slime splits), or they push the player before the next trace starts.

## Format (`ruthenium-trace/1`)

JSON Lines. The first line is the header; every other line is one record with a kind `k`.

- **Ticks and frames.** `n` is the client tick within the trace (0 is the first recorded tick, -1 before it) and `f` is the frame.
- **Exact floats.** Floats and doubles are written in Java's shortest round-trip form. Parse the text directly as `f32` / `f64`, as `ruthenium-trace` does, to get the exact value.

| `k` | When | Fields |
|---|---|---|
| `header` | first line | `format`, `game`, `protocol`, `scenario`, `description`, `world`, `reset`, `build`, `teleport`, `options` (toggle sprint/crouch, auto-jump, mouse sensitivity), `entityId`, `playerName` (the offline player's name, which teams list), `playerUuid` (its player-list entry), `harness` |
| `frame` | start of each client frame | `f` |
| `tick` | start of each client tick, before the game reads input | `n`, `f`, `down` (key mappings held), `clicks` (queued presses of keys the game reads as clicks: attack, use, pick, drop, swap, inventory, hotbar), `yRot`, `xRot`, `slot` |
| `state` | end of each client tick | `n`, `gameTime`, `pos`, `vel`, `yRot`, `xRot`, `onGround`, `hColl`, `minorHColl`, `vColl`, `vCollBelow`, `fall`, `sprinting`, `shiftDown`, `pose`, `flying`, `inWater`, `input` (the tick's movement input), `move` (move vector), `health`, `food`, `saturation`, `using` (`isUsingItem`) and while using `useHand`, `useItem`, `useRemaining`; `main` and `off` (the hands, `id*count`, empty as `""`); `hit` (what the tick's pick found: `MISS`, `BLOCK` with `hitBlock`, `hitFace` and `hitAt`, `ENTITY` with `hitEntity` and `hitAt`); breaking: `destroying`, `destroyPos` (kept after breaking ends), `destroyState` (the block state id there now), `destroyProgress`, `destroyTicks`, `destroyDelay` |
| `out` | every serverbound packet, as the game hands it to the connection | `n`, `f`, `thread` (`game` or `net`), `inTick`, `proto`, `name`, `hex` (packet id VarInt and payload) |
| `recv` | a clientbound packet arriving, on the network thread | `n`, `f`, `thread`, `proto`, `name` |
| `handle` | a clientbound packet being processed on the game thread (sub-packets of a bundle get `bundled: true`) | `n`, `f`, `thread`, `proto`, `name`, and `hex` for packets that affect the local player |
| `end` | last line | `ticks` |

Inbound bytes are recorded only for the packets that affect the local player or its surroundings in a scenario: position and rotation, motion, explosions, attributes, effects, abilities, health, game events, tick rate, held slot, keep-alive and ping, damage, entities added and removed, their data, movement and passengers, teams, the player list, block updates and acknowledgements, inventory slots, item cooldowns, sounds. Chunk, light, registry, tag, recipe and advancement packets appear by name only, so traces hold Ruthenium's own measurements and no game data (see `docs/reference-policy.md`, rule 5).

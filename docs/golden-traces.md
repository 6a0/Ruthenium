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
| `step_up` | walk over a slab, carpet, snow layers and stairs, jump onto a block |
| `head_hitter` | sprint-jump under a ceiling 2 blocks above the floor |
| `mouse_turn` | walk while the mouse turns the camera every tick |
| `combat_basics` | hotbar keys and scrolling, attacks on an armor stand, a missed swing, shield use |

They are defined in `tools/java/trace/src/dev/ruthenium/trace/Scenarios.java`. To add one, add an entry there and record again.

## Format (`ruthenium-trace/1`)

JSON Lines. The first line is the header; every other line is one record with a kind `k`.

- **Ticks and frames.** `n` is the client tick within the trace (0 is the first recorded tick, -1 before it) and `f` is the frame.
- **Exact floats.** Floats and doubles are written in Java's shortest round-trip form. Parse the text directly as `f32` / `f64`, as `ruthenium-trace` does, to get the exact value.

| `k` | When | Fields |
|---|---|---|
| `header` | first line | `format`, `game`, `protocol`, `scenario`, `description`, `world`, `reset`, `build`, `teleport`, `options` (toggle sprint/crouch, auto-jump, mouse sensitivity), `entityId`, `harness` |
| `frame` | start of each client frame | `f` |
| `tick` | start of each client tick, before the game reads input | `n`, `f`, `down` (key mappings held), `clicks` (queued presses of keys the game reads as clicks: attack, use, pick, drop, swap, inventory, hotbar), `yRot`, `xRot`, `slot` |
| `state` | end of each client tick | `n`, `gameTime`, `pos`, `vel`, `yRot`, `xRot`, `onGround`, `hColl`, `minorHColl`, `vColl`, `vCollBelow`, `fall`, `sprinting`, `shiftDown`, `pose`, `flying`, `inWater`, `input` (the tick's movement input), `move` (move vector), `health`, `food` |
| `out` | every serverbound packet, as the game hands it to the connection | `n`, `f`, `thread` (`game` or `net`), `inTick`, `proto`, `name`, `hex` (packet id VarInt and payload) |
| `recv` | a clientbound packet arriving, on the network thread | `n`, `f`, `thread`, `proto`, `name` |
| `handle` | a clientbound packet being processed on the game thread (sub-packets of a bundle get `bundled: true`) | `n`, `f`, `thread`, `proto`, `name`, and `hex` for packets that affect the local player |
| `end` | last line | `ticks` |

Inbound bytes are recorded only for the packets that affect the local player or its surroundings in a scenario: position and rotation, motion, explosions, attributes, effects, abilities, health, game events, tick rate, held slot, keep-alive and ping, damage, entity data and movement, block updates and acknowledgements, inventory slots, sounds. Chunk, light, registry, tag, recipe and advancement packets appear by name only, so traces hold Ruthenium's own measurements and no game data (see `docs/reference-policy.md`, rule 5).

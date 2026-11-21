# Use of the decompiled reference

Matching vanilla behaviour exactly is a core goal, and it requires studying how the vanilla client behaves. This policy sets out how that study is done without copying Mojang's code.

## Rules

1. **Local only.** Decompiled sources, the game jars and anything extracted from them live in `reference/`. That folder is git-ignored and is never committed, uploaded, shared or included in any artifact. `cargo xtask check-content` enforces this in CI.
2. **Behaviour, not code.** For each subsystem, first write a short behavioural note covering inputs, outputs, ordering and constants. Then implement it in Rust from that note. Code review rejects transliterated code: the same structure, the same local variable names or the same comments.
3. **Facts are fine.** Protocol packet layouts, numeric constants, thresholds, ordering rules and registry names are interoperability facts. They may appear in the code, cited by vanilla `path:line` so the behaviour can be re-checked against each new version. The tables `cargo xtask codegen` writes to `crates/ruthenium-data` (packet ids, registry names and ids, block-state properties, entity-data serializer ids) hold only such facts.
4. **Code-defined data comes from the player's install.** Entity models, block shapes and similar data are extracted on the player's machine from their own installed game, by Ruthenium's original exporter classes (`tools/java/`). They are never committed or redistributed.
5. **Golden traces.** Traces recorded from the vanilla client contain only Ruthenium's own measurements (positions, velocities, packet sequences), never game files. They are recorded with `cargo xtask trace` (see `docs/golden-traces.md`); inbound packet payloads are kept only for packets that affect the player, never chunk, registry or other game data.
6. **Exporters are original.** Classes in `tools/java/` are written for Ruthenium and may call the game's public classes. CI rejects any class in the exporter bundle that isn't Ruthenium's own. CI downloads the official client jar and its libraries, pinned by SHA-1, as build-time input only: to compile the exporters, and to run Mojang's data generator to check that the generated tables are current. Those files and the generator's output are never committed, cached or included in a build artifact. The development tools that drive the vanilla client (`tools/java/bench`, `tools/java/trace`) also load Fabric Loader and a few Fabric API modules: third-party Apache-2.0 jars, pinned by SHA-1, kept in `cache/` and never committed or shipped. Ruthenium itself does not use Fabric.

This is an engineering policy, not legal advice. It should be reviewed by counsel before the first public release.

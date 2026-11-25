//! Developer tasks. Run with `cargo xtask <task>`.

#![forbid(unsafe_code)]

mod bench;
mod bench_ab;
mod bench_dedicated;
mod bench_menu;
mod check_content;
mod codegen;
mod exporters;
mod fabric;
mod fetch_compile_deps;
mod fixtures;
mod game;
mod gpu;
mod javamath_dump;
mod jvm;
mod launch_pins;
mod mods;
mod moltenvk;
mod pin_version;
mod routes;
mod server;
mod shaders;
mod trace;

use std::process::ExitCode;

const USAGE: &str = "\
usage: cargo xtask <task>

tasks:
  check-content [--jar <client.jar>]... [--asset-index <index.json>]...
      Fail if any tracked or untracked (non-ignored) file looks like Mojang
      content. With --jar / --asset-index, also fail if any file is byte-identical
      to an entry of that jar or an object of that asset index. If no --jar is
      given and reference/26.3/jars/client.jar exists, it is used automatically.
  javamath-dump [--java <java>] [--javac <javac>] [--classpath <cp>] [--out <file>]
      Dump vanilla's Mth sine table and sin/cos probe results from a JVM to
      cache/javamath/mth.bin (read by ruthenium-javamath's tests). Defaults to
      reference/26.3 and JAVA_HOME or PATH. To check what players run, pass the
      launcher's runtime, e.g. --java <launcher>/java/java-runtime-epsilon/bin/java.
  pin-version [--json <version.json>] [--out <file>]
      Regenerate crates/ruthenium-prepare/src/pins.rs (hashes, sizes and paths of
      the client jar, asset index and class-path libraries) from Mojang's version
      JSON. Defaults to reference/26.3/jars/26.3.json.
  fetch-compile-deps [--out <folder>]
      Download the official client jar and its class-path libraries, pinned by
      SHA-1, into cache/compile-deps. Build-time input for CI only: never
      commit, cache or ship these files.
  exporters [--deps <folder> | --classpath <cp>] [--javac <javac>] [--out <jar>]
      Compile tools/java/exporters into target/java/ruthenium-exporters.jar and
      fail if the bundle holds any class outside dev/ruthenium/. Compiles against
      reference/26.3 or cache/compile-deps by default.
  check-bundle <jar>
      Fail if the jar holds anything but dev/ruthenium/ classes and a manifest.
  bench <setup|run|report> [--backend opengl,vulkan] [--scene menu,idle,spin,fly]
        [--reps N] [--warmup S] [--duration S] [--render-distance N] [--seed N]
        [--java <java>] [--screenshot] [--results <folder>]
      Baseline measurements of vanilla 26.3, run from the player's own install
      with Fabric Loader and tools/java/bench (needs a display; see
      docs/baseline.md). setup creates the benchmark world, run measures every
      scene on every backend, report prints Markdown tables for a results folder
      (dedicated-server runs next to the integrated-server ones of --compare
      <folder>, by default the newest other folder that has any).
  bench dedicated --accept-eula [--client vanilla,ruthenium] [--backend vulkan,opengl]
        [--scene idle,spin,walk,fly] [--reps N] [--headless-reps N] [--warmup S]
        [--duration S] [--gpu <name>] [--server-jar <server.jar>] [--bin <ruthenium>]
        [--data-dir <folder>] [--expect <scene>=<positions>[,<footprint>]]...
        [--results <folder>] [--compare <folder>] [--timeout S] [--width W]
        [--height H] [--any-drawable] [--screenshot] [--bare-picture | --hand-picture | --hud-picture]
        [--clouds <fast|fancy>[@<game time>]] [--weather-time <game time>]
        [--time <ticks of the day>] [--keep-worlds]
      The world scenes on a vanilla 26.3 dedicated server (a server.jar you
      supply, by default reference/26.3/jars/server.jar), started per trial on a
      fresh copy of the benchmark world, in offline mode on 127.0.0.1, and stopped
      afterwards. Both clients play the same tick-keyed routes: the vanilla client
      is measured as in bench run, Ruthenium joins headless through its
      development join path and must record the same positions and loaded chunks.
      Needs Mojang's EULA accepted, which only --accept-eula says. Everything is
      kept under cache/bench.
  bench ab [--a <rev>|work] [--b <rev>|work] [--a-args '<options>']
           [--b-args '<options>'] [--rounds N] [--duration S] [--gpu <name>]
           [-- <ruthenium options>]
      Quick A/B comparison of two Ruthenium builds (default HEAD against the
      working tree): release builds run in turn, A B B A..., uncapped, with the
      median, change and round-to-round spread of each metric; changes within
      the spread are reported as noise. --a-args and --b-args give one side its
      own options, to compare two settings of one build. Results in
      target/bench-ab.
  bench menu [--gpu <name>] [--duration S] [--launches N] [--pixels WxH]
             [-- <ruthenium options>]
      What the working tree's window and menus cost: launch times with and
      without the pipeline cache, memory with an empty window and with the
      menus at 1920x1080 pixels, and pacing with VSync. Results in
      target/bench-menu.
  trace [--only <scenario,...>] [--out <folder>] [--java <java>]
      Record golden traces from vanilla 26.3 (Fabric Loader, Fabric API's client
      gametest module and tools/java/trace; see docs/golden-traces.md) into
      cache/traces/26.3. Copy them to tests/golden/26.3 to update the committed
      ones.
  build-mod <bench|trace>
      Compile a development mod into target/java/ruthenium-<name>.jar.
  codegen [--deps <folder>] [--java <java>] [--reports <folder>] [--check]
      Regenerate crates/ruthenium-data/src/v26_3 (packet ids, registries,
      block states, entity-data serializer ids) from the pinned client jar.
      Runs Mojang's data generator from the jar (cached in cache/codegen) unless
      --reports names a folder of its reports. The jar is found like for
      exporters. With --check, write nothing and fail if any file is stale.
  fixtures [--bin <ruthenium>] [--features <cargo features>] [--gpu <name>]
           [--seconds S] [--out <folder>] [-- <ruthenium options>]
      Run the client's renderer checks, one short run each (needs a display): the
      colour fixture and the upload check, the texture table filled and reloaded
      at 256 slots on ordinary and on update-after-bind descriptor sets, and a
      second launch finding the pipeline cache of the first. Builds the debug
      client unless --bin names one. Output and the adapter report go to
      target/fixtures.
  moltenvk
      Download the pinned MoltenVK release (checked by SHA-256) and unpack its
      library and licence into cache/moltenvk.
  bundle [--binary <executable>] [--out <folder>]
      Assemble target/bundle/Ruthenium.app with the pinned MoltenVK in
      Contents/Frameworks, and sign it ad hoc. On macOS the release client is
      built first; elsewhere --binary wraps an executable to check the layout.
  shaders [--glslc <glslc>] [--check]
      Compile crates/ruthenium-shaders/glsl to the committed SPIR-V with the
      glslc of the pinned Vulkan SDK (downloaded once into cache/glslc on Linux
      x86-64; elsewhere pass that SDK's glslc). With --check, write nothing and
      fail if any committed file differs from glslc's output.";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("check-content") => check_content::run(&args[1..]),
        Some("javamath-dump") => javamath_dump::run(workspace_root(), &args[1..]),
        Some("pin-version") => pin_version::run(workspace_root(), &args[1..]),
        Some("fetch-compile-deps") => fetch_compile_deps::run(workspace_root(), &args[1..]),
        Some("exporters") => exporters::run(workspace_root(), &args[1..]),
        Some("check-bundle") => exporters::run_check(&args[1..]),
        Some("bench") => bench::run(workspace_root(), &args[1..]),
        Some("trace") => trace::run(workspace_root(), &args[1..]),
        Some("build-mod") if args.len() == 2 => {
            let extra = if args[1] == "trace" {
                match fabric::fetch_all(workspace_root(), fabric::GAMETEST_MODS) {
                    Ok(jars) => jars,
                    Err(e) => {
                        eprintln!("build-mod: {e}");
                        return ExitCode::FAILURE;
                    }
                }
            } else {
                Vec::new()
            };
            match mods::build(workspace_root(), &args[1], &extra) {
                Ok(p) => {
                    println!("{}", p.display());
                    ExitCode::SUCCESS
                }
                Err(e) => {
                    eprintln!("build-mod: {e}");
                    ExitCode::FAILURE
                }
            }
        }
        Some("codegen") => codegen::run(workspace_root(), &args[1..]),
        Some("shaders") => shaders::run(workspace_root(), &args[1..]),
        Some("fixtures") => fixtures::run(workspace_root(), &args[1..]),
        Some("moltenvk") => moltenvk::run_fetch(workspace_root(), &args[1..]),
        Some("bundle") => moltenvk::run_bundle(workspace_root(), &args[1..]),
        _ => {
            eprintln!("{USAGE}");
            ExitCode::FAILURE
        }
    }
}

fn workspace_root() -> &'static std::path::Path {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask lives inside the workspace")
}

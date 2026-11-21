//! Developer tasks. Run with `cargo xtask <task>`.

#![forbid(unsafe_code)]

mod check_content;
mod exporters;
mod fetch_compile_deps;
mod javamath_dump;
mod jvm;
mod pin_version;

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
      SHA-1, into cache/compile-deps. Compile-time input for CI only: never
      commit, cache or ship these files.
  exporters [--deps <folder> | --classpath <cp>] [--javac <javac>] [--out <jar>]
      Compile tools/java/exporters into target/java/ruthenium-exporters.jar and
      fail if the bundle holds any class outside dev/ruthenium/. Compiles against
      reference/26.3 or cache/compile-deps by default.
  check-bundle <jar>
      Fail if the jar holds anything but dev/ruthenium/ classes and a manifest.
  codegen
      Generate ruthenium-data tables (not implemented yet).";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("check-content") => check_content::run(&args[1..]),
        Some("javamath-dump") => javamath_dump::run(workspace_root(), &args[1..]),
        Some("pin-version") => pin_version::run(workspace_root(), &args[1..]),
        Some("fetch-compile-deps") => fetch_compile_deps::run(workspace_root(), &args[1..]),
        Some("exporters") => exporters::run(workspace_root(), &args[1..]),
        Some("check-bundle") => exporters::run_check(&args[1..]),
        Some("codegen") => {
            eprintln!("codegen: not implemented yet");
            ExitCode::FAILURE
        }
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

//! \`cargo xtask javamath-dump\`: record vanilla's \`Mth\` table and sin/cos
//! results from a real JVM, for ruthenium-javamath's bit-exact tests.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

const SOURCE: &str = "tools/java/javamath/dev/ruthenium/tools/DumpMth.java";
const MAIN_CLASS: &str = "dev.ruthenium.tools.DumpMth";

pub fn run(root: &Path, args: &[String]) -> ExitCode {
    let java_home = std::env::var_os("JAVA_HOME").map(PathBuf::from);
    let tool = |name: &str| {
        java_home
            .as_ref()
            .map(|h| h.join("bin").join(name))
            .unwrap_or_else(|| PathBuf::from(name))
    };
    let mut java = tool("java");
    let mut javac = tool("javac");
    let mut classpath: Option<OsString> = None;
    let mut out = root.join("cache/javamath/mth.bin");

    let mut it = args.iter();
    while let Some(arg) = it.next() {
        let Some(value) = it.next() else {
            eprintln!("javamath-dump: {arg} needs a value");
            return ExitCode::FAILURE;
        };
        match arg.as_str() {
            "--java" => java = PathBuf::from(value),
            "--javac" => javac = PathBuf::from(value),
            "--classpath" => classpath = Some(OsString::from(value)),
            "--out" => out = PathBuf::from(value),
            other => {
                eprintln!("javamath-dump: unknown argument {other:?}");
                return ExitCode::FAILURE;
            }
        }
    }

    let game_cp = match classpath {
        Some(cp) => cp,
        None => {
            let jar = root.join("reference/26.3/jars/client.jar");
            if !jar.is_file() {
                eprintln!(
                    "javamath-dump: {} not found; pass --classpath <client.jar and its libraries>",
                    jar.display()
                );
                return ExitCode::FAILURE;
            }
            // Java expands a trailing "*" classpath entry to every jar in that folder.
            join(&[jar, root.join("reference/26.3/libs/*")])
        }
    };

    let classes = root.join("target/java/javamath");
    let compiled = Command::new(&javac)
        .arg("--release")
        .arg("25")
        .arg("-d")
        .arg(&classes)
        .arg("-cp")
        .arg(&game_cp)
        .arg(root.join(SOURCE))
        .status();
    if !matches!(compiled, Ok(s) if s.success()) {
        eprintln!("javamath-dump: compiling with {} failed", javac.display());
        return ExitCode::FAILURE;
    }

    // Vanilla's logger creates logs/ in the working directory; keep it in target/.
    let out = match std::path::absolute(&out) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("javamath-dump: {}: {e}", out.display());
            return ExitCode::FAILURE;
        }
    };
    let mut run_cp = game_cp;
    run_cp.push(if cfg!(windows) { ";" } else { ":" });
    run_cp.push(&classes);
    let ran = Command::new(&java)
        .arg("-cp")
        .arg(&run_cp)
        .arg(MAIN_CLASS)
        .arg(&out)
        .current_dir(&classes)
        .status();
    if matches!(ran, Ok(s) if s.success()) {
        ExitCode::SUCCESS
    } else {
        eprintln!("javamath-dump: running with {} failed", java.display());
        ExitCode::FAILURE
    }
}

fn join(parts: &[PathBuf]) -> OsString {
    std::env::join_paths(parts).expect("classpath entries contain no separator")
}

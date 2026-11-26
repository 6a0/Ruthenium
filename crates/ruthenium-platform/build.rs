//! On macOS, links clang's runtime library (`libclang_rt.osx.a`) after everything else.
//!
//! SDL, built from source there, asks which macOS it runs on with Objective-C's
//! `@available` (in its windows' mouse handling, for one), which clang compiles to calls
//! of `__isPlatformVersionAtLeast`. That function is in clang's runtime library, which
//! clang links by default, but rustc links with `-nodefaultlibs`, so it is missing: the
//! release build of the client failed to link with it undefined. Only what is still
//! undefined is taken from the library.

use std::path::PathBuf;
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=CC");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("macos") {
        return;
    }
    let cc = std::env::var("CC").unwrap_or_else(|_| "cc".into());
    let resources = Command::new(&cc)
        .arg("-print-resource-dir")
        .output()
        .ok()
        .filter(|out| out.status.success())
        .map(|out| PathBuf::from(String::from_utf8_lossy(&out.stdout).trim()));
    let Some(dir) = resources.map(|r| r.join("lib/darwin")) else {
        println!("cargo:warning={cc} -print-resource-dir failed: clang's runtime is not linked");
        return;
    };
    if !dir.join("libclang_rt.osx.a").is_file() {
        println!(
            "cargo:warning=no libclang_rt.osx.a in {}: clang's runtime is not linked",
            dir.display()
        );
        return;
    }
    println!("cargo:rustc-link-search=native={}", dir.display());
    // Not bundled into the rlib: the archive holds every architecture's objects.
    println!("cargo:rustc-link-lib=static:-bundle=clang_rt.osx");
}

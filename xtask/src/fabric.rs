//! Fabric Loader and the few Fabric API modules the development mods need,
//! pinned by SHA-1 (checked against Maven's published `.sha1` files).
//!
//! Development-only, like the client jar from `fetch-compile-deps`: downloaded
//! into git-ignored `cache/fabric`, never committed, cached or shipped. These
//! are third-party (Apache-2.0) jars; Ruthenium does not use Fabric at runtime.

use std::path::{Path, PathBuf};

use crate::fetch_compile_deps::{fetch, sha1_of};

const FABRIC_MAVEN: &str = "https://maven.fabricmc.net";

/// A Maven artifact: `group:artifact:version` and the jar's SHA-1.
#[derive(Debug)]
pub struct Artifact {
    pub coords: &'static str,
    pub sha1: &'static str,
}

impl Artifact {
    fn rel_path(&self) -> PathBuf {
        let mut parts = self.coords.split(':');
        let (group, artifact, version) = (
            parts.next().expect("group"),
            parts.next().expect("artifact"),
            parts.next().expect("version"),
        );
        let mut path: PathBuf = group.split('.').collect();
        path.push(artifact);
        path.push(version);
        path.push(format!("{artifact}-{version}.jar"));
        path
    }

    fn url(&self) -> String {
        let rel = self.rel_path();
        let parts: Vec<_> = rel.iter().map(|c| c.to_string_lossy()).collect();
        format!("{FABRIC_MAVEN}/{}", parts.join("/"))
    }
}

/// The main class of Fabric Loader's production launcher.
pub const KNOT_CLIENT: &str = "net.fabricmc.loader.impl.launch.knot.KnotClient";

/// Fabric Loader 0.20.1 and its class-path libraries (from the loader's
/// `fabric-installer.json`, `common` libraries).
pub const LOADER: &[Artifact] = &[
    Artifact {
        coords: "net.fabricmc:fabric-loader:0.20.1",
        sha1: "7c7f9831e9f27b303d5d08decb5f4258a9f03296",
    },
    Artifact {
        coords: "org.ow2.asm:asm:9.10.1",
        sha1: "ada2141c0cc52ee8f5c48cd5fa4ce0e794f22236",
    },
    Artifact {
        coords: "org.ow2.asm:asm-analysis:9.10.1",
        sha1: "8d49f14d51f632cb1d87c88d1ceaf50db0d8af1b",
    },
    Artifact {
        coords: "org.ow2.asm:asm-commons:9.10.1",
        sha1: "4229e4c55fd8e01c23f9fe9884075cc628aacc50",
    },
    Artifact {
        coords: "org.ow2.asm:asm-tree:9.10.1",
        sha1: "e244332a17564c1d1572449399a842de35881be2",
    },
    Artifact {
        coords: "org.ow2.asm:asm-util:9.10.1",
        sha1: "7bb9d450e8d4cbf9f9e04096c44bbfe7fba80b15",
    },
    Artifact {
        coords: "net.fabricmc:sponge-mixin:0.17.5+mixin.0.8.7",
        sha1: "0cef8519a91129d0c21fa03bb9378f0e8ca810e5",
    },
];

/// The client gametest module of Fabric API 0.166.2+26.3 and the modules it
/// depends on. Only these are loaded, not all of Fabric API: in particular not
/// its renderer (Indigo), which replaces vanilla's terrain meshing.
pub const GAMETEST_MODS: &[Artifact] = &[
    Artifact {
        coords: "net.fabricmc.fabric-api:fabric-api-base:2.0.6+fcdff87f5d",
        sha1: "449053de5093613bbdf6d439bedd431a7ad77da2",
    },
    Artifact {
        coords: "net.fabricmc.fabric-api:fabric-networking-api-v1:6.4.1+c58129785d",
        sha1: "da15ba8755f95e3ab970a85447a16087f89a4a18",
    },
    Artifact {
        coords: "net.fabricmc.fabric-api:fabric-resource-loader-v1:3.1.0+5af7744b5d",
        sha1: "b012d538fada37b3dfaaf6df7f3e5f85e9d452fe",
    },
    Artifact {
        coords: "net.fabricmc.fabric-api:fabric-client-gametest-api-v1:6.0.12+131f8d475d",
        sha1: "fbec75ee7a6fec75537edb40f8da950ae2fea3ea",
    },
];

/// Downloads (unless already present and intact) and returns the local paths.
pub fn fetch_all(root: &Path, artifacts: &[Artifact]) -> Result<Vec<PathBuf>, String> {
    let dir = root.join("cache/fabric");
    artifacts
        .iter()
        .map(|a| {
            let dest = dir.join(a.rel_path());
            if sha1_of(&dest).as_deref() != Some(a.sha1) {
                fetch(&a.url(), &dest, a.sha1).map_err(|e| format!("{}: {e}", a.coords))?;
            }
            Ok(dest)
        })
        .collect()
}

//! Discovery and import against fake launcher folders. No game files: the
//! "jar", libraries and asset objects are a few bytes each, with a
//! `GameVersion` pinned to their hashes.

use std::path::{Path, PathBuf};

use ruthenium_prepare::discover::{self, Launcher, Rejection};
use ruthenium_prepare::import::{self, ImportError};
use ruthenium_prepare::version::{AssetIndex, GameVersion, JavaRequirement, Library, Pinned};
use sha1::{Digest, Sha1};

const JAR: &[u8] = b"not really a client jar";
const LIB: &[u8] = b"not really a library";
const OBJECTS: [&[u8]; 2] = [b"first object", b"second object"];
const LIB_PATH: &str = "org/example/lib/1.0/lib-1.0.jar";

fn sha1(bytes: &[u8]) -> String {
    Sha1::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn leak(s: String) -> &'static str {
    Box::leak(s.into_boxed_str())
}

fn index_json() -> String {
    let entries: Vec<String> = OBJECTS
        .iter()
        .enumerate()
        .map(|(i, o)| {
            format!(
                "\"obj/{i}\": {{\"hash\": \"{}\", \"size\": {}}}",
                sha1(o),
                o.len()
            )
        })
        .collect();
    format!("{{\"objects\": {{{}}}}}", entries.join(", "))
}

fn version() -> &'static GameVersion {
    let index = index_json();
    Box::leak(Box::new(GameVersion {
        id: "9.9",
        client_jar: Pinned {
            sha1: leak(sha1(JAR)),
            size: JAR.len() as u64,
        },
        asset_index: AssetIndex {
            id: "99",
            sha1: leak(sha1(index.as_bytes())),
            size: index.len() as u64,
        },
        java: JavaRequirement {
            component: "java-runtime-test",
            major: 25,
        },
        libraries: Box::leak(Box::new([Library {
            name: "org.example:lib:1.0",
            path: LIB_PATH,
            sha1: leak(sha1(LIB)),
            size: LIB.len() as u64,
        }])),
    }))
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir()
        .join("ruthenium-prepare-tests")
        .join(format!("{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn put(path: &Path, bytes: &[u8]) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, bytes).unwrap();
}

/// Writes a complete install; `jar` is relative to the root, `libs` and
/// `assets` are the launcher's library and asset folders.
fn fake_install(root: &Path, jar: &str, libs: &str, assets: &str) {
    put(&root.join(jar), JAR);
    put(&root.join(libs).join(LIB_PATH), LIB);
    put(
        &root.join(assets).join("indexes/99.json"),
        index_json().as_bytes(),
    );
    for o in OBJECTS {
        let h = sha1(o);
        put(&root.join(assets).join("objects").join(&h[..2]).join(&h), o);
    }
}

fn fake_prism(root: &Path) {
    fake_install(
        root,
        "libraries/com/mojang/minecraft/9.9/minecraft-9.9-client.jar",
        "libraries",
        "assets",
    );
}

#[test]
fn finds_each_launcher_layout() {
    let v = version();
    let cases = [
        (
            Launcher::Official,
            "versions/9.9/9.9.jar",
            "libraries",
            "assets",
        ),
        (
            Launcher::Prism,
            "libraries/com/mojang/minecraft/9.9/minecraft-9.9-client.jar",
            "libraries",
            "assets",
        ),
        (
            Launcher::Modrinth,
            "meta/versions/9.9-fabric/9.9-fabric.jar",
            "meta/libraries",
            "meta/assets",
        ),
        (
            Launcher::AtLauncher,
            "libraries/net/minecraft/client/9.9/client-9.9.jar",
            "libraries",
            "assets",
        ),
    ];
    for (launcher, jar, libs, assets) in cases {
        let root = scratch(&format!("layout-{launcher:?}"));
        fake_install(&root, jar, libs, assets);
        let install =
            discover::check(launcher, &root, v).unwrap_or_else(|e| panic!("{launcher}: {e}"));
        assert_eq!(install.client_jar, root.join(jar));
    }
}

#[test]
fn instance_folder_resolves_to_launcher_root() {
    let root = scratch("instance");
    fake_prism(&root);
    let instance = root.join("instances/PvP/minecraft");
    std::fs::create_dir_all(&instance).unwrap();
    let found = discover::search_folder(&instance, version());
    let install = discover::first_install(&found).expect("install found");
    assert_eq!(install.root, root);
    // Prism and MultiMC share a layout; the folder is reported once.
    assert_eq!(found.iter().filter(|c| c.outcome.is_ok()).count(), 1);
}

#[test]
fn rejects_what_does_not_match_the_pins() {
    let v = version();
    let root = scratch("reject");
    assert!(matches!(
        discover::check(Launcher::Prism, &root.join("nothing"), v),
        Err(Rejection::NotFound)
    ));

    fake_prism(&root);
    let jar = root.join("libraries/com/mojang/minecraft/9.9/minecraft-9.9-client.jar");
    // Same size, different bytes: a modified jar.
    let mut modified = JAR.to_vec();
    modified[0] ^= 1;
    std::fs::write(&jar, &modified).unwrap();
    assert!(matches!(
        discover::check(Launcher::Prism, &root, v),
        Err(Rejection::ClientJarMismatch { .. })
    ));
    std::fs::write(&jar, JAR).unwrap();

    std::fs::remove_file(root.join("libraries").join(LIB_PATH)).unwrap();
    match discover::check(Launcher::Prism, &root, v) {
        Err(Rejection::MissingLibraries(names)) => assert_eq!(names, ["org.example:lib:1.0"]),
        other => panic!("{other:?}"),
    }
    put(&root.join("libraries").join(LIB_PATH), LIB);

    std::fs::remove_file(root.join("assets/indexes/99.json")).unwrap();
    assert!(matches!(
        discover::check(Launcher::Prism, &root, v),
        Err(Rejection::NoAssetIndex(_))
    ));
}

#[test]
fn imports_once_and_verifies_objects() {
    let v = version();
    let root = scratch("import-src");
    fake_prism(&root);
    let cache = scratch("import-cache");
    let install = discover::check(Launcher::Prism, &root, v).unwrap();

    assert!(import::open(v, &cache).is_none());
    let (imported, stats) = import::import(&install, v, &cache).unwrap();
    assert_eq!(stats.objects, 2);
    assert_eq!(stats.linked + stats.copied, 5); // jar, library, index, 2 objects
    assert_eq!(std::fs::read(&imported.client_jar).unwrap(), JAR);
    assert!(import::open(v, &cache).is_some());

    // Re-importing keeps what is already there.
    let (_, again) = import::import(&install, v, &cache).unwrap();
    assert_eq!(again.present, 5);
}

#[test]
fn import_refuses_damaged_or_missing_objects() {
    let v = version();
    let root = scratch("damaged-src");
    fake_prism(&root);
    let install = discover::check(Launcher::Prism, &root, v).unwrap();
    let h = sha1(OBJECTS[0]);
    let object = root.join("assets/objects").join(&h[..2]).join(&h);

    std::fs::write(&object, b"tampered").unwrap();
    let cache = scratch("damaged-cache");
    assert!(matches!(
        import::import(&install, v, &cache),
        Err(ImportError::Mismatch { .. })
    ));
    assert!(import::open(v, &cache).is_none());

    std::fs::remove_file(&object).unwrap();
    match import::import(&install, v, &cache) {
        Err(ImportError::MissingObjects { missing, total, .. }) => {
            assert_eq!((missing, total), (1, 2));
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn steps_run_once_per_key() {
    let dir = scratch("step").join("out");
    let mut runs = 0;
    for _ in 0..2 {
        ruthenium_prepare::step::run::<std::io::Error>(&dir, "key-a", |tmp| {
            runs += 1;
            std::fs::write(tmp.join("result"), "a")
        })
        .unwrap();
    }
    assert_eq!(runs, 1);
    let ran = ruthenium_prepare::step::run::<std::io::Error>(&dir, "key-b", |tmp| {
        std::fs::write(tmp.join("result"), "b")
    })
    .unwrap();
    assert!(ran);
    assert_eq!(std::fs::read_to_string(dir.join("result")).unwrap(), "b");
}

#[cfg(not(windows))]
#[test]
fn default_roots_follow_xdg_and_flatpak() {
    let env = discover::Env {
        home: Some("/h".into()),
        ..Default::default()
    };
    let roots = discover::default_roots(&env);
    let has = |l: Launcher, p: &str| roots.iter().any(|(rl, rp)| *rl == l && rp == Path::new(p));
    assert!(has(Launcher::Official, "/h/.minecraft"));
    assert!(has(Launcher::Prism, "/h/.local/share/PrismLauncher"));
    assert!(has(
        Launcher::Prism,
        "/h/.var/app/org.prismlauncher.PrismLauncher/data/PrismLauncher"
    ));
    assert!(has(
        Launcher::GdLauncher,
        "/h/.config/gdlauncher_next/datastore"
    ));
    for launcher in Launcher::ALL {
        assert!(
            roots.iter().any(|(l, _)| *l == launcher),
            "{launcher} has no default folder"
        );
    }
}

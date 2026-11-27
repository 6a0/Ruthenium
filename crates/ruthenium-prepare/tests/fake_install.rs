//! Discovery and import against fake launcher folders. No game files: the
//! "jar", libraries and asset objects are a few bytes each, with a
//! `GameVersion` pinned to their hashes.

use std::path::{Path, PathBuf};

use ruthenium_prepare::asset_index::{self, IndexError};
use ruthenium_prepare::discover::{self, Launcher, Rejection};
use ruthenium_prepare::import::{self, ImportError};
use ruthenium_prepare::version::{AssetIndex, GameVersion, JavaRequirement, Library, Pinned};
use sha1::{Digest, Sha1};

const JAR: &[u8] = b"not really a client jar";
const LIB: &[u8] = b"not really a library";
const OBJECTS: [&[u8]; 2] = [b"first object", b"second object"];
/// A translation, and what Mojang republished it as.
const LANG: &[u8] = b"a translation";
const NEW_LANG: &[u8] = b"a better translation";
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

fn entry(name: &str, object: &[u8]) -> String {
    format!(
        "\"{name}\": {{\"hash\": \"{}\", \"size\": {}}}",
        sha1(object),
        object.len()
    )
}

/// The index with `lang` as its translation and `first` as its first object.
fn index_with(first: &[u8], lang: &[u8]) -> String {
    let entries = [
        entry("obj/0", first),
        entry("obj/1", OBJECTS[1]),
        entry("minecraft/lang/xx_yy.json", lang),
    ];
    format!("{{\"objects\": {{{}}}}}", entries.join(", "))
}

fn index_json() -> String {
    index_with(OBJECTS[0], LANG)
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
            pinned_sha256: leak(asset_index::pinned_digest(
                &asset_index::parse(index.as_bytes()).unwrap(),
            )),
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

/// A fresh folder, deleted with the guard even when the test fails.
fn scratch(name: &str) -> (tempfile::TempDir, PathBuf) {
    let tmp = tempfile::Builder::new()
        .prefix(&format!("ruthenium-{name}-"))
        .tempdir()
        .unwrap();
    let dir = tmp.path().to_path_buf();
    (tmp, dir)
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
    for o in OBJECTS.into_iter().chain([LANG]) {
        put(&object(&root.join(assets), o), o);
    }
}

/// Where an object is in an assets folder.
fn object(assets: &Path, bytes: &[u8]) -> PathBuf {
    let h = sha1(bytes);
    assets.join("objects").join(&h[..2]).join(&h)
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
        let (_root, root) = scratch(&format!("layout-{launcher:?}"));
        fake_install(&root, jar, libs, assets);
        let install =
            discover::check(launcher, &root, v).unwrap_or_else(|e| panic!("{launcher}: {e}"));
        assert_eq!(install.client_jar, root.join(jar));
    }
}

#[test]
fn instance_folder_resolves_to_launcher_root() {
    let (_root, root) = scratch("instance");
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
    let (_root, root) = scratch("reject");
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
    let (_root, root) = scratch("import-src");
    fake_prism(&root);
    let (_cache, cache) = scratch("import-cache");
    let install = discover::check(Launcher::Prism, &root, v).unwrap();

    assert!(import::open(v, &cache).is_none());
    let (imported, stats) = import::import(&install, v, &cache).unwrap();
    assert_eq!(stats.objects, 3);
    assert_eq!(stats.linked + stats.copied, 6); // jar, library, index, 3 objects
    assert_eq!(std::fs::read(&imported.client_jar).unwrap(), JAR);
    assert!(import::open(v, &cache).is_some());

    // Re-importing keeps what is already there.
    let (_, again) = import::import(&install, v, &cache).unwrap();
    assert_eq!(again.present, 6);
}

#[test]
fn accepts_an_index_with_republished_translations() {
    let v = version();
    let (_root, root) = scratch("lang-src");
    fake_prism(&root);
    let (_cache, cache) = scratch("lang-cache");
    let index = root.join("assets/indexes/99.json");
    let install = discover::check(Launcher::Prism, &root, v).unwrap();
    let (imported, _) = import::import(&install, v, &cache).unwrap();
    assert_eq!(
        std::fs::read_to_string(imported.asset_index()).unwrap(),
        index_json()
    );

    // The launcher fetches the republished index and its translation, writing
    // the index in place. The import already made is not touched by that.
    let republished = index_with(OBJECTS[0], NEW_LANG);
    assert_ne!(republished, index_json());
    std::fs::OpenOptions::new()
        .write(true)
        .truncate(true)
        .open(&index)
        .and_then(|mut f| std::io::Write::write_all(&mut f, republished.as_bytes()))
        .unwrap();
    put(&object(&root.join("assets"), NEW_LANG), NEW_LANG);
    assert_eq!(
        std::fs::read_to_string(imported.asset_index()).unwrap(),
        index_json()
    );
    assert!(import::open(v, &cache).is_some());

    // The install is still verified, and importing it brings its index and
    // its translation, checked like any object.
    let install = discover::check(Launcher::Prism, &root, v).unwrap();
    let (imported, stats) = import::import(&install, v, &cache).unwrap();
    assert_eq!(stats.objects, 3);
    assert_eq!((stats.linked + stats.copied, stats.present), (2, 4));
    assert_eq!(
        std::fs::read_to_string(imported.asset_index()).unwrap(),
        republished
    );
    assert_eq!(
        std::fs::read(object(&imported.assets, NEW_LANG)).unwrap(),
        NEW_LANG
    );

    // A republished translation that is damaged does not get in.
    let (_cache, cache) = scratch("lang-damaged-cache");
    std::fs::write(object(&root.join("assets"), NEW_LANG), b"tampered").unwrap();
    assert!(matches!(
        import::import(&install, v, &cache),
        Err(ImportError::Mismatch { .. })
    ));
    assert!(import::open(v, &cache).is_none());
}

#[test]
fn rejects_an_index_whose_other_files_changed() {
    let v = version();
    let (_root, root) = scratch("index-src");
    fake_prism(&root);
    let index = root.join("assets/indexes/99.json");
    let install = discover::check(Launcher::Prism, &root, v).unwrap();

    // Another file for obj/0, present and intact in the object store.
    let other: &[u8] = b"another object";
    put(&object(&root.join("assets"), other), other);
    let changed = index_with(other, LANG);
    // A file more, and a translation's name on something that is not one.
    let more = index_json().replace("\"obj/1\"", "\"obj/2\"");
    let disguised = index_json().replace("lang/xx_yy.json", "lang/../xx_yy.json");
    for bad in [&changed, &more, &disguised] {
        std::fs::write(&index, bad).unwrap();
        match discover::check(Launcher::Prism, &root, v) {
            Err(Rejection::AssetIndexMismatch {
                why: IndexError::Mismatch { .. },
                ..
            }) => {}
            other => panic!("{bad}: {other:?}"),
        }
        // An install verified before its index changed is not imported either.
        let (_cache, cache) = scratch("index-cache");
        match import::import(&install, v, &cache) {
            Err(ImportError::BadAssetIndex { .. }) => {}
            other => panic!("{bad}: {other:?}"),
        }
        assert!(import::open(v, &cache).is_none());
        assert!(!cache.join("assets/indexes/99.json").exists());
    }

    std::fs::write(&index, "not an index").unwrap();
    match discover::check(Launcher::Prism, &root, v) {
        Err(Rejection::AssetIndexMismatch {
            why: IndexError::Malformed(_),
            ..
        }) => {}
        other => panic!("{other:?}"),
    }
}

#[test]
fn import_refuses_damaged_or_missing_objects() {
    let v = version();
    let (_root, root) = scratch("damaged-src");
    fake_prism(&root);
    let install = discover::check(Launcher::Prism, &root, v).unwrap();
    let object = object(&root.join("assets"), OBJECTS[0]);

    std::fs::write(&object, b"tampered").unwrap();
    let (_cache, cache) = scratch("damaged-cache");
    assert!(matches!(
        import::import(&install, v, &cache),
        Err(ImportError::Mismatch { .. })
    ));
    assert!(import::open(v, &cache).is_none());

    std::fs::remove_file(&object).unwrap();
    match import::import(&install, v, &cache) {
        Err(ImportError::MissingObjects { missing, total, .. }) => {
            assert_eq!((missing, total), (1, 3));
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn steps_run_once_per_key() {
    let (_dir, dir) = scratch("step");
    let dir = dir.join("out");
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

#[cfg(target_os = "macos")]
#[test]
fn default_roots_use_application_support() {
    let env = discover::Env {
        home: Some("/h".into()),
        ..Default::default()
    };
    let roots = discover::default_roots(&env);
    let has = |l: Launcher, p: &str| roots.iter().any(|(rl, rp)| *rl == l && rp == Path::new(p));
    assert!(has(
        Launcher::Official,
        "/h/Library/Application Support/minecraft"
    ));
    assert!(has(
        Launcher::Prism,
        "/h/Library/Application Support/PrismLauncher"
    ));
    assert!(has(
        Launcher::Modrinth,
        "/h/Library/Application Support/ModrinthApp"
    ));
}

#[cfg(all(unix, not(target_os = "macos")))]
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

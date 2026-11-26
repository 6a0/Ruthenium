//! The asset index, and how much of it is pinned.
//!
//! Mojang republishes a released version's asset index when translations
//! change: the same file names, with new hashes for some files under
//! `minecraft/lang/`. A launcher has whichever index was current when it last
//! looked, so the file as a whole has no one hash to pin. What is pinned is a
//! digest of every entry that is not a translation: its name, SHA-1 and size.
//! Translations may differ from one install to the next. Like every asset
//! file, each is still checked against the SHA-1 its entry names when it is
//! imported.

use std::fmt;

use crate::hash::sha256_hex;
use crate::version::AssetIndex;

/// One file of an asset index.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Entry {
    /// The file's name in the game, e.g. `minecraft/sounds/ambient/cave/cave1.ogg`.
    pub name: String,
    /// Lowercase hex SHA-1 of its contents; also where it is in `objects/`.
    pub hash: String,
    pub size: u64,
}

/// Why an asset index is not the pinned one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IndexError {
    /// Not an asset index at all.
    Malformed(String),
    /// Its entries other than translations are not the pinned ones.
    Mismatch { digest: String },
}

impl fmt::Display for IndexError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            IndexError::Malformed(reason) => f.write_str(reason),
            IndexError::Mismatch { digest } => write!(
                f,
                "its files other than translations are not the official ones (digest {digest})"
            ),
        }
    }
}

impl std::error::Error for IndexError {}

/// Reads an asset index: its entries, sorted by name.
pub fn parse(bytes: &[u8]) -> Result<Vec<Entry>, IndexError> {
    let bad = IndexError::Malformed;
    let json: serde_json::Value = serde_json::from_slice(bytes).map_err(|e| bad(e.to_string()))?;
    let objects = json["objects"]
        .as_object()
        .ok_or_else(|| bad("no \"objects\" map".into()))?;
    let mut out = Vec::with_capacity(objects.len());
    for (name, obj) in objects {
        let hash = obj["hash"].as_str().filter(|h| {
            h.len() == 40
                && h.bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        });
        let (Some(hash), Some(size)) = (hash, obj["size"].as_u64()) else {
            return Err(bad(format!("bad entry {name:?}")));
        };
        out.push(Entry {
            name: name.clone(),
            hash: hash.to_owned(),
            size,
        });
    }
    out.sort();
    Ok(out)
}

/// Whether an entry is a translation, which the pinned digest leaves out:
/// `minecraft/lang/<code>.json`, the code made of lowercase letters, digits
/// and `_`. Anything else under `minecraft/lang/` is pinned like any other
/// file.
pub fn is_translation(name: &str) -> bool {
    name.strip_prefix("minecraft/lang/")
        .and_then(|rest| rest.strip_suffix(".json"))
        .is_some_and(|code| {
            !code.is_empty()
                && code
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
        })
}

/// The digest pinned for a version's asset index: SHA-256 over the name
/// (length first), SHA-1 and size of every entry that is not a translation,
/// in order of name. `cargo xtask pin-version` writes it into the pins.
pub fn pinned_digest(entries: &[Entry]) -> String {
    let mut pinned: Vec<&Entry> = entries
        .iter()
        .filter(|e| !is_translation(&e.name))
        .collect();
    pinned.sort();
    let mut bytes = b"ruthenium asset index v1\n".to_vec();
    for e in pinned {
        bytes.extend((e.name.len() as u64).to_le_bytes());
        bytes.extend(e.name.as_bytes());
        bytes.extend(e.hash.as_bytes());
        bytes.extend(e.size.to_le_bytes());
    }
    sha256_hex(&bytes)
}

/// Reads an asset index and checks it against the pin.
pub fn verify(bytes: &[u8], pin: &AssetIndex) -> Result<Vec<Entry>, IndexError> {
    let entries = parse(bytes)?;
    let digest = pinned_digest(&entries);
    if digest != pin.pinned_sha256 {
        return Err(IndexError::Mismatch { digest });
    }
    Ok(entries)
}

#[cfg(test)]
mod tests {
    use super::*;

    const INDEX: &str = r#"{"objects": {
        "minecraft/sounds/a.ogg": {"hash": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa", "size": 3},
        "icons/icon.png": {"hash": "0123456789abcdef0123456789abcdef01234567", "size": 4096},
        "minecraft/lang/de_de.json": {"hash": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb", "size": 7}
    }}"#;
    /// Worked out apart from this code: the pins hold digests made this way,
    /// so the way must not change.
    const DIGEST: &str = "5c60760ccf46fdb2c6df8f39d7ec2e92741f3b30cf9b28ca000e1f67f9eae100";

    fn digest(index: &str) -> String {
        pinned_digest(&parse(index.as_bytes()).unwrap())
    }

    #[test]
    fn the_digest_is_the_one_the_pins_were_made_with() {
        let entries = parse(INDEX.as_bytes()).unwrap();
        let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "icons/icon.png",
                "minecraft/lang/de_de.json",
                "minecraft/sounds/a.ogg"
            ]
        );
        assert_eq!(pinned_digest(&entries), DIGEST);
        // The order of the entries given is not part of it.
        let mut reversed = entries.clone();
        reversed.reverse();
        assert_eq!(pinned_digest(&reversed), DIGEST);
    }

    #[test]
    fn translations_may_change_and_nothing_else() {
        let same = [
            // Another translation of the same language, a new language, one fewer.
            INDEX.replace(&"b".repeat(40), &"c".repeat(40)),
            INDEX.replace("\"size\": 7", "\"size\": 70000"),
            INDEX.replace("de_de", "tlh_aa"),
            INDEX.replace(
                "\"icons/",
                "\"minecraft/lang/x9.json\": {\"hash\": \"dddddddddddddddddddddddddddddddddddddddd\", \"size\": 1}, \"icons/",
            ),
        ];
        for index in &same {
            assert_ne!(index, INDEX);
            assert_eq!(digest(index), DIGEST, "{index}");
        }
        let different = [
            // A sound's contents, its size, its name; an icon's contents.
            INDEX.replace(&"a".repeat(40), &"c".repeat(40)),
            INDEX.replace("\"size\": 3", "\"size\": 4"),
            INDEX.replace("a.ogg", "b.ogg"),
            INDEX.replace("0123456789abcdef0123", "1123456789abcdef0123"),
            // A file more, and a file that only looks like a translation.
            INDEX.replace(
                "\"icons/",
                "\"minecraft/x\": {\"hash\": \"dddddddddddddddddddddddddddddddddddddddd\", \"size\": 1}, \"icons/",
            ),
            INDEX.replace("lang/de_de.json", "lang/de_de.png"),
            INDEX.replace("lang/de_de.json", "lang/../textures/de_de.json"),
            INDEX.replace("lang/de_de.json", "lang/sub/de_de.json"),
            INDEX.replace("lang/de_de.json", "lang/DE_de.json"),
            INDEX.replace("lang/de_de.json", "lang/.json"),
            INDEX.replace("minecraft/lang/de_de.json", "realms/lang/de_de.json"),
        ];
        for index in &different {
            assert_ne!(digest(index), DIGEST, "{index}");
        }
        // A file fewer.
        let fewer = r#"{"objects": {"minecraft/sounds/a.ogg": {"hash": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa", "size": 3}}}"#;
        assert_ne!(digest(fewer), DIGEST);
    }

    #[test]
    fn one_long_name_is_not_two_entries() {
        let entry = |name: &str, hash: &str, size| Entry {
            name: name.into(),
            hash: hash.repeat(40),
            size,
        };
        let two = [entry("a", "c", 1), entry("b", "d", 2)];
        // One entry whose name spells out the first of the two and the second's
        // name: the same bytes of names, hashes and sizes, one after another.
        let name = format!("a{}\u{1}\0\0\0\0\0\0\0b", "c".repeat(40));
        let one = [entry(&name, "d", 2)];
        assert_ne!(pinned_digest(&two), pinned_digest(&one));
    }

    #[test]
    fn malformed_indexes_are_refused() {
        for bad in [
            "",
            "[]",
            "{}",
            r#"{"objects": []}"#,
            r#"{"objects": {"a": {"size": 1}}}"#,
            r#"{"objects": {"a": {"hash": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}}}"#,
            r#"{"objects": {"a": {"hash": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa", "size": -1}}}"#,
            // Too short, upper case, and a path instead of a hash.
            r#"{"objects": {"a": {"hash": "aaaa", "size": 1}}}"#,
            r#"{"objects": {"a": {"hash": "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA", "size": 1}}}"#,
            r#"{"objects": {"a": {"hash": "../../../../../../../../../../../../../x", "size": 1}}}"#,
        ] {
            assert!(
                matches!(parse(bad.as_bytes()), Err(IndexError::Malformed(_))),
                "{bad}"
            );
        }
    }

    #[test]
    fn verify_names_the_digest_it_found() {
        let pin = |sha: &'static str| AssetIndex {
            id: "1",
            pinned_sha256: sha,
        };
        assert_eq!(verify(INDEX.as_bytes(), &pin(DIGEST)).unwrap().len(), 3);
        let wrong = "0000000000000000000000000000000000000000000000000000000000000000";
        assert_eq!(
            verify(INDEX.as_bytes(), &pin(wrong)),
            Err(IndexError::Mismatch {
                digest: DIGEST.into()
            })
        );
    }
}

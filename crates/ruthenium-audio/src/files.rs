//! The sound files of a pack stack: every namespace's `sounds.json` and the `.ogg`
//! files under `sounds/` (`Sound.SOUND_LISTER`). In vanilla's pack they are asset index
//! objects, not in the client jar.

use std::collections::HashSet;
use std::sync::Arc;

use ruthenium_assets::{Id, Resources};

use crate::events::{SoundEvents, SoundsJson, id_key};

/// See the module docs.
#[derive(Debug)]
pub struct SoundFiles {
    resources: Arc<Resources>,
    /// Every `.ogg` under `sounds/`, as `namespace:path`.
    ogg: HashSet<String>,
}

impl SoundFiles {
    /// The sound files of `resources` (shared with others that read the same packs, or
    /// its own).
    pub fn new(resources: impl Into<Arc<Resources>>) -> SoundFiles {
        let resources = resources.into();
        let ogg = resources
            .list("sounds", |p| p.ends_with(".ogg"))
            .keys()
            .map(id_key)
            .collect();
        SoundFiles { resources, ogg }
    }

    pub fn has(&self, file: &Id) -> bool {
        self.ogg.contains(&id_key(file))
    }

    pub fn files(&self) -> usize {
        self.ogg.len()
    }

    /// Every sound file, as `namespace:path`.
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.ogg.iter().map(String::as_str)
    }

    pub fn read(&self, file: &Id) -> std::io::Result<Vec<u8>> {
        self.resources
            .get(file)
            .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, id_key(file)))?
            .read()
            .map(|b| b.into_owned())
    }

    /// The events of every pack's `sounds.json`, lowest pack first, with the warnings.
    pub fn events(&self) -> (SoundEvents, Vec<String>) {
        let mut namespaces: Vec<&str> = self
            .resources
            .packs()
            .iter()
            .flat_map(|p| p.namespaces().iter().map(String::as_str))
            .collect();
        namespaces.sort_unstable();
        namespaces.dedup();
        let mut files: Vec<(String, String, Vec<u8>)> = Vec::new();
        for ns in namespaces {
            let Some(id) = Id::new(ns, "sounds.json") else {
                continue;
            };
            for r in self.resources.stack(&id) {
                match r.read() {
                    Ok(b) => {
                        files.push((ns.to_owned(), r.pack().name().to_owned(), b.into_owned()))
                    }
                    Err(e) => eprintln!("[sound] {}: {ns}:sounds.json: {e}", r.pack().name()),
                }
            }
        }
        SoundEvents::load(
            files.iter().map(|(namespace, pack, bytes)| SoundsJson {
                namespace,
                pack,
                bytes,
            }),
            |f| self.has(f),
        )
    }
}

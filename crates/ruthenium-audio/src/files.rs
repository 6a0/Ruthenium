//! The sound files of a pack stack: every namespace's `sounds.json` and the `.ogg`
//! files under `sounds/` (`Sound.SOUND_LISTER`). In vanilla's pack they are asset index
//! objects, not in the client jar.

use std::collections::HashSet;
use std::sync::Arc;

use ruthenium_assets::{Id, Limits, Resources};

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

    /// A sound file's bytes, read while the game plays: not counted against its pack's
    /// budget for a load, but no larger than a file of it may be.
    pub fn read(&self, file: &Id) -> std::io::Result<Vec<u8>> {
        self.open(file).map(|(bytes, _)| bytes)
    }

    /// [`Self::read`], with the limits of the pack the file is in (what decoding it is
    /// held to).
    pub fn open(&self, file: &Id) -> std::io::Result<(Vec<u8>, Limits)> {
        let r = self
            .resources
            .get(file)
            .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, id_key(file)))?;
        Ok((r.read_on_demand()?.into_owned(), *r.limits()))
    }

    /// The events of every pack's `sounds.json`, lowest pack first, with the warnings.
    /// Streams one `sounds.json` at a time and respects cancellation.
    pub fn events(&self) -> (SoundEvents, Vec<String>) {
        let mut namespaces: Vec<&str> = self
            .resources
            .packs()
            .iter()
            .flat_map(|p| p.namespaces().iter().map(String::as_str))
            .collect();
        namespaces.sort_unstable();
        namespaces.dedup();
        let max_total = self.resources.limits().sound_events;
        let mut events = SoundEvents::default();
        let mut warnings = Vec::new();
        for ns in namespaces {
            if self.resources.cancelled() {
                break;
            }
            let Some(id) = Id::new(ns, "sounds.json") else {
                continue;
            };
            for r in self.resources.stack(&id) {
                if self.resources.cancelled() {
                    break;
                }
                // `text` keeps a file over the JSON limit from being read at all.
                match r.text() {
                    Ok(b) => {
                        let f = SoundsJson {
                            namespace: ns,
                            pack: r.pack().name(),
                            bytes: b.as_bytes(),
                            limits: *r.limits(),
                        };
                        events.add_file(&f, max_total, &|file| self.has(file), &mut warnings);
                    }
                    Err(e) => eprintln!("[sound] {}: {ns}:sounds.json: {e}", r.pack().name()),
                }
            }
        }
        if !self.resources.cancelled() {
            events.finish_loading();
        }
        (events, warnings)
    }
}

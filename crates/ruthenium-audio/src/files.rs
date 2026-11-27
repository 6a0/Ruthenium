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

    /// Whether the load of these files was given up (a reload cancelled: its packs then
    /// find nothing).
    pub fn cancelled(&self) -> bool {
        self.resources.cancelled()
    }

    /// The events of every pack's `sounds.json`, lowest pack first, with the warnings.
    /// Streams one `sounds.json` at a time and respects cancellation: a load given up
    /// part way hands on no events at all (those read so far were never weighed, so
    /// every one of them would play nothing, and only some of the packs were read).
    pub fn events(&self) -> (SoundEvents, Vec<String>) {
        self.events_unless(&|| self.resources.cancelled())
    }

    /// [`Self::events`], given up once `cancelled` says so.
    fn events_unless(&self, cancelled: &dyn Fn() -> bool) -> (SoundEvents, Vec<String>) {
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
            if cancelled() {
                break;
            }
            let Some(id) = Id::new(ns, "sounds.json") else {
                continue;
            };
            for r in self.resources.stack(&id) {
                if cancelled() {
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
        if cancelled() {
            warnings.push("the sounds' load was cancelled: no events".into());
            return (SoundEvents::default(), warnings);
        }
        events.finish_loading();
        (events, warnings)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    /// A folder pack with a `sounds.json` in two namespaces.
    fn two_namespaces() -> (tempfile::TempDir, SoundFiles) {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("pack");
        let write = |p: &str, b: &[u8]| {
            let path = root.join(p);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, b).unwrap();
        };
        write("pack.mcmeta", br#"{"pack": {"description": ""}}"#);
        for ns in ["a", "b"] {
            write(&format!("assets/{ns}/sounds/x.ogg"), b"OggS");
            write(
                &format!("assets/{ns}/sounds.json"),
                format!(r#"{{"e": {{"sounds": ["{ns}:x"]}}}}"#).as_bytes(),
            );
        }
        let pack = ruthenium_assets::Pack::open(&root).unwrap();
        (tmp, SoundFiles::new(Resources::new(vec![pack])))
    }

    /// A load cancelled after its first `sounds.json` hands on no events: those read
    /// so far were never weighed (each would play nothing), and the rest weren't read.
    #[test]
    fn a_cancelled_load_hands_on_no_events() {
        let (_tmp, files) = two_namespaces();
        let (events, w) = files.events();
        assert_eq!(events.len(), 2, "{w:?}");
        let mut random = ruthenium_javamath::random::JavaRandom::new(1);
        let choice = events.choose("a:e", &mut random);
        assert!(
            matches!(choice, crate::events::Choice::Sound(_)),
            "{choice:?} of {:?}",
            events.names().collect::<Vec<_>>()
        );
        // Cancelled once the first namespace's file has been read.
        let asked = Cell::new(0);
        let (events, w) = files.events_unless(&|| {
            asked.set(asked.get() + 1);
            asked.get() > 2
        });
        assert!(asked.get() > 2, "the cancel was looked at");
        assert!(
            events.is_empty(),
            "{:?}",
            events.names().collect::<Vec<_>>()
        );
        assert!(w.iter().any(|w| w.contains("cancelled")), "{w:?}");
    }
}

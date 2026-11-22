//! The resource stack: packs, bottom (vanilla) to top, with each pack's
//! filter applied to the packs below it.
//!
//! Lookup rules follow `server/packs/resources/MultiPackResourceManager.java`
//! and `FallbackResourceManager.java` (26.3): which pack wins, where a file's
//! `.mcmeta` comes from, and how filters hide lower packs' files.

use std::borrow::Cow;
use std::collections::{BTreeMap, HashMap};
use std::io;

use crate::id::Id;
use crate::meta::Metadata;
use crate::pack::{Loc, Pack};

/// One pack's place in a namespace's stack.
#[derive(Debug, Clone, Copy)]
struct Slot {
    pack: usize,
    /// The pack has files in this namespace.
    content: bool,
    /// The pack's filter names this namespace, so its path filter applies.
    filtered: bool,
}

/// The loaded packs.
#[derive(Debug)]
pub struct Resources {
    packs: Vec<Pack>,
    namespaces: HashMap<String, Vec<Slot>>,
}

/// A file found in the stack.
#[derive(Debug, Clone)]
pub struct Resource<'a> {
    res: &'a Resources,
    pack: usize,
    loc: Loc,
    meta: Option<(usize, Loc)>,
}

impl<'a> Resource<'a> {
    /// The pack the file comes from.
    pub fn pack(&self) -> &'a Pack {
        &self.res.packs[self.pack]
    }

    pub fn read(&self) -> io::Result<Cow<'a, [u8]>> {
        self.res.packs[self.pack].read(&self.loc)
    }

    /// The file's `.mcmeta`, or empty metadata if it has none.
    pub fn metadata(&self) -> Result<Metadata, String> {
        match &self.meta {
            None => Ok(Metadata::default()),
            Some((pack, loc)) => {
                let bytes = self.res.packs[*pack].read(loc).map_err(|e| e.to_string())?;
                Metadata::parse(&bytes)
            }
        }
    }
}

fn mcmeta(path: &str) -> String {
    format!("{path}.mcmeta")
}

impl Resources {
    /// `packs` from lowest to highest priority; the vanilla pack first.
    pub fn new(packs: Vec<Pack>) -> Resources {
        let mut all: Vec<&str> = packs
            .iter()
            .flat_map(|p| p.namespaces().iter().map(String::as_str))
            .collect();
        all.sort_unstable();
        all.dedup();
        let mut namespaces: HashMap<String, Vec<Slot>> = HashMap::new();
        for (i, pack) in packs.iter().enumerate() {
            for &ns in &all {
                let content = pack.namespaces().contains(ns);
                let filtered = pack.filter().is_some_and(|f| f.filters_namespace(ns));
                if content || filtered {
                    namespaces.entry(ns.to_owned()).or_default().push(Slot {
                        pack: i,
                        content,
                        filtered,
                    });
                }
            }
        }
        Resources { packs, namespaces }
    }

    pub fn packs(&self) -> &[Pack] {
        &self.packs
    }

    fn slots(&self, namespace: &str) -> &[Slot] {
        self.namespaces.get(namespace).map_or(&[], Vec::as_slice)
    }

    fn filters(&self, slot: &Slot, path: &str) -> bool {
        slot.filtered
            && self.packs[slot.pack]
                .filter()
                .is_some_and(|f| f.filters_path(path))
    }

    fn find(&self, slot: &Slot, namespace: &str, path: &str) -> Option<Loc> {
        slot.content
            .then(|| self.packs[slot.pack].find(namespace, path))
            .flatten()
    }

    /// The highest pack's copy of `id`. A filter above every copy hides it.
    /// Its metadata is the highest `.mcmeta` at or above that pack.
    pub fn get(&self, id: &Id) -> Option<Resource<'_>> {
        let (ns, path) = (id.namespace(), id.path());
        let slots = self.slots(ns);
        for (i, slot) in slots.iter().enumerate().rev() {
            if let Some(loc) = self.find(slot, ns, path) {
                let meta_path = mcmeta(path);
                let mut meta = None;
                for s in slots[i..].iter().rev() {
                    if let Some(l) = self.find(s, ns, &meta_path) {
                        meta = Some((s.pack, l));
                        break;
                    }
                    if self.filters(s, &meta_path) {
                        break;
                    }
                }
                return Some(Resource {
                    res: self,
                    pack: slot.pack,
                    loc,
                    meta,
                });
            }
            if self.filters(slot, path) {
                return None;
            }
        }
        None
    }

    /// Every copy of `id`, lowest pack first, down to the highest filter
    /// that hides it. Each copy's metadata comes from its own pack.
    pub fn stack(&self, id: &Id) -> Vec<Resource<'_>> {
        let (ns, path) = (id.namespace(), id.path());
        let meta_path = mcmeta(path);
        let mut out = Vec::new();
        let mut drop_meta = false;
        for slot in self.slots(ns).iter().rev() {
            if let Some(loc) = self.find(slot, ns, path) {
                let meta = if drop_meta {
                    None
                } else {
                    self.find(slot, ns, &meta_path).map(|l| (slot.pack, l))
                };
                out.push(Resource {
                    res: self,
                    pack: slot.pack,
                    loc,
                    meta,
                });
            }
            if self.filters(slot, path) {
                break;
            }
            if self.filters(slot, &meta_path) {
                drop_meta = true;
            }
        }
        out.reverse();
        out
    }

    /// The winning copy of every file under `<namespace>/<dir>/` in every
    /// namespace whose path passes `select`, sorted by id. A file's metadata
    /// is the highest `.mcmeta` at or above its pack.
    pub fn list(&self, dir: &str, select: impl Fn(&str) -> bool) -> BTreeMap<Id, Resource<'_>> {
        let mut out = BTreeMap::new();
        for (ns, slots) in &self.namespaces {
            let mut files: HashMap<String, (usize, usize, Loc)> = HashMap::new();
            let mut metas: HashMap<String, (usize, usize, Loc)> = HashMap::new();
            for (i, slot) in slots.iter().enumerate() {
                if slot.filtered {
                    files.retain(|p, _| !self.filters(slot, p));
                    metas.retain(|p, _| !self.filters(slot, p));
                }
                if !slot.content {
                    continue;
                }
                self.packs[slot.pack].list(ns, dir, &mut |path, loc| {
                    if let Some(base) = path.strip_suffix(".mcmeta") {
                        if select(base) {
                            metas.insert(path, (i, slot.pack, loc));
                        }
                    } else if select(&path) {
                        files.insert(path, (i, slot.pack, loc));
                    }
                });
            }
            for (path, (slot, pack, loc)) in files {
                let meta = metas
                    .get(&mcmeta(&path))
                    .filter(|(s, _, _)| *s >= slot)
                    .map(|(_, p, l)| (*p, l.clone()));
                if let Some(id) = Id::new(ns, &path) {
                    out.insert(
                        id,
                        Resource {
                            res: self,
                            pack,
                            loc,
                            meta,
                        },
                    );
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(name: &str) -> TempDir {
            let p = std::env::temp_dir()
                .join(format!("ruthenium-assets-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&p);
            std::fs::create_dir_all(&p).unwrap();
            TempDir(p)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn file(root: &Path, rel: &str, contents: &str) {
        let p = root.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, contents).unwrap();
    }

    fn pack(root: &Path, name: &str, mcmeta: &str, files: &[(&str, &str)]) -> Pack {
        let dir = root.join(name);
        file(&dir, "pack.mcmeta", mcmeta);
        for (rel, contents) in files {
            file(&dir, rel, contents);
        }
        Pack::open(&dir).unwrap()
    }

    fn text(r: &Resource<'_>) -> String {
        String::from_utf8(r.read().unwrap().into_owned()).unwrap()
    }

    const META: &str = r#"{"pack": {"description": "", "min_format": 97, "max_format": 97}}"#;

    #[test]
    fn top_pack_wins_and_metadata_comes_from_above() {
        let tmp = TempDir::new("stack");
        let low = pack(
            &tmp.0,
            "low",
            META,
            &[
                ("assets/minecraft/textures/a.png", "low a"),
                ("assets/minecraft/textures/b.png", "low b"),
                ("assets/minecraft/textures/b.png.mcmeta", r#"{"low": 1}"#),
            ],
        );
        let high = pack(
            &tmp.0,
            "high",
            META,
            &[
                ("assets/minecraft/textures/a.png", "high a"),
                ("assets/minecraft/textures/b.png.mcmeta", r#"{"high": 1}"#),
                ("assets/other/textures/c.png", "c"),
            ],
        );
        let res = Resources::new(vec![low, high]);
        let a = res.get(&Id::parse("textures/a.png").unwrap()).unwrap();
        assert_eq!((text(&a), a.pack().name()), ("high a".into(), "high"));
        // b's file is in "low", its metadata in "high": a higher .mcmeta applies.
        let b = res.get(&Id::parse("textures/b.png").unwrap()).unwrap();
        assert!(b.metadata().unwrap().section("high").is_some());
        // In a stack, each copy keeps its own pack's metadata.
        let stack = res.stack(&Id::parse("textures/b.png").unwrap());
        assert_eq!(stack.len(), 1);
        assert!(stack[0].metadata().unwrap().section("low").is_some());
        let listed = res.list("textures", |p| p.ends_with(".png"));
        let names: Vec<String> = listed.keys().map(Id::to_string).collect();
        assert_eq!(
            names,
            [
                "minecraft:textures/a.png",
                "minecraft:textures/b.png",
                "other:textures/c.png"
            ]
        );
        assert_eq!(
            text(&listed[&Id::parse("textures/a.png").unwrap()]),
            "high a"
        );
        assert!(
            listed[&Id::parse("textures/b.png").unwrap()]
                .metadata()
                .unwrap()
                .section("high")
                .is_some()
        );
    }

    #[test]
    fn filters_hide_lower_packs() {
        let tmp = TempDir::new("filter");
        let low = pack(
            &tmp.0,
            "low",
            META,
            &[
                ("assets/minecraft/textures/a.png", "low a"),
                ("assets/minecraft/textures/keep.png", "keep"),
            ],
        );
        let filter = r#"{"pack": {"description": ""}, "filter": {"block": [{"namespace": "minecraft", "path": "/a\\.png$"}]}}"#;
        // This pack has no minecraft files: it still filters that namespace.
        let high = pack(&tmp.0, "high", filter, &[("assets/other/x.txt", "x")]);
        let res = Resources::new(vec![low, high]);
        assert!(res.get(&Id::parse("textures/a.png").unwrap()).is_none());
        assert!(res.stack(&Id::parse("textures/a.png").unwrap()).is_empty());
        assert!(res.get(&Id::parse("textures/keep.png").unwrap()).is_some());
        let listed = res.list("textures", |_| true);
        assert_eq!(listed.len(), 1);
    }

    #[test]
    fn overlays_go_over_the_root() {
        let tmp = TempDir::new("overlay");
        let meta = r#"{"pack": {"description": ""}, "overlays": {"entries": [
            {"directory": "first", "min_format": 90, "max_format": 100},
            {"directory": "second", "min_format": 90, "max_format": 100},
            {"directory": "never", "min_format": 1, "max_format": 2}
        ]}}"#;
        let p = pack(
            &tmp.0,
            "p",
            meta,
            &[
                ("assets/minecraft/x.txt", "root"),
                ("first/assets/minecraft/x.txt", "first"),
                ("second/assets/minecraft/x.txt", "second"),
                ("never/assets/minecraft/x.txt", "never"),
                ("first/assets/minecraft/y.txt", "first y"),
            ],
        );
        let res = Resources::new(vec![p]);
        assert_eq!(
            text(&res.get(&Id::parse("x.txt").unwrap()).unwrap()),
            "second"
        );
        assert_eq!(
            text(&res.get(&Id::parse("y.txt").unwrap()).unwrap()),
            "first y"
        );
    }

    #[test]
    fn packs_without_metadata_are_rejected() {
        let tmp = TempDir::new("nometa");
        file(&tmp.0, "p/assets/minecraft/x.txt", "x");
        assert!(Pack::open(&tmp.0.join("p")).is_err());
        file(&tmp.0, "q/pack.mcmeta", "{}");
        assert!(Pack::open(&tmp.0.join("q")).is_err());
    }

    #[test]
    fn zipped_packs() {
        let tmp = TempDir::new("zip");
        let bytes = crate::zip::tests::build(&[
            ("pack.mcmeta", META.as_bytes(), false),
            ("assets/minecraft/textures/a.png", b"zip a", true),
            ("assets/minecraft/textures/sub/b.png", b"zip b", false),
        ]);
        let path = tmp.0.join("p.zip");
        std::fs::write(&path, bytes).unwrap();
        let res = Resources::new(vec![Pack::open(&path).unwrap()]);
        assert_eq!(
            text(&res.get(&Id::parse("textures/a.png").unwrap()).unwrap()),
            "zip a"
        );
        let names: Vec<String> = res
            .list("textures", |_| true)
            .keys()
            .map(Id::to_string)
            .collect();
        assert_eq!(
            names,
            ["minecraft:textures/a.png", "minecraft:textures/sub/b.png"]
        );
    }
}

//! The game's resources made again with a server's packs over vanilla's
//! (`Minecraft.reloadResourcePacks` with `DownloadedPackSource`'s packs at the top), on a
//! thread: everything drawn or heard that a pack can change (the world's assets: block,
//! item and GUI atlases, models, block entities, the HUD's font, the sky's, clouds' and
//! weather's pictures; the language; the sound events and files), handed over whole once
//! it is built.
//!
//! Vanilla puts a loading overlay over the game until a reload is done; here the old
//! resources stay in use (and drawn) until the new ones are ready, and are swapped for
//! them at once. A reload that fails (a pack that can't be opened, an atlas that doesn't
//! fit, a builder that panics) changes nothing: the resources in use stay, and the
//! session is told, which recovers as vanilla does.
//!
//! What is never read from a server's pack: the menus' own fonts, Unifont and pictures
//! (they read vanilla's pack, `game_files::vanilla`), the players' default skins, and
//! shaders or post effects of any pack (PLAN D5: nothing reads them).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Instant;

use ruthenium_assets::{Id, Limits, Pack, Resources};
use ruthenium_audio::SoundFiles;
use ruthenium_render::WorldAssets;

/// Bytes of `lang/en_us.json` read from all server packs together: vanilla's whole file
/// is about 600 KiB, so this leaves room for a server's own strings while a pack's
/// language can't grow the map without end.
pub const MAX_LANG_BYTES: usize = 8 << 20;
/// Entries of the language the server packs may add or change.
pub const MAX_LANG_ENTRIES: usize = 100_000;
/// The largest side of an atlas (blocks, items, the GUI) when a server's packs are in
/// the stack, whatever the GPU takes: its top level is then at most 256 MiB (with mip
/// levels about 340 MiB) on the reload's thread and the GPU, room for 64x packs
/// (vanilla's block atlas is 2048 at 16x). Atlases that don't fit fail the reload, as
/// vanilla's do when the GPU's limit is passed, and the recovery puts back what was.
pub const MAX_SERVER_ATLAS: u32 = 8192;

/// The resources of one reload.
pub struct Reloaded {
    /// The world's drawing assets (when they were asked for).
    pub world: Option<WorldAssets>,
    /// `en_us` through the stack: server messages and the HUD's text.
    pub lang: Arc<HashMap<String, String>>,
    /// The sound events and files through the stack.
    pub sounds: SoundFiles,
    /// How many server packs it has.
    pub packs: usize,
}

impl std::fmt::Debug for Reloaded {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Reloaded")
            .field("packs", &self.packs)
            .finish_non_exhaustive()
    }
}

/// The largest side of an atlas a reload builds: the GPU's, and with server packs in
/// the stack at most [`MAX_SERVER_ATLAS`].
pub fn atlas_limit(max_texture: u32, server_packs: usize) -> u32 {
    if server_packs == 0 {
        max_texture
    } else {
        max_texture.min(MAX_SERVER_ATLAS)
    }
}

/// Opens a server's downloaded pack as one whose every byte is hostile.
pub fn open_server_pack(path: &Path) -> Result<Pack, String> {
    Pack::open_untrusted(path, &Limits::untrusted()).map_err(|e| e.to_string())
}

/// Whether a downloaded file is a pack that can be loaded (`readPackMetadata`), for the
/// session's check when a reload starts.
pub fn is_loadable(path: &Path) -> bool {
    match open_server_pack(path) {
        Ok(_) => true,
        Err(e) => {
            eprintln!("[server packs] {e}");
            false
        }
    }
}

/// Builds the resources of `game` with `packs` (lowest first) over vanilla's.
/// `max_texture`: the largest texture the GPU takes; `world`: whether to build the
/// world's assets too.
pub fn build(
    game: &Path,
    packs: &[(u128, PathBuf)],
    max_texture: u32,
    world: bool,
) -> Result<Reloaded, String> {
    let started = Instant::now();
    let opened = packs
        .iter()
        .map(|(id, path)| open_server_pack(path).map_err(|e| format!("pack {id:032x}: {e}")))
        .collect::<Result<Vec<Pack>, String>>()?;
    let resources = crate::game_files::with_server_packs(game, opened)?;
    let max_texture = atlas_limit(max_texture, packs.len());
    let lang = Arc::new(language(&resources, packs.len()));
    let sounds = SoundFiles::new(resources.clone());
    let world = if world {
        Some(crate::join::load_world_assets_from(
            game,
            &resources,
            max_texture,
        )?)
    } else {
        None
    };
    eprintln!(
        "[ruthenium] resources with {} server pack(s) built in {:.0?}",
        packs.len(),
        started.elapsed()
    );
    Ok(Reloaded {
        world,
        lang,
        sounds,
        packs: packs.len(),
    })
}

/// `ClientLanguage.loadFrom` for `en_us`: every namespace's `lang/en_us.json` of every
/// pack, lowest first, later ones over earlier ones. What the `server` packs at the top
/// add is bounded ([`MAX_LANG_BYTES`], [`MAX_LANG_ENTRIES`]).
fn language(resources: &Resources, server: usize) -> HashMap<String, String> {
    let mut namespaces: Vec<&str> = resources
        .packs()
        .iter()
        .flat_map(|p| p.namespaces().iter().map(String::as_str))
        .collect();
    namespaces.sort_unstable();
    namespaces.dedup();
    let first_server = resources.packs().len().saturating_sub(server);
    let mut out = HashMap::new();
    let (mut bytes, mut added) = (0usize, 0usize);
    for ns in namespaces {
        let Some(id) = Id::new(ns, "lang/en_us.json") else {
            continue;
        };
        for r in resources.stack(&id) {
            let from_server = resources
                .packs()
                .iter()
                .position(|p| std::ptr::eq(p, r.pack()))
                .is_some_and(|i| i >= first_server);
            let Ok(data) = r.read() else { continue };
            if from_server {
                bytes += data.len();
                if bytes > MAX_LANG_BYTES {
                    eprintln!(
                        "[server packs] {}: {ns}:lang/en_us.json is past {} MiB of language; left out",
                        r.pack().name(),
                        MAX_LANG_BYTES >> 20
                    );
                    continue;
                }
            }
            let Ok(serde_json::Value::Object(map)) = serde_json::from_slice(&data) else {
                eprintln!(
                    "[ruthenium] {}: {ns}:lang/en_us.json isn't a JSON object",
                    r.pack().name()
                );
                continue;
            };
            for (k, v) in map {
                let Some(v) = v.as_str() else { continue };
                if from_server && !out.contains_key(&k) {
                    if added >= MAX_LANG_ENTRIES {
                        continue;
                    }
                    added += 1;
                }
                out.insert(k, v.to_owned());
            }
        }
    }
    out
}

/// A reload asked for, and who asked (the session's number, for its answer).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    pub packs: Vec<(u128, PathBuf)>,
    pub session: u64,
}

/// Makes one reload at a time; one asked for meanwhile waits, and a reload overtaken by
/// a newer one is not applied.
#[derive(Debug)]
pub struct Reloader {
    game: Option<PathBuf>,
    world: bool,
    running: Option<(Request, JoinHandle<Result<Reloaded, String>>)>,
    queued: Option<Request>,
}

impl Reloader {
    /// `world`: whether reloads build the world's assets (a window draws them).
    pub fn new(game: Option<PathBuf>, world: bool) -> Reloader {
        Reloader {
            game,
            world,
            running: None,
            queued: None,
        }
    }

    /// Asks for the resources with `request`'s packs; `max_texture` as [`build`]'s.
    pub fn request(&mut self, request: Request, max_texture: u32) {
        if self.running.is_some() {
            self.queued = Some(request);
            return;
        }
        self.start(request, max_texture);
    }

    fn start(&mut self, request: Request, max_texture: u32) {
        let Some(game) = self.game.clone() else {
            eprintln!("[ruthenium] no game files to reload resources from");
            return;
        };
        let (packs, world) = (request.packs.clone(), self.world);
        match std::thread::Builder::new()
            .name("resource-reload".into())
            .spawn(move || build(&game, &packs, max_texture, world))
        {
            Ok(h) => self.running = Some((request, h)),
            Err(e) => eprintln!("[ruthenium] no thread for the reload: {e}"),
        }
    }

    /// Whether a reload is under way or waiting.
    pub fn busy(&self) -> bool {
        self.running.is_some() || self.queued.is_some()
    }

    /// A reload that has ended: its request and what it built (`Err`: it failed, and
    /// nothing changes). One overtaken by a request that came meanwhile is dropped and
    /// the newer one starts; its request still comes back, as failed, for an answer.
    pub fn poll(&mut self, max_texture: u32) -> Option<(Request, Result<Reloaded, String>)> {
        if !self.running.as_ref().is_some_and(|(_, h)| h.is_finished()) {
            return None;
        }
        let (request, handle) = self.running.take()?;
        let result = handle
            .join()
            .unwrap_or_else(|_| Err("the reload's thread panicked".into()));
        if let Some(next) = self.queued.take() {
            self.start(next, max_texture);
            return Some((request, Err("overtaken by a newer reload".into())));
        }
        Some((request, result))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_zip::zip as zip_bytes;

    fn temp(name: &str) -> PathBuf {
        let d =
            std::env::temp_dir().join(format!("ruthenium-reload-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    const MCMETA: &[u8] = br#"{"pack":{"description":"test","min_format":97,"max_format":97}}"#;

    /// Server packs can't make an atlas larger than [`MAX_SERVER_ATLAS`]; vanilla's
    /// alone gets what the GPU takes.
    #[test]
    fn server_packs_cap_the_atlas() {
        assert_eq!(atlas_limit(32768, 0), 32768);
        assert_eq!(atlas_limit(32768, 1), MAX_SERVER_ATLAS);
        assert_eq!(atlas_limit(4096, 3), 4096);
    }

    /// A server pack's language goes over vanilla's, the newest pack over older ones.
    #[test]
    fn server_languages_stack_over_vanillas() {
        let dir = temp("lang");
        let pack = |n: &str, lang: &str| {
            let p = dir.join(n);
            std::fs::write(
                &p,
                zip_bytes(&[
                    ("pack.mcmeta", MCMETA),
                    ("assets/minecraft/lang/en_us.json", lang.as_bytes()),
                ]),
            )
            .unwrap();
            open_server_pack(&p).unwrap()
        };
        let low = pack("low.zip", r#"{"a":"low","b":"low"}"#);
        let high = pack("high.zip", r#"{"b":"high","c":7}"#);
        let r = Resources::new(vec![low, high]);
        let lang = language(&r, 2);
        assert_eq!(lang.get("a").map(String::as_str), Some("low"));
        assert_eq!(lang.get("b").map(String::as_str), Some("high"));
        assert!(!lang.contains_key("c"), "only strings");
    }

    /// Past the cap, a server pack's language is left out.
    #[test]
    fn a_huge_server_language_is_left_out() {
        let dir = temp("huge");
        let mut big = String::from("{");
        let n = MAX_LANG_BYTES / 20 + 10;
        for i in 0..n {
            big += &format!("\"k{i:08}\":\"vvvvv\",");
        }
        big += "\"end\":\"x\"}";
        let p = dir.join("big.zip");
        std::fs::write(
            &p,
            zip_bytes(&[
                ("pack.mcmeta", MCMETA),
                ("assets/minecraft/lang/en_us.json", big.as_bytes()),
            ]),
        )
        .unwrap();
        let r = Resources::new(vec![open_server_pack(&p).unwrap()]);
        assert!(language(&r, 1).is_empty());
    }

    /// Entries a server's packs add stop at the cap; those they change don't count.
    #[test]
    fn server_language_entries_are_capped() {
        let dir = temp("entries");
        let mut s = String::from("{");
        for i in 0..MAX_LANG_ENTRIES + 5 {
            s += &format!("\"{i}\":\"v\",");
        }
        s += "\"last\":\"v\"}";
        let p = dir.join("many.zip");
        std::fs::write(
            &p,
            zip_bytes(&[
                ("pack.mcmeta", MCMETA),
                ("assets/minecraft/lang/en_us.json", s.as_bytes()),
            ]),
        )
        .unwrap();
        let r = Resources::new(vec![open_server_pack(&p).unwrap()]);
        assert_eq!(language(&r, 1).len(), MAX_LANG_ENTRIES);
    }
}

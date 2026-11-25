//! The skins of players, of heads' owners and of mannequins: asked for as they come into
//! view, shown as their default until they are there.

use std::collections::HashMap;
use std::sync::Arc;

use ruthenium_gpu::{Arrival, TextureDesc, TextureId, TextureTable, UploadRing};
use ruthenium_sim::avatar::{DEFAULT_SKINS, SkinSource, default_skin};

use crate::sky::SkyImage;

/// Skins kept on the GPU, with their capes: 48 KiB each at most.
const KEPT: usize = 512;

/// The default skins' pictures, in their order, below `textures/entity/player`.
const NAMES: [&str; DEFAULT_SKINS / 2] = [
    "alex", "ari", "efe", "kai", "makena", "noor", "steve", "sunny", "zuri",
];

/// The path of a default skin's picture below `textures/entity`.
pub(super) fn default_path(index: usize) -> String {
    let arms = if is_slim(index) { "slim" } else { "wide" };
    format!("player/{arms}/{}", NAMES[index % NAMES.len()])
}

fn is_slim(index: usize) -> bool {
    index < NAMES.len()
}

/// A skin as it was found. What is missing is the default skin's.
#[derive(Debug, Clone, Default)]
pub struct LoadedSkin {
    pub body: Option<SkyImage>,
    pub cape: Option<SkyImage>,
    /// Whether the arms are the thin ones.
    pub slim: Option<bool>,
    /// The id a looked-up profile turned out to have: its default skin goes by it.
    pub id: Option<u128>,
}

/// What a skin is drawn with.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct Look {
    pub body: TextureId,
    pub cape: Option<TextureId>,
    pub slim: bool,
    /// It has texels that are seen through.
    pub blended: bool,
}

#[derive(Debug)]
enum State {
    Asked,
    /// To be put on the GPU.
    Found(Box<LoadedSkin>),
    Kept {
        body: Option<(TextureId, bool)>,
        cape: Option<TextureId>,
        slim: Option<bool>,
        default: usize,
    },
}

#[derive(Debug)]
struct Entry {
    state: State,
    /// The frame it was last wanted in.
    used: u64,
}

/// See the module docs.
#[derive(Debug)]
pub(super) struct Skins {
    entries: HashMap<Arc<SkinSource>, Entry>,
    requests: Vec<Arc<SkinSource>>,
    /// Something was found that is not on the GPU yet.
    found: bool,
    /// Pictures that were replaced.
    old: Vec<TextureId>,
    defaults: [Option<TextureId>; DEFAULT_SKINS],
    frame: u64,
}

/// Whether any texel is neither clear (less than a tenth) nor whole.
fn blended(i: &SkyImage) -> bool {
    i.rgba
        .as_chunks::<4>()
        .0
        .iter()
        .any(|p| (26..255).contains(&p[3]))
}

impl Skins {
    pub fn new(defaults: [Option<TextureId>; DEFAULT_SKINS]) -> Skins {
        Skins {
            entries: HashMap::new(),
            requests: Vec::new(),
            found: false,
            old: Vec::new(),
            defaults,
            frame: 0,
        }
    }

    pub fn begin(&mut self) {
        self.frame += 1;
    }

    /// The skins wanted since the last call that nobody has been asked for.
    pub fn take_requests(&mut self) -> Vec<Arc<SkinSource>> {
        std::mem::take(&mut self.requests)
    }

    /// The answer to a request, or a better one to one that was answered.
    pub fn found(&mut self, source: &Arc<SkinSource>, skin: LoadedSkin) {
        // Not wanted any more: let go of.
        if let Some(e) = self.entries.get_mut(source) {
            if let State::Kept { body, cape, .. } = &e.state {
                self.old.extend(body.iter().map(|b| b.0).chain(*cape));
            }
            e.state = State::Found(Box::new(skin));
            self.found = true;
        }
    }

    /// Puts what was found on the GPU, and lets go of what has not been wanted for
    /// longest if there are too many.
    pub fn stage(&mut self, textures: &mut TextureTable, uploads: &mut UploadRing) {
        if self.found {
            let mut later = false;
            for (source, e) in &mut self.entries {
                let State::Found(skin) = &e.state else {
                    continue;
                };
                let mut put = |i: &Option<SkyImage>| -> Result<Option<TextureId>, ()> {
                    let Some(i) = i.as_ref().filter(|i| {
                        !i.rgba.is_empty()
                            && i.rgba.len() == i.width as usize * i.height as usize * 4
                    }) else {
                        return Ok(None);
                    };
                    let desc = TextureDesc::rgba8(i.width, i.height);
                    textures
                        .create_with(uploads, desc, &[&i.rgba], Arrival::Background)
                        .map(Some)
                        .map_err(|_| ())
                };
                // Full for now: the next frame tries again.
                let Ok(body) = put(&skin.body) else {
                    later = true;
                    break;
                };
                let Ok(cape) = put(&skin.cape) else {
                    if let Some(body) = body {
                        textures.destroy(body);
                    }
                    later = true;
                    break;
                };
                let body = body.zip(skin.body.as_ref().map(blended));
                e.state = State::Kept {
                    body,
                    cape,
                    slim: skin.slim,
                    default: default_skin(skin.id.unwrap_or_else(|| source.id())),
                };
            }
            self.found = later;
        }
        for id in self.old.drain(..) {
            textures.destroy(id);
        }
        while self.entries.len() > KEPT {
            let oldest = self
                .entries
                .iter()
                .min_by_key(|(_, e)| e.used)
                .filter(|(_, e)| e.used < self.frame)
                .map(|(s, _)| s.clone());
            let Some(oldest) = oldest else {
                break;
            };
            if let Some(Entry {
                state: State::Kept { body, cape, .. },
                ..
            }) = self.entries.remove(&oldest)
            {
                for id in body.map(|b| b.0).into_iter().chain(cape) {
                    textures.destroy(id);
                }
            }
        }
    }

    /// What to draw `source` with now; `None` before the default skins are there.
    pub fn look(&mut self, source: &Arc<SkinSource>, textures: &TextureTable) -> Option<Look> {
        self.look_by(source, &|id| textures.is_ready(id))
    }

    fn look_by(
        &mut self,
        source: &Arc<SkinSource>,
        ready: &dyn Fn(TextureId) -> bool,
    ) -> Option<Look> {
        if let Some(e) = self.entries.get_mut(source) {
            e.used = self.frame;
            if let State::Kept {
                body,
                cape,
                slim,
                default,
            } = e.state
            {
                // One that is on its way shows the default until it has arrived.
                if body.is_none_or(|(id, _)| ready(id)) {
                    let (body, blended) = match body {
                        Some(b) => b,
                        None => (self.default(default, ready)?, false),
                    };
                    return Some(Look {
                        body,
                        cape: cape.filter(|id| ready(*id)),
                        slim: slim.unwrap_or(is_slim(default)),
                        blended,
                    });
                }
            }
        } else {
            self.requests.push(source.clone());
            self.entries.insert(
                source.clone(),
                Entry {
                    state: State::Asked,
                    used: self.frame,
                },
            );
        }
        let index = default_skin(source.id());
        Some(Look {
            body: self.default(index, ready)?,
            cape: None,
            slim: is_slim(index),
            blended: false,
        })
    }

    fn default(&self, index: usize, ready: &dyn Fn(TextureId) -> bool) -> Option<TextureId> {
        self.defaults[index].filter(|id| ready(*id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source(id: u128) -> Arc<SkinSource> {
        Arc::new(SkinSource::Player { id, textures: None })
    }

    fn skins() -> Skins {
        Skins::new([Some(TextureId::MISSING); DEFAULT_SKINS])
    }

    #[test]
    fn the_default_skins_are_in_the_games_order() {
        assert_eq!(default_path(0), "player/slim/alex");
        assert_eq!(default_path(6), "player/slim/steve");
        assert_eq!(default_path(9), "player/wide/alex");
        assert_eq!(default_path(17), "player/wide/zuri");
        assert!(is_slim(8) && !is_slim(9));
    }

    #[test]
    fn a_skin_is_asked_for_once_and_shows_its_default() {
        let mut s = skins();
        let ready = |_| true;
        let look = s.look_by(&source(9), &ready).unwrap();
        assert!(!look.slim && look.cape.is_none() && !look.blended);
        assert!(s.look_by(&source(8), &ready).unwrap().slim);
        s.look_by(&source(9), &ready);
        assert_eq!(s.take_requests(), [source(9), source(8)]);
        assert!(s.take_requests().is_empty());
        // Without the defaults' pictures there is nothing to draw.
        assert!(s.look_by(&source(9), &|_| false).is_none());
    }

    #[test]
    fn what_was_found_is_shown_once_it_has_arrived() {
        let mut s = skins();
        s.look_by(&source(9), &|_| true);
        let kept = |slim, default| State::Kept {
            body: Some((TextureId::MISSING, true)),
            cape: Some(TextureId::MISSING),
            slim,
            default,
        };
        s.entries.get_mut(&source(9)).unwrap().state = kept(Some(true), 9);
        let look = s.look_by(&source(9), &|_| true).unwrap();
        assert!(look.slim && look.blended && look.cape.is_some());
        s.entries.get_mut(&source(9)).unwrap().state = kept(None, 3);
        assert!(s.look_by(&source(9), &|_| true).unwrap().slim);
        // On its way: the default until it has arrived.
        let asked = std::cell::Cell::new(0);
        let later = |_| asked.replace(asked.get() + 1) > 0;
        let look = s.look_by(&source(9), &later).unwrap();
        assert!(look.cape.is_none() && !look.blended);
        // A better answer takes the place of what was shown.
        s.found(&source(9), LoadedSkin::default());
        assert_eq!(s.old, [TextureId::MISSING; 2]);
        assert!(std::mem::take(&mut s.found));
        // Nothing asked for twice.
        assert_eq!(s.take_requests().len(), 1);
        // An answer nobody waits for is dropped.
        s.found(&source(77), LoadedSkin::default());
        assert!(!s.found && !s.entries.contains_key(&source(77)));
    }

    #[test]
    fn seen_through_is_neither_clear_nor_whole() {
        let picture = |alpha: [u8; 2]| SkyImage {
            width: 2,
            height: 1,
            rgba: alpha.iter().flat_map(|a| [1, 2, 3, *a]).collect(),
        };
        assert!(!blended(&picture([0, 255])));
        assert!(!blended(&picture([25, 255])));
        assert!(blended(&picture([26, 255])) && blended(&picture([0, 254])));
    }
}

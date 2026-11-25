//! Decoded sounds kept for the next time they play, up to a budget of bytes; the least
//! recently played go first. (Vanilla keeps every sound it has decoded until the next
//! resource reload.) A sound still playing when it leaves the cache stays alive through
//! its voice until it ends.

use std::collections::HashMap;
use std::sync::Arc;

use crate::decode::Pcm;

/// The default budget: every sound a long PvP session plays, with room to spare
/// (vanilla's non-music sounds are a few hundred MB decoded).
pub const DEFAULT_BUDGET: usize = 96 << 20;

#[derive(Debug)]
pub struct PcmCache {
    budget: usize,
    bytes: usize,
    clock: u64,
    entries: HashMap<String, (Arc<Pcm>, u64)>,
}

impl PcmCache {
    pub fn new(budget: usize) -> PcmCache {
        PcmCache {
            budget,
            bytes: 0,
            clock: 0,
            entries: HashMap::new(),
        }
    }

    /// The sound, marked as just used.
    pub fn get(&mut self, file: &str) -> Option<Arc<Pcm>> {
        self.clock += 1;
        let clock = self.clock;
        self.entries.get_mut(file).map(|(p, used)| {
            *used = clock;
            p.clone()
        })
    }

    /// Keeps a sound, letting the least recently used go until the rest fit. A sound
    /// larger than the whole budget isn't kept.
    pub fn insert(&mut self, file: String, pcm: Arc<Pcm>) {
        let size = pcm.bytes();
        if size > self.budget {
            return;
        }
        self.clock += 1;
        if let Some((old, _)) = self.entries.insert(file, (pcm, self.clock)) {
            self.bytes -= old.bytes();
        }
        self.bytes += size;
        while self.bytes > self.budget {
            let Some(oldest) = self
                .entries
                .iter()
                .min_by_key(|(_, (_, used))| *used)
                .map(|(k, _)| k.clone())
            else {
                break;
            };
            if let Some((p, _)) = self.entries.remove(&oldest) {
                self.bytes -= p.bytes();
            }
        }
    }

    pub fn bytes(&self) -> usize {
        self.bytes
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn clear(&mut self) {
        self.entries.clear();
        self.bytes = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pcm(samples: usize) -> Arc<Pcm> {
        Arc::new(Pcm {
            channels: 1,
            rate: 48_000,
            samples: vec![0; samples].into_boxed_slice(),
        })
    }

    #[test]
    fn the_least_recently_used_go_first() {
        let one = pcm(1000).bytes();
        let mut c = PcmCache::new(one * 3);
        c.insert("a".into(), pcm(1000));
        c.insert("b".into(), pcm(1000));
        c.insert("c".into(), pcm(1000));
        assert!(c.get("a").is_some());
        c.insert("d".into(), pcm(1000));
        assert!(c.get("b").is_none(), "b was the least recently used");
        assert!(c.get("a").is_some() && c.get("c").is_some() && c.get("d").is_some());
        assert_eq!(c.bytes(), one * 3);
        // Too large for the budget: not kept, nothing else lost.
        c.insert("huge".into(), pcm(10_000));
        assert!(c.get("huge").is_none());
        assert_eq!(c.len(), 3);
    }

    #[test]
    fn a_sound_in_use_outlives_its_entry() {
        let mut c = PcmCache::new(pcm(10).bytes());
        let playing = pcm(10);
        c.insert("a".into(), playing.clone());
        c.insert("b".into(), pcm(10));
        assert!(c.get("a").is_none());
        assert_eq!(playing.frames(), 10);
    }
}

//! Sound categories (`SoundSource`): what each volume slider governs.

/// `net.minecraft.sounds.SoundSource`, in its order (the wire sends the ordinal).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum SoundSource {
    Master,
    Music,
    Records,
    Weather,
    Blocks,
    Hostile,
    Neutral,
    Players,
    Ambient,
    Voice,
    Ui,
}

impl SoundSource {
    pub const COUNT: usize = 11;
    pub const ALL: [SoundSource; SoundSource::COUNT] = [
        SoundSource::Master,
        SoundSource::Music,
        SoundSource::Records,
        SoundSource::Weather,
        SoundSource::Blocks,
        SoundSource::Hostile,
        SoundSource::Neutral,
        SoundSource::Players,
        SoundSource::Ambient,
        SoundSource::Voice,
        SoundSource::Ui,
    ];

    /// The ordinal, as the wire and arrays index it.
    pub fn index(self) -> usize {
        self as usize
    }

    /// The category with this ordinal (`readEnum`).
    pub fn from_index(i: i32) -> Option<SoundSource> {
        usize::try_from(i)
            .ok()
            .and_then(|i| SoundSource::ALL.get(i).copied())
    }

    /// `getName`: `options.txt` keeps a category's volume as `soundCategory_<name>`.
    pub fn name(self) -> &'static str {
        match self {
            SoundSource::Master => "master",
            SoundSource::Music => "music",
            SoundSource::Records => "record",
            SoundSource::Weather => "weather",
            SoundSource::Blocks => "block",
            SoundSource::Hostile => "hostile",
            SoundSource::Neutral => "neutral",
            SoundSource::Players => "player",
            SoundSource::Ambient => "ambient",
            SoundSource::Voice => "voice",
            SoundSource::Ui => "ui",
        }
    }

    pub fn from_name(name: &str) -> Option<SoundSource> {
        SoundSource::ALL.into_iter().find(|s| s.name() == name)
    }
}

/// The volume sliders' values (0 to 1, each category's own, master apart).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Volumes(pub [f32; SoundSource::COUNT]);

impl Default for Volumes {
    /// Every slider at 100%, as vanilla starts.
    fn default() -> Volumes {
        Volumes([1.0; SoundSource::COUNT])
    }
}

impl Volumes {
    pub fn get(&self, source: SoundSource) -> f32 {
        self.0[source.index()]
    }

    pub fn set(&mut self, source: SoundSource, value: f32) {
        self.0[source.index()] = value;
    }

    /// `Options.getFinalSoundSourceVolume`: a category's slider times the master's
    /// (the master's alone for master sounds).
    pub fn final_volume(&self, source: SoundSource) -> f32 {
        let master = self.get(SoundSource::Master);
        if source == SoundSource::Master {
            master
        } else {
            self.get(source) * master
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ordinals_and_names_are_vanillas() {
        assert_eq!(SoundSource::from_index(0), Some(SoundSource::Master));
        assert_eq!(SoundSource::from_index(7), Some(SoundSource::Players));
        assert_eq!(SoundSource::from_index(10), Some(SoundSource::Ui));
        assert_eq!(SoundSource::from_index(11), None);
        assert_eq!(SoundSource::from_index(-1), None);
        assert_eq!(SoundSource::Records.name(), "record");
        assert_eq!(SoundSource::from_name("block"), Some(SoundSource::Blocks));
        for (i, s) in SoundSource::ALL.into_iter().enumerate() {
            assert_eq!(s.index(), i);
            assert_eq!(SoundSource::from_name(s.name()), Some(s));
        }
    }

    #[test]
    fn master_scales_every_other_category() {
        let mut v = Volumes::default();
        v.set(SoundSource::Master, 0.5);
        v.set(SoundSource::Players, 0.4);
        assert_eq!(v.final_volume(SoundSource::Players), 0.2);
        assert_eq!(v.final_volume(SoundSource::Master), 0.5);
        assert_eq!(v.final_volume(SoundSource::Blocks), 0.5);
    }
}

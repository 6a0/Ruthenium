//! Sound for the window and `connect`: the engine started on the imported game's sound
//! files, the volumes from the settings, the listener every frame, the menus' click and
//! the sliders' preview (`SoundPreviewHandler`).

use std::path::PathBuf;

use ruthenium_audio::{
    Audio, InstanceId, Listener, Output, SoundFiles, SoundHandle, SoundInstance, SoundSource,
    Volumes,
};
use ruthenium_render::CameraPose;

/// Where sound goes for a run (`--sound`, `--sound-wav`).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum SoundMode {
    /// The system's default output device.
    #[default]
    Device,
    /// No sound at all: no device, no decoding.
    Off,
    /// A WAV file of what the mixer makes, in place of the device.
    Wav(PathBuf),
}

/// Starts sound for the game imported in `game` (none: silent, with a line saying so).
pub fn start(mode: &SoundMode, game: Option<PathBuf>, volumes: Volumes) -> Audio {
    let output = match mode {
        SoundMode::Off => {
            eprintln!("[sound] off");
            return Audio::off();
        }
        SoundMode::Device => Output::Device,
        SoundMode::Wav(path) => Output::Wav(path.clone()),
    };
    Audio::start(output, volumes, move || {
        let game = game.ok_or("no data folder")?;
        // The vanilla pack of the imported game, which the menus and the world share.
        crate::game_files::vanilla(&game).map(SoundFiles::new)
    })
}

/// The listener at the frame's camera (`SoundEngine.updateSource`).
pub fn listener(camera: &CameraPose) -> Listener {
    Listener::camera(camera.position, camera.y_rot, camera.x_rot)
}

/// `AbstractWidget.playButtonClickSound`: `ui.button.click` for the UI at pitch 1.
pub fn click(sound: &SoundHandle, seed: i64) {
    sound.play(SoundInstance::ui(
        "minecraft:ui.button.click",
        1.0,
        0.25,
        seed,
    ));
}

/// What a category's slider plays while it moves outside a world
/// (`SoundPreviewHandler.preview`).
fn preview_event(source: SoundSource) -> Option<&'static str> {
    Some(match source {
        SoundSource::Records => "minecraft:block.note_block.guitar",
        SoundSource::Weather => "minecraft:entity.lightning_bolt.thunder",
        SoundSource::Blocks => "minecraft:block.grass.place",
        SoundSource::Hostile => "minecraft:entity.zombie.ambient",
        SoundSource::Neutral => "minecraft:entity.cow.ambient",
        SoundSource::Players => "minecraft:entity.generic.eat",
        SoundSource::Ambient => "minecraft:ambient.cave",
        SoundSource::Ui => "minecraft:ui.button.click",
        SoundSource::Master | SoundSource::Music | SoundSource::Voice => return None,
    })
}

/// `SoundPreviewHandler`: one preview at a time; another category's stops it.
#[derive(Debug, Default)]
pub struct Preview {
    playing: Option<InstanceId>,
    category: Option<SoundSource>,
}

impl Preview {
    pub fn slider_moved(
        &mut self,
        sound: &SoundHandle,
        source: SoundSource,
        volume: f32,
        seed: i64,
    ) {
        if self.category != Some(source) {
            self.category = Some(source);
            if let Some(id) = self.playing.take() {
                sound.stop(id);
            }
        }
        if self.playing.is_some_and(|id| sound.is_active(id)) {
            return;
        }
        let Some(event) = preview_event(source) else {
            return;
        };
        self.playing = sound.play(SoundInstance::ui(event, 1.0, volume, seed)).id();
    }
}

/// Vanilla's volumes from `options.txt` (`soundCategory_<name>`, each a double from 0
/// to 1); keys missing or unreadable keep vanilla's default of 1.
pub fn parse_options(text: &str) -> crate::settings::SoundSettings {
    let mut s = crate::settings::SoundSettings::default();
    for line in text.lines() {
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let Some(source) = key
            .strip_prefix("soundCategory_")
            .and_then(SoundSource::from_name)
        else {
            continue;
        };
        if let Ok(v) = value.parse::<f64>()
            && (0.0..=1.0).contains(&v)
        {
            s.volumes[source.index()] = v;
        }
    }
    s
}

/// The volumes of the most recently saved `options.txt`, and where it is.
pub fn vanilla_volumes() -> Option<(crate::settings::SoundSettings, PathBuf)> {
    let env = ruthenium_prepare::discover::Env::from_process();
    let newest = crate::controls::options_files(&env)
        .into_iter()
        .max_by_key(|p| crate::controls::modified(p))?;
    let text = std::fs::read_to_string(&newest).ok()?;
    Some((parse_options(&text), newest))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_vanillas_volumes_exactly() {
        let s = parse_options(
            "version:4671\nsoundCategory_master:0.3943661971830986\nsoundCategory_player:0.0\n\
             soundCategory_record:1.5\nsoundCategory_ui:half\nmouseSensitivity:0.5\n",
        );
        let v = |src: SoundSource| s.volumes[src.index()];
        assert_eq!(
            v(SoundSource::Master).to_bits(),
            0.3943661971830986f64.to_bits()
        );
        assert_eq!(v(SoundSource::Players), 0.0);
        // Out of range or broken values keep the default.
        assert_eq!((v(SoundSource::Records), v(SoundSource::Ui)), (1.0, 1.0));
        assert_eq!(v(SoundSource::Music), 1.0);
    }

    #[test]
    fn previews_are_vanillas() {
        assert_eq!(preview_event(SoundSource::Master), None);
        assert_eq!(preview_event(SoundSource::Music), None);
        assert_eq!(
            preview_event(SoundSource::Players),
            Some("minecraft:entity.generic.eat")
        );
        for s in SoundSource::ALL {
            if let Some(e) = preview_event(s) {
                assert!(
                    ruthenium_data::v26_3::registries::SoundEvent::NAMES.contains(&e),
                    "{e}"
                );
            }
        }
    }
}

//! What the tests against the player's real assets share.

use std::path::PathBuf;

use ruthenium_assets::{Pack, Resources};
use ruthenium_audio::SoundFiles;

/// The sound files of the imported game in `RUTHENIUM_GAME_DIR` (`<data>/game`, as
/// `ruthenium prepare` writes it), if it is set.
pub fn vanilla_files() -> Option<SoundFiles> {
    let dir = PathBuf::from(std::env::var_os("RUTHENIUM_GAME_DIR")?);
    let pack = Pack::vanilla(
        &dir.join("versions/26.3/client.jar"),
        &dir.join("assets/indexes/34.json"),
        &dir.join("assets/objects"),
    )
    .expect("vanilla pack");
    Some(SoundFiles::new(Resources::new(vec![pack])))
}

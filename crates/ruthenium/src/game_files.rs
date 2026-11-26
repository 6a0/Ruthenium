//! The imported game's vanilla pack (the client jar and the asset index's objects),
//! opened once and shared: the sound files, Unifont for the menus, the skins' pictures
//! and the world's assets all read the same stack. Each opening read the jar's
//! directory and the asset index again, and one that was dropped (Unifont's, at
//! start-up) left its heap with the allocator.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};

use ruthenium_assets::{Pack, Resources};

/// The stack opened, and the game folder it is of.
static OPEN: Mutex<Option<(PathBuf, Arc<Resources>)>> = Mutex::new(None);

/// The vanilla pack of the game imported in `game`: the one already open, else opened
/// now and kept for the rest of the run. Threads that ask while it is being opened wait
/// for it.
pub fn vanilla(game: &Path) -> Result<Arc<Resources>, String> {
    let mut open = OPEN.lock().unwrap_or_else(PoisonError::into_inner);
    if let Some((at, resources)) = &*open
        && at == game
    {
        return Ok(resources.clone());
    }
    let resources = Arc::new(Resources::new(vec![pack(game)?]));
    *open = Some((game.to_owned(), resources.clone()));
    Ok(resources)
}

/// A stack of vanilla's pack with `server` over it (lowest first): vanilla's opened anew
/// (the shared one belongs to the menus and is never stacked under a server's packs),
/// or the shared one when there are none.
pub fn with_server_packs(game: &Path, server: Vec<Pack>) -> Result<Arc<Resources>, String> {
    if server.is_empty() {
        return vanilla(game);
    }
    let mut packs = vec![pack(game)?];
    packs.extend(server);
    Ok(Arc::new(Resources::new(packs)))
}

/// The vanilla pack of the game imported in `game`, opened anew.
fn pack(game: &Path) -> Result<Pack, String> {
    let version = ruthenium_prepare::supported_version();
    let imported = ruthenium_prepare::import::open(version, game)
        .ok_or("the game files are not imported; run `ruthenium prepare`")?;
    Pack::vanilla(
        &imported.client_jar,
        &imported.asset_index(),
        &imported.assets.join("objects"),
    )
    .map_err(|e| format!("cannot open the game files: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Without imported game files there is nothing to share, and nothing is kept.
    #[test]
    fn a_folder_without_the_game_opens_nothing() {
        let nowhere = Path::new("/nowhere/ruthenium-game");
        let e = vanilla(nowhere).unwrap_err();
        assert!(e.contains("not imported"), "{e}");
    }

    /// The game files, where they are imported: asked twice (from two threads at once),
    /// one stack.
    #[test]
    fn the_game_files_are_opened_once() {
        let Some(game) = std::env::var_os("RUTHENIUM_GAME_DIR").map(PathBuf::from) else {
            eprintln!("RUTHENIUM_GAME_DIR is not set; skipped");
            return;
        };
        let (a, b) = std::thread::scope(|s| {
            let a = s.spawn(|| vanilla(&game));
            let b = s.spawn(|| vanilla(&game));
            (a.join().unwrap(), b.join().unwrap())
        });
        let Ok(a) = a else {
            eprintln!("no imported game in {}; skipped", game.display());
            return;
        };
        assert!(Arc::ptr_eq(&a, &b.unwrap()));
        assert!(Arc::ptr_eq(&a, &vanilla(&game).unwrap()));
    }
}

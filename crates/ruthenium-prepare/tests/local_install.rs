//! Checks the real launcher installs on this machine against the pinned
//! hashes. Skips when none is found (as in CI, which has no game files).

use ruthenium_prepare::discover::{self, Env};
use ruthenium_prepare::supported_version;

#[test]
fn local_install_matches_pins() {
    let found = discover::search(
        &discover::default_roots(&Env::from_process()),
        supported_version(),
    );
    let mut verified = 0;
    for c in &found {
        match &c.outcome {
            Ok(install) => {
                eprintln!("verified: {} at {}", c.launcher, install.root.display());
                verified += 1;
            }
            Err(discover::Rejection::NotFound) => {}
            Err(why) => eprintln!("rejected: {} at {}: {why}", c.launcher, c.root.display()),
        }
    }
    if verified == 0 {
        eprintln!("no verified install on this machine; skipping");
    }
}

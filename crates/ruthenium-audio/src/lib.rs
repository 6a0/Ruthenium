//! Sound, as vanilla 26.3 plays it.
//!
//! - [`events`]: `sounds.json` from every pack, and the choice of a sound for a play.
//! - [`decode`]: Ogg Vorbis to 16-bit samples; [`cache`]: decoded sounds kept by bytes.
//! - [`spatial`]: OpenAL Soft's distance model and stereo panning, and the listener.
//! - [`mixer`]: the lock-free mixer that runs in the output's callback.
//! - [`engine`]: vanilla's `SoundEngine`: instances, pools, ticks and volumes.
//! - [`loader`]: the thread that decodes; [`output`]: the device or a WAV file;
//!   [`audio`]: all of it together, with the handle the game plays through.

pub mod audio;
pub mod cache;
pub mod decode;
pub mod engine;
pub mod events;
pub mod files;
mod loader;
pub mod mixer;
pub mod output;
pub mod source;
pub mod spatial;
pub mod wav;

pub use audio::{Audio, SoundHandle};
pub use engine::{Engine, EntityState, Follow, InstanceId, PlayResult, SoundInstance};
pub use files::SoundFiles;
pub use loader::trace;
pub use output::Output;
pub use source::{SoundSource, Volumes};
pub use spatial::Listener;

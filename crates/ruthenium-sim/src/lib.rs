//! The game thread: everything vanilla's main thread does with a server connection.
//!
//! - [`session`]: one session with a server, from connecting into play: the
//!   configuration phase (registries, tags, known packs, features, cookies, resource-pack
//!   answers, the code of conduct), the switch into play and back, and the ticks.
//! - [`clock`]: when ticks run (vanilla's millisecond tick clock).
//! - [`level_load`]: when the player counts as loaded into a level.
//! - [`player`]: the local player: movement, collisions, flight, the server's view of
//!   it (abilities, entity data) and the packets it sends.
//! - [`effect`]: mob effects as the client keeps them.
//! - [`entity`]: the other entities: where they are, their sizes, and their pushes.
//! - [`input`]: the mouse's turn, the scroll wheel and the movement keys.
//! - [`blocks`], [`shape`], [`world`], [`fluid`]: the level the player moves in.

#![forbid(unsafe_code)]

pub mod blocks;
pub mod clock;
pub mod effect;
pub mod entity;
pub mod fluid;
pub mod input;
pub mod level_load;
pub mod player;
pub mod session;
pub mod shape;
pub mod world;

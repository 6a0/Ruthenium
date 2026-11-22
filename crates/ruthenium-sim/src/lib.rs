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
//! - [`item`], [`inventory`], [`interact`]: item stacks and their components, the
//!   inventory and cooldowns, using items with either hand, attacking and breaking
//!   blocks.
//! - [`mining`]: how fast blocks break, and which the player may break.
//! - [`predict`]: block changes the client predicts until the server confirms them.
//! - [`pick`]: what the crosshair is on, and the raycasts items make.
//! - [`input`]: the mouse's turn, the scroll wheel and the movement keys.
//! - [`blocks`], [`shape`], [`world`], [`fluid`]: the level the player moves in.
//! - [`pick`]: what the crosshair points at (block and entity raycasts).

#![forbid(unsafe_code)]

pub mod block;
pub mod blocks;
pub mod clock;
pub mod direction;
pub mod effect;
pub mod entity;
pub mod fluid;
pub mod input;
pub mod interact;
pub mod inventory;
pub mod item;
pub mod level_load;
pub mod mining;
pub mod pick;
pub mod place;
pub mod player;
pub mod predict;
pub mod session;
pub mod shape;
pub mod transform;
pub mod use_on;
pub mod world;

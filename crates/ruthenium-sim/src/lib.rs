//! The game thread: everything vanilla's main thread does with a server connection.
//!
//! - [`session`]: one session with a server, from connecting into play: the
//!   configuration phase (registries, tags, known packs, features, cookies, resource-pack
//!   answers, the code of conduct), the switch into play and back, and the ticks.
//! - [`server_pack`]: the server's resource packs: vanilla's `ServerPackManager`, what it
//!   asks to be downloaded and loaded, and what the server is told.
//! - [`clock`]: when ticks run (vanilla's millisecond tick clock).
//! - [`level_load`]: when the player counts as loaded into a level.
//! - [`player`]: the local player: movement, collisions, flight, the server's view of
//!   it (abilities, entity data) and the packets it sends.
//! - [`first_person`]: what of the local player is only drawn: its hands' swings, its
//!   view's bob and tilt, what its hands hold up, and the blocks being broken.
//! - [`fov`]: how far speed, flight and items widen or narrow the view.
//! - [`effect`]: mob effects as the client keeps them.
//! - [`entity`]: the other entities: where they are, their sizes, and their pushes.
//! - [`item`], [`inventory`], [`interact`]: item stacks and their components, the
//!   inventory and cooldowns, using items with either hand, attacking and breaking
//!   blocks.
//! - [`enchantment`]: what enchantments do to the client's values (riptide).
//! - [`mining`]: how fast blocks break, and which the player may break.
//! - [`predict`]: block changes the client predicts until the server confirms them.
//! - [`pick`]: what the crosshair is on, and the raycasts items make.
//! - [`input`]: the mouse's turn, the scroll wheel and the movement keys;
//!   [`keybinds`]: the key bindings.
//! - [`blocks`], [`shape`], [`world`], [`fluid`]: the level the player moves in;
//!   [`inside`]: the blocks a movement passes through; [`border`]: the world border.
//! - [`pick`]: what the crosshair points at (block and entity raycasts).

#![forbid(unsafe_code)]

pub mod attributes;
pub mod avatar;
pub mod biome;
pub mod block;
pub mod block_entity;
pub mod blocks;
pub mod border;
pub mod chat;
pub mod clock;
pub mod command;
pub mod direction;
pub mod effect;
pub mod enchantment;
pub mod entity;
pub mod environment;
pub mod first_person;
pub mod fluid;
pub mod fov;
pub mod handoff;
pub mod hud;
pub mod hud_text;
pub mod input;
pub mod inside;
pub mod interact;
pub mod inventory;
pub mod item;
pub mod item_name;
pub mod keybinds;
pub mod level_load;
pub mod light;
pub mod menu;
pub mod mining;
pub mod pick;
pub mod piston;
pub mod place;
pub mod player;
mod potion_colours;
pub mod predict;
pub mod scene;
pub mod scoreboard;
pub mod server_pack;
pub mod session;
pub mod shape;
pub mod sound;
pub mod transform;
pub mod use_on;
pub mod vehicle;
pub mod weather;
pub mod world;

//! Ruthenium's GUI, on the CPU; the renderer draws what it produces and nothing here
//! touches the GPU. Two kinds of screen:
//!
//! - [`modern`]: Ruthenium's own out-of-game menus (title, server list, accounts,
//!   settings), a modern space-themed look with glass pills, in resolution-independent
//!   UI units.
//! - [`draw`], [`scale`], [`widget`], [`hud`]: vanilla's in-game GUI, exactly: the
//!   integer GUI scale, a draw list of quads built with vanilla's sprite rules (stretch,
//!   tile, nine-slice) in vanilla's GUI units, and the HUD laid out with them.

#![forbid(unsafe_code)]

mod clip;
pub mod draw;
pub mod face;
pub mod hud;
pub mod logo;
pub mod modern;
pub mod scale;
pub mod text;
pub mod widget;

pub use clip::Clip;
pub use draw::{DrawList, NO_TEXTURE, Quad, SpriteInfo, Sprites};
pub use scale::GuiScale;

//! Ruthenium's GUI, on the CPU; the renderer draws what it produces and nothing here
//! touches the GPU. Two kinds of screen:
//!
//! - [`modern`]: Ruthenium's own out-of-game menus (title, server list, accounts,
//!   settings), a modern space-themed look with glass pills, in resolution-independent
//!   UI units.
//! - [`draw`], [`scale`], [`widget`]: vanilla's in-game GUI, exactly: the integer GUI
//!   scale and a draw list of quads built with vanilla's sprite rules (stretch, tile,
//!   nine-slice), in vanilla's GUI units.

#![forbid(unsafe_code)]

pub mod draw;
pub mod modern;
pub mod scale;
pub mod widget;

pub use draw::{DrawList, NO_TEXTURE, Quad, SpriteInfo, Sprites};
pub use scale::GuiScale;

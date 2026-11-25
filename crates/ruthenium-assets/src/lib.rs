//! Game assets, read in place from the imported install and resource packs.
//!
//! - [`zip`]: the client jar and zipped packs, memory-mapped.
//! - [`pack`] + [`resources`]: the pack stack (vanilla at the bottom) with
//!   vanilla's lookup, metadata and filter rules.
//! - [`image`]: PNG decoding to RGBA, as vanilla's loader does it.
//! - [`meta`]: `.mcmeta` sections for sprites.
//! - [`atlas`], [`stitch`], [`mipmap`]: the 12 sprite atlases, laid out
//!   and mip-mapped exactly as 26.3 does, composed on the CPU.
//! - [`font`]: the characters' pictures and the room they take.
//! - [`model`]: every block state's model, baked into quads as 26.3 bakes them.

pub mod animation;
pub mod atlas;
pub mod font;
pub mod id;
pub mod image;
pub mod meta;
pub mod mipmap;
pub mod model;
pub mod pack;
pub mod resources;
pub mod stitch;
pub mod zip;

pub use animation::AtlasAnimations;
pub use atlas::{ATLASES, Atlas, AtlasDef, AtlasOptions};
pub use font::Font;
pub use id::Id;
pub use image::Image;
pub use model::{BakedQuad, BlockModels, Layer, ModelPart};
pub use pack::Pack;
pub use resources::{Resource, Resources};

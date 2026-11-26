//! The world's terrain: every block state's model and rules ([`BlockTable`]), and the
//! mesher that turns a section of the level into vertices ([`Mesher`]).

mod camera;
pub(crate) mod facing;
mod fluid;
mod lightmap;
pub(crate) mod medium;
mod mesher;
mod smooth;
mod sort;
pub(crate) mod table;
#[cfg(test)]
pub(crate) mod testing;
mod tint;
mod tracker;
mod visibility;
pub(crate) mod world;

pub use camera::{CameraPose, NEAR_PLANE, View};
pub use fluid::FluidModels;
pub use lightmap::LightmapInputs;
pub use mesher::{MeshOptions, Mesher, SectionMesh};
pub use table::BlockTable;
pub use tint::{BiomeTints, ColourMaps, MAX_BLEND_RADIUS};
pub use tracker::{CAUSES, TrackerStats};
pub use world::{
    DEFAULT_FADE_MS, LatencySummary, POOL_ENVELOPE, TerrainDraws, TerrainShader, WorldAssets,
    WorldLatencies, WorldStats,
};

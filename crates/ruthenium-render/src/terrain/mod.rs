//! The world's terrain: every block state's model and rules ([`BlockTable`]), and the
//! mesher that turns a section of the level into vertices ([`Mesher`]).

mod camera;
mod lightmap;
mod mesher;
mod table;
#[cfg(test)]
mod testing;
mod tracker;
pub(crate) mod world;

pub use camera::{CameraPose, NEAR_PLANE, View};
pub use lightmap::LightmapInputs;
pub(crate) use lightmap::colour_attribute;
pub use mesher::{MeshOptions, Mesher, SectionMesh};
pub use table::BlockTable;
pub use tracker::{CAUSES, TrackerStats};
pub use world::{
    LatencySummary, POOL_ENVELOPE, TerrainDraws, WorldAssets, WorldLatencies, WorldStats,
};

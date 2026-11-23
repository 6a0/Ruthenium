//! The two terrain vertex layouts the first terrain pipeline will choose between, as
//! candidates: nothing draws with them yet. They are here so that every device's report
//! says, before the mesher exists, whether it can read them and with which stride
//! (a portability-subset device, Metal behind MoltenVK, wants strides aligned).
//!
//! Both pack a vertex into formats every Vulkan device must read from a vertex buffer:
//! - 16 bytes: three 16-bit section-local positions with the two 8-bit lights in the
//!   fourth word, an RGBA8 colour, two 16-bit texture coordinates;
//! - 20 bytes: the same and one more 32-bit word, for more precise coordinates or a
//!   sprite index.

use ruthenium_gpu::{VertexAttribute, vk};

/// A vertex layout to check a device against.
#[derive(Clone, Copy, Debug)]
pub struct VertexCandidate {
    pub name: &'static str,
    /// Bytes per vertex as packed.
    pub bytes: u32,
    pub attributes: &'static [VertexAttribute],
}

const POSITION_AND_LIGHT: VertexAttribute = VertexAttribute {
    offset: 0,
    format: vk::Format::R16G16B16A16_UINT,
};
const COLOUR: VertexAttribute = VertexAttribute {
    offset: 8,
    format: vk::Format::R8G8B8A8_UNORM,
};
const UV: VertexAttribute = VertexAttribute {
    offset: 12,
    format: vk::Format::R16G16_UINT,
};
const EXTRA: VertexAttribute = VertexAttribute {
    offset: 16,
    format: vk::Format::R32_UINT,
};

pub const TERRAIN_VERTEX_CANDIDATES: [VertexCandidate; 2] = [
    VertexCandidate {
        name: "terrain 16 B",
        bytes: 16,
        attributes: &[POSITION_AND_LIGHT, COLOUR, UV],
    },
    VertexCandidate {
        name: "terrain 20 B",
        bytes: 20,
        attributes: &[POSITION_AND_LIGHT, COLOUR, UV, EXTRA],
    },
];

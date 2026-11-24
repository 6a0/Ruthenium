//! The terrain vertex: 16 bytes, in formats every Vulkan device must read from a vertex
//! buffer.
//!
//! - Three 16-bit positions, in 1/2048 of a block from 8 blocks before the section's
//!   corner: a section's own blocks and everything a model may reach beyond them (-8 to
//!   just under 24 blocks) fit, and a corner on a block's sixteenth is exact, so two
//!   sections give a shared edge the same place.
//! - The fourth 16-bit word holds the two lights as vanilla's vertices carry them, 0 to
//!   240 (sixteen times a light level): block light in the low byte, sky light in the
//!   high one. They are read as integers, not as fractions of 255.
//! - An RGBA8 colour: the face's shade times its tint.
//! - Two 16-bit texture coordinates in eighths of a texel of the atlas, which reaches
//!   the far edge of a 4096-texel atlas (32768) and just short of 8192.
//!
//! A second layout with one more 32-bit word (more precise coordinates, or a sprite
//! index) is described for every device's report; nothing draws with it yet.

use ruthenium_gpu::{VertexAttribute, VertexInput, vk};

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

/// Bytes of one terrain vertex.
pub const VERTEX_BYTES: usize = 16;

/// The layout the terrain pipelines read: the attributes are `terrain.vert`'s inputs at
/// locations 0, 1 and 2.
pub const TERRAIN_VERTEX: VertexInput = VertexInput {
    stride: VERTEX_BYTES as u32,
    attributes: TERRAIN_VERTEX_CANDIDATES[0].attributes,
};

/// Position codes per block, and the blocks before a section's corner that code 0 is.
pub const POSITION_SCALE: f32 = 2048.0;
pub const POSITION_BIAS: f32 = 8.0;
/// Texture coordinate codes per texel.
pub const UV_SCALE: f32 = 8.0;

/// The code of a position along one axis, in blocks from the section's corner. `None`
/// outside -8 to just under 24 blocks.
pub fn position_code(blocks: f32) -> Option<u16> {
    let code = ((blocks + POSITION_BIAS) * POSITION_SCALE).round();
    (0.0..=65535.0).contains(&code).then_some(code as u16)
}

/// The position a code stands for.
pub fn position_of(code: u16) -> f32 {
    f32::from(code) / POSITION_SCALE - POSITION_BIAS
}

/// The code of a texture coordinate in texels of the atlas. `None` beyond 8191.875.
pub fn uv_code(texels: f32) -> Option<u16> {
    let code = (texels * UV_SCALE).round();
    (0.0..=65535.0).contains(&code).then_some(code as u16)
}

/// One vertex before it is packed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Vertex {
    pub position: [u16; 3],
    /// 0 to 240.
    pub block_light: u8,
    pub sky_light: u8,
    /// Red, green, blue, alpha.
    pub colour: [u8; 4],
    pub uv: [u16; 2],
}

impl Vertex {
    /// Appends the vertex's 16 bytes.
    pub fn pack(&self, out: &mut Vec<u8>) {
        for c in self.position {
            out.extend_from_slice(&c.to_le_bytes());
        }
        out.push(self.block_light);
        out.push(self.sky_light);
        out.extend_from_slice(&self.colour);
        for c in self.uv {
            out.extend_from_slice(&c.to_le_bytes());
        }
    }

    /// Reads a packed vertex back.
    pub fn unpack(bytes: &[u8; VERTEX_BYTES]) -> Vertex {
        let word = |i: usize| u16::from_le_bytes([bytes[i], bytes[i + 1]]);
        Vertex {
            position: [word(0), word(2), word(4)],
            block_light: bytes[6],
            sky_light: bytes[7],
            colour: [bytes[8], bytes[9], bytes[10], bytes[11]],
            uv: [word(12), word(14)],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn positions_span_eight_blocks_before_a_section_to_eight_after() {
        assert_eq!(position_code(-8.0), Some(0));
        assert_eq!(position_code(0.0), Some(16384));
        assert_eq!(position_code(16.0), Some(49152));
        assert_eq!(position_code(24.0 - 1.0 / 2048.0), Some(65535));
        assert_eq!(position_code(24.0), None);
        assert_eq!(position_code(-8.001), None);
        assert_eq!(position_code(f32::NAN), None);
        // Every sixteenth of a block, and the 22.5 degree corners models use, to 1/4096.
        for sixteenth in -128..384 {
            let p = sixteenth as f32 / 16.0;
            assert_eq!(position_of(position_code(p).unwrap()), p);
        }
        let rotated = 0.5 + 0.5 * std::f32::consts::FRAC_1_SQRT_2;
        assert!((position_of(position_code(rotated).unwrap()) - rotated).abs() <= 1.0 / 4096.0);
    }

    #[test]
    fn two_sections_put_a_shared_edge_in_the_same_place() {
        // The east edge of one section's last block and the west edge of the next
        // section's first: 16 blocks from one corner, 0 from the other, 16 apart.
        let (east, west) = (position_code(16.0).unwrap(), position_code(0.0).unwrap());
        assert_eq!(position_of(east), position_of(west) + 16.0);
        // So for a model corner that both reach, half a block over the border.
        let (over, under) = (position_code(16.5).unwrap(), position_code(0.5).unwrap());
        assert_eq!(position_of(over), position_of(under) + 16.0);
    }

    #[test]
    fn texture_coordinates_reach_the_far_edge_of_a_4096_atlas() {
        assert_eq!(uv_code(0.0), Some(0));
        assert_eq!(uv_code(4096.0), Some(32768));
        assert_eq!(uv_code(8191.875), Some(65535));
        assert_eq!(uv_code(8192.0), None);
        assert_eq!(uv_code(-0.1), None);
        // Texels and their halves are exact; anything else is at most 1/16 texel off.
        assert_eq!(uv_code(7.5), Some(60));
        for i in 0..1000 {
            let t = i as f32 * 0.0371;
            let back = f32::from(uv_code(t).unwrap()) / UV_SCALE;
            assert!((back - t).abs() <= 1.0 / 16.0 + 1e-6);
        }
    }

    #[test]
    fn a_vertex_packs_into_the_pipeline_layout() {
        let v = Vertex {
            position: [0x0102, 0x0304, 0x0506],
            block_light: 0x70,
            sky_light: 0xF0,
            colour: [1, 2, 3, 4],
            uv: [0x0A0B, 0x0C0D],
        };
        let mut bytes = Vec::new();
        v.pack(&mut bytes);
        assert_eq!(
            bytes,
            [
                2, 1, 4, 3, 6, 5, 0x70, 0xF0, 1, 2, 3, 4, 0x0B, 0x0A, 0x0D, 0x0C
            ]
        );
        assert_eq!(Vertex::unpack(bytes[..].try_into().unwrap()), v);
        // The fourth 16-bit word of the first attribute holds the lights, block light
        // low: what the shader splits with & 0xFF and >> 8.
        let w = u16::from_le_bytes([bytes[6], bytes[7]]);
        assert_eq!((w & 0xFF, w >> 8), (0x70, 0xF0));
        assert_eq!(TERRAIN_VERTEX.stride as usize, bytes.len());
        assert_eq!(TERRAIN_VERTEX.attributes[1].offset, 8);
        assert_eq!(TERRAIN_VERTEX.attributes[2].offset, 12);
    }
}

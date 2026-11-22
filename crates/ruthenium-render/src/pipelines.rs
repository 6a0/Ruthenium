//! The closed set of pipelines the renderer uses. All of them are built at start-up; adding
//! one means adding an id here and its shaders in `ruthenium-shaders`.

use ruthenium_gpu::{Blend, GraphicsDesc, vk};
use ruthenium_shaders as shaders;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(usize)]
pub enum PipelineId {
    /// Full-screen gradient behind the menus.
    Background,
}

impl PipelineId {
    pub const ALL: [PipelineId; 1] = [PipelineId::Background];

    pub fn index(self) -> usize {
        self as usize
    }

    fn desc(self) -> GraphicsDesc {
        match self {
            PipelineId::Background => GraphicsDesc {
                name: "background",
                vertex: shaders::BACKGROUND_VERT.words(),
                fragment: shaders::BACKGROUND_FRAG.words(),
                topology: vk::PrimitiveTopology::TRIANGLE_LIST,
                blend: Blend::Opaque,
            },
        }
    }
}

/// Descriptions of every pipeline, indexed by [`PipelineId::index`].
pub fn descs() -> [GraphicsDesc; PipelineId::ALL.len()] {
    PipelineId::ALL.map(PipelineId::desc)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_index_the_table_in_order() {
        for (i, id) in PipelineId::ALL.iter().enumerate() {
            assert_eq!(id.index(), i);
        }
    }

    #[test]
    fn push_constant_budgets_agree() {
        assert_eq!(
            ruthenium_gpu::PUSH_CONSTANT_BYTES,
            shaders::PUSH_CONSTANT_BYTES
        );
    }

    /// The value of `#define name value` in a GLSL source.
    fn define(source: &str, name: &str) -> Option<u32> {
        source.lines().find_map(|l| {
            let mut words = l.split_whitespace();
            (words.next() == Some("#define") && words.next() == Some(name))
                .then(|| words.next()?.parse().ok())
                .flatten()
        })
    }

    #[test]
    fn texture_table_indices_agree_with_glsl() {
        let glsl = include_str!("../../ruthenium-shaders/glsl/include/textures.glsl");
        assert_eq!(
            define(glsl, "SAMPLER_COUNT"),
            Some(ruthenium_gpu::SAMPLER_COUNT)
        );
        for s in ruthenium_gpu::SamplerId::ALL {
            assert_eq!(define(glsl, s.glsl_name()), Some(s.index()), "{s:?}");
        }
        assert_eq!(
            define(glsl, "TEXTURE_MISSING"),
            Some(ruthenium_gpu::TextureId::MISSING.index())
        );
        let push = include_str!("../../ruthenium-shaders/glsl/include/push.glsl");
        assert_eq!(
            define(push, "PUSH_CONSTANT_BYTES"),
            Some(ruthenium_gpu::PUSH_CONSTANT_BYTES)
        );
    }
}

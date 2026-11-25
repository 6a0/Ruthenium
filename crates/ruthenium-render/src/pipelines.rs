//! The closed set of pipelines the renderer uses. All of them are built at start-up; adding
//! one means adding an id here and its shaders in `ruthenium-shaders`.

use ruthenium_gpu::{Blend, Depth, GraphicsDesc, vk};
use ruthenium_shaders as shaders;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(usize)]
pub enum PipelineId {
    /// Full-screen gradient behind the menus.
    Background,
    /// The menus' blurred backdrop (downsample and Gaussian passes).
    Blur,
    /// GUI quads (ruthenium-ui's draw list).
    Gui,
    /// Rounded shapes and glass of the modern menus.
    Shape,
    /// Text of the modern menus (distance-field glyphs).
    Text,
    /// The development overlay's rectangles.
    Overlay,
    /// Terrain whose texels are all opaque, terrain with holes (leaves, plants, panes)
    /// and terrain that is seen through (water, stained glass, ice), which is blended
    /// over what is behind it; once for each way the atlas is filtered
    /// ([`crate::terrain::TerrainShader`]): texels with hard edges; far texels averaged,
    /// with the samples that count for nothing left out; the same with every sample
    /// taken; and hard edges without fog or fade.
    TerrainNearest,
    TerrainNearestCutout,
    TerrainNearestTranslucent,
    TerrainRgss,
    TerrainRgssCutout,
    TerrainRgssTranslucent,
    TerrainRgssReference,
    TerrainRgssReferenceCutout,
    TerrainRgssReferenceTranslucent,
    TerrainPlain,
    TerrainPlainCutout,
    TerrainPlainTranslucent,
    /// Models made of parts (block entities): without holes; with holes, seen from the
    /// front; with holes, seen from both sides; what is blended over a model in the
    /// plane it lies in (a banner's patterns); and what is blended over what is behind it
    /// (players and their heads), seen from both sides or (the invisible, as those see
    /// them who do) from the front.
    ModelSolid,
    ModelCutout,
    ModelCutoutFaces,
    ModelPattern,
    ModelTranslucent,
    ModelTranslucentFront,
    /// The text on signs, and what surrounds it where it glows.
    SignText,
    SignOutline,
    /// The names over heads: where nothing is in front of them, and seen through
    /// everything.
    NameTag,
    NameTagSheer,
    /// The shadows under entities, blended over the ground they lie on.
    Shadow,
    /// The cracks in a block being broken, which darken and lighten the faces they lie
    /// on; and the outline of the block the crosshair is on.
    Crack,
    Outline,
    /// The sky drawn before the world, over the cleared target: what covers what is
    /// there (the dome), what is blended over it (a sunrise's glow, the End's sky) and
    /// what adds light to it (the sun, the moon, the stars).
    SkyFlat,
    SkyBlend,
    SkyGlow,
    /// The same three drawn after the world's opaque layers, only where nothing was
    /// drawn.
    SkyFlatBehind,
    SkyBlendBehind,
    SkyGlowBehind,
    /// The clouds, blended over the world: Fancy cells seen from outside, and Fast
    /// squares seen from both sides.
    CloudsFancy,
    CloudsFast,
    /// The four pipelines that also draw over the world, built for a pass with a depth
    /// attachment (which they neither test nor write).
    WorldGui,
    WorldShape,
    WorldText,
    WorldOverlay,
    /// GUI quads of the HUD that are not blended as the others are, over the world: the
    /// crosshair, which turns what is behind it to its opposite, and the vignette, which
    /// darkens it.
    Crosshair,
    Vignette,
    /// A menu's layer laid over the world behind it.
    Layer,
}

impl PipelineId {
    pub const ALL: [PipelineId; 46] = [
        PipelineId::Background,
        PipelineId::Blur,
        PipelineId::Gui,
        PipelineId::Shape,
        PipelineId::Text,
        PipelineId::Overlay,
        PipelineId::TerrainNearest,
        PipelineId::TerrainNearestCutout,
        PipelineId::TerrainNearestTranslucent,
        PipelineId::TerrainRgss,
        PipelineId::TerrainRgssCutout,
        PipelineId::TerrainRgssTranslucent,
        PipelineId::TerrainRgssReference,
        PipelineId::TerrainRgssReferenceCutout,
        PipelineId::TerrainRgssReferenceTranslucent,
        PipelineId::TerrainPlain,
        PipelineId::TerrainPlainCutout,
        PipelineId::TerrainPlainTranslucent,
        PipelineId::ModelSolid,
        PipelineId::ModelCutout,
        PipelineId::ModelCutoutFaces,
        PipelineId::ModelPattern,
        PipelineId::ModelTranslucent,
        PipelineId::ModelTranslucentFront,
        PipelineId::SignText,
        PipelineId::SignOutline,
        PipelineId::NameTag,
        PipelineId::NameTagSheer,
        PipelineId::Shadow,
        PipelineId::Crack,
        PipelineId::Outline,
        PipelineId::SkyFlat,
        PipelineId::SkyBlend,
        PipelineId::SkyGlow,
        PipelineId::SkyFlatBehind,
        PipelineId::SkyBlendBehind,
        PipelineId::SkyGlowBehind,
        PipelineId::CloudsFancy,
        PipelineId::CloudsFast,
        PipelineId::WorldGui,
        PipelineId::WorldShape,
        PipelineId::WorldText,
        PipelineId::WorldOverlay,
        PipelineId::Crosshair,
        PipelineId::Vignette,
        PipelineId::Layer,
    ];

    /// The pipeline that draws the same in a pass with a depth attachment, for the
    /// four that have one.
    pub fn over_world(self) -> PipelineId {
        match self {
            PipelineId::Gui => PipelineId::WorldGui,
            PipelineId::Shape => PipelineId::WorldShape,
            PipelineId::Text => PipelineId::WorldText,
            PipelineId::Overlay => PipelineId::WorldOverlay,
            other => other,
        }
    }

    pub fn index(self) -> usize {
        self as usize
    }

    /// Whether this is the pipeline of a translucent terrain layer.
    pub fn is_translucent_terrain(self) -> bool {
        matches!(
            self,
            PipelineId::TerrainNearestTranslucent
                | PipelineId::TerrainRgssTranslucent
                | PipelineId::TerrainRgssReferenceTranslucent
                | PipelineId::TerrainPlainTranslucent
        )
    }

    /// A terrain pipeline's name and its two stages; `None` for the others.
    pub fn terrain_stages(
        self,
    ) -> Option<(
        &'static str,
        &'static shaders::Spirv,
        &'static shaders::Spirv,
    )> {
        use shaders::{TERRAIN_PLAIN_VERT as PLAIN, TERRAIN_VERT as FULL};
        Some(match self {
            PipelineId::TerrainNearest => {
                ("terrain_nearest", &FULL, &shaders::TERRAIN_NEAREST_FRAG)
            }
            PipelineId::TerrainNearestCutout => (
                "terrain_nearest_cutout",
                &FULL,
                &shaders::TERRAIN_NEAREST_CUTOUT_FRAG,
            ),
            PipelineId::TerrainNearestTranslucent => (
                "terrain_nearest_translucent",
                &FULL,
                &shaders::TERRAIN_NEAREST_TRANSLUCENT_FRAG,
            ),
            PipelineId::TerrainRgss => ("terrain_rgss", &FULL, &shaders::TERRAIN_RGSS_FRAG),
            PipelineId::TerrainRgssTranslucent => (
                "terrain_rgss_translucent",
                &FULL,
                &shaders::TERRAIN_RGSS_TRANSLUCENT_FRAG,
            ),
            PipelineId::TerrainRgssReferenceTranslucent => (
                "terrain_rgss_reference_translucent",
                &FULL,
                &shaders::TERRAIN_RGSS_REFERENCE_TRANSLUCENT_FRAG,
            ),
            PipelineId::TerrainPlainTranslucent => (
                "terrain_plain_translucent",
                &PLAIN,
                &shaders::TERRAIN_PLAIN_TRANSLUCENT_FRAG,
            ),
            PipelineId::TerrainRgssCutout => (
                "terrain_rgss_cutout",
                &FULL,
                &shaders::TERRAIN_RGSS_CUTOUT_FRAG,
            ),
            PipelineId::TerrainRgssReference => (
                "terrain_rgss_reference",
                &FULL,
                &shaders::TERRAIN_RGSS_REFERENCE_FRAG,
            ),
            PipelineId::TerrainRgssReferenceCutout => (
                "terrain_rgss_reference_cutout",
                &FULL,
                &shaders::TERRAIN_RGSS_REFERENCE_CUTOUT_FRAG,
            ),
            PipelineId::TerrainPlain => ("terrain_plain", &PLAIN, &shaders::TERRAIN_PLAIN_FRAG),
            PipelineId::TerrainPlainCutout => (
                "terrain_plain_cutout",
                &PLAIN,
                &shaders::TERRAIN_PLAIN_CUTOUT_FRAG,
            ),
            _ => return None,
        })
    }

    fn desc(self) -> GraphicsDesc {
        // The translucent layer is blended over what is there and writes its depth, as
        // vanilla's translucent terrain pipeline does.
        let terrain =
            |name, vertex: &shaders::Spirv, fragment: &shaders::Spirv, blend| GraphicsDesc {
                name,
                vertex: vertex.words(),
                fragment: fragment.words(),
                topology: vk::PrimitiveTopology::TRIANGLE_LIST,
                blend,
                vertex_input: Some(crate::terrain_vertex::TERRAIN_VERTEX),
                depth: Depth::TestAndWrite,
                cull_back: true,
            };
        let over_world = |name, of: PipelineId| GraphicsDesc {
            name,
            depth: Depth::Ignore,
            ..of.desc()
        };
        // Nothing of the sky is culled: its pieces are seen from inside.
        let sky = |name, blend, depth| GraphicsDesc {
            name,
            vertex: shaders::SKY_VERT.words(),
            fragment: shaders::SKY_FRAG.words(),
            topology: vk::PrimitiveTopology::TRIANGLE_LIST,
            blend,
            vertex_input: Some(crate::sky::SKY_VERTEX),
            depth,
            cull_back: false,
        };
        // The clouds are blended and write their depth, as vanilla's.
        let clouds = |name, cull_back| GraphicsDesc {
            name,
            vertex: shaders::CLOUDS_VERT.words(),
            fragment: shaders::CLOUDS_FRAG.words(),
            topology: vk::PrimitiveTopology::TRIANGLE_LIST,
            blend: Blend::Alpha,
            vertex_input: Some(crate::clouds::CLOUD_FACE),
            depth: Depth::TestAndWrite,
            cull_back,
        };
        if let Some((name, vertex, fragment)) = self.terrain_stages() {
            let blend = if self.is_translucent_terrain() {
                Blend::Alpha
            } else {
                Blend::Opaque
            };
            return terrain(name, vertex, fragment, blend);
        }
        let model = |name, fragment: &shaders::Spirv, blend, depth, cull_back| GraphicsDesc {
            name,
            vertex: shaders::MODEL_VERT.words(),
            fragment: fragment.words(),
            topology: vk::PrimitiveTopology::TRIANGLE_LIST,
            blend,
            vertex_input: Some(crate::block_entity::renderer::MODEL_VERTEX),
            depth,
            cull_back,
        };
        let sign = |name, vertex: &shaders::Spirv, depth| GraphicsDesc {
            name,
            vertex: vertex.words(),
            fragment: shaders::MODEL_CUTOUT_FRAG.words(),
            topology: vk::PrimitiveTopology::TRIANGLE_LIST,
            blend: Blend::Alpha,
            vertex_input: None,
            depth,
            cull_back: true,
        };
        match self {
            PipelineId::ModelSolid => model(
                "model_solid",
                &shaders::MODEL_SOLID_FRAG,
                Blend::Opaque,
                Depth::TestAndWrite,
                true,
            ),
            PipelineId::ModelCutout => model(
                "model_cutout",
                &shaders::MODEL_CUTOUT_FRAG,
                Blend::Opaque,
                Depth::TestAndWrite,
                true,
            ),
            PipelineId::ModelCutoutFaces => model(
                "model_cutout_faces",
                &shaders::MODEL_CUTOUT_FACES_FRAG,
                Blend::Opaque,
                Depth::TestAndWrite,
                false,
            ),
            PipelineId::ModelPattern => model(
                "model_pattern",
                &shaders::MODEL_SOLID_FRAG,
                Blend::Alpha,
                Depth::Test,
                true,
            ),
            PipelineId::ModelTranslucent => model(
                "model_translucent",
                &shaders::MODEL_CUTOUT_FACES_FRAG,
                Blend::Alpha,
                Depth::TestAndWrite,
                false,
            ),
            PipelineId::ModelTranslucentFront => model(
                "model_translucent_front",
                &shaders::MODEL_CUTOUT_FRAG,
                Blend::Alpha,
                Depth::TestAndWrite,
                true,
            ),
            PipelineId::SignText => sign("sign_text", &shaders::SIGN_TEXT_VERT, Depth::Decal),
            PipelineId::SignOutline => sign(
                "sign_outline",
                &shaders::SIGN_OUTLINE_VERT,
                Depth::TestAndWrite,
            ),
            PipelineId::NameTag => sign("name_tag", &shaders::SIGN_TEXT_VERT, Depth::TestAndWrite),
            PipelineId::NameTagSheer => {
                sign("name_tag_sheer", &shaders::SIGN_TEXT_VERT, Depth::Ignore)
            }
            // Seen from above only, as the game's.
            PipelineId::Shadow => GraphicsDesc {
                name: "shadow",
                vertex: shaders::SHADOW_VERT.words(),
                fragment: shaders::MODEL_SOLID_FRAG.words(),
                topology: vk::PrimitiveTopology::TRIANGLE_LIST,
                blend: Blend::Alpha,
                vertex_input: None,
                depth: Depth::Test,
                cull_back: true,
            },
            // In the planes of the faces they are on, and a little nearer.
            PipelineId::Crack => GraphicsDesc {
                name: "crack",
                vertex: shaders::CRACK_VERT.words(),
                fragment: shaders::MODEL_REPEAT_FRAG.words(),
                topology: vk::PrimitiveTopology::TRIANGLE_LIST,
                blend: Blend::Multiply,
                vertex_input: None,
                depth: Depth::TestDecal,
                cull_back: true,
            },
            // A line's strip faces whichever way its ends lie on the target.
            PipelineId::Outline => GraphicsDesc {
                name: "outline",
                vertex: shaders::OUTLINE_VERT.words(),
                fragment: shaders::OUTLINE_FRAG.words(),
                topology: vk::PrimitiveTopology::TRIANGLE_LIST,
                blend: Blend::Alpha,
                vertex_input: None,
                depth: Depth::TestAndWrite,
                cull_back: false,
            },
            PipelineId::SkyFlat => sky("sky_flat", Blend::Opaque, Depth::Ignore),
            PipelineId::SkyBlend => sky("sky_blend", Blend::Alpha, Depth::Ignore),
            PipelineId::SkyGlow => sky("sky_glow", Blend::Glow, Depth::Ignore),
            PipelineId::SkyFlatBehind => sky("sky_flat_behind", Blend::Opaque, Depth::Test),
            PipelineId::SkyBlendBehind => sky("sky_blend_behind", Blend::Alpha, Depth::Test),
            PipelineId::SkyGlowBehind => sky("sky_glow_behind", Blend::Glow, Depth::Test),
            PipelineId::CloudsFancy => clouds("clouds_fancy", true),
            PipelineId::CloudsFast => clouds("clouds_fast", false),
            // Handled above.
            PipelineId::TerrainNearest
            | PipelineId::TerrainNearestCutout
            | PipelineId::TerrainRgss
            | PipelineId::TerrainRgssCutout
            | PipelineId::TerrainRgssReference
            | PipelineId::TerrainRgssReferenceCutout
            | PipelineId::TerrainPlain
            | PipelineId::TerrainPlainCutout
            | PipelineId::TerrainNearestTranslucent
            | PipelineId::TerrainRgssTranslucent
            | PipelineId::TerrainRgssReferenceTranslucent
            | PipelineId::TerrainPlainTranslucent => unreachable!("a terrain pipeline"),
            PipelineId::WorldGui => over_world("gui_world", PipelineId::Gui),
            PipelineId::WorldShape => over_world("shape_world", PipelineId::Shape),
            PipelineId::WorldText => over_world("text_world", PipelineId::Text),
            PipelineId::WorldOverlay => over_world("overlay_world", PipelineId::Overlay),
            PipelineId::Crosshair => GraphicsDesc {
                blend: Blend::Invert,
                ..over_world("crosshair", PipelineId::Gui)
            },
            PipelineId::Vignette => GraphicsDesc {
                blend: Blend::Darken,
                ..over_world("vignette", PipelineId::Gui)
            },
            PipelineId::Background => GraphicsDesc::flat(
                "background",
                shaders::BACKGROUND_VERT.words(),
                shaders::BACKGROUND_FRAG.words(),
                Blend::Opaque,
            ),
            PipelineId::Blur => GraphicsDesc::flat(
                "blur",
                shaders::BACKGROUND_VERT.words(),
                shaders::BLUR_FRAG.words(),
                Blend::Opaque,
            ),
            PipelineId::Layer => GraphicsDesc::flat(
                "layer",
                shaders::BACKGROUND_VERT.words(),
                shaders::BLUR_FRAG.words(),
                Blend::Premultiplied,
            ),
            PipelineId::Shape => GraphicsDesc::flat(
                "shape",
                shaders::SHAPE_VERT.words(),
                shaders::SHAPE_FRAG.words(),
                Blend::Alpha,
            ),
            PipelineId::Text => GraphicsDesc::flat(
                "text",
                shaders::TEXT_VERT.words(),
                shaders::TEXT_FRAG.words(),
                Blend::Alpha,
            ),
            PipelineId::Gui => GraphicsDesc::flat(
                "gui",
                shaders::GUI_VERT.words(),
                shaders::GUI_FRAG.words(),
                Blend::Alpha,
            ),
            PipelineId::Overlay => GraphicsDesc::flat(
                "overlay",
                shaders::OVERLAY_VERT.words(),
                shaders::OVERLAY_FRAG.words(),
                Blend::Alpha,
            ),
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
    fn pipelines_stay_inside_the_portability_subset() {
        // Triangle lists only: fans are optional in the subset (triangleFans).
        for d in descs() {
            assert_eq!(
                d.topology,
                vk::PrimitiveTopology::TRIANGLE_LIST,
                "{}",
                d.name
            );
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
        let gui = include_str!("../../ruthenium-shaders/glsl/gui.frag");
        assert!(gui.contains(&format!(
            "#define GUI_NO_TEXTURE {:#X}u",
            ruthenium_ui::NO_TEXTURE
        )));
        let frame = include_str!("../../ruthenium-shaders/glsl/include/frame.glsl");
        assert_eq!(
            define(frame, "FRAME_DATA_SET"),
            Some(ruthenium_gpu::FRAME_DATA_SET)
        );
        let push = include_str!("../../ruthenium-shaders/glsl/include/push.glsl");
        assert_eq!(
            define(push, "PUSH_CONSTANT_BYTES"),
            Some(ruthenium_gpu::PUSH_CONSTANT_BYTES)
        );
    }
}

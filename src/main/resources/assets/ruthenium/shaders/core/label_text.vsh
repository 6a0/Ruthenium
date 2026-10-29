#version 330
#extension GL_ARB_separate_shader_objects : require
#extension GL_ARB_gpu_shader5 : require

#if !defined(IS_GUI) && !defined(IS_SEE_THROUGH)
#include <minecraft:fog.glsl>
#include <minecraft:sample_lightmap.glsl>
#endif

#include <minecraft:dynamictransforms.glsl>
#include <minecraft:projection.glsl>

// Ruthenium: vanilla's core/text.vsh for a label's glyphs kept in label space (RetainedLabels). Each vertex names its
// label's region; the region's pose rows (camera-relative) and light are this frame's, in LabelPoses (4 texels per
// region: three pose rows, then the light as UV2's two values)
layout(location = 0) in vec3 Position;
layout(location = 1) in vec4 Color;
layout(location = 2) in vec2 UV0;
layout(location = 3) in ivec2 LabelRegion;

uniform samplerBuffer LabelPoses;

#if !defined(IS_GUI) && !defined(IS_SEE_THROUGH)
uniform sampler2D Sampler2;
layout(location = 0) out float sphericalVertexDistance;
layout(location = 1) out float cylindricalVertexDistance;
#endif

layout(location = 2) out vec4 vertexColor;
layout(location = 3) out vec2 texCoord0;

void main() {
    int base = LabelRegion.x * 4;
    vec4 row0 = texelFetch(LabelPoses, base);
    vec4 row1 = texelFetch(LabelPoses, base + 1);
    vec4 row2 = texelFetch(LabelPoses, base + 2);
    // The same operations in the same order as JOML's Matrix4f.transformPosition (no fused multiply-add), so the
    // position is bit-identical to the one vanilla writes into the vertex
    precise vec3 pos;
    pos.x = row0.x * Position.x + (row0.y * Position.y + (row0.z * Position.z + row0.w));
    pos.y = row1.x * Position.x + (row1.y * Position.y + (row1.z * Position.z + row1.w));
    pos.z = row2.x * Position.x + (row2.y * Position.y + (row2.z * Position.z + row2.w));
    gl_Position = ProjMat * ModelViewMat * vec4(pos, 1.0);

#if !defined(IS_GUI) && !defined(IS_SEE_THROUGH)
    ivec2 uv2 = ivec2(texelFetch(LabelPoses, base + 3).xy);
    sphericalVertexDistance = fog_spherical_distance(pos);
    cylindricalVertexDistance = fog_cylindrical_distance(pos);
    vertexColor = Color * sample_lightmap(Sampler2, uv2);
#else
    vertexColor = Color;
#endif
    texCoord0 = UV0;
}

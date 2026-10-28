#version 330
#extension GL_ARB_separate_shader_objects : require

// Vanilla's core/terrain vertex shader for Rubidium's compressed terrain vertices (CompressedTerrain.FORMAT):
// the position is 3x21 bit fixed point (1/65536 block, offset -8) and UV0 is 16 bit fixed point.

#include <minecraft:fog.glsl>
#include <minecraft:globals.glsl>
#include <minecraft:projection.glsl>
#include <minecraft:sample_lightmap.glsl>
#include <minecraft:terrainglobals.glsl>
#ifndef MULTIDRAW_TERRAIN
    #include <minecraft:chunksection.glsl>
#endif

layout(location = 0) in uvec2 Position;
layout(location = 1) in vec4 Color;
layout(location = 2) in uvec2 UV0;
layout(location = 3) in ivec2 UV2;
#ifdef MULTIDRAW_TERRAIN
layout(location = 4) in ivec3 ChunkPosition;
layout(location = 5) in float ChunkVisibility;
#endif

#ifndef OIT_ALPHA_ONLY
uniform sampler2D Sampler2;
#endif

layout(location = 0) out float sphericalVertexDistance;
layout(location = 1) out float cylindricalVertexDistance;
layout(location = 2) out vec4 vertexColor;
layout(location = 3) out vec2 texCoord0;
layout(location = 4) out float chunkVisibility;

vec3 decodePosition(uvec2 packed) {
    uint x = packed.x & 0x1FFFFFu;
    uint y = (packed.x >> 21u) | ((packed.y & 0x3FFu) << 11u);
    uint z = (packed.y >> 10u) & 0x1FFFFFu;
    return vec3(x, y, z) * (1.0 / 65536.0) - 8.0;
}

void main() {
    vec3 pos = decodePosition(Position) + (ChunkPosition - CameraBlockPos) + CameraOffset;
    gl_Position = ProjMat * ModelViewMat * vec4(pos, 1.0);

    sphericalVertexDistance = fog_spherical_distance(pos);
    cylindricalVertexDistance = fog_cylindrical_distance(pos);
    #ifndef OIT_ALPHA_ONLY
    vertexColor = Color * sample_lightmap(Sampler2, UV2);
    #else
    vertexColor = Color;
    #endif
    texCoord0 = vec2(UV0) * (1.0 / 65536.0);

    const float chunkFullyVisibleRange = 16.0;
    float dist = length(pos);
    chunkVisibility = mix(1.0, ChunkVisibility, clamp((dist - chunkFullyVisibleRange) / chunkFullyVisibleRange, 0.0, 1.0));
}

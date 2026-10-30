#version 330
#extension GL_ARB_separate_shader_objects : require

// Whether any part of a section's box (grown by a block) can be in front of the depth the occluding terrain wrote.
// One instance per section info, of TILES x TILES points that each read a part of the depth under the box's screen
// rectangle: all write to the section's texel of the visibility target with MAX blending, so it is 1 if any part
// is visible.

#include <minecraft:globals.glsl>
#include <minecraft:projection.glsl>
#include <minecraft:terrainglobals.glsl>

layout(std140) uniform OcclusionInfo {
    ivec2 DepthSize;
    ivec2 VisibilitySize;
    ivec2 CommandSize;
    int HiZLevels;
};

uniform sampler2D DepthSampler;
uniform sampler2D HiZSampler;

layout(location = 0) in ivec3 ChunkPosition;
layout(location = 1) in float ChunkVisibility;

layout(location = 0) flat out float visible;

const float MARGIN = 1.0;
// Boxes reaching this close to the camera plane are drawn
const float NEAR_W = 0.1;
// Depth pixels or pyramid texels read per axis at most, by TILES points per axis
const int SPAN = 16;
const int TILES = 4;

// Whether any of the texels from a to b (inclusive) this point reads is as far as the box's nearest point
bool anyAsFar(sampler2D depth, int level, ivec2 a, ivec2 b, float nearest) {
    ivec2 size = b - a + 1;
    ivec2 tile = ivec2(gl_VertexIndex % TILES, gl_VertexIndex / TILES);
    ivec2 from = a + size * tile / TILES;
    ivec2 to = a + size * (tile + 1) / TILES - 1;
    for (int y = from.y; y <= to.y; y++) {
        for (int x = from.x; x <= to.x; x++) {
            if (texelFetch(depth, ivec2(x, y), level).r <= nearest) {
                return true;
            }
        }
    }
    return false;
}

bool isVisible() {
    vec3 low = vec3(ChunkPosition - CameraBlockPos) + CameraOffset - MARGIN;
    vec3 high = low + 16.0 + 2.0 * MARGIN;
    mat4 viewProjection = ProjMat * ModelViewMat;
    vec2 minNdc = vec2(1e30);
    vec2 maxNdc = vec2(-1e30);
    // Reversed-Z: the box's nearest point has the largest depth
    float nearest = 0.0;
    for (int i = 0; i < 8; i++) {
        vec3 corner = vec3((i & 1) != 0 ? high.x : low.x, (i & 2) != 0 ? high.y : low.y, (i & 4) != 0 ? high.z : low.z);
        vec4 clip = viewProjection * vec4(corner, 1.0);
        if (clip.w < NEAR_W) {
            return true;
        }
        vec3 ndc = clip.xyz / clip.w;
        minNdc = min(minNdc, ndc.xy);
        maxNdc = max(maxNdc, ndc.xy);
        nearest = max(nearest, ndc.z);
    }
    if (any(lessThan(maxNdc, vec2(-1.0))) || any(greaterThan(minNdc, vec2(1.0)))) {
        return false;
    }
    if (HiZLevels == 0) {
        return true;
    }

    // Every depth pixel the box's screen rectangle touches: read directly while there are few
    ivec2 lastPixel = DepthSize - 1;
    ivec2 fromPixel = clamp(ivec2(floor((minNdc * 0.5 + 0.5) * vec2(DepthSize))), ivec2(0), lastPixel);
    ivec2 toPixel = clamp(ivec2(floor((maxNdc * 0.5 + 0.5) * vec2(DepthSize))), ivec2(0), lastPixel);
    if (all(lessThan(toPixel - fromPixel, ivec2(SPAN)))) {
        return anyAsFar(DepthSampler, 0, fromPixel, toPixel, nearest);
    }

    // Else the pyramid's texels holding them (4x4 pixels in mip 0, 4x4 mip 0 texels in mip 2, ...: only even mips
    // are written), at the finest level where they are at most SPAN x SPAN
    ivec2 from = fromPixel >> 2;
    ivec2 to = toPixel >> 2;
    int level = 0;
    while (level + 2 < HiZLevels && any(greaterThan((to >> level) - (from >> level), ivec2(SPAN - 1)))) {
        level += 2;
    }
    if (any(greaterThan((to >> level) - (from >> level), ivec2(SPAN - 1)))) {
        return true;
    }
    ivec2 lastTexel = textureSize(HiZSampler, level) - 1;
    return anyAsFar(HiZSampler, level, min(from >> level, lastTexel), min(to >> level, lastTexel), nearest);
}

void main() {
    int index = gl_InstanceIndex;
    ivec2 texel = ivec2(index % VisibilitySize.x, index / VisibilitySize.x);
    gl_Position = vec4((vec2(texel) + 0.5) / vec2(VisibilitySize) * 2.0 - 1.0, 0.0, 1.0);
    gl_PointSize = 1.0;
    visible = isVisible() ? 1.0 : 0.0;
}

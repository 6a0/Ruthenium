#version 460
// Ruthenium: terrain. One vertex per corner of a quad, packed in 16 bytes; where the
// section is and how the camera looks come from the frame's data.
//
// As vanilla's terrain shader: positions are taken relative to the camera's block in
// whole numbers before the camera's fraction of a block is taken off, and the light map
// is sampled here, per vertex, so that the lit colour is what is interpolated.

#include "push.glsl"
#include "textures.glsl"
#include "world.glsl"

layout(push_constant) uniform Push {
    uint world;    // word offset of the frame's WORLD record
    uint matrix;   // of its WORLD_MATRIX record
    uint sections; // of the first SECTION record
    uint enabled;  // 0: draw nothing (the start-up warm-up, with zeroed push constants)
} pc;

// xyz: the position in 1/2048 of a block, from 8 blocks before the section's corner.
// w: block light in the low byte and sky light in the high byte, each 0 to 240.
layout(location = 0) in uvec4 a_position_light;
layout(location = 1) in vec4 a_color;
// In eighths of a texel of the atlas.
layout(location = 2) in uvec2 a_uv;

layout(location = 0) out vec4 v_color;
layout(location = 1) out vec2 v_uv;

// The solid and the cutout pipeline draw faces that lie in one plane (a grass block's
// side and its overlay): their depths have to come out the same to the bit.
invariant gl_Position;

void main() {
    if (pc.enabled == 0u) {
        gl_Position = vec4(2.0, 2.0, 0.0, 1.0);
        v_color = vec4(0.0);
        v_uv = vec2(0.0);
        return;
    }
    uint section = pc.sections + uint(gl_InstanceIndex) * SECTION_WORDS;
    vec3 local = vec3(a_position_light.xyz) * (1.0 / 2048.0) - 8.0;
    vec3 position = local + vec3(section_origin(section)) + world_camera_offset(pc.world);
    gl_Position = world_matrix_view_projection(pc.matrix) * vec4(position, 1.0);

    // Vanilla's sample_lightmap: sixteenths of a level, at texel centres.
    vec2 light = vec2(a_position_light.w & 0xFFu, a_position_light.w >> 8u);
    vec2 light_uv = clamp(light / 256.0 + 0.5 / 16.0, vec2(0.5 / 16.0), vec2(15.5 / 16.0));
    v_color = a_color * textureLod(
        sampler2D(u_textures[nonuniformEXT(world_lightmap(pc.world))], u_samplers[SAMPLER_LINEAR_CLAMP]),
        light_uv, 0.0);
    v_uv = vec2(a_uv) / (8.0 * world_atlas_size(pc.world));
}

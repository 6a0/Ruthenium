#version 460
// Ruthenium: models made of parts (block entities). The vertices are a part's, in its own
// space; where the part is, its light, colour and texture come from its record in the
// frame's data.
//
// As vanilla's entity shader: two lights shade a face by its normal, and the light map is
// sampled here, per vertex. What is enchanted has its glint's picture laid over it, moved
// and turned by the glint's record. Pipelines that draw both sides of a face shade its back by
// the opposite of its normal.

#include "push.glsl"
#include "textures.glsl"
#include "world.glsl"

layout(push_constant) uniform Push {
    uint world;  // word offset of the frame's WORLD record
    uint matrix; // of its WORLD_MATRIX record
    uint parts;  // of the first MODEL_PART record
    uint lights; // 0: draw nothing (the start-up warm-up); 1: the usual lights; 2: the Nether's
} pc;

layout(location = 0) in vec3 a_position;
layout(location = 1) in vec2 a_uv;
layout(location = 2) in vec4 a_normal;

layout(location = 0) out vec4 v_front;
layout(location = 1) out vec4 v_back;
layout(location = 2) out vec2 v_uv;
layout(location = 3) out float v_spherical;
layout(location = 4) out float v_cylindrical;
layout(location = 5) flat out uint v_texture;
layout(location = 6) flat out vec3 v_added;
layout(location = 7) out vec2 v_glint_uv;
layout(location = 8) flat out uint v_glint_texture;
layout(location = 9) flat out float v_glint_strength;

// A banner's patterns lie in its cloth's plane: their depths have to come out the same.
invariant gl_Position;

const float AMBIENT = 0.4;
const float POWER = 0.6;
// How much of its own colour something hurt keeps; the rest is red.
const float HURT_KEEPS = 178.0 / 255.0;

float shade(vec2 light) {
    vec2 lit = max(light, vec2(0.0));
    return min(1.0, (lit.x + lit.y) * POWER + AMBIENT);
}

void main() {
    if (pc.lights == 0u) {
        gl_Position = vec4(2.0, 2.0, 0.0, 1.0);
        v_front = vec4(0.0);
        v_back = vec4(0.0);
        v_uv = vec2(0.0);
        v_spherical = 0.0;
        v_cylindrical = 0.0;
        v_texture = 0u;
        v_added = vec3(0.0);
        v_glint_uv = vec2(0.0);
        v_glint_texture = 0u;
        v_glint_strength = 0.0;
        return;
    }
    uint part = pc.parts + uint(gl_InstanceIndex) * MODEL_PART_WORDS;
    vec4 rows[3] = vec4[](model_part_row_x(part), model_part_row_y(part), model_part_row_z(part));
    vec4 place = vec4(a_position, 1.0);
    vec3 position = vec3(dot(rows[0], place), dot(rows[1], place), dot(rows[2], place));
    gl_Position = world_matrix_view_projection(pc.matrix) * vec4(position, 1.0);

    // Parts are scaled alike along every axis, so a normal turns as a direction does.
    vec3 n = normalize(vec3(
        dot(rows[0].xyz, a_normal.xyz), dot(rows[1].xyz, a_normal.xyz), dot(rows[2].xyz, a_normal.xyz)));
    vec3 first = normalize(vec3(0.2, 1.0, -0.7));
    vec3 second = normalize(vec3(-0.2, pc.lights == 2u ? -1.0 : 1.0, 0.7));
    vec2 facing = vec2(dot(first, n), dot(second, n));

    uint packed_light = model_part_light(part);
    vec2 light = vec2(packed_light & 0xFFu, packed_light >> 8u & 0xFFu);
    vec2 light_uv = clamp(light / 256.0 + 0.5 / 16.0, vec2(0.5 / 16.0), vec2(15.5 / 16.0));
    vec4 lit = textureLod(
        sampler2D(u_textures[nonuniformEXT(world_lightmap(pc.world))], u_samplers[SAMPLER_LINEAR_CLAMP]),
        light_uv, 0.0);
    vec4 color = unpackUnorm4x8(model_part_colour(part)) * lit;
    uint effects = model_part_overlay(part);
    bool hurt = (effects & 1u) != 0u;
    uint glint = effects >> 8u;
    if (glint != 0u) {
        vec3 uv = vec3(a_uv, 1.0);
        v_glint_uv = vec2(dot(glint_row_u(glint), uv), dot(glint_row_v(glint), uv));
        v_glint_texture = glint_texture(glint);
        v_glint_strength = glint_strength(glint);
    } else {
        v_glint_uv = vec2(0.0);
        v_glint_texture = 0u;
        v_glint_strength = 0.0;
    }
    color.rgb *= hurt ? HURT_KEEPS : 1.0;
    v_added = hurt ? vec3(1.0 - HURT_KEEPS, 0.0, 0.0) * lit.rgb : vec3(0.0);
    v_front = vec4(color.rgb * shade(facing), color.a);
    v_back = vec4(color.rgb * shade(-facing), color.a);
    v_uv = a_uv;
    v_spherical = length(position);
    v_cylindrical = max(length(position.xz), abs(position.y));
    v_texture = model_part_texture(part);
}

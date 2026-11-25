#version 460
// Ruthenium: rain and snow, as vanilla's particle shader colours them
// (`core/particle.fsh`): the picture times the colour, nothing below a tenth of alpha,
// in the world's fog.

#include "push.glsl"
#include "textures.glsl"
#include "fog.glsl"

layout(push_constant) uniform Push {
    uint matrix;
    uint world;
    uint first;
    uint enabled;
    vec4 fog_color;
    vec2 fog_environmental;
    vec2 fog_render_distance;
    uint texture_index;
} pc;

layout(location = 0) in vec2 v_uv;
layout(location = 1) in vec4 v_color;
layout(location = 2) in float v_spherical;
layout(location = 3) in float v_cylindrical;
layout(location = 0) out vec4 o_color;

void main() {
    vec4 color = sample_texture(pc.texture_index, SAMPLER_NEAREST_REPEAT, v_uv) * v_color;
    if (color.a < 0.1) {
        discard;
    }
    float amount = max(
        fog_amount(v_spherical, pc.fog_environmental),
        fog_amount(v_cylindrical, pc.fog_render_distance));
    o_color = vec4(mix(color.rgb, pc.fog_color.rgb, amount * pc.fog_color.a), color.a);
}

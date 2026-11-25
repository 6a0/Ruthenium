#version 460
// Ruthenium: lines in one colour, in the world's fog.

#include "push.glsl"
#include "fog.glsl"

layout(push_constant) uniform Push {
    uint world;
    uint matrix;
    uint lines;
    uint enabled;
    // As the terrain's.
    vec4 fog_color;
    vec2 fog_environmental;
    vec2 fog_render_distance;
    vec2 screen;
} pc;

layout(location = 0) in vec4 v_color;
layout(location = 1) in float v_spherical;
layout(location = 2) in float v_cylindrical;
layout(location = 0) out vec4 o_color;

void main() {
    float amount = max(
        fog_amount(v_spherical, pc.fog_environmental),
        fog_amount(v_cylindrical, pc.fog_render_distance));
    o_color = vec4(mix(v_color.rgb, pc.fog_color.rgb, amount * pc.fog_color.a), v_color.a);
}

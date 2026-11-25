#version 460
// Ruthenium: the clouds' colour, fading out with distance as vanilla's (`core/clouds.fsh`).

#include "push.glsl"
#include "fog.glsl"

layout(push_constant) uniform Push {
    uint matrix;
    uint enabled;
    uint pad0;
    uint pad1;
    vec4 color;
    vec3 offset;
    float fog_end;
} pc;

layout(location = 0) in float v_distance;
layout(location = 1) in vec4 v_color;
layout(location = 0) out vec4 o_color;

void main() {
    vec4 color = v_color;
    color.a *= 1.0 - fog_amount(v_distance, vec2(0.0, pc.fog_end));
    o_color = color;
}

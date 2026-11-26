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
    // One over where the clouds have faded out, as fog.glsl's scale of a fog from the
    // camera (atmosphere.rs, fog_line; its bias is 0).
    float fog_scale;
} pc;

layout(location = 0) in float v_distance;
layout(location = 1) in vec4 v_color;
layout(location = 0) out vec4 o_color;

void main() {
    vec4 color = v_color;
    color.a *= 1.0 - fog_amount(v_distance, vec2(pc.fog_scale, 0.0));
    o_color = color;
}

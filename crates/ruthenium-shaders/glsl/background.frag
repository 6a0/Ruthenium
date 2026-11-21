#version 460
// Ruthenium: vertical gradient behind the menus (placeholder for the starfield).

#include "push.glsl"

layout(location = 0) in vec2 v_uv;
layout(location = 0) out vec4 o_color;

layout(push_constant) uniform Push {
    vec4 top;
    vec4 bottom;
} pc;

void main() {
    o_color = mix(pc.top, pc.bottom, v_uv.y);
}

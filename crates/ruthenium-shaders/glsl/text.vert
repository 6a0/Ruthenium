#version 460
// Ruthenium: text of the modern menus, one quad per glyph from the frame's data, drawn
// from the glyph distance-field atlas. Rectangles are in window pixels.

#include "push.glsl"

struct Glyph {
    vec4 rect;   // x0, y0, x1, y1 in pixels
    vec4 uv;     // u0, v0, u1, v1 in the atlas
    vec4 color;  // straight-alpha RGBA
    vec4 params; // x: the field's range in screen pixels; y, z: the clip's top and bottom fades in pixels; w: opacity
    vec4 clip;   // x0, y0, x1, y1 in pixels
};
#define FRAME_ELEMENT Glyph
#include "frame.glsl"

layout(push_constant) uniform Push {
    vec2 screen;
    uint atlas;
    uint first;
} pc;

layout(location = 0) out vec2 v_uv;
layout(location = 1) out vec4 v_color;
layout(location = 2) out float v_range;
layout(location = 3) flat out vec4 v_clip;
layout(location = 4) flat out vec3 v_fade; // top and bottom fades, opacity

void main() {
    // The warm-up draws every pipeline with zeroed push constants: draw nothing then.
    if (pc.screen.x == 0.0) {
        gl_Position = vec4(2.0, 2.0, 0.0, 1.0);
        v_uv = vec2(0.0);
        v_color = vec4(0.0);
        v_range = 1.0;
        v_clip = vec4(0.0);
        v_fade = vec3(0.0);
        return;
    }
    Glyph g = frame_data[pc.first + gl_InstanceIndex];
    vec2 corner = vec2((0x32 >> gl_VertexIndex) & 1, (0x2C >> gl_VertexIndex) & 1);
    vec2 pixel = mix(g.rect.xy, g.rect.zw, corner);
    v_uv = mix(g.uv.xy, g.uv.zw, corner);
    v_color = g.color;
    v_range = g.params.x;
    v_clip = g.clip;
    v_fade = g.params.yzw;
    gl_Position = vec4(pixel / pc.screen * 2.0 - 1.0, 0.0, 1.0);
}

#version 460
// Ruthenium: GUI quads (ruthenium-ui's draw list), six vertices each, one instance per
// quad, pulled from the frame's data. Positions are in GUI units, scaled to pixels by
// the integer GUI scale.

#include "push.glsl"

struct Quad {
    vec4 rect;    // x0, y0, x1, y1 in GUI units from the top left
    vec4 uv;      // u0, v0, u1, v1
    uint color;   // 0xAARRGGBB
    uint texture; // texture table index, or GUI_NO_TEXTURE
    uint smp;     // how the texture is read: a SAMPLER_* index (0: nearest, clamped)
    uint slant;   // how far the top and the bottom edge lie to the right: two halves
};
#define FRAME_ELEMENT Quad
#include "frame.glsl"

layout(push_constant) uniform Push {
    vec2 screen; // target size in pixels
    float scale; // GUI scale
    uint first;  // index of the first quad
} pc;

layout(location = 0) out vec2 v_uv;
layout(location = 1) out vec4 v_color;
layout(location = 2) flat out uint v_texture;
layout(location = 3) flat out uint v_sampler;

void main() {
    // The start-up warm-up draws every pipeline with zeroed push constants and unwritten
    // frame data: draw nothing then, and sample no texture.
    if (pc.scale == 0.0) {
        gl_Position = vec4(2.0, 2.0, 0.0, 1.0);
        v_uv = vec2(0.0);
        v_color = vec4(0.0);
        v_texture = 0xFFFFFFFFu;
        v_sampler = 0u;
        return;
    }
    Quad q = frame_data[pc.first + gl_InstanceIndex];
    // Corners of two triangles: (0,0) (1,0) (0,1), (0,1) (1,0) (1,1).
    vec2 corner = vec2((0x32 >> gl_VertexIndex) & 1, (0x2C >> gl_VertexIndex) & 1);
    vec2 place = mix(q.rect.xy, q.rect.zw, corner);
    vec2 slant = unpackHalf2x16(q.slant);
    place.x += mix(slant.x, slant.y, corner.y);
    vec2 pixel = place * pc.scale;
    v_uv = mix(q.uv.xy, q.uv.zw, corner);
    // unpackUnorm4x8 reads the lowest byte first: B, G, R, A.
    v_color = unpackUnorm4x8(q.color).zyxw;
    v_texture = q.texture;
    v_sampler = q.smp;
    gl_Position = vec4(pixel / pc.screen * 2.0 - 1.0, 0.0, 1.0);
}

#version 460
// Ruthenium: solid rectangles in window pixels (the development overlay), six vertices
// each, one instance per rectangle, pulled from the frame's data.

#include "push.glsl"

struct Rect {
    vec4 rect;  // x, y, width, height in pixels from the top left
    vec4 color; // display-encoded RGBA, straight alpha
};
#define FRAME_ELEMENT Rect
#include "frame.glsl"

layout(push_constant) uniform Push {
    vec2 screen; // target size in pixels
    uint first;  // index of the first rectangle
} pc;

layout(location = 0) out vec4 v_color;

void main() {
    Rect r = frame_data[pc.first + gl_InstanceIndex];
    // Corners of two triangles: (0,0) (1,0) (0,1), (0,1) (1,0) (1,1).
    vec2 corner = vec2((0x32 >> gl_VertexIndex) & 1, (0x2C >> gl_VertexIndex) & 1);
    vec2 pixel = r.rect.xy + corner * r.rect.zw;
    v_color = r.color;
    gl_Position = vec4(pixel / pc.screen * 2.0 - 1.0, 0.0, 1.0);
}

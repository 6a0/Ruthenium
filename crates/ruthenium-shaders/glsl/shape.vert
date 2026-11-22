#version 460
// Ruthenium: rounded shapes of the modern menus (ruthenium-ui's ShapeList), one instance
// each, pulled from the frame's data. Rectangles are in window pixels.

#include "push.glsl"

struct Shape {
    vec4 rect;   // x0, y0, x1, y1 in pixels from the top left
    vec4 fill;   // straight-alpha RGBA: the colour, the glass tint, or the image's tint
    vec4 params; // corner radius (px), kind (0 flat, 1 glass, 2 image), highlight, refraction (px)
    vec4 uv;     // image: u0, v0, u1, v1
    uvec4 tex;   // x: image texture index; y: ring width (float bits); z, w: clip fades (float bits)
    vec4 clip;   // x0, y0, x1, y1 in pixels
    vec4 extra;  // x: opacity
};
#define FRAME_ELEMENT Shape
#include "frame.glsl"

layout(push_constant) uniform Push {
    vec2 screen;
    uint scene;
    uint blurred;
    uint first;
    uint backdrop; // 1 when scene and blurred are drawn and may be sampled
} pc;

layout(location = 0) flat out uint v_index;

void main() {
    // The warm-up draws every pipeline with zeroed push constants: draw nothing then.
    if (pc.screen.x == 0.0) {
        gl_Position = vec4(2.0, 2.0, 0.0, 1.0);
        v_index = 0u;
        return;
    }
    uint i = pc.first + gl_InstanceIndex;
    Shape s = frame_data[i];
    vec2 corner = vec2((0x32 >> gl_VertexIndex) & 1, (0x2C >> gl_VertexIndex) & 1);
    // One pixel of margin for the anti-aliased edge.
    vec2 pixel = mix(s.rect.xy - 1.0, s.rect.zw + 1.0, corner);
    v_index = i;
    gl_Position = vec4(pixel / pc.screen * 2.0 - 1.0, 0.0, 1.0);
}

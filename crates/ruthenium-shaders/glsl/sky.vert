#version 460
// Ruthenium: the sky. Every draw is of something around the camera that only turns with
// it; where it goes on the screen comes from the draw's record in the frame's data.

#include "push.glsl"
#include "textures.glsl"
#include "world.glsl"

layout(push_constant) uniform Push {
    uint sky;     // word offset of the draw's SKY record
    uint enabled; // 0: draw nothing (the start-up warm-up, with zeroed push constants)
} pc;

layout(location = 0) in vec3 a_position;
layout(location = 1) in vec2 a_uv;
layout(location = 2) in vec4 a_color;

layout(location = 0) out vec4 v_color;
layout(location = 1) out vec2 v_uv;
layout(location = 2) out float v_spherical;
layout(location = 3) out float v_cylindrical;

void main() {
    if (pc.enabled == 0u) {
        gl_Position = vec4(2.0, 2.0, 0.0, 1.0);
        v_color = vec4(0.0);
        v_uv = vec2(0.0);
        v_spherical = 0.0;
        v_cylindrical = 0.0;
        return;
    }
    gl_Position = sky_transform(pc.sky) * vec4(a_position, 1.0);
    // As far away as anything can be: behind everything that wrote its depth, and
    // never cut off by a near or a far plane.
    gl_Position.z = 0.0;
    v_color = a_color;
    v_uv = a_uv;
    // The fog's distances are of the place in the draw's own space, as in vanilla.
    v_spherical = length(a_position);
    v_cylindrical = max(length(a_position.xz), abs(a_position.y));
}

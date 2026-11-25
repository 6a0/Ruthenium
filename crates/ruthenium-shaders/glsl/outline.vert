#version 460
// Ruthenium: the outline of the block the crosshair is on. A line is one instance of six
// vertices, with its ends, its width and its colour in its record in the frame's data.
//
// As vanilla's line shader: a line is a strip as wide on the screen wherever it is, laid
// out in the target's own space from its two ends, and drawn a little nearer than it is,
// so that it lies over the faces whose edges it follows.

#include "push.glsl"
#include "world.glsl"

layout(push_constant) uniform Push {
    uint world;   // word offset of the frame's WORLD record (not read here)
    uint matrix;  // of its WORLD_MATRIX record
    uint lines;   // of the first LINE record
    uint enabled; // 0: draw nothing (the start-up warm-up)
    // What the fragment stage reads.
    vec4 fog_color;
    vec2 fog_environmental;
    vec2 fog_render_distance;
    // The target's size in pixels.
    vec2 screen;
} pc;

layout(location = 0) out vec4 v_color;
layout(location = 1) out float v_spherical;
layout(location = 2) out float v_cylindrical;

// Lines are drawn this much nearer than they are: the game's layering of what lies on
// surfaces, and its line shader's own.
const float NEARER = (1.0 - 1.0 / 4096.0) * (1.0 - 1.0 / 256.0);

void main() {
    if (pc.enabled == 0u) {
        gl_Position = vec4(2.0, 2.0, 0.0, 1.0);
        v_color = vec4(0.0);
        v_spherical = 0.0;
        v_cylindrical = 0.0;
        return;
    }
    uint line = pc.lines + uint(gl_InstanceIndex) * LINE_WORDS;
    vec3 from = line_from(line);
    vec3 to = line_to(line);
    // One side of the first end, the other side of it, then the same of the second.
    const uint corners[6] = uint[](0u, 1u, 2u, 2u, 1u, 3u);
    uint corner = corners[gl_VertexIndex];
    vec3 position = corner >= 2u ? to : from;
    vec3 along = normalize(to - from);
    mat4 view_projection = world_matrix_view_projection(pc.matrix);
    vec4 here = view_projection * vec4(position * NEARER, 1.0);
    vec4 ahead = view_projection * vec4((position + along) * NEARER, 1.0);
    vec3 ndc = here.xyz / here.w;
    vec2 direction = normalize((ahead.xy / ahead.w - ndc.xy) * pc.screen);
    vec2 aside = vec2(-direction.y, direction.x) * line_width(line) / pc.screen;
    if (aside.x < 0.0) {
        aside = -aside;
    }
    if ((corner & 1u) != 0u) {
        aside = -aside;
    }
    gl_Position = vec4((ndc + vec3(aside, 0.0)) * here.w, here.w);

    v_color = unpackUnorm4x8(line_colour(line));
    v_spherical = length(position);
    v_cylindrical = max(length(position.xz), abs(position.y));
}

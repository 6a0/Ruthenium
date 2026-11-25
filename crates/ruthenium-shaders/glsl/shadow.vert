#version 460
// Ruthenium: the shadows under entities. A piece is one instance of six vertices: the part
// of a shadow that lies on one block, with its place and darkness in its record in the
// frame's data.
//
// As vanilla's shadow shader: the picture is laid over the whole shadow, each piece
// showing its part of it, in no light of its own.

#include "push.glsl"
#include "textures.glsl"
#include "world.glsl"

layout(push_constant) uniform Push {
    uint world;   // word offset of the frame's WORLD record
    uint matrix;  // of its WORLD_MATRIX record
    uint shadows; // of the first SHADOW record
    uint picture; // the shadow's picture in the texture table; 0: draw nothing (the start-up warm-up)
} pc;

// What model_frag.glsl takes.
layout(location = 0) out vec4 v_front;
layout(location = 1) out vec4 v_back;
layout(location = 2) out vec2 v_uv;
layout(location = 3) out float v_spherical;
layout(location = 4) out float v_cylindrical;
layout(location = 5) flat out uint v_texture;
layout(location = 6) flat out vec3 v_added;
layout(location = 7) out vec2 v_glint_uv;
layout(location = 8) flat out uint v_glint_texture;
layout(location = 9) flat out float v_glint_strength;

// Shadows are drawn this much nearer than the ground they lie on.
const float NEARER = 1.0 - 1.0 / 4096.0;

void main() {
    v_added = vec3(0.0);
    v_glint_uv = vec2(0.0);
    v_glint_texture = 0u;
    v_glint_strength = 0.0;
    v_texture = pc.picture;
    if (pc.picture == 0u) {
        gl_Position = vec4(2.0, 2.0, 0.0, 1.0);
        v_front = vec4(0.0);
        v_back = vec4(0.0);
        v_uv = vec2(0.0);
        v_spherical = 0.0;
        v_cylindrical = 0.0;
        return;
    }
    uint shadow = pc.shadows + uint(gl_InstanceIndex) * SHADOW_WORDS;
    // North-west, south-west, south-east, north-east: seen from above.
    const uint corners[6] = uint[](0u, 1u, 2u, 2u, 3u, 0u);
    uint corner = corners[gl_VertexIndex];
    bool east = corner >= 2u;
    bool south = corner == 1u || corner == 2u;
    vec4 box = shadow_box(shadow);
    vec2 at = vec2(east ? box.z : box.x, south ? box.w : box.y);
    vec3 position = shadow_origin(shadow) + vec3(at.x, shadow_height(shadow), at.y);
    gl_Position = world_matrix_view_projection(pc.matrix) * vec4(position * NEARER, 1.0);

    v_uv = -at / 2.0 / shadow_radius(shadow) + 0.5;
    v_front = vec4(1.0, 1.0, 1.0, shadow_alpha(shadow));
    v_back = v_front;
    v_spherical = length(position);
    v_cylindrical = max(length(position.xz), abs(position.y));
}

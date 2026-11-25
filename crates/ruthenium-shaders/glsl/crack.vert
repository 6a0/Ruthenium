#version 460
// Ruthenium: the cracks in a block being broken. A quad of the block's model is one
// instance of six vertices, with its corners and where the cracks' picture is read at
// each in its record in the frame's data.
//
// As vanilla's crumbling shader: the picture alone, in no light, in the world's fog.

#include "push.glsl"
#include "textures.glsl"
#include "world.glsl"

layout(push_constant) uniform Push {
    uint world;   // word offset of the frame's WORLD record
    uint matrix;  // of its WORLD_MATRIX record
    uint cracks;  // of the first CRACK record
    uint picture; // the stage's picture in the texture table; 0: draw nothing (the start-up warm-up)
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

// The cracks lie in the planes of the faces they are on: their depths have to come out
// as the terrain's, whose own are invariant.
invariant gl_Position;

void main() {
    v_added = vec3(0.0);
    v_glint_uv = vec2(0.0);
    v_glint_texture = 0u;
    v_glint_strength = 0.0;
    v_texture = pc.picture;
    v_front = vec4(1.0);
    v_back = vec4(1.0);
    if (pc.picture == 0u) {
        gl_Position = vec4(2.0, 2.0, 0.0, 1.0);
        v_uv = vec2(0.0);
        v_spherical = 0.0;
        v_cylindrical = 0.0;
        return;
    }
    uint crack = pc.cracks + uint(gl_InstanceIndex) * CRACK_WORDS;
    const uint corners[6] = uint[](0u, 1u, 2u, 0u, 2u, 3u);
    uint corner = corners[gl_VertexIndex];
    vec3 position;
    if (corner == 0u) {
        position = crack_a(crack);
        v_uv = crack_uv_a(crack);
    } else if (corner == 1u) {
        position = crack_b(crack);
        v_uv = crack_uv_b(crack);
    } else if (corner == 2u) {
        position = crack_c(crack);
        v_uv = crack_uv_c(crack);
    } else {
        position = crack_d(crack);
        v_uv = crack_uv_d(crack);
    }
    // As the terrain's vertices: from the camera's block first, then the camera's place in
    // it, so that the cracks' depths are those of the faces they lie on.
    position += world_camera_offset(pc.world);
    gl_Position = world_matrix_view_projection(pc.matrix) * vec4(position, 1.0);
    v_spherical = length(position);
    v_cylindrical = max(length(position.xz), abs(position.y));
}

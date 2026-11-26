#version 460
// Ruthenium: the clouds. A face is one instance of six vertices, its cell, direction and
// flags in one word; its corners and colour are worked out here as vanilla's clouds
// shader does (`core/clouds.vsh`).

#include "push.glsl"
#include "world.glsl"

layout(push_constant) uniform Push {
    uint matrix;  // word offset of the frame's WORLD_MATRIX record
    uint enabled; // 0: draw nothing (the start-up warm-up)
    uint pad0;
    uint pad1;
    // The clouds' colour, as the air gives it.
    vec4 color;
    // Where the camera's cell starts, in blocks from the camera: the layer's bottom in y.
    vec3 offset;
    // The fragment stage's fade with distance.
    float fog_scale;
} pc;

layout(location = 0) in uint a_face;

layout(location = 0) out float v_distance;
layout(location = 1) out vec4 v_color;

const uint FACE_INSIDE = 1u << 23;
const uint FACE_TOP_COLOUR = 1u << 24;
const vec3 CELL_SIZE = vec3(12.0, 4.0, 12.0);

// The four corners of each direction's face, down, up, north, south, west and east, one
// bit of a coordinate per corner.
const int PACKED_X = 0xF03CC3;
const int PACKED_Y = 0x6666F0;
const int PACKED_Z = 0xC3F066;

vec3 corner(int index) {
    return vec3(float((PACKED_X >> index) & 1), float((PACKED_Y >> index) & 1), float((PACKED_Z >> index) & 1));
}

// How light each direction's faces are.
const vec4 FACE_COLORS[6] = vec4[](
    vec4(0.7, 0.7, 0.7, 1.0),
    vec4(1.0, 1.0, 1.0, 1.0),
    vec4(0.8, 0.8, 0.8, 1.0),
    vec4(0.8, 0.8, 0.8, 1.0),
    vec4(0.9, 0.9, 0.9, 1.0),
    vec4(0.9, 0.9, 0.9, 1.0)
);

void main() {
    if (pc.enabled == 0u) {
        gl_Position = vec4(2.0, 2.0, 0.0, 1.0);
        v_distance = 0.0;
        v_color = vec4(0.0);
        return;
    }
    // A quad's two triangles, as the game's quads are drawn.
    const int corners[6] = int[](0, 1, 2, 2, 3, 0);
    int quad_vertex = corners[gl_VertexIndex];
    int x = int(a_face & 0x3FFu) - 512;
    int z = int((a_face >> 10) & 0x3FFu) - 512;
    int direction = int((a_face >> 20) & 7u);
    bool inside = (a_face & FACE_INSIDE) != 0u;
    bool top_colour = (a_face & FACE_TOP_COLOUR) != 0u;
    vec3 face_vertex = corner(direction * 4 + (inside ? 3 - quad_vertex : quad_vertex));
    vec3 pos = (face_vertex * CELL_SIZE) + (vec3(x, 0, z) * CELL_SIZE) + pc.offset;
    gl_Position = world_matrix_view_projection(pc.matrix) * vec4(pos, 1.0);
    v_distance = length(pos);
    v_color = (top_colour ? FACE_COLORS[1] : FACE_COLORS[direction]) * pc.color;
}

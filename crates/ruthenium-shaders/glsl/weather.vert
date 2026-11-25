#version 460
// Ruthenium: rain and snow. A column is one instance of six vertices, its quad, picture
// coordinates, alpha and light in its record; as vanilla's particle shader colours it
// (`core/particle.vsh`): white at the column's alpha, times the light map.

#include "push.glsl"
#include "textures.glsl"
#include "world.glsl"

layout(push_constant) uniform Push {
    uint matrix;  // word offset of the frame's WORLD_MATRIX record
    uint world;   // of its WORLD record (the light map)
    uint first;   // of the first WEATHER record
    uint enabled; // 0: draw nothing (the start-up warm-up)
    // What the fragment stage reads.
    vec4 fog_color;
    vec2 fog_environmental;
    vec2 fog_render_distance;
    uint texture_index;
} pc;

layout(location = 0) out vec2 v_uv;
layout(location = 1) out vec4 v_color;
layout(location = 2) out float v_spherical;
layout(location = 3) out float v_cylindrical;

void main() {
    if (pc.enabled == 0u) {
        gl_Position = vec4(2.0, 2.0, 0.0, 1.0);
        v_uv = vec2(0.0);
        v_color = vec4(0.0);
        v_spherical = 0.0;
        v_cylindrical = 0.0;
        return;
    }
    uint at = pc.first + uint(gl_InstanceIndex) * WEATHER_WORDS;
    // The quad's corners: the first side's top, the other's top, the other's bottom,
    // the first's bottom; two triangles as the game's quads.
    const uint corners[6] = uint[](0u, 1u, 2u, 2u, 3u, 0u);
    uint corner = corners[gl_VertexIndex];
    bool other = corner == 1u || corner == 2u;
    bool bottom = corner >= 2u;
    vec2 side = other ? weather_end(at) : weather_start(at);
    vec2 y = weather_y(at);
    vec3 position = vec3(side.x, bottom ? y.x : y.y, side.y);
    vec2 u = weather_u(at);
    vec2 v = weather_v(at);
    v_uv = vec2(other ? u.y : u.x, bottom ? v.y : v.x);
    gl_Position = world_matrix_view_projection(pc.matrix) * vec4(position, 1.0);

    // Vanilla's sample_lightmap: sixteenths of a level, at texel centres.
    uint packed = weather_light(at);
    vec2 light = vec2(packed & 0xFFFFu, packed >> 16u);
    vec2 light_uv = clamp(light / 256.0 + 0.5 / 16.0, vec2(0.5 / 16.0), vec2(15.5 / 16.0));
    vec4 lit = textureLod(
        sampler2D(u_textures[nonuniformEXT(world_lightmap(pc.world))], u_samplers[SAMPLER_LINEAR_CLAMP]),
        light_uv, 0.0);
    v_color = vec4(1.0, 1.0, 1.0, float(weather_alpha(at)) / 255.0) * lit;
    v_spherical = length(position);
    v_cylindrical = max(length(position.xz), abs(position.y));
}

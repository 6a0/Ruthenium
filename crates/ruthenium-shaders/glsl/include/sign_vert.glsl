// Ruthenium: the vertex stage of the text on signs. A glyph is one instance of six
// vertices; its box, colour and place in a glyph picture come from its record in the
// frame's data, and where the sign is, its light and its outline's colour from the sign's.
// Before including it a shader defines OUTLINE to draw what surrounds glowing text.
//
// As vanilla's text shader: the light map is sampled here, per vertex, and nothing is
// shaded by which way it faces.

#include "push.glsl"
#include "textures.glsl"
#include "world.glsl"

layout(push_constant) uniform Push {
    uint world;  // word offset of the frame's WORLD record
    uint matrix; // of its WORLD_MATRIX record
    uint glyphs; // of the first GLYPH record
    uint signs;  // of the first SIGN record; 0: draw nothing (the start-up warm-up)
} pc;

// What model_frag.glsl takes.
layout(location = 0) out vec4 v_front;
layout(location = 1) out vec4 v_back;
layout(location = 2) out vec2 v_uv;
layout(location = 3) out float v_spherical;
layout(location = 4) out float v_cylindrical;
layout(location = 5) flat out uint v_texture;
layout(location = 6) flat out vec3 v_added;

// How far inside the edge of its texels a glyph's picture is sampled, in texels.
const float INSIDE = 0.01;

void main() {
    v_added = vec3(0.0);
    if (pc.signs == 0u) {
        gl_Position = vec4(2.0, 2.0, 0.0, 1.0);
        v_front = vec4(0.0);
        v_back = vec4(0.0);
        v_uv = vec2(0.0);
        v_spherical = 0.0;
        v_cylindrical = 0.0;
        v_texture = 0u;
        return;
    }
#ifdef OUTLINE
    // Each glyph eight times, around itself.
    const ivec2 around[8] = ivec2[](
        ivec2(-1, -1), ivec2(-1, 0), ivec2(-1, 1), ivec2(0, -1),
        ivec2(0, 1), ivec2(1, -1), ivec2(1, 0), ivec2(1, 1));
    uint number = uint(gl_InstanceIndex) / 8u;
#else
    uint number = uint(gl_InstanceIndex);
#endif
    uint glyph = pc.glyphs + number * GLYPH_WORDS;
    uint sign = pc.signs + glyph_sign(glyph) * SIGN_WORDS;
    // Down the left side, along the bottom, up the right: y runs down the sign.
    const uint corners[6] = uint[](0u, 1u, 2u, 2u, 3u, 0u);
    uint corner = corners[gl_VertexIndex];
    bool right = corner >= 2u;
    bool bottom = corner == 1u || corner == 2u;

    vec4 box = glyph_box(glyph);
    vec2 slant = glyph_slant(glyph);
    vec4 place = vec4(
        (right ? box.z : box.x) + (bottom ? slant.y : slant.x),
        bottom ? box.w : box.y,
        glyph_nearer(glyph),
        1.0);
    uint colour = glyph_colour(glyph);
#ifdef OUTLINE
    // Bars have none: nothing is left of them.
    float distance = glyph_around(glyph);
    place.xy = distance == 0.0 ? vec2(0.0) : place.xy + vec2(around[gl_InstanceIndex % 8]) * distance;
    colour = sign_outline(sign);
#endif
    vec3 position = vec3(
        dot(sign_row_x(sign), place), dot(sign_row_y(sign), place), dot(sign_row_z(sign), place));
    gl_Position = world_matrix_view_projection(pc.matrix) * vec4(position, 1.0);

    uint texture_index = glyph_texture(glyph);
    uint texels = glyph_texels(glyph);
    uint size = glyph_size(glyph);
    vec2 from = vec2(texels & 0xFFFFu, texels >> 16u);
    vec2 to = from + vec2(size & 0xFFFFu, size >> 16u);
    vec2 page = vec2(textureSize(
        sampler2D(u_textures[nonuniformEXT(texture_index)], u_samplers[SAMPLER_NEAREST_CLAMP]), 0));
    v_uv = vec2(right ? to.x - INSIDE : from.x + INSIDE, bottom ? to.y - INSIDE : from.y + INSIDE) / page;

    uint packed_light = sign_light(sign);
    vec2 light = vec2(packed_light & 0xFFu, packed_light >> 8u & 0xFFu);
    vec2 light_uv = clamp(light / 256.0 + 0.5 / 16.0, vec2(0.5 / 16.0), vec2(15.5 / 16.0));
    v_front = unpackUnorm4x8(colour) * textureLod(
        sampler2D(u_textures[nonuniformEXT(world_lightmap(pc.world))], u_samplers[SAMPLER_LINEAR_CLAMP]),
        light_uv, 0.0);
    v_back = v_front;
    v_spherical = length(position);
    v_cylindrical = max(length(position.xz), abs(position.y));
    v_texture = texture_index;
}

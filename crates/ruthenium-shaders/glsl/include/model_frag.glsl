// Ruthenium: the fragment stage of models. Before including it a shader defines
// ALPHA_CUTOUT to leave out texels with holes, PER_FACE if it draws both sides of a
// face, each in its own light, and SAMPLE_REPEAT if its picture repeats.

#include "push.glsl"
#include "textures.glsl"
#include "fog.glsl"

layout(push_constant) uniform Push {
    uint world;
    uint matrix;
    uint parts;
    uint lights;
    // As the terrain's.
    vec4 fog_color;
    vec2 fog_environmental;
    vec2 fog_render_distance;
} pc;

layout(location = 0) in vec4 v_front;
layout(location = 1) in vec4 v_back;
layout(location = 2) in vec2 v_uv;
layout(location = 3) in float v_spherical;
layout(location = 4) in float v_cylindrical;
layout(location = 5) flat in uint v_texture;
layout(location = 6) flat in vec3 v_added;
layout(location = 7) in vec2 v_glint_uv;
layout(location = 8) flat in uint v_glint_texture;
layout(location = 9) flat in float v_glint_strength;
layout(location = 0) out vec4 o_color;

void main() {
#ifdef SAMPLE_REPEAT
    vec4 color = sample_texture(v_texture, SAMPLER_NEAREST_REPEAT, v_uv);
#else
    vec4 color = sample_texture(v_texture, SAMPLER_NEAREST_CLAMP, v_uv);
#endif
#ifdef ALPHA_CUTOUT
    if (color.a < ALPHA_CUTOUT) {
        discard;
    }
#endif
#ifdef PER_FACE
    color *= gl_FrontFacing ? v_front : v_back;
#else
    color *= v_front;
#endif
    color.rgb += v_added;
    if (v_glint_strength > 0.0) {
        // Its picture has one level: read without the slopes a discard leaves undefined.
        vec3 glint = v_glint_strength * textureLod(
            sampler2D(u_textures[nonuniformEXT(v_glint_texture)], u_samplers[SAMPLER_LINEAR_REPEAT]),
            v_glint_uv, 0.0).rgb;
        color.rgb += glint * glint;
        color.a = max(color.a, v_glint_strength);
    }
    float amount = max(
        fog_amount(v_spherical, pc.fog_environmental),
        fog_amount(v_cylindrical, pc.fog_render_distance));
    color.rgb = mix(color.rgb, pc.fog_color.rgb, amount * pc.fog_color.a);
    o_color = color;
}

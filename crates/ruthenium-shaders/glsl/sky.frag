#version 460
// Ruthenium: the sky's three kinds of draw, as vanilla's sky, position_color and
// position_tex shaders colour them.

#include "push.glsl"
#include "textures.glsl"

#define SKY_FOGGED 0u
#define SKY_COLOURED 1u
#define SKY_TEXTURED 2u

// What a draw's fragments share comes in push constants, after the two words the
// vertex stage reads.
layout(push_constant) uniform Push {
    uint sky;
    uint enabled;
    // 0: the colour, fogged. 1: the vertex's colour times the colour. 2: the texture
    // times both.
    uint kind;
    // The texture of a textured draw, an index into the texture table.
    uint texture;
    // What the draw's colour is multiplied by, or the colour itself.
    vec4 color;
    // The fog's colour, and where the sky's fog is complete, in blocks of the draw's
    // own space: for the kind that is fogged.
    vec4 fog_color;
    float fog_end;
    // The sampler the texture is read with.
    uint sampler_id;
    // One over fog_end, as fog.glsl's scale (atmosphere.rs, fog_line).
    float fog_scale;
} pc;

layout(location = 0) in vec4 v_color;
layout(location = 1) in vec2 v_uv;
layout(location = 2) in float v_spherical;
layout(location = 3) in float v_cylindrical;
layout(location = 0) out vec4 o_color;

void main() {
    vec4 color = pc.color;
    uint kind = pc.kind;
    if (kind == SKY_FOGGED) {
        // The sky's dome fades into the fog's colour towards its rim, and is all fog
        // beyond where the sky's fog ends, sideways or in height.
        vec4 fog = pc.fog_color;
        float towards = clamp(v_spherical * pc.fog_scale, 0.0, 1.0);
        float beyond = v_cylindrical <= pc.fog_end ? 0.0 : 1.0;
        o_color = vec4(mix(color.rgb, fog.rgb, max(towards, beyond) * fog.a), color.a);
        return;
    }
    vec4 own = v_color;
    if (kind == SKY_TEXTURED) {
        own *= sample_texture(pc.texture, pc.sampler_id, v_uv);
    }
    if (own.a == 0.0) {
        discard;
    }
    o_color = own * color;
}

#version 460
// Ruthenium: GUI quads. As vanilla's GUI pipelines: texture times vertex colour, and
// fully transparent fragments are discarded.

#include "textures.glsl"

#define GUI_NO_TEXTURE 0xFFFFFFFFu

layout(location = 0) in vec2 v_uv;
layout(location = 1) in vec4 v_color;
layout(location = 2) flat in uint v_texture;
layout(location = 3) flat in uint v_sampler;
layout(location = 0) out vec4 o_color;

void main() {
    vec4 color = v_color;
    if (v_texture != GUI_NO_TEXTURE) {
        color *= sample_texture(v_texture, v_sampler, v_uv);
    }
    if (color.a == 0.0) {
        discard;
    }
    o_color = color;
}

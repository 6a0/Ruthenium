#version 460
// Ruthenium: glyphs from a multi-channel signed distance field: the median of the three
// channels is the distance (0.5 on the outline), scaled to screen pixels for a one-pixel
// anti-aliased edge at any size.

#include "push.glsl"
#include "textures.glsl"
#include "clip.glsl"

layout(push_constant) uniform Push {
    vec2 screen;
    uint atlas;
    uint first;
} pc;

layout(location = 0) in vec2 v_uv;
layout(location = 1) in vec4 v_color;
layout(location = 2) in float v_range;
layout(location = 3) flat in vec4 v_clip;
layout(location = 4) flat in vec4 v_fade;
layout(location = 5) flat in float v_opacity;
layout(location = 0) out vec4 o_color;

float median(vec3 c) {
    return max(min(c.r, c.g), min(max(c.r, c.g), c.b));
}

void main() {
    vec3 field = sample_texture(pc.atlas, SAMPLER_LINEAR_CLAMP, v_uv).rgb;
    float distance = (median(field) - 0.5) * max(v_range, 1.0);
    float coverage = clamp(distance + 0.5, 0.0, 1.0) * clip_coverage(v_clip, v_fade) * v_opacity;
    if (coverage <= 0.0) {
        discard;
    }
    o_color = vec4(v_color.rgb, v_color.a * coverage);
}

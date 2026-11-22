#version 460
// Ruthenium: the frosted-glass backdrop. Drawn with background.vert's full-screen
// triangle into a quarter-size target: mode 0 averages a 4x4 block of the source (four
// bilinear taps), mode 1 is a 9-tap Gaussian along `dir` (five bilinear taps). Mode 2
// copies a same-size source texel for texel (the backdrop into the window).

#include "push.glsl"
#include "textures.glsl"

layout(location = 0) in vec2 v_uv;
layout(location = 0) out vec4 o_color;

layout(push_constant) uniform Push {
    vec2 texel; // one source texel in UV
    vec2 dir;   // (1, 0) or (0, 1) for mode 1
    uint src;
    uint mode;
} pc;

vec4 tap(vec2 uv) {
    return sample_texture(pc.src, SAMPLER_LINEAR_CLAMP, uv);
}

void main() {
    if (pc.mode == 0u) {
        vec2 o = pc.texel;
        o_color = 0.25 * (tap(v_uv + vec2(-o.x, -o.y)) + tap(v_uv + vec2(o.x, -o.y))
                        + tap(v_uv + vec2(-o.x, o.y)) + tap(v_uv + vec2(o.x, o.y)));
    } else if (pc.mode == 2u) {
        o_color = sample_texture(pc.src, SAMPLER_NEAREST_CLAMP, v_uv);
    } else {
        vec2 step = pc.dir * pc.texel;
        vec4 c = tap(v_uv) * 0.2270270270;
        c += (tap(v_uv + step * 1.3846153846) + tap(v_uv - step * 1.3846153846)) * 0.3162162162;
        c += (tap(v_uv + step * 3.2307692308) + tap(v_uv - step * 3.2307692308)) * 0.0702702703;
        o_color = c;
    }
}

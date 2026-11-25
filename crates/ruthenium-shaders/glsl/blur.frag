#version 460
// Ruthenium: the frosted-glass backdrop. Drawn with background.vert's full-screen
// triangle: mode 0 averages a 4x4 block of the source (four bilinear taps) into a
// quarter-size target, mode 1 is a 9-tap Gaussian along `dir` (five bilinear taps). Mode 2
// copies a same-size source texel for texel (the backdrop into the window). Mode 3 is the
// world behind a menu: blurred over a disk `dir.x` pixels wide and graded. Mode 4 lays the
// menu's layer, drawn over nothing, over it: `amount` of the blurred one (`second`) in
// place of the sharp one (`src`), as opaque as `dir.x`.

#include "push.glsl"
#include "textures.glsl"
#include "grade.glsl"

layout(location = 0) in vec2 v_uv;
layout(location = 0) out vec4 o_color;

layout(push_constant) uniform Push {
    vec2 texel; // one source texel in UV
    vec2 dir;   // mode 1: (step, 0) or (0, step); modes 3 and 4: see there
    uint src;
    uint mode;
    vec2 grade; // saturation, brightness
    uint second;
    float amount;
} pc;

vec4 tap(vec2 uv) {
    return sample_texture(pc.src, SAMPLER_LINEAR_CLAMP, uv);
}

// Taps of the disk: evenly over its area, each a golden angle on from the last.
const int DISK = 16;
const float GOLDEN = 2.39996323;

void main() {
    if (pc.mode == 0u) {
        vec2 o = pc.texel;
        o_color = 0.25 * (tap(v_uv + vec2(-o.x, -o.y)) + tap(v_uv + vec2(o.x, -o.y))
                        + tap(v_uv + vec2(-o.x, o.y)) + tap(v_uv + vec2(o.x, o.y)));
    } else if (pc.mode == 2u) {
        o_color = sample_texture(pc.src, SAMPLER_NEAREST_CLAMP, v_uv);
    } else if (pc.mode == 3u) {
        vec4 c = sample_texture(pc.src, SAMPLER_NEAREST_CLAMP, v_uv);
        if (pc.dir.x > 0.0) {
            for (int i = 0; i < DISK; i++) {
                float r = sqrt((float(i) + 0.5) / float(DISK)) * pc.dir.x;
                float a = float(i) * GOLDEN;
                c += tap(v_uv + vec2(cos(a), sin(a)) * r * pc.texel);
            }
            c /= float(DISK + 1);
        }
        o_color = vec4(graded(c.rgb, pc.grade), c.a);
    } else if (pc.mode == 4u) {
        vec4 sharp = sample_texture(pc.src, SAMPLER_NEAREST_CLAMP, v_uv);
        vec4 soft = sample_texture(pc.second, SAMPLER_LINEAR_CLAMP, v_uv);
        o_color = mix(sharp, soft, pc.amount) * pc.dir.x;
    } else {
        vec2 step = pc.dir * pc.texel;
        vec4 c = tap(v_uv) * 0.2270270270;
        c += (tap(v_uv + step * 1.3846153846) + tap(v_uv - step * 1.3846153846)) * 0.3162162162;
        c += (tap(v_uv + step * 3.2307692308) + tap(v_uv - step * 3.2307692308)) * 0.0702702703;
        o_color = c;
    }
}

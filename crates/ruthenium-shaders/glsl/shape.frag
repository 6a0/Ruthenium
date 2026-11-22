#version 460
// Ruthenium: rounded shapes with anti-aliased edges. Flat shapes are filled; glass
// shapes (the "Liquid Glass" pills) show the blurred backdrop through a tint, bent
// inwards near the rim like a thick lens, with a light rim from the top left; image
// shapes show a texture (the logo).

#include "push.glsl"
#include "textures.glsl"

struct Shape {
    vec4 rect;
    vec4 fill;
    vec4 params;
    vec4 uv;
    uvec4 tex;
};
#define FRAME_ELEMENT Shape
#include "frame.glsl"

layout(push_constant) uniform Push {
    vec2 screen;
    uint scene;
    uint blurred;
    uint first;
    uint backdrop;
} pc;

layout(location = 0) flat in uint v_index;
layout(location = 0) out vec4 o_color;

// Signed distance to a rounded rectangle centred at the origin (negative inside).
float rounded_rect(vec2 p, vec2 half_size, float r) {
    vec2 q = abs(p) - half_size + r;
    return length(max(q, 0.0)) + min(max(q.x, q.y), 0.0) - r;
}

void main() {
    Shape s = frame_data[v_index];
    vec2 p = gl_FragCoord.xy;
    vec2 centre = 0.5 * (s.rect.xy + s.rect.zw);
    vec2 half_size = 0.5 * (s.rect.zw - s.rect.xy);
    float radius = min(s.params.x, min(half_size.x, half_size.y));
    float d = rounded_rect(p - centre, half_size, radius);
    float coverage = clamp(0.5 - d, 0.0, 1.0);
    if (coverage <= 0.0) {
        discard;
    }
    if (s.params.y == 2.0) {
        // An image in premultiplied alpha, mip-mapped: back to straight alpha for blending.
        vec2 size = s.rect.zw - s.rect.xy;
        vec2 t = clamp((p - s.rect.xy) / size, 0.0, 1.0);
        // UV change per pixel, exact for an axis-aligned rectangle.
        vec2 per_px = (s.uv.zw - s.uv.xy) / size;
        vec4 c = sample_texture_grad(s.tex.x, SAMPLER_LINEAR_CLAMP, mix(s.uv.xy, s.uv.zw, t),
                                     vec2(per_px.x, 0.0), vec2(0.0, per_px.y)) * s.fill;
        if (c.a <= 0.0) {
            discard;
        }
        o_color = vec4(c.rgb / c.a, c.a * coverage);
        return;
    }
    if (s.params.y == 0.0 || pc.backdrop == 0u) {
        // Flat (or glass before the backdrop exists): plain fill.
        float a = s.fill.a * coverage;
        if (s.params.y != 0.0) {
            a = max(a, 0.18 * coverage);
        }
        o_color = vec4(s.fill.rgb, a);
        return;
    }

    // The outward normal, from the distance field's gradient.
    vec2 e = vec2(0.5, 0.0);
    vec2 n = normalize(vec2(
        rounded_rect(p - centre + e.xy, half_size, radius) - rounded_rect(p - centre - e.xy, half_size, radius),
        rounded_rect(p - centre + e.yx, half_size, radius) - rounded_rect(p - centre - e.yx, half_size, radius)
    ) + 1e-6);
    float inside = max(-d, 0.0);
    // 1 at the rim, falling to 0 a bevel's width in.
    float bevel = max(radius, 1.0);
    float rim = 1.0 - smoothstep(0.0, bevel, inside);

    // Lensing: near the rim the backdrop is pulled from further inside the shape.
    vec2 offset = -n * s.params.w * rim * rim;
    vec2 uv = (p + offset) / pc.screen;
    vec3 frost = sample_texture(pc.blurred, SAMPLER_LINEAR_CLAMP, uv).rgb;
    vec3 sharp = sample_texture(pc.scene, SAMPLER_LINEAR_CLAMP, uv).rgb;
    // Mostly frosted, a little of the sharp scene at the rim where the glass is thin.
    vec3 c = mix(frost, sharp, 0.35 * rim);
    // Glass lifts what is behind it slightly, then the tint.
    c = c * 1.03 + 0.012;
    c = mix(c, s.fill.rgb, s.fill.a);

    // Light from the top left: a bright rim facing it, a dimmer one opposite.
    vec2 light = normalize(vec2(-0.55, -0.85));
    float facing = dot(n, light);
    float edge = 1.0 - smoothstep(0.0, 2.5, inside);
    float sheen = rim * rim * (max(facing, 0.0) * 0.55 + max(-facing, 0.0) * 0.2);
    c += s.params.z * (sheen * 0.6 + edge * (0.18 + 0.32 * max(facing, 0.0)));

    o_color = vec4(min(c, vec3(1.0)), coverage);
}

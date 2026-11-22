#version 460
// Ruthenium: the menu background: a vertical gradient, a soft procedural nebula, and a
// tiled starfield texture (baked at start-up) added on top, one texel per pixel. The
// menus draw it once into their backdrop image, so the nebula's cost is not per frame.

#include "push.glsl"
#include "textures.glsl"

layout(location = 0) in vec2 v_uv;
layout(location = 0) out vec4 o_color;

layout(push_constant) uniform Push {
    vec4 top;
    vec4 bottom;
    uint stars;
} pc;

// An integer hash of a lattice point, 0..1 (exact for any coordinates, unlike float
// hashes, which break into seams far from the origin).
float hash(ivec2 i) {
    uvec2 q = uvec2(i) * uvec2(1597334673u, 3812015801u);
    uint n = (q.x ^ q.y) * 1597334673u;
    n ^= n >> 16;
    n *= 2246822519u;
    n ^= n >> 13;
    return float(n) * (1.0 / 4294967296.0);
}

// Value noise with a smooth fade, 0..1.
float noise(vec2 p) {
    vec2 fl = floor(p);
    ivec2 i = ivec2(fl);
    vec2 f = p - fl;
    vec2 u = f * f * (3.0 - 2.0 * f);
    return mix(mix(hash(i), hash(i + ivec2(1, 0)), u.x),
               mix(hash(i + ivec2(0, 1)), hash(i + ivec2(1, 1)), u.x), u.y);
}

float fbm(vec2 p) {
    float v = 0.0;
    float a = 0.5;
    mat2 turn = mat2(0.8, -0.6, 0.6, 0.8);
    for (int i = 0; i < 6; i++) {
        v += a * noise(p);
        p = turn * p * 2.03 + 17.0;
        a *= 0.5;
    }
    return v;
}

void main() {
    vec4 sky = mix(pc.top, pc.bottom, v_uv.y);
    // Resolution-independent coordinates: the height is 1.
    float aspect = abs(dFdy(v_uv.y)) / max(abs(dFdx(v_uv.x)), 1e-6);
    vec2 p = vec2(v_uv.x * aspect, v_uv.y) * 2.2;

    // Domain-warped fbm gives wispy clouds; two colour layers, brightest off-centre.
    vec2 warp = vec2(fbm(p + vec2(1.7, 9.2)), fbm(p + vec2(8.3, 2.8)));
    float cloud = fbm(p + 1.6 * warp);
    float dust = fbm(p * 1.7 - 2.4 * warp + 5.0);
    vec2 glow_at = vec2(0.72 * aspect, 0.38);
    float glow = exp(-2.2 * dot(vec2(v_uv.x * aspect, v_uv.y) - glow_at, vec2(v_uv.x * aspect, v_uv.y) - glow_at));
    float body = smoothstep(0.35, 0.85, cloud) * (0.35 + 0.65 * glow);
    vec3 violet = vec3(0.32, 0.12, 0.55);
    vec3 blue = vec3(0.08, 0.22, 0.55);
    vec3 rose = vec3(0.60, 0.16, 0.38);
    vec3 neb = mix(blue, violet, smoothstep(0.3, 0.7, warp.x)) * body;
    neb += rose * smoothstep(0.55, 0.9, dust) * glow * 0.6;
    // Dark lanes cut through the cloud.
    neb *= 0.55 + 0.45 * smoothstep(0.25, 0.6, dust);

    vec2 uv = gl_FragCoord.xy / vec2(texture_size(pc.stars));
    // Stars are stored premultiplied by their brightness: add them.
    vec3 star = sample_texture(pc.stars, SAMPLER_NEAREST_REPEAT, uv).rgb;
    o_color = vec4(min(sky.rgb + 0.55 * neb + star, vec3(1.0)), sky.a);
}

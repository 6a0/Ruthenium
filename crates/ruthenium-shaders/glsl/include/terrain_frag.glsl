// Ruthenium: terrain's fragment stage, shared by every terrain pipeline. Before
// including it a shader defines how the atlas is filtered (TERRAIN_RGSS, or
// TERRAIN_RGSS_REFERENCE, or neither: nearest), ALPHA_CUTOUT for the cutout layer, and
// TERRAIN_PLAIN to leave the fog and the fade out.

#include "push.glsl"
#include "textures.glsl"

// What the fragments of a frame share comes in push constants, after the four words
// the vertex stage reads: read from the frame's data for every pixel, the same few
// values cost a fifth of a frame's GPU time.
layout(push_constant) uniform Push {
    uint world;
    uint matrix;
    uint sections;
    uint enabled;
    // The fog's colour; its alpha scales how much of it there is.
    vec4 fog_color;
    // Where the fog of the place starts and where it is complete, in blocks from the
    // camera in a straight line; and the same for the fog at the edge of the view, in
    // blocks sideways or in height, whichever is more.
    vec2 fog_environmental;
    vec2 fog_render_distance;
    // One texel of the block atlas in its coordinates, and the atlas's index in the
    // texture table.
    vec2 atlas_texel;
    uint atlas;
} pc;

layout(location = 0) in vec4 v_color;
layout(location = 1) in vec2 v_uv;
#ifndef TERRAIN_PLAIN
layout(location = 2) in float v_spherical;
layout(location = 3) in float v_cylindrical;
layout(location = 4) in float v_fade;
#endif
layout(location = 0) out vec4 o_color;

// The atlas sampled as vanilla's sampleNearest does it: every texel keeps hard edges,
// but where two texels meet the colour goes from one to the other over the width of one
// pixel of the screen instead of at once. The bilinear sampler does the blending; the
// coordinates are moved to a texel's centre everywhere but in that one-pixel band.
// Explicit derivatives choose the mip level, as the moved coordinates have none of
// their own worth using. `pixel` is how much of the texture one pixel of the screen
// covers along each axis.
vec4 sample_nearest(uint atlas, vec2 uv, vec2 texel, vec2 du, vec2 dv, vec2 pixel) {
    vec2 at = uv / texel;
    // The nearest border between two texels, and how far past the centre of the texel
    // before it the sample is (0 to 1: one texel centre to the next).
    vec2 border = round(at) - 0.5;
    vec2 along = at - border;
    along = clamp((along - 0.5) * texel / pixel + 0.5, 0.0, 1.0);
    return sample_texture_grad(atlas, SAMPLER_LINEAR_CLAMP, (border + along) * texel, du, dv);
}

#if defined(TERRAIN_RGSS) || defined(TERRAIN_RGSS_REFERENCE)
// Where a pixel is sampled four times for vanilla's sampleRGSS, in texels from its
// middle: a square turned so that no two samples share a row or a column.
const vec2 RGSS_OFFSETS[4] = vec2[](
    vec2(0.125, 0.375),
    vec2(-0.125, -0.375),
    vec2(0.375, -0.125),
    vec2(-0.375, 0.125));

// The mean of the four samples at one mip level.
vec4 sample_four(uint atlas, vec2 uv, vec2 texel, float level) {
    vec4 sum = vec4(0.0);
    for (int i = 0; i < 4; ++i) {
        sum += textureLod(
            sampler2D(u_textures[nonuniformEXT(atlas)], u_samplers[SAMPLER_LINEAR_CLAMP]),
            uv + RGSS_OFFSETS[i] * texel, level);
    }
    return sum * 0.25;
}
#endif

// The atlas's colour at `uv`; `texel` is one texel's size in coordinates.
vec4 sample_atlas(uint atlas, vec2 uv, vec2 texel) {
    // Derivatives first: after a branch they are not defined.
    vec2 du = dFdx(uv);
    vec2 dv = dFdy(uv);
    vec2 pixel = sqrt(du * du + dv * dv);
#if defined(TERRAIN_RGSS) || defined(TERRAIN_RGSS_REFERENCE)
    // Vanilla's sampleRGSS: while a texel is larger than a pixel, the nearest sample
    // above; once a pixel covers two texels, four samples at each of the two mip levels
    // around the one the pixel's size asks for; and between, both, blended.
    float texel_size = min(texel.x, texel.y);
    float blend = smoothstep(texel_size, texel_size * 2.0, max(pixel.x, pixel.y));
#ifndef TERRAIN_RGSS_REFERENCE
    // The same result as below with the samples left out whose weight is exactly
    // nothing: one sample where texels are magnified, which is most of what is near.
    if (blend <= 0.0) {
        return sample_nearest(atlas, uv, texel, du, dv, pixel);
    }
#endif
    float du_length = length(du);
    float dv_length = length(dv);
    float derivative = sqrt(min(du_length, dv_length) * max(du_length, dv_length));
    float level = max(0.0, log2(derivative / texel_size));
    float low = floor(level);
    float between = fract(level);
#ifdef TERRAIN_RGSS_REFERENCE
    // All nine samples, always: what the shorter way is measured against.
    vec4 far = mix(sample_four(atlas, uv, texel, low), sample_four(atlas, uv, texel, low + 1.0), between);
    return mix(sample_nearest(atlas, uv, texel, du, dv, pixel), far, blend);
#else
    vec4 far = sample_four(atlas, uv, texel, low);
    if (between > 0.0) {
        far = mix(far, sample_four(atlas, uv, texel, low + 1.0), between);
    }
    if (blend >= 1.0) {
        return far;
    }
    return mix(sample_nearest(atlas, uv, texel, du, dv, pixel), far, blend);
#endif
#else
    return sample_nearest(atlas, uv, texel, du, dv, pixel);
#endif
}

#ifndef TERRAIN_PLAIN
// How much fog there is at a distance, from none at `range.x` to all at `range.y`.
float fog_amount(float distance, vec2 range) {
    if (distance <= range.x) {
        return 0.0;
    }
    if (distance >= range.y) {
        return 1.0;
    }
    return (distance - range.x) / (range.y - range.x);
}
#endif

void main() {
    vec4 color = sample_atlas(pc.atlas, v_uv, pc.atlas_texel) * v_color;
#ifndef TERRAIN_PLAIN
    vec4 fog = pc.fog_color;
    // A section that has just appeared comes out of the fog's colour, with the holes
    // of its texels kept.
    color = mix(fog * vec4(1.0, 1.0, 1.0, color.a), color, v_fade);
#endif
#ifdef ALPHA_CUTOUT
    if (color.a < ALPHA_CUTOUT) {
        discard;
    }
#endif
#ifndef TERRAIN_PLAIN
    // The stronger of the place's fog and the fog at the edge of the view.
    float amount = max(
        fog_amount(v_spherical, pc.fog_environmental),
        fog_amount(v_cylindrical, pc.fog_render_distance));
    color.rgb = mix(color.rgb, fog.rgb, amount * fog.a);
#endif
    o_color = color;
}

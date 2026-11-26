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
    // The fog of the place, by the distance from the camera in a straight line, and the
    // fog at the edge of the view, by the distance sideways or in height, whichever is
    // more: each as fog.glsl's (scale, bias).
    vec2 fog_environmental;
    vec2 fog_render_distance;
    // One texel of the block atlas in its coordinates, and the atlas's size in texels
    // (one over that).
    vec2 atlas_texel;
    vec2 atlas_size;
    // The atlas's index in the texture table.
    uint atlas;
    // One over the shorter side of a texel (the atlas's larger size), squared, and its
    // base-2 logarithm: the pixel's reach and the mip level are measured in it.
    float texel_scale_squared;
    float texel_scale_log2;
} pc;

layout(location = 0) in vec4 v_color;
layout(location = 1) in vec2 v_uv;
#ifndef TERRAIN_PLAIN
layout(location = 2) in float v_spherical;
layout(location = 3) in float v_cylindrical;
layout(location = 4) in float v_fade;
#endif
layout(location = 0) out vec4 o_color;

#ifdef TERRAIN_RGSS_REFERENCE
// Vanilla's sampleNearest and sampleRGSS as they are written, every sample taken and
// every square root and division of theirs: what the shorter way below is measured
// against (--terrain-shader rgss-reference).

// The atlas sampled as vanilla's sampleNearest does it: every texel keeps hard edges,
// but where two texels meet the colour goes from one to the other over the width of one
// pixel of the screen instead of at once. The bilinear sampler does the blending; the
// coordinates are moved to a texel's centre everywhere but in that one-pixel band.
// Explicit derivatives choose the mip level, as the moved coordinates have none of
// their own worth using. `pixel` is how much of the texture one pixel of the screen
// covers along each axis.
vec4 sample_nearest(vec2 uv, vec2 texel, vec2 du, vec2 dv, vec2 pixel) {
    vec2 at = uv / texel;
    // The nearest border between two texels, and how far past the centre of the texel
    // before it the sample is (0 to 1: one texel centre to the next).
    vec2 border = round(at) - 0.5;
    vec2 along = at - border;
    along = clamp((along - 0.5) * texel / pixel + 0.5, 0.0, 1.0);
    return sample_texture_grad(pc.atlas, SAMPLER_LINEAR_CLAMP, (border + along) * texel, du, dv);
}
#else
// The atlas sampled as vanilla's sampleNearest does it: every texel keeps hard edges,
// but where two texels meet the colour goes from one to the other over the width of one
// pixel of the screen instead of at once. The bilinear sampler does the blending; the
// coordinates are moved to a texel's centre everywhere but in that one-pixel band.
// Explicit derivatives choose the mip level, as the moved coordinates have none of
// their own worth using. `footprint` is the square of how much of the texture one
// pixel of the screen covers along each axis.
//
// Vanilla's arithmetic with its divisions turned into multiplications by what the push
// constants hold and its square root into an inverse one: the atlas's sizes are powers
// of two, so the coordinates in texels are the same numbers.
vec4 sample_nearest(vec2 uv, vec2 du, vec2 dv, vec2 footprint) {
    vec2 at = uv * pc.atlas_size;
    // The nearest border between two texels, and how far past the centre of the texel
    // before it the sample is (0 to 1: one texel centre to the next).
    vec2 border = round(at) - 0.5;
    vec2 along = at - border;
    // How many pixels one texel is wide, along each axis.
    vec2 texel_pixels = pc.atlas_texel * inversesqrt(footprint);
    along = clamp((along - 0.5) * texel_pixels + 0.5, 0.0, 1.0);
    return sample_texture_grad(pc.atlas, SAMPLER_LINEAR_CLAMP, (border + along) * pc.atlas_texel, du, dv);
}
#endif

#if defined(TERRAIN_RGSS) || defined(TERRAIN_RGSS_REFERENCE)
// Where a pixel is sampled four times for vanilla's sampleRGSS, in texels from its
// middle: a square turned so that no two samples share a row or a column.
const vec2 RGSS_OFFSETS[4] = vec2[](
    vec2(0.125, 0.375),
    vec2(-0.125, -0.375),
    vec2(0.375, -0.125),
    vec2(-0.375, 0.125));

// The mean of the four samples at one mip level.
vec4 sample_four(vec2 uv, float level) {
    vec4 sum = vec4(0.0);
    for (int i = 0; i < 4; ++i) {
        sum += textureLod(
            sampler2D(u_textures[nonuniformEXT(pc.atlas)], u_samplers[SAMPLER_LINEAR_CLAMP]),
            uv + RGSS_OFFSETS[i] * pc.atlas_texel, level);
    }
    return sum * 0.25;
}
#endif

// The atlas's colour at `uv`.
vec4 sample_atlas(vec2 uv) {
    // Derivatives first: after a branch they are not defined.
    vec2 du = dFdx(uv);
    vec2 dv = dFdy(uv);
#ifdef TERRAIN_RGSS_REFERENCE
    // Vanilla's sampleRGSS: while a texel is larger than a pixel, the nearest sample
    // above; once a pixel covers two texels, four samples at each of the two mip levels
    // around the one the pixel's size asks for; and between, both, blended.
    vec2 texel = pc.atlas_texel;
    vec2 pixel = sqrt(du * du + dv * dv);
    float texel_size = min(texel.x, texel.y);
    float blend = smoothstep(texel_size, texel_size * 2.0, max(pixel.x, pixel.y));
    float du_length = length(du);
    float dv_length = length(dv);
    float derivative = sqrt(min(du_length, dv_length) * max(du_length, dv_length));
    float level = max(0.0, log2(derivative / texel_size));
    float low = floor(level);
    float between = fract(level);
    vec4 far = mix(sample_four(uv, low), sample_four(uv, low + 1.0), between);
    return mix(sample_nearest(uv, texel, du, dv, pixel), far, blend);
#else
    vec2 du2 = du * du;
    vec2 dv2 = dv * dv;
    vec2 footprint = du2 + dv2;
#ifdef TERRAIN_RGSS
    // Vanilla's sampleRGSS, leaving out the samples whose weight is exactly nothing:
    // while a texel is larger than a pixel, the nearest sample above, which is most of
    // what is near; once a pixel covers two texels, four samples at each of the two mip
    // levels around the one the pixel's size asks for; and between, both, blended.
    //
    // How far the pixel reaches along its longer axis, in the shorter side of a texel,
    // squared: vanilla's blend is nothing up to 1 and full from 2, so the square root
    // is taken only between.
    float reach = max(footprint.x, footprint.y) * pc.texel_scale_squared;
    if (reach <= 1.0) {
        return sample_nearest(uv, du, dv, footprint);
    }
    // The mip level: the logarithm of the geometric mean of the two derivatives'
    // lengths in texels, which is a quarter of the logarithm of the product of their
    // squares.
    float level = max(0.0, 0.25 * log2((du2.x + du2.y) * (dv2.x + dv2.y)) + pc.texel_scale_log2);
    float low = floor(level);
    float between = fract(level);
    vec4 far = sample_four(uv, low);
    if (between > 0.0) {
        far = mix(far, sample_four(uv, low + 1.0), between);
    }
    if (reach >= 4.0) {
        return far;
    }
    // smoothstep from one texel to two.
    float t = sqrt(reach) - 1.0;
    float blend = t * t * (3.0 - 2.0 * t);
    return mix(sample_nearest(uv, du, dv, footprint), far, blend);
#else
    return sample_nearest(uv, du, dv, footprint);
#endif
#endif
}

#ifndef TERRAIN_PLAIN
#include "fog.glsl"
#endif

void main() {
    vec4 color = sample_atlas(v_uv) * v_color;
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

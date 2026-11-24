// Ruthenium: terrain's fragment stage, shared by the solid and the cutout pipeline
// (which defines ALPHA_CUTOUT before including this).

#include "push.glsl"
#include "textures.glsl"
#include "world.glsl"

layout(push_constant) uniform Push {
    uint world;
    uint matrix;
    uint sections;
    uint enabled;
} pc;

layout(location = 0) in vec4 v_color;
layout(location = 1) in vec2 v_uv;
layout(location = 0) out vec4 o_color;

// The atlas sampled as vanilla's sampleNearest does it: every texel keeps hard edges,
// but where two texels meet the colour goes from one to the other over the width of one
// pixel of the screen instead of at once. The bilinear sampler does the blending; the
// coordinates are moved to a texel's centre everywhere but in that one-pixel band.
// Explicit derivatives choose the mip level, as the moved coordinates have none of
// their own worth using.
vec4 sample_atlas(uint atlas, vec2 uv, vec2 texel) {
    vec2 du = dFdx(uv);
    vec2 dv = dFdy(uv);
    // How much of the texture one pixel of the screen covers, along each axis.
    vec2 pixel = sqrt(du * du + dv * dv);
    vec2 at = uv / texel;
    // The nearest border between two texels, and how far past the centre of the texel
    // before it the sample is (0 to 1: one texel centre to the next).
    vec2 border = round(at) - 0.5;
    vec2 along = at - border;
    along = clamp((along - 0.5) * texel / pixel + 0.5, 0.0, 1.0);
    return sample_texture_grad(atlas, SAMPLER_LINEAR_CLAMP, (border + along) * texel, du, dv);
}

void main() {
    vec4 color = sample_atlas(world_atlas(pc.world), v_uv, 1.0 / world_atlas_size(pc.world)) * v_color;
#ifdef ALPHA_CUTOUT
    if (color.a < ALPHA_CUTOUT) {
        discard;
    }
#endif
    o_color = color;
}
